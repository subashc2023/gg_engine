use std::collections::HashMap;

use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Tween};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};

/// Wrapper around kira's AudioManager, providing entity-keyed playback.
pub(crate) struct AudioEngine {
    manager: AudioManager,
    /// Active sound handles keyed by entity UUID.
    active_sounds: HashMap<u64, StaticSoundHandle>,
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

    /// Play a sound for the given entity.
    pub fn play_sound(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) {
        // Stop any existing sound for this entity.
        self.stop_sound(entity_uuid);

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
                self.active_sounds.insert(entity_uuid, handle);
            }
            Err(e) => {
                log::error!("Failed to play sound for entity {}: {}", entity_uuid, e);
            }
        }
    }

    /// Stop playback for the given entity.
    pub fn stop_sound(&mut self, entity_uuid: u64) {
        if let Some(mut handle) = self.active_sounds.remove(&entity_uuid) {
            handle.stop(Tween::default());
        }
    }

    /// Set volume for an active sound.
    pub fn set_volume(&mut self, entity_uuid: u64, volume: f32) {
        if let Some(handle) = self.active_sounds.get_mut(&entity_uuid) {
            handle.set_volume(volume, Tween::default());
        }
    }

    /// Stop all active sounds.
    pub fn stop_all(&mut self) {
        for (_, mut handle) in self.active_sounds.drain() {
            handle.stop(Tween::default());
        }
    }
}
