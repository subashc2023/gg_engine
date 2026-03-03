use gg_engine::prelude::*;

pub(crate) struct CameraController {
    speed: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self { speed: 5.0 }
    }
}

impl NativeScript for CameraController {
    fn on_create(&mut self, entity: Entity, _scene: &mut Scene) {
        info!("CameraController created (entity {})", entity.id());
    }

    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {
        if let Some(mut transform) = scene.get_component_mut::<TransformComponent>(entity) {
            let speed = self.speed * dt.seconds();
            if input.is_key_pressed(KeyCode::A) {
                transform.translation.x -= speed;
            }
            if input.is_key_pressed(KeyCode::D) {
                transform.translation.x += speed;
            }
            if input.is_key_pressed(KeyCode::W) {
                transform.translation.y += speed;
            }
            if input.is_key_pressed(KeyCode::S) {
                transform.translation.y -= speed;
            }
        }
    }
}
