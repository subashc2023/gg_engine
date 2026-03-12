use rapier3d::na;
use rapier3d::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// Result of a 3D raycast query.
pub struct RaycastHit3D {
    pub entity_uuid: u64,
    pub hit_x: f32,
    pub hit_y: f32,
    pub hit_z: f32,
    pub normal_x: f32,
    pub normal_y: f32,
    pub normal_z: f32,
    pub toi: f32,
}

/// Fixed physics timestep (1/60 s ≈ 16.67 ms).
const FIXED_TIMESTEP: f32 = 1.0 / 60.0;

/// Maximum frame delta fed into the accumulator (caps at 250 ms to prevent
/// a "spiral of death" after long hitches).
const MAX_FRAME_DT: f32 = 0.25;

/// Bundles all rapier3d simulation state needed for a single 3D physics world.
pub(crate) struct PhysicsWorld3D {
    gravity: na::Vector3<f32>,
    pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    pub(crate) island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    pub(crate) bodies: RigidBodySet,
    pub(crate) colliders: ColliderSet,
    pub(crate) impulse_joints: ImpulseJointSet,
    pub(crate) multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    /// Leftover time from the previous frame, carried forward for the next
    /// fixed-step accumulation.
    accumulator: f32,
    /// Pre-step positions/rotations for interpolation.
    prev_transforms: HashMap<RigidBodyHandle, (na::Vector3<f32>, na::UnitQuaternion<f32>)>,
    /// Maps collider handles to entity UUIDs for collision event dispatch.
    pub(crate) collider_to_uuid: HashMap<ColliderHandle, u64>,
    /// Collision event collector — filled during physics step, drained after.
    collision_collector: CollisionCollector3D,
}

impl PhysicsWorld3D {
    pub(crate) fn new(gravity_x: f32, gravity_y: f32, gravity_z: f32) -> Self {
        let params = IntegrationParameters {
            dt: FIXED_TIMESTEP,
            ..Default::default()
        };
        Self {
            gravity: na::Vector3::new(gravity_x, gravity_y, gravity_z),
            pipeline: PhysicsPipeline::new(),
            integration_parameters: params,
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            accumulator: 0.0,
            prev_transforms: HashMap::new(),
            collider_to_uuid: HashMap::new(),
            collision_collector: CollisionCollector3D::new(),
        }
    }

    /// Feed frame delta-time into the accumulator (clamped to MAX_FRAME_DT).
    pub(crate) fn accumulate(&mut self, dt: f32) {
        self.accumulator += dt.min(MAX_FRAME_DT);
    }

    /// Returns `true` if the accumulator has enough time for another fixed step.
    pub(crate) fn can_step(&self) -> bool {
        self.accumulator >= FIXED_TIMESTEP
    }

    /// Snapshot current body positions/rotations as "previous" for interpolation.
    /// Call this *before* `step_once()`.
    pub(crate) fn snapshot_transforms(&mut self) {
        for (handle, body) in self.bodies.iter() {
            let pos = *body.translation();
            let rot = *body.rotation();
            self.prev_transforms.insert(handle, (pos, rot));
        }
    }

    /// Clear all user-applied forces/torques on every body.
    pub(crate) fn reset_all_forces(&mut self) {
        for (_, body) in self.bodies.iter_mut() {
            body.reset_forces(false);
        }
    }

