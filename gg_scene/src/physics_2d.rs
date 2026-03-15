use rapier2d::na;
use rapier2d::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

use super::physics_common::{PhysicsTimestep, FIXED_TIMESTEP};

/// Bundles all rapier2d simulation state needed for a single physics world.
pub struct PhysicsWorld2D {
    gravity: na::Vector2<f32>,
    pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    pub island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    /// Fixed-timestep accumulator (shared logic with 3D).
    timestep: PhysicsTimestep,
    /// Pre-step positions/rotations for interpolation (position_x, position_y, angle).
    prev_transforms: HashMap<RigidBodyHandle, (f32, f32, f32)>,
    /// Maps collider handles to entity UUIDs for collision event dispatch.
    pub collider_to_uuid: HashMap<ColliderHandle, u64>,
    /// Collision event collector — filled during physics step, drained after.
    collision_collector: CollisionCollector,
}

impl PhysicsWorld2D {
    pub fn new(gravity_x: f32, gravity_y: f32) -> Self {
        let params = IntegrationParameters {
            dt: FIXED_TIMESTEP,
            ..Default::default()
        };
        Self {
            gravity: na::Vector2::new(gravity_x, gravity_y),
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
            timestep: PhysicsTimestep::new(),
            prev_transforms: HashMap::new(),
            collider_to_uuid: HashMap::new(),
            collision_collector: CollisionCollector::new(),
        }
    }

    /// Feed frame delta-time into the accumulator (clamped to MAX_FRAME_DT).
    pub fn accumulate(&mut self, dt: f32) {
        self.timestep.accumulate(dt);
    }

    /// Returns `true` if the accumulator has enough time for another fixed step.
    pub fn can_step(&self) -> bool {
        self.timestep.can_step()
    }

    /// Snapshot current body positions as "previous" for interpolation.
    /// Call this *before* `step_once()`.
    pub fn snapshot_transforms(&mut self) {
        self.prev_transforms.clear();
        for (handle, body) in self.bodies.iter() {
            let pos = body.translation();
            let angle = body.rotation().angle();
            self.prev_transforms.insert(handle, (pos.x, pos.y, angle));
        }
    }

    /// Clear all user-applied forces/torques on every body.
    ///
    /// Call this at the start of each fixed step, **before** scripts run,
    /// so only forces applied during the current step are integrated.
    /// rapier 0.22 does NOT auto-clear forces after `pipeline.step()`.
    pub fn reset_all_forces(&mut self) {
        for (_, body) in self.bodies.iter_mut() {
            body.reset_forces(false);
        }
    }

    /// Execute a single rapier physics step and drain one FIXED_TIMESTEP
    /// from the accumulator.
    pub fn step_once(&mut self) {
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
        self.timestep.consume_step();
    }

