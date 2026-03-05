use rapier2d::na;
use rapier2d::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// Fixed physics timestep (1/60 s ≈ 16.67 ms).
const FIXED_TIMESTEP: f32 = 1.0 / 60.0;

/// Maximum frame delta fed into the accumulator (caps at 250 ms to prevent
/// a "spiral of death" after long hitches).
const MAX_FRAME_DT: f32 = 0.25;

/// Bundles all rapier2d simulation state needed for a single physics world.
pub(crate) struct PhysicsWorld2D {
    gravity: na::Vector2<f32>,
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
    /// Pre-step positions/rotations for interpolation (position_x, position_y, angle).
    prev_transforms: HashMap<RigidBodyHandle, (f32, f32, f32)>,
    /// Maps collider handles to entity UUIDs for collision event dispatch.
    pub(crate) collider_to_uuid: HashMap<ColliderHandle, u64>,
    /// Collision event collector — filled during physics step, drained after.
    collision_collector: CollisionCollector,
}

impl PhysicsWorld2D {
    pub(crate) fn new(gravity_x: f32, gravity_y: f32) -> Self {
        let params = IntegrationParameters { dt: FIXED_TIMESTEP, ..Default::default() };
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
            accumulator: 0.0,
            prev_transforms: HashMap::new(),
            collider_to_uuid: HashMap::new(),
            collision_collector: CollisionCollector::new(),
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

    /// Snapshot current body positions as "previous" for interpolation.
    /// Call this *before* `step_once()`.
    pub(crate) fn snapshot_transforms(&mut self) {
        for (handle, body) in self.bodies.iter() {
            let pos = body.translation();
            let angle = body.rotation().angle();
            self.prev_transforms.insert(handle, (pos.x, pos.y, angle));
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
    ///
    /// Returns pairs of `(uuid_a, uuid_b, started)` where `started` is `true`
    /// for collision start and `false` for collision stop.
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

    /// The fixed timestep value (1/60 s).
    pub(crate) fn fixed_timestep(&self) -> f32 {
        FIXED_TIMESTEP
    }

    /// Interpolation alpha: fraction of a timestep remaining in the accumulator.
    /// Ranges from 0.0 (just stepped) to ~1.0 (about to step).
    pub(crate) fn alpha(&self) -> f32 {
        self.accumulator / FIXED_TIMESTEP
    }

    /// Get the pre-step (previous) transform for a body, if available.
    pub(crate) fn prev_transform(&self, handle: RigidBodyHandle) -> Option<(f32, f32, f32)> {
        self.prev_transforms.get(&handle).copied()
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
