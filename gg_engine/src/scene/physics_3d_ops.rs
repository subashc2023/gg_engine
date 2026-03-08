use super::physics_3d::PhysicsWorld3D;
use super::{
    BoxCollider3DComponent, CapsuleCollider3DComponent, Entity, IdComponent, RelationshipComponent,
    RigidBody3DComponent, Scene, SphereCollider3DComponent, TransformComponent,
};
use rapier3d::na;

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

/// Apply shared physics material and collision properties to a 3D collider builder.
#[allow(clippy::too_many_arguments)]
fn configure_collider_3d(
    builder: rapier3d::geometry::ColliderBuilder,
    density: f32,
    friction: f32,
    restitution: f32,
    offset: glam::Vec3,
    scale: glam::Vec3,
    collision_layer: u32,
    collision_mask: u32,
    is_sensor: bool,
    entity_uuid: u64,
) -> rapier3d::geometry::Collider {
    let density = validate_physics_value(density, 0.0, "density", entity_uuid);
    let friction = validate_physics_value(friction, 0.0, "friction", entity_uuid);
    let restitution = validate_physics_value(restitution, 0.0, "restitution", entity_uuid);

    let mut builder = builder
        .density(density)
        .friction(friction)
        .restitution(restitution)
        .translation(na::Vector3::new(
            offset.x * scale.x.abs(),
            offset.y * scale.y.abs(),
            offset.z * scale.z.abs(),
        ))
        .collision_groups(rapier3d::geometry::InteractionGroups::new(
            collision_layer.into(),
            collision_mask.into(),
        ))
        .active_events(rapier3d::prelude::ActiveEvents::COLLISION_EVENTS)
        .sensor(is_sensor);

    if friction == 0.0 {
        builder = builder.friction_combine_rule(rapier3d::prelude::CoefficientCombineRule::Min);
    }

    builder.build()
}

/// Extracted body data for 3D physics body creation.
struct BodySetup3D {
    handle: hecs::Entity,
    uuid: u64,
    translation: glam::Vec3,
    rotation: glam::Quat,
    scale: glam::Vec3,
    body_type: super::RigidBody3DType,
    lock_rotation_x: bool,
    lock_rotation_y: bool,
    lock_rotation_z: bool,
    gravity_scale: f32,
    linear_damping: f32,
    angular_damping: f32,
}

impl Scene {
    // -----------------------------------------------------------------
    // 3D Physics (shared helpers)
    // -----------------------------------------------------------------

