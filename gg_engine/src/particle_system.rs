use glam::{Vec2, Vec3, Vec4};

use crate::renderer::Renderer;
use crate::timestep::Timestep;

// ---------------------------------------------------------------------------
// Minimal xorshift32 PRNG — good enough for visual particle randomness.
// ---------------------------------------------------------------------------

struct Rng {
    state: u32,
}

impl Rng {
    fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0xDEAD_BEEF } else { seed },
        }
    }

    fn from_time() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let stack_addr = &nanos as *const _ as usize as u32;
        Self::new(nanos ^ stack_addr)
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    /// Returns a float in [0.0, 1.0).
    fn random(&mut self) -> f32 {
        (self.next_u32() as f64 / u32::MAX as f64) as f32
    }

    /// Returns a float in [-1.0, 1.0).
    fn random_signed(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------------------
// ParticleProps — user-facing emission configuration.
// ---------------------------------------------------------------------------

/// Configuration for emitting particles. Pass to [`ParticleSystem::emit`].
///
/// `velocity_variation` and `size_variation` add random spread — the actual
/// value becomes `base + random_in_minus1_to_1 * variation`.
#[derive(Clone)]
pub struct ParticleProps {
    pub position: Vec2,
    pub velocity: Vec2,
    pub velocity_variation: Vec2,
    pub color_begin: Vec4,
    pub color_end: Vec4,
    pub size_begin: f32,
    pub size_end: f32,
    pub size_variation: f32,
    pub lifetime: f32,
}

impl Default for ParticleProps {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            velocity: Vec2::new(0.0, 0.0),
            velocity_variation: Vec2::new(3.0, 3.0),
            color_begin: Vec4::new(0.98, 0.33, 0.16, 1.0),
            color_end: Vec4::new(0.98, 0.84, 0.16, 0.0),
            size_begin: 0.1,
            size_end: 0.0,
            size_variation: 0.05,
            lifetime: 5.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal particle state.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Particle {
    position: Vec2,
    velocity: Vec2,
    rotation: f32,
    rotation_speed: f32,
    color_begin: Vec4,
    color_end: Vec4,
    size_begin: f32,
    size_end: f32,
    lifetime: f32,
    life_remaining: f32,
    active: bool,
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            velocity: Vec2::ZERO,
            rotation: 0.0,
            rotation_speed: 0.0,
            color_begin: Vec4::ONE,
            color_end: Vec4::ONE,
            size_begin: 1.0,
            size_end: 0.0,
            lifetime: 1.0,
            life_remaining: 0.0,
            active: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ParticleSystem — pool-based 2D particle system.
// ---------------------------------------------------------------------------

/// A pool-based 2D particle system.
///
/// Particles are stored in a fixed-size pool. [`ParticleSystem::emit`] cycles
/// through the pool, overwriting the oldest particles when full. Update and
/// render are split across [`ParticleSystem::on_update`] and
/// [`ParticleSystem::on_render`] to match the engine's `&mut self` / `&self`
/// lifecycle.
pub struct ParticleSystem {
    pool: Vec<Particle>,
    pool_index: usize,
    rng: Rng,
}

impl ParticleSystem {
    /// Create a particle system with the given pool capacity.
    pub fn new(max_particles: usize) -> Self {
        Self {
            pool: vec![Particle::default(); max_particles],
            pool_index: 0,
            rng: Rng::from_time(),
        }
    }

    /// Emit a single particle with random variation applied to `props`.
    pub fn emit(&mut self, props: &ParticleProps) {
        let particle = &mut self.pool[self.pool_index];

        particle.active = true;
        particle.position = props.position;
        particle.rotation = self.rng.random() * std::f32::consts::TAU;
        particle.rotation_speed = self.rng.random_signed() * 4.0;

        // Polar sampling for circular spread (sqrt for uniform area coverage).
        let angle = self.rng.random() * std::f32::consts::TAU;
        let radius = self.rng.random().sqrt();
        particle.velocity = Vec2::new(
            props.velocity.x + angle.cos() * radius * props.velocity_variation.x,
            props.velocity.y + angle.sin() * radius * props.velocity_variation.y,
        );

        particle.color_begin = props.color_begin;
        particle.color_end = props.color_end;

        particle.size_begin =
            (props.size_begin + self.rng.random_signed() * props.size_variation).max(0.01);
        particle.size_end = props.size_end;

        particle.lifetime = props.lifetime;
        particle.life_remaining = props.lifetime;

        self.pool_index = (self.pool_index + 1) % self.pool.len();
    }

    /// Update all active particles. Call from `Application::on_update`.
    pub fn on_update(&mut self, dt: Timestep) {
        let dt_s = dt.seconds();
        for particle in &mut self.pool {
            if !particle.active {
                continue;
            }

            particle.life_remaining -= dt_s;
            if particle.life_remaining <= 0.0 {
                particle.active = false;
                continue;
            }

            // Velocity damping — particles slow down over time, feels organic.
            particle.velocity *= 1.0 - 2.0 * dt_s;
            particle.position += particle.velocity * dt_s;
            particle.rotation += particle.rotation_speed * dt_s;
        }
    }

    /// Render all active particles. Call from `Application::on_render`.
    ///
    /// Uses `draw_rotated_quad` — each particle is a rotated, colored quad
    /// at z = -0.1 (in front of z=0 scene geometry). Color and size are
    /// interpolated from the particle's life fraction.
    pub fn on_render(&self, renderer: &Renderer) {
        for particle in &self.pool {
            if !particle.active {
                continue;
            }

            let life = particle.life_remaining / particle.lifetime;
            let color = particle.color_begin.lerp(particle.color_end, 1.0 - life);
            let size = particle.size_begin + (particle.size_end - particle.size_begin) * (1.0 - life);

            if size <= 0.0 {
                continue;
            }

            // Newer particles (life~1) get more negative z (closer to camera in LH),
            // dying particles (life~0) sit behind. Avoids z-fighting/random draw order.
            let z = -0.1 - life * 0.05;
            let position = Vec3::new(particle.position.x, particle.position.y, z);
            renderer.draw_particle(&position, size, particle.rotation, color);
        }
    }

    /// Number of currently active particles.
    pub fn active_count(&self) -> usize {
        self.pool.iter().filter(|p| p.active).count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_produces_values_in_range() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let v = rng.random();
            assert!((0.0..1.0).contains(&v), "random() out of range: {v}");
        }
    }

    #[test]
    fn rng_signed_range() {
        let mut rng = Rng::new(42);
        for _ in 0..1000 {
            let v = rng.random_signed();
            assert!((-1.0..1.0).contains(&v), "random_signed() out of range: {v}");
        }
    }

    #[test]
    fn rng_zero_seed_uses_fallback() {
        let mut rng = Rng::new(0);
        let _ = rng.random();
    }

    #[test]
    fn emit_activates_particle() {
        let mut ps = ParticleSystem::new(10);
        assert_eq!(ps.active_count(), 0);
        ps.emit(&ParticleProps::default());
        assert_eq!(ps.active_count(), 1);
    }

    #[test]
    fn particles_deactivate_after_lifetime() {
        let mut ps = ParticleSystem::new(10);
        let props = ParticleProps {
            lifetime: 0.5,
            ..Default::default()
        };
        ps.emit(&props);
        assert_eq!(ps.active_count(), 1);

        ps.on_update(Timestep::from_seconds(0.6));
        assert_eq!(ps.active_count(), 0);
    }

    #[test]
    fn pool_wraps_around() {
        let mut ps = ParticleSystem::new(3);
        let props = ParticleProps::default();
        for _ in 0..5 {
            ps.emit(&props);
        }
        assert_eq!(ps.active_count(), 3);
    }

    #[test]
    fn default_props_are_sensible() {
        let props = ParticleProps::default();
        assert!(props.lifetime > 0.0);
        assert!(props.size_begin > 0.0);
        assert!(props.color_begin.w > 0.0);
    }
}
