use gg_engine::prelude::*;

/// Native camera follow script.
///
/// Finds a target entity by name and locks the camera's XY to it.
/// Demonstrates `Scene::find_entity_by_name` from native code —
/// the Rust equivalent of `Engine.find_entity_by_name` in Lua.
pub(crate) struct NativeCameraFollow {
    pub target_name: String,
    target: Option<Entity>,
}

impl NativeCameraFollow {
    pub fn new(target_name: &str) -> Self {
        Self {
            target_name: target_name.to_string(),
            target: None,
        }
    }
}

impl Default for NativeCameraFollow {
    fn default() -> Self {
        Self::new("Player")
    }
}

impl NativeScript for NativeCameraFollow {
    fn on_create(&mut self, entity: Entity, scene: &mut Scene) {
        // Look up the target by name — O(n) scan, done once.
        if let Some((target_entity, uuid)) = scene.find_entity_by_name(&self.target_name) {
            self.target = Some(target_entity);
            info!(
                "NativeCameraFollow: found '{}' (uuid={}) for camera entity {}",
                self.target_name, uuid, entity.id()
            );
        } else {
            warn!(
                "NativeCameraFollow: target '{}' not found!",
                self.target_name
            );
        }
    }

    fn on_update(&mut self, entity: Entity, scene: &mut Scene, _dt: Timestep, _input: &Input) {
        let target = match self.target {
            Some(t) => t,
            None => return,
        };

        // Read target position.
        let (tx, ty) = {
            if let Some(tc) = scene.get_component::<TransformComponent>(target) {
                (tc.translation.x, tc.translation.y)
            } else {
                return;
            }
        };

        // Set camera position to follow target XY, preserve Z.
        if let Some(mut cam_tc) = scene.get_component_mut::<TransformComponent>(entity) {
            cam_tc.translation.x = tx;
            cam_tc.translation.y = ty;
        }
    }
}