    /// Create the 3D physics world and populate it with rigid bodies / colliders
    /// from all entities that have 3D physics components.
    pub(super) fn on_physics_3d_start(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_physics_3d_start");
        let mut physics = PhysicsWorld3D::new(0.0, -9.81, 0.0);

        let body_entities: Vec<BodySetup3D> = self
            .world
            .query::<(
                hecs::Entity,
                &IdComponent,
                &TransformComponent,
                &RigidBody3DComponent,
                &RelationshipComponent,
            )>()
            .iter()
            .filter_map(|(handle, id, transform, rb, rel)| {
                if rel.parent.is_some() {
                    log::warn!(
                        "Entity UUID {} has RigidBody3D but is parented — skipping physics body creation. \
                         Detach from parent or remove the RigidBody3D component.",
                        id.id.raw(),
                    );
                    return None;
                }
                Some(BodySetup3D {
                    handle,
                    uuid: id.id.raw(),
                    translation: transform.translation,
                    rotation: transform.rotation,
                    scale: transform.scale,
                    body_type: rb.body_type,
                    lock_rotation_x: rb.lock_rotation_x,
                    lock_rotation_y: rb.lock_rotation_y,
                    lock_rotation_z: rb.lock_rotation_z,
                    gravity_scale: rb.gravity_scale,
                    linear_damping: rb.linear_damping,
                    angular_damping: rb.angular_damping,
                })
            })
            .collect();

        for bs in body_entities {
            let (x, y, z, w) = (bs.rotation.x, bs.rotation.y, bs.rotation.z, bs.rotation.w);
            let rapier_rot = na::UnitQuaternion::new_normalize(na::Quaternion::new(w, x, y, z));

            let mut body_builder =
                rapier3d::dynamics::RigidBodyBuilder::new(bs.body_type.to_rapier())
                    .translation(na::Vector3::new(
                        bs.translation.x,
                        bs.translation.y,
                        bs.translation.z,
                    ))
                    .rotation(rapier_rot.scaled_axis())
                    .gravity_scale(bs.gravity_scale)
                    .linear_damping(bs.linear_damping)
                    .angular_damping(bs.angular_damping);

            if bs.lock_rotation_x || bs.lock_rotation_y || bs.lock_rotation_z {
                body_builder = body_builder.enabled_rotations(
                    !bs.lock_rotation_x,
                    !bs.lock_rotation_y,
                    !bs.lock_rotation_z,
                );
            }

            let body_handle = physics.bodies.insert(body_builder.build());

            // Store the handle back on the component.
            if let Ok(mut rb) = self.world.get::<&mut RigidBody3DComponent>(bs.handle) {
                rb.runtime_body = Some(body_handle);
            }

            // Box collider.
            if let Ok(mut bc) = self.world.get::<&mut BoxCollider3DComponent>(bs.handle) {
                let half_x = bc.size.x * bs.scale.x.abs();
                let half_y = bc.size.y * bs.scale.y.abs();
                let half_z = bc.size.z * bs.scale.z.abs();

                if half_x <= 0.0 || half_y <= 0.0 || half_z <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-size 3D box collider ({} x {} x {}), skipping",
                        bs.uuid,
                        half_x * 2.0,
                        half_y * 2.0,
                        half_z * 2.0,
                    );
                } else {
                    let collider = configure_collider_3d(
                        rapier3d::geometry::ColliderBuilder::cuboid(half_x, half_y, half_z),
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

            // Sphere collider.
            if let Ok(mut sc) = self.world.get::<&mut SphereCollider3DComponent>(bs.handle) {
                let max_scale = bs.scale.x.abs().max(bs.scale.y.abs()).max(bs.scale.z.abs());
                let scaled_radius = sc.radius * max_scale;

                if scaled_radius <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-radius 3D sphere collider, skipping",
                        bs.uuid
                    );
                } else {
                    let collider = configure_collider_3d(
                        rapier3d::geometry::ColliderBuilder::ball(scaled_radius),
                        sc.density,
                        sc.friction,
                        sc.restitution,
                        sc.offset,
                        bs.scale,
                        sc.collision_layer,
                        sc.collision_mask,
                        sc.is_sensor,
                        bs.uuid,
                    );
                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    sc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, bs.uuid);
                }
            }

            // Capsule collider.
            if let Ok(mut cc) = self.world.get::<&mut CapsuleCollider3DComponent>(bs.handle) {
                let scale_y = bs.scale.y.abs();
                let max_scale_xz = bs.scale.x.abs().max(bs.scale.z.abs());
                let scaled_half_height = cc.half_height * scale_y;
                let scaled_radius = cc.radius * max_scale_xz;

                if scaled_radius <= 0.0 || scaled_half_height < 0.0 {
                    log::warn!(
                        "Entity {} has zero-size 3D capsule collider, skipping",
                        bs.uuid
                    );
                } else {
                    let collider = configure_collider_3d(
                        rapier3d::geometry::ColliderBuilder::capsule_y(
                            scaled_half_height,
                            scaled_radius,
                        ),
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

        self.physics_world_3d = Some(physics);
    }

    /// Tear down the 3D physics world and clear all runtime handles.
    pub(super) fn on_physics_3d_stop(&mut self) {
        self.physics_world_3d = None;

        for rb in self.world.query_mut::<&mut RigidBody3DComponent>() {
            rb.runtime_body = None;
        }
        for bc in self.world.query_mut::<&mut BoxCollider3DComponent>() {
            bc.runtime_fixture = None;
        }
        for sc in self.world.query_mut::<&mut SphereCollider3DComponent>() {
            sc.runtime_fixture = None;
        }
        for cc in self.world.query_mut::<&mut CapsuleCollider3DComponent>() {
            cc.runtime_fixture = None;
        }
    }

    // -----------------------------------------------------------------
    // 3D Physics scripting API
    // -----------------------------------------------------------------

    /// Apply a linear impulse to the entity's 3D rigid body.
    pub fn apply_impulse_3d(&mut self, entity: Entity, impulse: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse(na::Vector3::new(impulse.x, impulse.y, impulse.z), true);
            }
        }
    }

