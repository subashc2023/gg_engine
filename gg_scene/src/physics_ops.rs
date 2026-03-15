use super::physics_2d::PhysicsWorld2D;
use super::{
    BoxCollider2DComponent, CircleCollider2DComponent, Entity, IdComponent, RelationshipComponent,
    RigidBody2DComponent, Scene, TransformComponent,
};
use rapier2d::na;

/// Pack a rapier arena (index, generation) pair into a u64 for Lua/external use.
fn joint_handle_to_u64(index: u32, generation: u32) -> u64 {
    (generation as u64) << 32 | index as u64
}

/// Unpack a u64 into a rapier arena (index, generation) pair.
fn u64_to_joint_handle(packed: u64) -> (u32, u32) {
    let index = packed as u32;
    let generation = (packed >> 32) as u32;
    (index, generation)
}

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

/// Apply shared physics material and collision properties to a collider builder.
#[allow(clippy::too_many_arguments)]
fn configure_collider(
    builder: rapier2d::geometry::ColliderBuilder,
    density: f32,
    friction: f32,
    restitution: f32,
    offset: glam::Vec2,
    scale: glam::Vec3,
    collision_layer: u32,
    collision_mask: u32,
    is_sensor: bool,
    entity_uuid: u64,
) -> rapier2d::geometry::Collider {
    let density = validate_physics_value(density, 0.0, "density", entity_uuid);
    let friction = validate_physics_value(friction, 0.0, "friction", entity_uuid);
    let restitution = validate_physics_value(restitution, 0.0, "restitution", entity_uuid);

    let mut builder = builder
        .density(density)
        .friction(friction)
        .restitution(restitution)
        .translation(na::Vector2::new(
            offset.x * scale.x.abs(),
            offset.y * scale.y.abs(),
        ))
        .collision_groups(rapier2d::geometry::InteractionGroups::new(
            collision_layer.into(),
            collision_mask.into(),
        ))
        .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS)
        .sensor(is_sensor);

    if friction == 0.0 {
        builder = builder.friction_combine_rule(rapier2d::prelude::CoefficientCombineRule::Min);
    }

    builder.build()
}

/// Extracted body data for physics body creation (avoids complex tuple type).
struct BodySetup {
    handle: hecs::Entity,
    uuid: u64,
    translation: glam::Vec3,
    rotation: glam::Quat,
    scale: glam::Vec3,
    body_type: super::RigidBody2DType,
    fixed_rotation: bool,
    gravity_scale: f32,
    linear_damping: f32,
    angular_damping: f32,
}

impl Scene {
    // -----------------------------------------------------------------
    // Physics (shared helpers)
    // -----------------------------------------------------------------

