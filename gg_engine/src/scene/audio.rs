use std::collections::HashMap;

use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::PlaybackState;

/// Wrapper around kira's AudioManager, providing entity-keyed playback.
///
/// Supports multiple simultaneous sounds per entity (e.g. footsteps + breathing).
pub(crate) struct AudioEngine {
    manager: AudioManager,
    /// Active sound handles keyed by entity UUID. Each entity can have
    /// multiple concurrent sounds.
    active_sounds: HashMap<u64, Vec<StaticSoundHandle>>,
    /// Cached sound data keyed by file path (avoids re-loading from disk).
    sound_cache: HashMap<String, StaticSoundData>,
}

impl AudioEngine {
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Some(Self {
                manager,
                active_sounds: HashMap::new(),
                sound_cache: HashMap::new(),
            }),
            Err(e) => {
                log::error!("Failed to create AudioManager: {}", e);
                None
            }
        }
    }

    /// Play a sound for the given entity. Multiple sounds can overlap.
    /// Finished sounds are automatically pruned.
    pub fn play_sound(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) {
        // Prune finished sounds for this entity.
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            handles.retain(|h| h.state() != PlaybackState::Stopped);
        }

        // Load or retrieve cached sound data.
        let sound_data = match self.sound_cache.get(path) {
            Some(data) => data.clone(),
            None => {
                match StaticSoundData::from_file(path) {
                    Ok(data) => {
                        self.sound_cache.insert(path.to_string(), data.clone());
                        data
                    }
                    Err(e) => {
                        log::error!("Failed to load audio file '{}': {}", path, e);
                        return;
                    }
                }
            }
        };

        // Configure playback settings.
        let mut settings = StaticSoundSettings::new()
            .volume(volume)
            .playback_rate(pitch as f64);
        if looping {
            settings = settings.loop_region(..);
        }

        match self.manager.play(sound_data.with_settings(settings)) {
            Ok(handle) => {
                self.active_sounds
                    .entry(entity_uuid)
                    .or_default()
                    .push(handle);
            }
            Err(e) => {
                log::error!("Failed to play sound for entity {}: {}", entity_uuid, e);
            }
        }
    }

    /// Stop all sounds for the given entity.
    pub fn stop_sound(&mut self, entity_uuid: u64) {
        if let Some(handles) = self.active_sounds.remove(&entity_uuid) {
            for mut handle in handles {
                handle.stop(Tween::default());
            }
        }
    }

    /// Set volume for all active sounds on an entity.
    pub fn set_volume(&mut self, entity_uuid: u64, volume: f32) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            for handle in handles.iter_mut() {
                handle.set_volume(volume, Tween::default());
            }
        }
    }

    /// Stop all active sounds.
    pub fn stop_all(&mut self) {
        for (_, handles) in self.active_sounds.drain() {
            for mut handle in handles {
                handle.stop(Tween::default());
            }
        }
    }
}
