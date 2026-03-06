use std::collections::HashMap;

use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, Tween};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};

/// Unified handle for both static and streaming sounds.
enum SoundHandle {
    Static(StaticSoundHandle),
    Streaming(StreamingSoundHandle<FromFileError>),
}

impl SoundHandle {
    fn state(&self) -> PlaybackState {
        match self {
            Self::Static(h) => h.state(),
            Self::Streaming(h) => h.state(),
        }
    }

    fn stop(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.stop(tween),
            Self::Streaming(h) => h.stop(tween),
        }
    }

    fn set_volume(&mut self, volume: kira::Decibels, tween: Tween) {
        match self {
            Self::Static(h) => h.set_volume(volume, tween),
            Self::Streaming(h) => h.set_volume(volume, tween),
        }
    }

    fn set_panning(&mut self, panning: Panning, tween: Tween) {
        match self {
            Self::Static(h) => h.set_panning(panning, tween),
            Self::Streaming(h) => h.set_panning(panning, tween),
        }
    }
}

/// Wrapper around kira's AudioManager, providing entity-keyed playback.
///
/// Supports multiple simultaneous sounds per entity (e.g. footsteps + breathing),
/// streaming playback for music tracks, and spatial panning/attenuation.
pub(crate) struct AudioEngine {
    manager: AudioManager,
    /// Active sound handles keyed by entity UUID. Each entity can have
    /// multiple concurrent sounds.
    active_sounds: HashMap<u64, Vec<SoundHandle>>,
    /// Cached sound data keyed by file path (avoids re-loading from disk).
    /// Only used for static (non-streaming) sounds.
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
        streaming: bool,
    ) {
        // Prune finished sounds for this entity.
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            handles.retain(|h| h.state() != PlaybackState::Stopped);
        }

        if streaming {
            self.play_streaming(entity_uuid, path, volume, pitch, looping);
        } else {
            self.play_static(entity_uuid, path, volume, pitch, looping);
        }
    }

    fn play_static(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) {
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
                    .push(SoundHandle::Static(handle));
            }
            Err(e) => {
                log::error!("Failed to play sound for entity {}: {}", entity_uuid, e);
            }
        }
    }

    fn play_streaming(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) {
        let sound_data = match StreamingSoundData::from_file(path) {
            Ok(data) => data,
            Err(e) => {
                log::error!("Failed to open streaming audio '{}': {}", path, e);
                return;
            }
        };

        let mut data = sound_data
            .volume(volume)
            .playback_rate(pitch as f64);
        if looping {
            data = data.loop_region(..);
        }

        match self.manager.play(data) {
            Ok(handle) => {
                self.active_sounds
                    .entry(entity_uuid)
                    .or_default()
                    .push(SoundHandle::Streaming(handle));
            }
            Err(e) => {
                log::error!("Failed to play streaming sound for entity {}: {}", entity_uuid, e);
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
    /// Volume is passed as a raw value to kira (f32 → Decibels).
    pub fn set_volume(&mut self, entity_uuid: u64, volume: f32) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            let db = Decibels::from(volume);
            for handle in handles.iter_mut() {
                handle.set_volume(db, Tween::default());
            }
        }
    }

    /// Set panning for all active sounds on an entity.
    /// Panning: -1.0 = hard left, 0.0 = center, 1.0 = hard right.
    pub fn set_panning(&mut self, entity_uuid: u64, panning: f32) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            let p = Panning(panning.clamp(-1.0, 1.0));
            for handle in handles.iter_mut() {
                handle.set_panning(p, Tween::default());
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
