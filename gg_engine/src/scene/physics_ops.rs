use super::physics_2d::PhysicsWorld2D;
use super::{
    BoxCollider2DComponent, CircleCollider2DComponent, Entity, IdComponent, RelationshipComponent,
    RigidBody2DComponent, Scene, TransformComponent,
};
use rapier2d::na;

/// Clamp a physics property to a minimum, logging a warning if it was invalid.
fn validate_physics_value(value: f32, min: f32, name: &str, entity_uuid: u64) -> f32 {
    if value < min {
        log::warn!(
            "Entity {}: negative {} ({:.3}), clamped to {}",
            entity_uuid,
            name,
            value,
            min
        );
        min
    } else {
        value
    }
}

impl Scene {
    // -----------------------------------------------------------------
    // Physics (shared helpers)
    // -----------------------------------------------------------------

    /// Create the physics world and populate it with rigid bodies / colliders
    /// from all entities that have physics components.
    ///
    /// Shared by both runtime and simulation start paths.
    pub(super) fn on_physics_2d_start(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_physics_2d_start");
        let mut physics = PhysicsWorld2D::new(0.0, -9.81);

        // Snapshot entities with RigidBody2DComponent to avoid borrow conflicts.
        // Skip parented entities — physics bodies ignore parent transforms, so
        // allowing them would cause confusing mismatches between visual and physics position.
        let body_entities: Vec<(hecs::Entity, u64, glam::Vec3, glam::Vec3, glam::Vec3, super::RigidBody2DType, bool)> = self
            .world
            .query::<(
                hecs::Entity,
                &IdComponent,
                &TransformComponent,
                &RigidBody2DComponent,
                &RelationshipComponent,
            )>()
            .iter()
            .filter_map(|(handle, id, transform, rb, rel)| {
                if rel.parent.is_some() {
                    log::warn!(
                        "Entity UUID {} has RigidBody2D but is parented — skipping physics body creation. \
                         Detach from parent or remove the RigidBody2D component.",
                        id.id.raw(),
                    );
                    return None;
                }
                Some((
                    handle,
                    id.id.raw(),
                    transform.translation,
                    transform.rotation,
                    transform.scale,
                    rb.body_type,
                    rb.fixed_rotation,
                ))
            })
            .collect();

        for (handle, entity_uuid, translation, rotation, scale, body_type, fixed_rotation) in
            body_entities
        {
            // Create rapier rigid body.
            let mut body_builder = rapier2d::dynamics::RigidBodyBuilder::new(body_type.to_rapier())
                .translation(na::Vector2::new(translation.x, translation.y))
                .rotation(rotation.z);

            if fixed_rotation {
                body_builder = body_builder.lock_rotations();
            }

            let body_handle = physics.bodies.insert(body_builder.build());

            // Store the handle back on the component.
            if let Ok(mut rb) = self.world.get::<&mut RigidBody2DComponent>(handle) {
                rb.runtime_body = Some(body_handle);
            }

            // If entity also has a BoxCollider2DComponent, create a collider.
            if let Ok(mut bc) = self.world.get::<&mut BoxCollider2DComponent>(handle) {
                let half_x = bc.size.x * scale.x.abs();
                let half_y = bc.size.y * scale.y.abs();

                if half_x <= 0.0 || half_y <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-size box collider ({} x {}), skipping",
                        entity_uuid,
                        half_x * 2.0,
                        half_y * 2.0
                    );
                } else {
                    let density = validate_physics_value(bc.density, 0.0, "density", entity_uuid);
                    let friction =
                        validate_physics_value(bc.friction, 0.0, "friction", entity_uuid);
                    let restitution =
                        validate_physics_value(bc.restitution, 0.0, "restitution", entity_uuid);

                    let mut builder = rapier2d::geometry::ColliderBuilder::cuboid(half_x, half_y)
                        .density(density)
                        .friction(friction)
                        .restitution(restitution)
                        .translation(na::Vector2::new(
                            bc.offset.x * scale.x.abs(),
                            bc.offset.y * scale.y.abs(),
                        ))
                        .collision_groups(rapier2d::geometry::InteractionGroups::new(
                            bc.collision_layer.into(),
                            bc.collision_mask.into(),
                        ))
                        .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS);
                    // When friction is 0, use Min combine rule so the zero
                    // wins against any surface (prevents wall sticking).
                    if friction == 0.0 {
                        builder = builder
                            .friction_combine_rule(rapier2d::prelude::CoefficientCombineRule::Min);
                    }
                    let collider = builder.build();

                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    bc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, entity_uuid);
                }
            }

            // If entity also has a CircleCollider2DComponent, create a collider.
            if let Ok(mut cc) = self.world.get::<&mut CircleCollider2DComponent>(handle) {
                let scaled_radius = cc.radius * scale.x.abs().max(scale.y.abs());

                if scaled_radius <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-radius circle collider, skipping",
                        entity_uuid
                    );
                } else {
                    let density = validate_physics_value(cc.density, 0.0, "density", entity_uuid);
                    let friction =
                        validate_physics_value(cc.friction, 0.0, "friction", entity_uuid);
                    let restitution =
                        validate_physics_value(cc.restitution, 0.0, "restitution", entity_uuid);

                    let mut builder = rapier2d::geometry::ColliderBuilder::ball(scaled_radius)
                        .density(density)
                        .friction(friction)
                        .restitution(restitution)
                        .translation(na::Vector2::new(
                            cc.offset.x * scale.x.abs(),
                            cc.offset.y * scale.y.abs(),
                        ))
                        .collision_groups(rapier2d::geometry::InteractionGroups::new(
                            cc.collision_layer.into(),
                            cc.collision_mask.into(),
                        ))
                        .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS);
                    if friction == 0.0 {
                        builder = builder
                            .friction_combine_rule(rapier2d::prelude::CoefficientCombineRule::Min);
                    }
                    let collider = builder.build();

                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    cc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, entity_uuid);
                }
            }
        }

        self.physics_world = Some(physics);
    }

    /// Tear down the physics world and clear all runtime handles.
    ///
    /// Shared by both runtime and simulation stop paths.
    pub(super) fn on_physics_2d_stop(&mut self) {
        self.physics_world = None;

        // Clear runtime handles on all physics components.
        for rb in self.world.query_mut::<&mut RigidBody2DComponent>() {
            rb.runtime_body = None;
        }
        for bc in self.world.query_mut::<&mut BoxCollider2DComponent>() {
            bc.runtime_fixture = None;
        }
        for cc in self.world.query_mut::<&mut CircleCollider2DComponent>() {
            cc.runtime_fixture = None;
        }
    }

    // -----------------------------------------------------------------
    // Physics scripting API (used by both native + Lua scripts)
    // -----------------------------------------------------------------

    /// Apply a linear impulse to the entity's rigid body.
    ///
    /// No-op if the physics world is inactive (edit mode) or the entity
    /// lacks a [`RigidBody2DComponent`] with a valid runtime body.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse(na::Vector2::new(impulse.x, impulse.y), true);
            }
        }
    }

    /// Apply a linear impulse at a world-space point on the entity's rigid body.
    ///
    /// This can produce both translational and rotational motion depending on
    /// the point relative to the body's center of mass.
    pub fn apply_impulse_at_point(
        &mut self,
        entity: Entity,
        impulse: glam::Vec2,
        point: glam::Vec2,
    ) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse_at_point(
                    na::Vector2::new(impulse.x, impulse.y),
                    na::Point2::new(point.x, point.y),
                    true,
                );
            }
        }
    }

    /// Apply a continuous force to the entity's rigid body.
    ///
    /// Unlike impulses, forces are accumulated and applied during the next
    /// physics step. Call every frame for sustained acceleration.
    pub fn apply_force(&mut self, entity: Entity, force: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.add_force(na::Vector2::new(force.x, force.y), true);
            }
        }
    }

    /// Get the linear velocity of the entity's rigid body.
    ///
    /// Returns `None` if the physics world is inactive or the entity lacks
    /// a runtime rigid body.
    pub fn get_linear_velocity(&self, entity: Entity) -> Option<glam::Vec2> {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world) {
            if let Some(body) = physics.bodies.get(handle) {
                let v = body.linvel();
                return Some(glam::Vec2::new(v.x, v.y));
            }
        }
        None
    }

    /// Set the linear velocity of the entity's rigid body.
    pub fn set_linear_velocity(&mut self, entity: Entity, vel: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_linvel(na::Vector2::new(vel.x, vel.y), true);
            }
        }
    }

    /// Get the angular velocity (radians/sec) of the entity's rigid body.
    ///
    /// Returns `None` if the physics world is inactive or the entity lacks
    /// a runtime rigid body.
    pub fn get_angular_velocity(&self, entity: Entity) -> Option<f32> {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world) {
            if let Some(body) = physics.bodies.get(handle) {
                return Some(body.angvel());
            }
        }
        None
    }

    /// Set the angular velocity (radians/sec) of the entity's rigid body.
    pub fn set_angular_velocity(&mut self, entity: Entity, omega: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_angvel(omega, true);
            }
        }
    }

    /// Cast a ray and return the first hit: `(entity_uuid, hit_x, hit_y, normal_x, normal_y, toi)`.
    ///
    /// `exclude_entity` optionally filters out a specific entity (e.g. the caster).
    pub fn raycast(
        &self,
        origin: glam::Vec2,
        direction: glam::Vec2,
        max_toi: f32,
        exclude_entity: Option<Entity>,
    ) -> Option<(u64, f32, f32, f32, f32, f32)> {
        use rapier2d::na;
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world {
            physics.raycast(
                na::Point2::new(origin.x, origin.y),
                na::Vector2::new(direction.x, direction.y),
                max_toi,
                exclude_uuid,
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_physics_value_clamps_negative() {
        // Negative → clamped to min.
        assert_eq!(validate_physics_value(-1.0, 0.0, "test", 0), 0.0);
        // Valid → unchanged.
        assert_eq!(validate_physics_value(0.5, 0.0, "test", 0), 0.5);
        // Zero → unchanged (not < min).
        assert_eq!(validate_physics_value(0.0, 0.0, "test", 0), 0.0);
    }
}