    /// Apply a linear impulse at a world-space point on the entity's 3D rigid body.
    pub fn apply_impulse_at_point_3d(
        &mut self,
        entity: Entity,
        impulse: glam::Vec3,
        point: glam::Vec3,
    ) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse_at_point(
                    na::Vector3::new(impulse.x, impulse.y, impulse.z),
                    na::Point3::new(point.x, point.y, point.z),
                    true,
                );
            }
        }
    }

    /// Apply a torque impulse to the entity's 3D rigid body.
    pub fn apply_torque_impulse_3d(&mut self, entity: Entity, torque: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_torque_impulse(na::Vector3::new(torque.x, torque.y, torque.z), true);
            }
        }
    }

    /// Apply a continuous force to the entity's 3D rigid body.
    pub fn apply_force_3d(&mut self, entity: Entity, force: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.add_force(na::Vector3::new(force.x, force.y, force.z), true);
            }
        }
    }

    /// Apply a continuous torque to the entity's 3D rigid body.
    pub fn apply_torque_3d(&mut self, entity: Entity, torque: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.add_torque(na::Vector3::new(torque.x, torque.y, torque.z), true);
            }
        }
    }

    /// Get the linear velocity of the entity's 3D rigid body.
    pub fn get_linear_velocity_3d(&self, entity: Entity) -> Option<glam::Vec3> {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world_3d) {
            if let Some(body) = physics.bodies.get(handle) {
                let v = body.linvel();
                return Some(glam::Vec3::new(v.x, v.y, v.z));
            }
        }
        None
    }

    /// Set the linear velocity of the entity's 3D rigid body.
    pub fn set_linear_velocity_3d(&mut self, entity: Entity, vel: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_linvel(na::Vector3::new(vel.x, vel.y, vel.z), true);
            }
        }
    }

    /// Get the angular velocity of the entity's 3D rigid body.
    pub fn get_angular_velocity_3d(&self, entity: Entity) -> Option<glam::Vec3> {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world_3d) {
            if let Some(body) = physics.bodies.get(handle) {
                let w = body.angvel();
                return Some(glam::Vec3::new(w.x, w.y, w.z));
            }
        }
        None
    }

    /// Set the angular velocity of the entity's 3D rigid body.
    pub fn set_angular_velocity_3d(&mut self, entity: Entity, omega: glam::Vec3) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_angvel(na::Vector3::new(omega.x, omega.y, omega.z), true);
            }
        }
    }

    /// Sync the 3D physics body position to match the transform.
    pub fn sync_physics_translation_3d(&mut self, entity: Entity, x: f32, y: f32, z: f32) {
        let body_handle = self
            .get_component::<RigidBody3DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world_3d) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_translation(na::Vector3::new(x, y, z), true);
            }
        }
    }

    /// Set the global 3D gravity vector.
    pub fn set_gravity_3d(&mut self, x: f32, y: f32, z: f32) {
        if let Some(ref mut physics) = self.physics_world_3d {
            physics.set_gravity(x, y, z);
        }
    }

    /// Get the current 3D gravity vector.
    pub fn get_gravity_3d(&self) -> (f32, f32, f32) {
        if let Some(ref physics) = self.physics_world_3d {
            physics.get_gravity()
        } else {
            (0.0, -9.81, 0.0)
        }
    }

    /// Cast a 3D ray and return the first hit:
    /// `(entity_uuid, hit_x, hit_y, hit_z, normal_x, normal_y, normal_z, toi)`.
    pub fn raycast_3d(
        &self,
        origin: glam::Vec3,
        direction: glam::Vec3,
        max_toi: f32,
        exclude_entity: Option<Entity>,
    ) -> Option<(u64, f32, f32, f32, f32, f32, f32, f32)> {
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world_3d {
            physics.raycast(
                na::Point3::new(origin.x, origin.y, origin.z),
                na::Vector3::new(direction.x, direction.y, direction.z),
                max_toi,
                exclude_uuid,
            )
        } else {
            None
        }
    }
}
