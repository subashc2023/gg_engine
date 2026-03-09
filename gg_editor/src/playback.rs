use gg_engine::prelude::*;

use super::camera_controller::NativeCameraFollow;
use super::physics_player::PhysicsPlayer;
use super::{GGEditor, SceneState};

#[cfg(feature = "lua-scripting")]
use gg_engine::scene::LuaScriptComponent;

impl GGEditor {
    pub(super) fn on_scene_play(&mut self) {
        self.validate_scene();
        self.playback.scene_state = SceneState::Play;
        let runtime_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, runtime_scene);
        self.playback.editor_scene = Some(editor_scene);

        // Attach native scripts to known entities by tag name.
        // NativeScriptComponent is runtime-only (not serialized), so we bind
        // them here on the runtime copy before starting.
        self.attach_native_scripts();

        self.scene.on_runtime_start();
    }

    pub(super) fn on_scene_simulate(&mut self) {
        self.validate_scene();
        self.playback.scene_state = SceneState::Simulate;
        let sim_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, sim_scene);
        self.playback.editor_scene = Some(editor_scene);
        self.scene.on_simulation_start();
    }

    /// Validate the current scene and populate `scene_warnings`.
    pub(super) fn validate_scene(&mut self) {
        let mut warnings = Vec::new();

        // 1. Check for primary camera.
        if self.scene.get_primary_camera_entity().is_none() {
            warnings.push(
                "No primary camera found. The scene will not render correctly at runtime."
                    .to_string(),
            );
        }

        // Iterate all entities once, checking component-based validations.
        let entities = self.scene.each_entity_with_tag();
        for (entity, tag) in &entities {
            let entity = *entity;

            // 2. Orphaned colliders (collider without a RigidBody2D).
            if self.scene.has_component::<BoxCollider2DComponent>(entity)
                && !self.scene.has_component::<RigidBody2DComponent>(entity)
            {
                warnings.push(format!(
                    "Entity '{}' has BoxCollider2D but no RigidBody2D.",
                    tag
                ));
            }
            if self
                .scene
                .has_component::<CircleCollider2DComponent>(entity)
                && !self.scene.has_component::<RigidBody2DComponent>(entity)
            {
                warnings.push(format!(
                    "Entity '{}' has CircleCollider2D but no RigidBody2D.",
                    tag
                ));
            }

            // 3. Missing texture assets.
            if let Some(sr) = self.scene.get_component::<SpriteRendererComponent>(entity) {
                let raw = sr.texture_handle.raw();
                if raw != 0 {
                    if let Some(ref am) = self.project_state.asset_manager {
                        let handle = Uuid::from_raw(raw);
                        if am.get_metadata(&handle).is_none() {
                            warnings.push(format!(
                                "Entity '{}' references a missing texture asset.",
                                tag
                            ));
                        }
                    }
                }
            }

            // 4. Missing audio assets.
            if let Some(ac) = self.scene.get_component::<AudioSourceComponent>(entity) {
                let raw = ac.audio_handle.raw();
                if raw != 0 {
                    if let Some(ref am) = self.project_state.asset_manager {
                        let handle = Uuid::from_raw(raw);
                        if am.get_metadata(&handle).is_none() {
                            warnings.push(format!(
                                "Entity '{}' references a missing audio asset.",
                                tag
                            ));
                        }
                    }
                }
            }
        }

        // Log warnings.
        for w in &warnings {
            warn!("[Scene Validation] {}", w);
        }

        self.scene_ctx.warnings = warnings;
    }

    pub(super) fn on_scene_stop(&mut self) {
        match self.playback.scene_state {
            SceneState::Play => self.scene.on_runtime_stop(),
            SceneState::Simulate => self.scene.on_simulation_stop(),
            SceneState::Edit => return,
        }

        self.playback.scene_state = SceneState::Edit;
        self.playback.paused = false;
        self.playback.step_frames = 0;

        // Discard any in-progress gizmo drag — the runtime scene is about to be
        // replaced with the editor snapshot, so pushing an undo entry here would
        // create an orphaned snapshot of the runtime scene.
        if self.gizmo_state.editing {
            self.undo_system.cancel_edit();
            self.gizmo_state.editing = false;
        }
        self.tilemap_paint.painting_in_progress = false;
        self.tilemap_paint.painted_this_stroke.clear();

        if let Some(editor_scene) = self.playback.editor_scene.take() {
            let old = std::mem::replace(&mut self.scene, editor_scene);
            self.scene_ctx.pending_drop_scenes.push(old);
            self.selection.clear();

            let (w, h) = self.viewport.size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
        }
    }

    pub(super) fn on_scene_pause(&mut self) {
        if self.playback.scene_state == SceneState::Edit {
            return;
        }
        self.playback.paused = !self.playback.paused;
        if !self.playback.paused {
            self.playback.step_frames = 0;
        }
    }

    pub(super) fn on_scene_step(&mut self) {
        if !self.playback.paused {
            return;
        }
        self.playback.step_frames = 1;
    }

    /// Attach known native scripts to entities by tag name.
    ///
    /// Since `NativeScriptComponent` is runtime-only (not serialized to
    /// `.ggscene` files), we bind them here on the runtime scene copy.
    /// This lets `.ggscene` files work with native scripts — the editor
    /// recognizes entity names and attaches the correct script.
    fn attach_native_scripts(&mut self) {
        // Bind PhysicsPlayer (WASD+Space) to "Player" or "Native Player"
        // if they don't already have a script.
        for name in &["Player", "Native Player"] {
            if let Some((entity, _)) = self.scene.find_entity_by_name(name) {
                let has_lua = {
                    #[cfg(feature = "lua-scripting")]
                    {
                        self.scene.has_component::<LuaScriptComponent>(entity)
                    }
                    #[cfg(not(feature = "lua-scripting"))]
                    {
                        false
                    }
                };
                if !has_lua && !self.scene.has_component::<NativeScriptComponent>(entity) {
                    self.scene
                        .add_component(entity, NativeScriptComponent::bind::<PhysicsPlayer>());
                }
            }
        }

        // Bind NativeCameraFollow to "Camera" if it doesn't have a Lua script.
        if let Some((camera, _)) = self.scene.find_entity_by_name("Camera") {
            let has_lua = {
                #[cfg(feature = "lua-scripting")]
                {
                    self.scene.has_component::<LuaScriptComponent>(camera)
                }
                #[cfg(not(feature = "lua-scripting"))]
                {
                    false
                }
            };
            if !has_lua && !self.scene.has_component::<NativeScriptComponent>(camera) {
                self.scene
                    .add_component(camera, NativeScriptComponent::bind::<NativeCameraFollow>());
            }
        }
    }
}
