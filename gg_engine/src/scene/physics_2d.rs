use rapier2d::na;
use rapier2d::prelude::*;

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
        }
    }

    /// Accumulate `dt` and step the simulation in fixed increments.
    ///
    /// Returns `true` if at least one physics step was taken (i.e. transforms
    /// may have changed and should be written back).
    pub(crate) fn step(&mut self, dt: f32) -> bool {
        self.accumulator += dt.min(MAX_FRAME_DT);

        let mut stepped = false;
        while self.accumulator >= FIXED_TIMESTEP {
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
            stepped = true;
        }
        stepped
    }
}
