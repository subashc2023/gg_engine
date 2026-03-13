use super::{
    AudioCategory, AudioListenerComponent, AudioSourceComponent, CameraComponent, Entity,
    IdComponent, Scene, TagComponent, TransformComponent,
};

impl Scene {
    // -----------------------------------------------------------------
    // Audio lifecycle
    // -----------------------------------------------------------------

    /// Create the audio engine and play sounds with `play_on_start`.
    pub(super) fn on_audio_start(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_audio_start");
        let engine = match super::audio::AudioEngine::new() {
            Some(e) => e,
            None => return,
        };
        self.audio_engine = Some(engine);

        // Collect entities that should auto-play.
        let auto_play: Vec<(u64, String, f32, f32, bool, bool, AudioCategory)> = self
            .world
            .query::<(hecs::Entity, &IdComponent, &AudioSourceComponent)>()
            .iter()
            .filter(|(_, _, asc)| asc.play_on_start && asc.resolved_path.is_some())
            .map(|(_, id, asc)| {
                (
                    id.id.raw(),
                    asc.resolved_path.clone().unwrap(),
                    asc.volume,
                    asc.pitch,
                    asc.looping,
                    asc.streaming,
                    asc.category,
                )
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, path, volume, pitch, looping, streaming, category) in auto_play {
                let effective = volume
                    * self.core.category_volumes[category as usize]
                    * self.core.master_volume;
                engine.play_sound(uuid, &path, effective, pitch, looping, streaming);
            }
        }
    }

    /// Stop all sounds and drop the audio engine.
    pub(super) fn on_audio_stop(&mut self) {
        if let Some(ref mut engine) = self.audio_engine {
            engine.stop_all();
        }
        self.audio_engine = None;
    }

    /// Resolve audio handles to file paths via the asset manager.
    pub fn resolve_audio_handles(&mut self, asset_manager: &mut crate::asset::EditorAssetManager) {
        let needs_resolve: Vec<(hecs::Entity, crate::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &AudioSourceComponent)>()
            .iter()
            .filter_map(|(handle, asc)| {
                if asc.audio_handle.raw() != 0 && asc.resolved_path.is_none() {
                    Some((handle, asc.audio_handle))
                } else {
                    None
                }
            })
            .collect();

        for (handle, asset_handle) in needs_resolve {
            if let Some(abs_path) = asset_manager.get_absolute_path(&asset_handle) {
                if abs_path.exists() {
                    let path_str = abs_path.to_string_lossy().to_string();
                    if let Ok(mut asc) = self.world.get::<&mut AudioSourceComponent>(handle) {
                        asc.resolved_path = Some(path_str);
                    }
                }
            }
        }
    }

