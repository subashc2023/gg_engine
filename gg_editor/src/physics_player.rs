use gg_engine::prelude::*;

/// Native script for WASD impulse-based movement with velocity clamping
/// and Space to jump. Mirrors the Lua `physics_player.lua`.
pub(crate) struct PhysicsPlayer {
    move_speed: f32,
    jump_impulse: f32,
    max_speed: f32,
}

impl Default for PhysicsPlayer {
    fn default() -> Self {
        Self {
            move_speed: 1.0,
            jump_impulse: 5.0,
            max_speed: 10.0,
        }
    }
}

impl NativeScript for PhysicsPlayer {
    fn on_create(&mut self, entity: Entity, _scene: &mut Scene) {
        info!("PhysicsPlayer (native) created (entity {})", entity.id());
    }

    fn on_update(&mut self, entity: Entity, scene: &mut Scene, _dt: Timestep, input: &Input) {
        if !scene.has_component::<RigidBody2DComponent>(entity) {
            return;
        }

        // Horizontal movement via impulses.
        if input.is_key_pressed(KeyCode::A) {
            scene.apply_impulse(entity, Vec2::new(-self.move_speed, 0.0));
        }
        if input.is_key_pressed(KeyCode::D) {
            scene.apply_impulse(entity, Vec2::new(self.move_speed, 0.0));
        }

        // Clamp horizontal velocity.
        if let Some(vel) = scene.get_linear_velocity(entity) {
            if vel.x > self.max_speed {
                scene.set_linear_velocity(entity, Vec2::new(self.max_speed, vel.y));
            } else if vel.x < -self.max_speed {
                scene.set_linear_velocity(entity, Vec2::new(-self.max_speed, vel.y));
            }
        }

        // Jump (only when roughly grounded).
        if input.is_key_pressed(KeyCode::Space) {
            if let Some(vel) = scene.get_linear_velocity(entity) {
                if vel.y.abs() < 0.1 {
                    scene.apply_impulse(entity, Vec2::new(0.0, self.jump_impulse));
                }
            }
        }
    }
}