    /// Create the physics world and populate it with rigid bodies / colliders
    /// from all entities that have physics components.
    ///
    /// Shared by both runtime and simulation start paths.
    pub fn on_physics_2d_start(&mut self) {
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::on_physics_2d_start");
        let mut physics = PhysicsWorld2D::new(0.0, -9.81);

        // Snapshot entities with RigidBody2DComponent to avoid borrow conflicts.
        // Skip parented entities — physics bodies ignore parent transforms, so
        // allowing them would cause confusing mismatches between visual and physics position.
        let body_entities: Vec<BodySetup> = self
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
                Some(BodySetup {
                    handle,
                    uuid: id.id.raw(),
                    translation: transform.translation,
                    rotation: transform.rotation,
                    scale: transform.scale,
                    body_type: rb.body_type,
                    fixed_rotation: rb.fixed_rotation,
                    gravity_scale: rb.gravity_scale,
                    linear_damping: rb.linear_damping,
                    angular_damping: rb.angular_damping,
                })
            })
            .collect();

        for bs in body_entities {
            // Create rapier rigid body.
            let mut body_builder =
                rapier2d::dynamics::RigidBodyBuilder::new(bs.body_type.to_rapier_2d())
                    .translation(na::Vector2::new(bs.translation.x, bs.translation.y))
                    .rotation(bs.rotation.to_euler(glam::EulerRot::XYZ).2)
                    .gravity_scale(bs.gravity_scale)
                    .linear_damping(bs.linear_damping)
                    .angular_damping(bs.angular_damping);

            if bs.fixed_rotation {
                body_builder = body_builder.lock_rotations();
            }

            let body_handle = physics.bodies.insert(body_builder.build());

            // Store the handle back on the component.
            if let Ok(mut rb) = self.world.get::<&mut RigidBody2DComponent>(bs.handle) {
                rb.runtime_body = Some(body_handle);
            }

            // If entity also has a BoxCollider2DComponent, create a collider.
            if let Ok(mut bc) = self.world.get::<&mut BoxCollider2DComponent>(bs.handle) {
                let half_x = bc.size.x * bs.scale.x.abs();
                let half_y = bc.size.y * bs.scale.y.abs();

                if half_x <= 0.0 || half_y <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-size box collider ({} x {}), skipping",
                        bs.uuid,
                        half_x * 2.0,
                        half_y * 2.0
                    );
                } else {
                    let collider = configure_collider(
                        rapier2d::geometry::ColliderBuilder::cuboid(half_x, half_y),
                        bc.density,
                        bc.friction,
                        bc.restitution,
                        bc.offset,
                        bs.scale,
                        bc.collision_layer,
                        bc.collision_mask,
                        bc.is_sensor,
                        bs.uuid,
                    );
                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    bc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, bs.uuid);
                }
            }

            // If entity also has a CircleCollider2DComponent, create a collider.
            if let Ok(mut cc) = self.world.get::<&mut CircleCollider2DComponent>(bs.handle) {
                let scaled_radius = cc.radius * bs.scale.x.abs().max(bs.scale.y.abs());

                if scaled_radius <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-radius circle collider, skipping",
                        bs.uuid
                    );
                } else {
                    let collider = configure_collider(
                        rapier2d::geometry::ColliderBuilder::ball(scaled_radius),
                        cc.density,
                        cc.friction,
                        cc.restitution,
                        cc.offset,
                        bs.scale,
                        cc.collision_layer,
                        cc.collision_mask,
                        cc.is_sensor,
                        bs.uuid,
                    );
                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    cc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, bs.uuid);
                }
            }
        }

        self.physics_world = Some(physics);
    }

    /// Tear down the physics world and clear all runtime handles.
    ///
    /// Shared by both runtime and simulation stop paths.
    pub fn on_physics_2d_stop(&mut self) {
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

    /// Apply a torque impulse (angular impulse) to the entity's 2D rigid body.
    ///
    /// Positive values rotate counter-clockwise.
    pub fn apply_torque_impulse(&mut self, entity: Entity, torque: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_torque_impulse(torque, true);
            }
        }
    }

    /// Apply a continuous torque to the entity's 2D rigid body.
    ///
    /// Unlike torque impulses, torques are accumulated and applied during the
    /// next physics step. Call every frame for sustained angular acceleration.
    pub fn apply_torque(&mut self, entity: Entity, torque: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.add_torque(torque, true);
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

    /// Sync the physics body position to match the transform.
    ///
    /// Called when scripts set an entity's translation directly, so the
    /// physics body stays in sync.
    pub fn sync_physics_translation(&mut self, entity: Entity, x: f32, y: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_translation(na::Vector2::new(x, y), true);
            }
        }
    }

    /// Set the gravity scale for a specific entity's rigid body at runtime.
    ///
    /// `1.0` = normal gravity, `0.0` = no gravity, negative = inverted.
    pub fn set_gravity_scale(&mut self, entity: Entity, scale: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_gravity_scale(scale, true);
            }
        }
    }

    /// Get the gravity scale for a specific entity's rigid body.
    pub fn get_gravity_scale(&self, entity: Entity) -> Option<f32> {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world) {
            if let Some(body) = physics.bodies.get(handle) {
                return Some(body.gravity_scale());
            }
        }
        None
    }

    /// Set the global gravity vector for the physics world.
    pub fn set_gravity(&mut self, x: f32, y: f32) {
        if let Some(ref mut physics) = self.physics_world {
            physics.set_gravity(x, y);
        }
    }

    /// Get the current gravity vector.
    pub fn get_gravity(&self) -> (f32, f32) {
        if let Some(ref physics) = self.physics_world {
            physics.get_gravity()
        } else {
            (0.0, -9.81)
        }
    }

    /// Change the body type of a 2D rigid body at runtime.
    ///
    /// Also updates the component so it stays in sync.
    pub fn set_body_type(&mut self, entity: Entity, body_type: super::RigidBody2DType) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_body_type(body_type.to_rapier_2d(), true);
            }
        }
        // Update the component field to stay in sync.
        if let Ok(mut rb) = self.world.get::<&mut RigidBody2DComponent>(entity.handle()) {
            rb.body_type = body_type;
        }
    }

    /// Get the body type of a 2D rigid body.
    pub fn get_body_type(&self, entity: Entity) -> Option<super::RigidBody2DType> {
        self.get_component::<RigidBody2DComponent>(entity)
            .map(|rb| rb.body_type)
    }

    // -----------------------------------------------------------------
    // Shape overlap queries
    // -----------------------------------------------------------------

    /// Find all entities whose 2D colliders contain the given point.
    pub fn point_query(&self, point: glam::Vec2) -> Vec<u64> {
        if let Some(ref physics) = self.physics_world {
            physics.point_query(na::Point2::new(point.x, point.y))
        } else {
            Vec::new()
        }
    }

    /// Find all entities whose 2D collider AABBs overlap the given AABB.
    pub fn aabb_query(&self, min: glam::Vec2, max: glam::Vec2) -> Vec<u64> {
        if let Some(ref physics) = self.physics_world {
            physics.aabb_query(na::Point2::new(min.x, min.y), na::Point2::new(max.x, max.y))
        } else {
            Vec::new()
        }
    }

    /// Test if a circle at a given position overlaps any 2D colliders.
    /// Returns all overlapping entity UUIDs.
    pub fn overlap_circle(
        &self,
        center: glam::Vec2,
        radius: f32,
        exclude_entity: Option<Entity>,
    ) -> Vec<u64> {
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world {
            let position = na::Isometry2::translation(center.x, center.y);
            let shape = rapier2d::parry::shape::Ball::new(radius);
            physics.shape_overlap(&position, &shape, exclude_uuid)
        } else {
            Vec::new()
        }
    }

    /// Test if an axis-aligned box at a given position overlaps any 2D colliders.
    /// Returns all overlapping entity UUIDs.
    pub fn overlap_box(
        &self,
        center: glam::Vec2,
        half_extents: glam::Vec2,
        exclude_entity: Option<Entity>,
    ) -> Vec<u64> {
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world {
            let position = na::Isometry2::translation(center.x, center.y);
            let shape = rapier2d::parry::shape::Cuboid::new(na::Vector2::new(
                half_extents.x,
                half_extents.y,
            ));
            physics.shape_overlap(&position, &shape, exclude_uuid)
        } else {
            Vec::new()
        }
    }

    // -----------------------------------------------------------------
    // 2D Joints
    // -----------------------------------------------------------------

    /// Create a revolute (hinge) joint between two entities.
    ///
    /// `anchor_a` and `anchor_b` are in local space of each body.
    /// Returns the joint handle index, or `None` if either entity lacks a physics body.
    pub fn create_revolute_joint(
        &mut self,
        entity_a: Entity,
        entity_b: Entity,
        anchor_a: glam::Vec2,
        anchor_b: glam::Vec2,
    ) -> Option<u64> {
        let body_a = self
            .get_component::<RigidBody2DComponent>(entity_a)
            .and_then(|rb| rb.runtime_body)?;
        let body_b = self
            .get_component::<RigidBody2DComponent>(entity_b)
            .and_then(|rb| rb.runtime_body)?;
        let physics = self.physics_world.as_mut()?;
        let handle = physics.create_revolute_joint(
            body_a,
            body_b,
            na::Point2::new(anchor_a.x, anchor_a.y),
            na::Point2::new(anchor_b.x, anchor_b.y),
        );
        let (idx, gen) = handle.0.into_raw_parts();
        Some(joint_handle_to_u64(idx, gen))
    }

    /// Create a fixed joint between two entities (locks relative transform).
    ///
    /// Anchors are in local space of each body (position only, no rotation offset).
    pub fn create_fixed_joint(
        &mut self,
        entity_a: Entity,
        entity_b: Entity,
        anchor_a: glam::Vec2,
        anchor_b: glam::Vec2,
    ) -> Option<u64> {
        let body_a = self
            .get_component::<RigidBody2DComponent>(entity_a)
            .and_then(|rb| rb.runtime_body)?;
        let body_b = self
            .get_component::<RigidBody2DComponent>(entity_b)
            .and_then(|rb| rb.runtime_body)?;
        let physics = self.physics_world.as_mut()?;
        let frame_a = na::Isometry2::translation(anchor_a.x, anchor_a.y);
        let frame_b = na::Isometry2::translation(anchor_b.x, anchor_b.y);
        let handle = physics.create_fixed_joint(body_a, body_b, frame_a, frame_b);
        let (idx, gen) = handle.0.into_raw_parts();
        Some(joint_handle_to_u64(idx, gen))
    }

    /// Create a prismatic (slider) joint between two entities.
    ///
    /// `axis` is the local-space sliding direction (normalized automatically).
    pub fn create_prismatic_joint(
        &mut self,
        entity_a: Entity,
        entity_b: Entity,
        anchor_a: glam::Vec2,
        anchor_b: glam::Vec2,
        axis: glam::Vec2,
    ) -> Option<u64> {
        let body_a = self
            .get_component::<RigidBody2DComponent>(entity_a)
            .and_then(|rb| rb.runtime_body)?;
        let body_b = self
            .get_component::<RigidBody2DComponent>(entity_b)
            .and_then(|rb| rb.runtime_body)?;
        let physics = self.physics_world.as_mut()?;
        let unit_axis = match na::UnitVector2::try_new(na::Vector2::new(axis.x, axis.y), 1.0e-6) {
            Some(a) => a,
            None => {
                log::warn!("Prismatic joint axis is near-zero, defaulting to X axis");
                na::UnitVector2::new_normalize(na::Vector2::new(1.0, 0.0))
            }
        };
        let handle = physics.create_prismatic_joint(
            body_a,
            body_b,
            na::Point2::new(anchor_a.x, anchor_a.y),
            na::Point2::new(anchor_b.x, anchor_b.y),
            unit_axis,
        );
        let (idx, gen) = handle.0.into_raw_parts();
        Some(joint_handle_to_u64(idx, gen))
    }

    /// Remove a 2D joint by its packed handle.
    pub fn remove_joint(&mut self, joint_id: u64) {
        if let Some(ref mut physics) = self.physics_world {
            let (idx, gen) = u64_to_joint_handle(joint_id);
            let index = rapier2d::data::Index::from_raw_parts(idx, gen);
            let handle = rapier2d::dynamics::ImpulseJointHandle(index);
            physics.remove_joint(handle);
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

    /// Cast a ray and return **all** hits sorted by distance.
    ///
    /// Each hit is `(entity_uuid, hit_x, hit_y, normal_x, normal_y, toi)`.
    /// `exclude_entity` optionally filters out a specific entity.
    pub fn raycast_all(
        &self,
        origin: glam::Vec2,
        direction: glam::Vec2,
        max_toi: f32,
        exclude_entity: Option<Entity>,
    ) -> Vec<(u64, f32, f32, f32, f32, f32)> {
        use rapier2d::na;
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world {
            physics.raycast_all(
                na::Point2::new(origin.x, origin.y),
                na::Vector2::new(direction.x, direction.y),
                max_toi,
                exclude_uuid,
            )
        } else {
            Vec::new()
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
