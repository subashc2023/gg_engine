use gg_engine::prelude::*;

/// Native script for WASD force-based movement with jump.
/// Mirrors the Lua `physics_player.lua`.
pub(crate) struct PhysicsPlayer {
    move_speed: f32,
    move_accel: f32,
    jump_impulse: f32,
}

impl Default for PhysicsPlayer {
    fn default() -> Self {
        Self {
            move_speed: 5.0,
            move_accel: 50.0,
            jump_impulse: 5.0,
        }
    }
}

impl NativeScript for PhysicsPlayer {
    fn on_create(&mut self, entity: Entity, _scene: &mut Scene) {
        info!("PhysicsPlayer (native) created (entity {})", entity.id());
    }

    fn on_fixed_update(&mut self, entity: Entity, scene: &mut Scene, _dt: Timestep, input: &Input) {
        if !scene.has_component::<RigidBody2DComponent>(entity) {
            return;
        }

        let vel = scene.get_linear_velocity(entity).unwrap_or(Vec2::ZERO);

        // Horizontal movement: force toward target velocity.
        let target_vx = if input.is_key_pressed(KeyCode::A) {
            -self.move_speed
        } else if input.is_key_pressed(KeyCode::D) {
            self.move_speed
        } else {
            0.0
        };
        let force_x = (target_vx - vel.x) * self.move_accel;
        scene.apply_force(entity, Vec2::new(force_x, 0.0));

        // Ground check: short downward raycast from entity center.
        let grounded = if let Some(tc) = scene.get_component::<TransformComponent>(entity) {
            let pos = Vec2::new(tc.translation.x, tc.translation.y);
            scene
                .raycast(pos, Vec2::new(0.0, -1.0), 0.55, Some(entity))
                .is_some()
        } else {
            false
        };

        // Jump when grounded. Raycast ground check prevents spam — after the
        // impulse the player rises past the skin distance within one step.
        if input.is_key_pressed(KeyCode::Space) && grounded {
            scene.apply_impulse(entity, Vec2::new(0.0, self.jump_impulse));
        }
    }
}