    /// Drain collected collision events, resolving collider handles to entity UUIDs.
    ///
    /// Returns pairs of `(uuid_a, uuid_b, started)` where `started` is `true`
    /// for collision start and `false` for collision stop.
    pub fn drain_collision_events(&self) -> Vec<(u64, u64, bool)> {
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
    pub fn register_collider(&mut self, collider: ColliderHandle, uuid: u64) {
        self.collider_to_uuid.insert(collider, uuid);
    }

    /// Set the global gravity vector.
    pub fn set_gravity(&mut self, x: f32, y: f32) {
        self.gravity = na::Vector2::new(x, y);
    }

    /// Get the current gravity vector.
    pub fn get_gravity(&self) -> (f32, f32) {
        (self.gravity.x, self.gravity.y)
    }

    /// The fixed timestep value (1/60 s).
    pub fn fixed_timestep(&self) -> f32 {
        self.timestep.fixed_timestep()
    }

    /// Interpolation alpha: fraction of a timestep remaining in the accumulator.
    /// Ranges from 0.0 (just stepped) to ~1.0 (about to step).
    pub fn alpha(&self) -> f32 {
        self.timestep.alpha()
    }

    /// Get the pre-step (previous) transform for a body, if available.
    pub fn prev_transform(&self, handle: RigidBodyHandle) -> Option<(f32, f32, f32)> {
        self.prev_transforms.get(&handle).copied()
    }

    /// Cast a ray and return the first hit: `(entity_uuid, hit_x, hit_y, normal_x, normal_y, toi)`.
    ///
    /// `origin` is the ray start, `direction` is the (unnormalized) ray direction,
    /// `max_toi` is the maximum "time of impact" (ray length in direction-units),
    /// and `exclude_uuid` optionally filters out a specific entity.
    pub fn raycast(
        &self,
        origin: na::Point2<f32>,
        direction: na::Vector2<f32>,
        max_toi: f32,
        exclude_uuid: Option<u64>,
    ) -> Option<(u64, f32, f32, f32, f32, f32)> {
        let ray = rapier2d::geometry::Ray::new(origin, direction);
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
                return Some((
                    uuid,
                    hit_point.x,
                    hit_point.y,
                    intersection.normal.x,
                    intersection.normal.y,
                    intersection.time_of_impact,
                ));
            }
        }
        None
    }

    /// Cast a ray and return **all** hits up to `max_toi`, sorted by distance.
    ///
    /// Each hit is `(entity_uuid, hit_x, hit_y, normal_x, normal_y, toi)`.
    pub fn raycast_all(
        &self,
        origin: na::Point2<f32>,
        direction: na::Vector2<f32>,
        max_toi: f32,
        exclude_uuid: Option<u64>,
    ) -> Vec<(u64, f32, f32, f32, f32, f32)> {
        let ray = rapier2d::geometry::Ray::new(origin, direction);
        let predicate = |handle: ColliderHandle, _collider: &Collider| {
            if let Some(exclude) = exclude_uuid {
                if let Some(&uuid) = self.collider_to_uuid.get(&handle) {
                    return uuid != exclude;
                }
            }
            true
        };
        let filter = QueryFilter::default().predicate(&predicate);

        let mut hits = Vec::new();
        self.query_pipeline.intersections_with_ray(
            &self.bodies,
            &self.colliders,
            &ray,
            max_toi,
            true,
            filter,
            |collider_handle, intersection| {
                if let Some(&uuid) = self.collider_to_uuid.get(&collider_handle) {
                    let hit_point = ray.point_at(intersection.time_of_impact);
                    hits.push((
                        uuid,
                        hit_point.x,
                        hit_point.y,
                        intersection.normal.x,
                        intersection.normal.y,
                        intersection.time_of_impact,
                    ));
                }
                true // continue iterating
            },
        );
        hits.sort_by(|a, b| a.5.partial_cmp(&b.5).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }
    // -----------------------------------------------------------------
    // Shape overlap queries
    // -----------------------------------------------------------------

    /// Find all entities whose colliders contain the given point.
    pub fn point_query(&self, point: na::Point2<f32>) -> Vec<u64> {
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
    pub fn aabb_query(&self, min: na::Point2<f32>, max: na::Point2<f32>) -> Vec<u64> {
        let aabb = rapier2d::parry::bounding_volume::Aabb::new(
            na::Point2::new(min.x, min.y),
            na::Point2::new(max.x, max.y),
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
    pub fn shape_overlap(
        &self,
        position: &na::Isometry2<f32>,
        shape: &dyn rapier2d::parry::shape::Shape,
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

    /// Create a revolute (hinge) joint between two bodies.
    /// Returns the rapier joint handle.
    pub fn create_revolute_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Point2<f32>,
        anchor_b: na::Point2<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier2d::dynamics::RevoluteJointBuilder::new()
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Create a fixed joint between two bodies (locks relative transform).
    pub fn create_fixed_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Isometry2<f32>,
        anchor_b: na::Isometry2<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier2d::dynamics::FixedJointBuilder::new()
            .local_frame1(anchor_a)
            .local_frame2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Create a prismatic (slider) joint between two bodies.
    pub fn create_prismatic_joint(
        &mut self,
        body_a: RigidBodyHandle,
        body_b: RigidBodyHandle,
        anchor_a: na::Point2<f32>,
        anchor_b: na::Point2<f32>,
        axis: na::UnitVector2<f32>,
    ) -> ImpulseJointHandle {
        let joint = rapier2d::dynamics::PrismaticJointBuilder::new(axis)
            .local_anchor1(anchor_a)
            .local_anchor2(anchor_b)
            .build();
        self.impulse_joints.insert(body_a, body_b, joint, true)
    }

    /// Remove a joint by handle.
    pub fn remove_joint(&mut self, handle: ImpulseJointHandle) {
        self.impulse_joints.remove(handle, true);
    }
}

// ---------------------------------------------------------------------------
// Collision event collector
// ---------------------------------------------------------------------------

/// Collects collision events from the rapier physics pipeline.
///
/// Uses `Mutex` for interior mutability since rapier's `EventHandler` trait
/// requires `Sync`. Events are drained after each physics step.
struct CollisionCollector {
    /// `(collider1, collider2, started)` — `started` = true for begin, false for end.
    events: Mutex<Vec<(ColliderHandle, ColliderHandle, bool)>>,
}

impl CollisionCollector {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl EventHandler for CollisionCollector {
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
