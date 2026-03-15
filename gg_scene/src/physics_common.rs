/// Fixed physics timestep (1/60 s ≈ 16.67 ms).
pub const FIXED_TIMESTEP: f32 = 1.0 / 60.0;

/// Maximum frame delta fed into the accumulator (caps at 250 ms to prevent
/// a "spiral of death" after long hitches).
pub const MAX_FRAME_DT: f32 = 0.25;

/// Shared fixed-timestep accumulator logic used by both 2D and 3D physics worlds.
pub struct PhysicsTimestep {
    accumulator: f32,
}

impl PhysicsTimestep {
    pub fn new() -> Self {
        Self { accumulator: 0.0 }
    }

    /// Feed frame delta-time into the accumulator (clamped to MAX_FRAME_DT).
    pub fn accumulate(&mut self, dt: f32) {
        self.accumulator += dt.min(MAX_FRAME_DT);
    }

    /// Returns `true` if the accumulator has enough time for another fixed step.
    pub fn can_step(&self) -> bool {
        self.accumulator >= FIXED_TIMESTEP
    }

    /// Consume one fixed timestep from the accumulator.
    pub fn consume_step(&mut self) {
        self.accumulator -= FIXED_TIMESTEP;
    }

    /// The fixed timestep value (1/60 s).
    pub fn fixed_timestep(&self) -> f32 {
        FIXED_TIMESTEP
    }

    /// Interpolation alpha: fraction of a timestep remaining in the accumulator.
    /// Ranges from 0.0 (just stepped) to ~1.0 (about to step).
    pub fn alpha(&self) -> f32 {
        self.accumulator / FIXED_TIMESTEP
    }
}