    /// Execute a single rapier physics step and drain one FIXED_TIMESTEP
    /// from the accumulator.
    pub(crate) fn step_once(&mut self) {
        self.pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &self.collision_collector,
        );
        self.accumulator -= FIXED_TIMESTEP;
    }

    /// Drain collected collision events, resolving collider handles to entity UUIDs.
    pub(crate) fn drain_collision_events(&self) -> Vec<(u64, u64, bool)> {
        let mut events = Vec::new();
        let mut raw = self.collision_collector.events.lock().unwrap();
        for (h1, h2, started) in raw.drain(..) {
            let uuid1 = self.collider_to_uuid.get(&h1).copied();
            let uuid2 = self.collider_to_uuid.get(&h2).copied();
            if let (Some(u1), Some(u2)) = (uuid1, uuid2) {
                events.push((u1, u2, started));
            }
        }
        events
    }

    /// Register a collider handle → entity UUID mapping.
    pub(crate) fn register_collider(&mut self, collider: ColliderHandle, uuid: u64) {
        self.collider_to_uuid.insert(collider, uuid);
    }

    /// Set the global gravity vector.
    pub(crate) fn set_gravity(&mut self, x: f32, y: f32, z: f32) {
        self.gravity = na::Vector3::new(x, y, z);
    }

    /// Get the current gravity vector.
    pub(crate) fn get_gravity(&self) -> (f32, f32, f32) {
        (self.gravity.x, self.gravity.y, self.gravity.z)
    }

    /// The fixed timestep value (1/60 s).
    pub(crate) fn fixed_timestep(&self) -> f32 {
        FIXED_TIMESTEP
    }

    /// Interpolation alpha: fraction of a timestep remaining in the accumulator.
    pub(crate) fn alpha(&self) -> f32 {
        self.accumulator / FIXED_TIMESTEP
    }

    /// Get the pre-step (previous) transform for a body, if available.
    pub(crate) fn prev_transform(
        &self,
        handle: RigidBodyHandle,
    ) -> Option<(na::Vector3<f32>, na::UnitQuaternion<f32>)> {
        self.prev_transforms.get(&handle).copied()
    }

    /// Cast a ray and return the first hit.
    pub(crate) fn raycast(
        &self,
        origin: na::Point3<f32>,
        direction: na::Vector3<f32>,
        max_toi: f32,
        exclude_uuid: Option<u64>,
    ) -> Option<RaycastHit3D> {
        let ray = rapier3d::geometry::Ray::new(origin, direction);
        let predicate = |handle: ColliderHandle, _collider: &Collider| {
            if let Some(exclude) = exclude_uuid {
                if let Some(&uuid) = self.collider_to_uuid.get(&handle) {
                    return uuid != exclude;
                }
            }
            true
        };
        let filter = QueryFilter::default().predicate(&predicate);

        if let Some((collider_handle, intersection)) = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_toi,
            true,
            filter,
        ) {
            if let Some(&uuid) = self.collider_to_uuid.get(&collider_handle) {
                let hit_point = ray.point_at(intersection.time_of_impact);
                return Some(RaycastHit3D {
                    entity_uuid: uuid,
                    hit_x: hit_point.x,
                    hit_y: hit_point.y,
                    hit_z: hit_point.z,
                    normal_x: intersection.normal.x,
                    normal_y: intersection.normal.y,
                    normal_z: intersection.normal.z,
                    toi: intersection.time_of_impact,
                });
            }
        }
        None
    }
    // -----------------------------------------------------------------
    // Shape overlap queries
    // -----------------------------------------------------------------

    /// Find all entities whose colliders contain the given point.
    pub(crate) fn point_query(&self, point: na::Point3<f32>) -> Vec<u64> {
        let mut results = Vec::new();
        self.query_pipeline.intersections_with_point(
            &self.bodies,
            &self.colliders,
            &point,
            QueryFilter::default(),
            |collider_handle| {
                if let Some(&uuid) = self.collider_to_uuid.get(&collider_handle) {
                    results.push(uuid);
                }
                true // continue iterating
            },
        );
        results
    }

    /// Find all entities whose collider AABBs overlap the given axis-aligned bounding box.
    pub(crate) fn aabb_query(&self, min: na::Point3<f32>, max: na::Point3<f32>) -> Vec<u64> {
        let aabb = rapier3d::parry::bounding_volume::Aabb::new(
            na::Point3::new(min.x, min.y, min.z),
            na::Point3::new(max.x, max.y, max.z),
        );
        let mut results = Vec::new();
        self.query_pipeline
            .colliders_with_aabb_intersecting_aabb(&aabb, |&collider_handle| {
                if let Some(&uuid) = self.collider_to_uuid.get(&collider_handle) {
                    results.push(uuid);
                }
                true // continue iterating
            });
        results
    }

    /// Test if a specific shape at a given position/rotation overlaps any colliders.
    /// Returns all overlapping entity UUIDs.
    pub(crate) fn shape_overlap(
        &self,
        position: &na::Isometry3<f32>,
        shape: &dyn rapier3d::parry::shape::Shape,
        exclude_uuid: Option<u64>,
    ) -> Vec<u64> {
        let predicate = |handle: ColliderHandle, _collider: &Collider| {
            if let Some(exclude) = exclude_uuid {
                if let Some(&uuid) = self.collider_to_uuid.get(&handle) {
                    return uuid != exclude;
                }
            }
            true
        };
        let filter = QueryFilter::default().predicate(&predicate);
        let mut results = Vec::new();
        self.query_pipeline.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            position,
            shape,
            filter,
            |collider_handle| {
                if let Some(&uuid) = self.collider_to_uuid.get(&collider_handle) {
                    results.push(uuid);
                }
                true // continue iterating
            },
        );
        results
    }

    // -----------------------------------------------------------------
    // Joints
    // -----------------------------------------------------------------

    /// Create a revolute (hinge) joint between two bodies around a given axis.
    pub(crate) fn create_revolute_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Point3<f32>,
        anchor_b: na::Point3<f32>,
        axis: na::UnitVector3<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier3d::dynamics::RevoluteJointBuilder::new(axis)
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Create a fixed joint between two bodies (locks relative transform).
    pub(crate) fn create_fixed_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Isometry3<f32>,
        anchor_b: na::Isometry3<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier3d::dynamics::FixedJointBuilder::new()
            .local_frame1(anchor_a)
            .local_frame2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Create a ball (spherical) joint between two bodies.
    pub(crate) fn create_ball_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Point3<f32>,
        anchor_b: na::Point3<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier3d::dynamics::SphericalJointBuilder::new()
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Create a prismatic (slider) joint between two bodies along a given axis.
    pub(crate) fn create_prismatic_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Point3<f32>,
        anchor_b: na::Point3<f32>,
        axis: na::UnitVector3<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier3d::dynamics::PrismaticJointBuilder::new(axis)
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Remove a joint by handle.
    pub(crate) fn remove_joint(&mut self, handle: ImpulseJointHandle) {
        self.impulse_joints.remove(handle, true);
    }
}

// ---------------------------------------------------------------------------
// Collision event collector
// ---------------------------------------------------------------------------

struct CollisionCollector3D {
    events: Mutex<Vec<(ColliderHandle, ColliderHandle, bool)>>,
}

impl CollisionCollector3D {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl EventHandler for CollisionCollector3D {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let (h1, h2, started) = match event {
            CollisionEvent::Started(h1, h2, _) => (h1, h2, true),
            CollisionEvent::Stopped(h1, h2, _) => (h1, h2, false),
        };
        if let Ok(mut events) = self.events.lock() {
            events.push((h1, h2, started));
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
        // Not used — we only care about collision start/stop.
    }
}
