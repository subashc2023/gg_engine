use super::{
    AudioListenerComponent, AudioSourceComponent, CameraComponent, Entity, IdComponent, Scene,
    TagComponent, TransformComponent,
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
        let auto_play: Vec<(u64, String, f32, f32, bool, bool)> = self
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
                )
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, path, volume, pitch, looping, streaming) in auto_play {
                engine.play_sound(uuid, &path, volume, pitch, looping, streaming);
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

        refs
    }

    /// Play audio for an entity (used by Lua scripts).
    pub fn play_entity_sound(&mut self, entity: Entity) {
        let (uuid, path, volume, pitch, looping, streaming) = {
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
            (id, path, asc.volume, asc.pitch, asc.looping, asc.streaming)
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.play_sound(uuid, &path, volume, pitch, looping, streaming);
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
    pub fn set_entity_volume(&mut self, entity: Entity, volume: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_volume(uuid, volume);
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

    /// Update spatial audio: compute panning and distance attenuation for
    /// all spatial audio sources based on the listener position.
    ///
    /// If an entity has an active [`AudioListenerComponent`], its position is
    /// used as the listener. Otherwise, the primary camera position is used.
    pub fn update_spatial_audio(&mut self) {
        if self.audio_engine.is_none() {
            return;
        }

        // Prefer explicit AudioListenerComponent, fall back to primary camera.
        let listener_pos = self
            .world
            .query::<(&AudioListenerComponent, &TransformComponent)>()
            .iter()
            .filter(|(al, _)| al.active)
            .map(|(_, tf)| tf.translation.truncate())
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
                // Combine: component volume (dB-ish) + distance attenuation.
                let effective_volume = asc.volume + atten_db;
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
