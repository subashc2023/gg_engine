use rapier2d::na;
use rapier2d::prelude::*;
use std::collections::HashMap;

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
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    pub(crate) bodies: RigidBodySet,
    pub(crate) colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    /// Leftover time from the previous frame, carried forward for the next
    /// fixed-step accumulation.
    accumulator: f32,
    /// Pre-step positions/rotations for interpolation (position_x, position_y, angle).
    prev_transforms: HashMap<RigidBodyHandle, (f32, f32, f32)>,
}

impl PhysicsWorld2D {
    pub(crate) fn new(gravity_x: f32, gravity_y: f32) -> Self {
        let mut params = IntegrationParameters::default();
        params.dt = FIXED_TIMESTEP;
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
            &(),
        );
        self.accumulator -= FIXED_TIMESTEP;
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