    /// Find all entities that reference the given asset handle.
    ///
    /// Scans `SpriteRendererComponent::texture_handle`,
    /// `TilemapComponent::texture_handle`, and
    /// `AudioSourceComponent::audio_handle`.
    ///
    /// Returns a list of `(entity_name, component_kind)` pairs describing
    /// each reference, e.g. `("Player", "Sprite")`.
    pub fn find_asset_references(
        &self,
        asset_handle: crate::uuid::Uuid,
    ) -> Vec<(String, &'static str)> {
        let mut refs = Vec::new();

        for (tag, sprite) in self
            .world
            .query::<(&TagComponent, &super::SpriteRendererComponent)>()
            .iter()
        {
            if sprite.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "Sprite"));
            }
        }

        for (tag, tilemap) in self
            .world
            .query::<(&TagComponent, &super::TilemapComponent)>()
            .iter()
        {
            if tilemap.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "Tilemap"));
            }
        }

        for (tag, asc) in self
            .world
            .query::<(&TagComponent, &AudioSourceComponent)>()
            .iter()
        {
            if asc.audio_handle == asset_handle {
                refs.push((tag.tag.clone(), "Audio"));
            }
        }

        for (tag, mesh) in self
            .world
            .query::<(&TagComponent, &super::MeshRendererComponent)>()
            .iter()
        {
            if mesh.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "Mesh"));
            }
        }

        for (tag, img) in self
            .world
            .query::<(&TagComponent, &super::UIImageComponent)>()
            .iter()
        {
            if img.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "UIImage"));
            }
        }

        refs
    }

    // -----------------------------------------------------------------
    // Playback control
    // -----------------------------------------------------------------

    /// Compute effective volume combining entity, category, and master volumes.
    fn effective_volume(&self, entity_volume: f32, category: AudioCategory) -> f32 {
        entity_volume * self.category_volumes[category as usize] * self.master_volume
    }

    /// Play audio for an entity (used by Lua scripts).
    pub fn play_entity_sound(&mut self, entity: Entity) {
        let (uuid, path, effective_vol, pitch, looping, streaming) = {
            let id = match self.get_component::<IdComponent>(entity) {
                Some(id) => id.id.raw(),
                None => return,
            };
            let asc = match self.get_component::<AudioSourceComponent>(entity) {
                Some(a) => a,
                None => return,
            };
            let path = match &asc.resolved_path {
                Some(p) => p.clone(),
                None => return,
            };
            let vol = self.effective_volume(asc.volume, asc.category);
            (id, path, vol, asc.pitch, asc.looping, asc.streaming)
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.play_sound(uuid, &path, effective_vol, pitch, looping, streaming);
        }
    }

    /// Pause audio for an entity (used by Lua scripts).
    pub fn pause_entity_sound(&mut self, entity: Entity) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.pause_sound(uuid);
        }
    }

    /// Resume audio for an entity (used by Lua scripts).
    pub fn resume_entity_sound(&mut self, entity: Entity) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.resume_sound(uuid);
        }
    }

    /// Stop audio for an entity (used by Lua scripts).
    pub fn stop_entity_sound(&mut self, entity: Entity) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.stop_sound(uuid);
        }
    }

    /// Set audio volume for an entity (used by Lua scripts).
    /// Accepts linear amplitude (0.0–1.0), converted to dB internally.
    pub fn set_entity_volume(&mut self, entity: Entity, volume: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            let volume_db = super::audio::linear_to_db(volume);
            engine.set_volume(uuid, volume_db);
        }
    }

    /// Set panning for an entity (used by Lua scripts).
    /// -1.0 = hard left, 0.0 = center, 1.0 = hard right.
    pub fn set_entity_panning(&mut self, entity: Entity, panning: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_panning(uuid, panning);
        }
    }

    // -----------------------------------------------------------------
    // Fade in/out
    // -----------------------------------------------------------------

    /// Fade in an entity's audio from silence.
    /// If the entity has paused sounds, resumes them with a fade.
    /// If no sounds are active, plays the entity's audio with a fade from silence.
    pub fn fade_in_entity_sound(&mut self, entity: Entity, duration_secs: f32) {
        let (uuid, effective_vol, maybe_play) = {
            let id = match self.get_component::<IdComponent>(entity) {
                Some(id) => id.id.raw(),
                None => return,
            };
            let asc = match self.get_component::<AudioSourceComponent>(entity) {
                Some(a) => a,
                None => return,
            };
            let vol = self.effective_volume(asc.volume, asc.category);
            let play_info = asc
                .resolved_path
                .as_ref()
                .map(|p| (p.clone(), asc.pitch, asc.looping, asc.streaming));
            (id, vol, play_info)
        };

        let target_db = super::audio::linear_to_db(effective_vol);

        if let Some(ref mut engine) = self.audio_engine {
            // Try to fade in existing sounds first.
            let handled = engine.fade_in(uuid, target_db, duration_secs);

            // If no active sounds exist, play with fade from silence.
            if !handled {
                if let Some((path, pitch, looping, streaming)) = maybe_play {
                    engine.play_sound_fade(
                        uuid,
                        &path,
                        effective_vol,
                        pitch,
                        looping,
                        streaming,
                        duration_secs,
                    );
                }
            }
        }
    }

    /// Fade out and stop an entity's audio.
    pub fn fade_out_entity_sound(&mut self, entity: Entity, duration_secs: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.fade_out_stop(uuid, duration_secs);
        }
    }

    /// Fade an entity's volume to a target level over time.
    /// `target_volume` is linear (0.0–1.0).
    pub fn fade_to_entity_volume(
        &mut self,
        entity: Entity,
        target_volume: f32,
        duration_secs: f32,
    ) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        let target_db = super::audio::linear_to_db(target_volume);
        if let Some(ref mut engine) = self.audio_engine {
            engine.fade_to(uuid, target_db, duration_secs);
        }
    }

    // -----------------------------------------------------------------
    // Master / category volume
    // -----------------------------------------------------------------

    /// Set the global master volume (0.0–1.0).
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
    }

    /// Get the global master volume.
    pub fn get_master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Set volume for a sound category (0.0–1.0).
    pub fn set_category_volume(&mut self, category: AudioCategory, volume: f32) {
        self.category_volumes[category as usize] = volume.clamp(0.0, 1.0);
    }

    /// Get volume for a sound category.
    pub fn get_category_volume(&self, category: AudioCategory) -> f32 {
        self.category_volumes[category as usize]
    }

    // -----------------------------------------------------------------
    // Sound completion detection
    // -----------------------------------------------------------------

    /// Drain entities whose sounds have finished naturally and dispatch
    /// `on_sound_finished()` Lua callbacks.
    #[cfg(feature = "lua-scripting")]
    pub(super) fn dispatch_sound_finished_events(&mut self) {
        use super::lua_ops::ScriptEngineGuard;
        use super::script_glue::SceneScriptContext;

        let completed: Vec<u64> = match self.audio_engine.as_mut() {
            Some(engine) => engine.drain_completed(),
            None => return,
        };

        if completed.is_empty() {
            return;
        }

        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let scene_ptr: *mut Scene = self;
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for uuid in &completed {
            guard.engine_mut().call_entity_on_sound_finished(*uuid);
        }

        // Guard drop restores engine and cleans up SceneScriptContext.
    }

    /// Drain completed sounds without Lua dispatch (when lua-scripting is disabled).
    #[cfg(not(feature = "lua-scripting"))]
    pub(super) fn dispatch_sound_finished_events(&mut self) {
        if let Some(ref mut engine) = self.audio_engine {
            engine.drain_completed();
        }
    }

    // -----------------------------------------------------------------
    // Spatial audio
    // -----------------------------------------------------------------

    /// Update spatial audio: compute panning and distance attenuation for
    /// all spatial audio sources based on the listener position.
    ///
    /// If an entity has an active [`AudioListenerComponent`], its position is
    /// used as the listener. Otherwise, the primary camera position is used.
    pub fn update_spatial_audio(&mut self) {
        if self.audio_engine.is_none() {
            return;
        }

        // Also drain completed sounds (fires on_sound_finished callbacks).
        self.dispatch_sound_finished_events();

        // Prefer explicit AudioListenerComponent, fall back to primary camera.
        let active_listeners: Vec<glam::Vec2> = self
            .world
            .query::<(&AudioListenerComponent, &TransformComponent)>()
            .iter()
            .filter(|(al, _)| al.active)
            .map(|(_, tf)| tf.translation.truncate())
            .collect();
        if active_listeners.len() > 1 {
            log::warn!(
                "Multiple active AudioListenerComponents ({}) — using last found. \
                 Disable extras to ensure deterministic behavior.",
                active_listeners.len()
            );
        }
        let listener_pos = active_listeners
            .into_iter()
            .last()
            .or_else(|| {
                self.world
                    .query::<(&CameraComponent, &TransformComponent)>()
                    .iter()
                    .filter(|(cam, _)| cam.primary)
                    .map(|(_, tf)| tf.translation.truncate())
                    .last()
            })
            .unwrap_or(glam::Vec2::ZERO);

        // Read master + category volumes for effective volume computation.
        let master = self.master_volume;
        let cat_vols = self.category_volumes;

        // Collect spatial updates (uuid, panning, effective_volume).
        let updates: Vec<(u64, f32, f32)> = self
            .world
            .query::<(&IdComponent, &AudioSourceComponent, &TransformComponent)>()
            .iter()
            .filter(|(_, asc, _)| asc.spatial)
            .map(|(id, asc, tf)| {
                let entity_pos = tf.translation.truncate();
                let delta = entity_pos - listener_pos;
                let dist = delta.length();
                // Panning: proportional to horizontal offset relative to max_distance.
                let panning = (delta.x / asc.max_distance.max(0.01)).clamp(-1.0, 1.0);
                // Attenuation: linear falloff between min and max distance.
                // Expressed in decibels: 0 dB at min_distance, -60 dB (silence) at max_distance.
                let atten_db = if dist <= asc.min_distance {
                    0.0
                } else if dist >= asc.max_distance {
                    -60.0
                } else {
                    let t = (dist - asc.min_distance) / (asc.max_distance - asc.min_distance);
                    -60.0 * t
                };
                // Combine entity volume × category × master, then convert to dB.
                let effective_linear = asc.volume * cat_vols[asc.category as usize] * master;
                let volume_db = super::audio::linear_to_db(effective_linear);
                let effective_volume = volume_db + atten_db;
                (id.id.raw(), panning, effective_volume)
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, panning, volume) in updates {
                engine.set_panning(uuid, panning);
                engine.set_volume(uuid, volume);
            }
        }
    }
}
