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
        struct AutoPlayInfo {
            uuid: u64,
            path: String,
            volume: f32,
            pitch: f32,
            looping: bool,
            streaming: bool,
            category: AudioCategory,
            spatial: bool,
            hrtf: bool,
            min_distance: f32,
            max_distance: f32,
            position: glam::Vec3,
        }

        let auto_play: Vec<AutoPlayInfo> = self
            .world
            .query::<(hecs::Entity, &IdComponent, &AudioSourceComponent, &TransformComponent)>()
            .iter()
            .filter(|(_, _, asc, _)| asc.play_on_start && asc.resolved_path.is_some())
            .map(|(_, id, asc, tf)| AutoPlayInfo {
                uuid: id.id.raw(),
                path: asc.resolved_path.clone().unwrap(),
                volume: asc.volume,
                pitch: asc.pitch,
                looping: asc.looping,
                streaming: asc.streaming,
                category: asc.category,
                spatial: asc.spatial,
                hrtf: asc.hrtf,
                min_distance: asc.min_distance,
                max_distance: asc.max_distance,
                position: tf.translation,
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            // Sync bus volumes from SceneCore state.
            engine.set_master_volume(self.core.master_volume);
            for i in 0..AudioCategory::COUNT {
                if let Some(cat) = AudioCategory::from_index(i) {
                    engine.set_bus_volume(cat, self.core.category_volumes[i]);
                    if self.core.category_muted[i] {
                        engine.mute_bus(cat);
                    }
                }
            }

            for info in &auto_play {
                // Ensure spatial track for spatial sources.
                if info.hrtf || info.spatial {
                    engine.ensure_spatial_track(
                        info.uuid,
                        info.position,
                        info.min_distance,
                        info.max_distance,
                        info.hrtf,
                        info.category,
                    );
                }
                // Entity volume only — bus handles category × master.
                engine.play_sound(
                    info.uuid,
                    &info.path,
                    info.volume,
                    info.pitch,
                    info.looping,
                    info.streaming,
                    info.category,
                );
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

        for (tag, sac) in self
            .world
            .query::<(&TagComponent, &super::SkeletalAnimationComponent)>()
            .iter()
        {
            if sac.mesh_asset == asset_handle {
                refs.push((tag.tag.clone(), "SkeletalAnimation"));
            }
        }

        refs
    }

    // -----------------------------------------------------------------
    // Playback control
    // -----------------------------------------------------------------

    /// Play audio for an entity (used by Lua scripts).
    pub fn play_entity_sound(&mut self, entity: Entity) {
        // Extract all needed data before mutable borrow of audio_engine.
        let info = {
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
            let data = (
                id,
                path,
                asc.volume,
                asc.pitch,
                asc.looping,
                asc.streaming,
                asc.spatial,
                asc.hrtf,
                asc.min_distance,
                asc.max_distance,
                asc.category,
            );
            drop(asc);
            let pos = self
                .get_component::<TransformComponent>(entity)
                .map(|t| t.translation)
                .unwrap_or(glam::Vec3::ZERO);
            (data, pos)
        };
        let ((uuid, path, volume, pitch, looping, streaming, spatial, hrtf, min_d, max_d, category), pos) = info;
        if let Some(ref mut engine) = self.audio_engine {
            if spatial {
                engine.ensure_spatial_track(uuid, pos, min_d, max_d, hrtf, category);
            }
            engine.play_sound(uuid, &path, volume, pitch, looping, streaming, category);
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
        let (uuid, entity_vol, category, maybe_play) = {
            let id = match self.get_component::<IdComponent>(entity) {
                Some(id) => id.id.raw(),
                None => return,
            };
            let asc = match self.get_component::<AudioSourceComponent>(entity) {
                Some(a) => a,
                None => return,
            };
            let vol = asc.volume;
            let cat = asc.category;
            let play_info = asc
                .resolved_path
                .as_ref()
                .map(|p| (p.clone(), asc.pitch, asc.looping, asc.streaming));
            (id, vol, cat, play_info)
        };

        let target_db = super::audio::linear_to_db(entity_vol);

        if let Some(ref mut engine) = self.audio_engine {
            // Try to fade in existing sounds first.
            let handled = engine.fade_in(uuid, target_db, duration_secs);

            // If no active sounds exist, play with fade from silence.
            if !handled {
                if let Some((path, pitch, looping, streaming)) = maybe_play {
                    engine.play_sound_fade(
                        uuid,
                        &path,
                        entity_vol,
                        pitch,
                        looping,
                        streaming,
                        duration_secs,
                        category,
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
    // Master / category volume / bus mute
    // -----------------------------------------------------------------

    /// Set the global master volume (0.0–1.0).
    /// Immediately affects all playing sounds via the kira main track.
    pub fn set_master_volume(&mut self, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        self.master_volume = v;
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_master_volume(v);
        }
    }

    /// Get the global master volume.
    pub fn get_master_volume(&self) -> f32 {
        self.master_volume
    }

    /// Set volume for a sound category (0.0–1.0).
    /// Immediately affects all playing sounds routed to this category's bus.
    pub fn set_category_volume(&mut self, category: AudioCategory, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        self.category_volumes[category as usize] = v;
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_bus_volume(category, v);
        }
    }

    /// Get volume for a sound category.
    pub fn get_category_volume(&self, category: AudioCategory) -> f32 {
        self.category_volumes[category as usize]
    }

    /// Mute a sound category (pauses the bus track, silencing all routed sounds).
    pub fn mute_category(&mut self, category: AudioCategory) {
        self.category_muted[category as usize] = true;
        if let Some(ref mut engine) = self.audio_engine {
            engine.mute_bus(category);
        }
    }

    /// Unmute a sound category (resumes the bus track).
    pub fn unmute_category(&mut self, category: AudioCategory) {
        self.category_muted[category as usize] = false;
        if let Some(ref mut engine) = self.audio_engine {
            engine.unmute_bus(category);
        }
    }

    /// Check if a sound category is muted.
    pub fn is_category_muted(&self, category: AudioCategory) -> bool {
        self.category_muted[category as usize]
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
    /// For HRTF sources, updates kira spatial track positions and binaural
    /// effect parameters (azimuth/elevation).
    ///
    /// For non-HRTF spatial sources, kira spatial tracks handle distance
    /// attenuation and panning natively.
    ///
    /// Non-spatial sources with `spatial: true` but without HRTF fall back
    /// to the legacy manual panning/volume path when no spatial track exists.
    pub fn update_spatial_audio(&mut self) {
        if self.audio_engine.is_none() {
            return;
        }

        // Drain completed sounds (fires on_sound_finished callbacks).
        self.dispatch_sound_finished_events();

        // ----- Determine listener position and orientation -----

        let (listener_pos, listener_rot) = self.find_listener_transform();

        // Update kira listener.
        if let Some(ref mut engine) = self.audio_engine {
            engine.update_listener(listener_pos, listener_rot);
        }

        // Master + category volumes are now handled by kira's spatial
        // track volume, applied when playing the sound (effective_volume).

        // ----- Collect spatial source data -----

        struct SpatialUpdate {
            uuid: u64,
            position: glam::Vec3,
            hrtf: bool,
            min_distance: f32,
            max_distance: f32,
            category: AudioCategory,
        }

        let updates: Vec<SpatialUpdate> = self
            .world
            .query::<(&IdComponent, &AudioSourceComponent, &TransformComponent)>()
            .iter()
            .filter(|(_, asc, _)| asc.spatial)
            .map(|(id, asc, tf)| SpatialUpdate {
                uuid: id.id.raw(),
                position: tf.translation,
                hrtf: asc.hrtf,
                min_distance: asc.min_distance,
                max_distance: asc.max_distance,
                category: asc.category,
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            let listener_inv_rot = listener_rot.inverse();

            for update in &updates {
                // Ensure spatial track exists (as child of category bus).
                engine.ensure_spatial_track(
                    update.uuid,
                    update.position,
                    update.min_distance,
                    update.max_distance,
                    update.hrtf,
                    update.category,
                );
                // Update track position (kira handles distance attenuation).
                engine.update_spatial_position(update.uuid, update.position);

                // Compute listener-relative direction for the binaural effect.
                let relative_pos =
                    listener_inv_rot * (update.position - listener_pos);
                let (azimuth, elevation) =
                    super::hrtf::direction_to_azimuth_elevation(relative_pos);

                if let Some(params) = engine.get_binaural_params(update.uuid) {
                    params.set_direction(azimuth, elevation);
                }
            }
        }
    }

    /// Find the listener position and orientation from the scene.
    ///
    /// Prefers an active `AudioListenerComponent`, falls back to primary camera.
    fn find_listener_transform(&self) -> (glam::Vec3, glam::Quat) {
        // Check for explicit AudioListenerComponent.
        let mut listener_count = 0u32;
        let mut result = None;
        for (al, tf) in self
            .world
            .query::<(&AudioListenerComponent, &TransformComponent)>()
            .iter()
        {
            if al.active {
                listener_count += 1;
                result = Some((tf.translation, tf.rotation));
            }
        }
        if listener_count > 1 {
            log::warn!(
                "Multiple active AudioListenerComponents ({}) — using last found. \
                 Disable extras to ensure deterministic behavior.",
                listener_count
            );
        }
        if let Some(r) = result {
            return r;
        }

        // Fall back to primary camera.
        for (cam, tf) in self
            .world
            .query::<(&CameraComponent, &TransformComponent)>()
            .iter()
        {
            if cam.primary {
                return (tf.translation, tf.rotation);
            }
        }

        (glam::Vec3::ZERO, glam::Quat::IDENTITY)
    }
}
