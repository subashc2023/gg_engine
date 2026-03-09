use std::collections::HashMap;

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, Tween};

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

    fn pause(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.pause(tween),
            Self::Streaming(h) => h.pause(tween),
        }
    }

    fn resume(&mut self, tween: Tween) {
        match self {
            Self::Static(h) => h.resume(tween),
            Self::Streaming(h) => h.resume(tween),
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

/// Maximum number of cached static sound data entries before LRU eviction kicks in.
const SOUND_CACHE_MAX: usize = 128;

/// Convert linear amplitude (0.0–1.0) to decibels for kira.
/// Returns -80 dB for silence (volume <= 0).
pub(crate) fn linear_to_db(volume: f32) -> f32 {
    if volume <= 0.0 {
        -80.0
    } else {
        20.0 * volume.log10()
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
    /// LRU order for sound cache eviction. Most recently used at back.
    cache_order: Vec<String>,
}

impl AudioEngine {
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => Some(Self {
                manager,
                active_sounds: HashMap::new(),
                sound_cache: HashMap::new(),
                cache_order: Vec::new(),
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
        let sound_data = if let Some(data) = self.sound_cache.get(path) {
            // Move to back of LRU.
            if let Some(pos) = self.cache_order.iter().position(|s| s == path) {
                self.cache_order.remove(pos);
            }
            self.cache_order.push(path.to_string());
            data.clone()
        } else {
            match StaticSoundData::from_file(path) {
                Ok(data) => {
                    // Evict oldest entries if cache is full.
                    while self.sound_cache.len() >= SOUND_CACHE_MAX {
                        if let Some(oldest) = self.cache_order.first().cloned() {
                            self.sound_cache.remove(&oldest);
                            self.cache_order.remove(0);
                        } else {
                            break;
                        }
                    }
                    self.sound_cache.insert(path.to_string(), data.clone());
                    self.cache_order.push(path.to_string());
                    data
                }
                Err(e) => {
                    log::error!("Failed to load audio file '{}': {}", path, e);
                    return;
                }
            }
        };

        let mut settings = StaticSoundSettings::new()
            .volume(Decibels(linear_to_db(volume)))
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
            .volume(Decibels(linear_to_db(volume)))
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
                log::error!(
                    "Failed to play streaming sound for entity {}: {}",
                    entity_uuid,
                    e
                );
            }
        }
    }

    /// Pause all sounds for the given entity (can be resumed).
    pub fn pause_sound(&mut self, entity_uuid: u64) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            for handle in handles.iter_mut() {
                if handle.state() == PlaybackState::Playing {
                    handle.pause(Tween::default());
                }
            }
        }
    }

    /// Resume all paused sounds for the given entity.
    pub fn resume_sound(&mut self, entity_uuid: u64) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            for handle in handles.iter_mut() {
                if handle.state() == PlaybackState::Paused {
                    handle.resume(Tween::default());
                }
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
    /// Volume is in decibels (0.0 = unity gain, -60.0 = near-silent).
    pub fn set_volume(&mut self, entity_uuid: u64, volume_db: f32) {
        if let Some(handles) = self.active_sounds.get_mut(&entity_uuid) {
            let db = Decibels(volume_db);
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
