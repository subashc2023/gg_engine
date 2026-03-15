use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use kira::listener::ListenerHandle;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::track::{SpatialTrackBuilder, SpatialTrackDistances, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, Tween};

use super::hrtf::{BinauralEffectBuilder, BinauralHandle, BinauralParams};
use super::AudioCategory;

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

/// Default global voice limit.
const DEFAULT_MAX_VOICES: usize = 32;

/// Default per-entity voice limit.
const DEFAULT_MAX_VOICES_PER_ENTITY: usize = 4;

/// Convert linear amplitude (0.0–1.0) to decibels for kira.
/// Returns -80 dB for silence (volume <= 0).
pub(crate) fn linear_to_db(volume: f32) -> f32 {
    if volume <= 0.0 {
        -80.0
    } else {
        20.0 * volume.log10()
    }
}

/// Create a kira `Tween` with the given fade duration in seconds.
/// If `duration_secs <= 0`, returns an instant (default) tween.
fn fade_tween(duration_secs: f32) -> Tween {
    if duration_secs <= 0.0 {
        Tween::default()
    } else {
        Tween {
            duration: Duration::from_secs_f32(duration_secs),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Voice entry (sound handle + metadata for voice stealing)
// ---------------------------------------------------------------------------

/// A single playing sound with metadata for priority-based voice stealing.
struct VoiceEntry {
    handle: SoundHandle,
    /// Voice priority (0–255). Higher = more important, harder to steal.
    priority: u8,
    /// Linear volume at play time (for steal comparison).
    volume_linear: f32,
    /// Monotonic birth counter (lower = older).
    birth: u64,
}

// ---------------------------------------------------------------------------
// Spatial track state (per-entity)
// ---------------------------------------------------------------------------

/// Per-entity state for a kira spatial track with optional HRTF effect.
pub(crate) struct SpatialTrackState {
    pub track: kira::track::SpatialTrackHandle,
    /// HRTF binaural handle (None if HRTF disabled for this source).
    pub binaural: Option<BinauralHandle>,
}

// ---------------------------------------------------------------------------
// AudioEngine
// ---------------------------------------------------------------------------

/// Wrapper around kira's AudioManager, providing entity-keyed playback
/// with per-category bus routing.
///
/// Supports multiple simultaneous sounds per entity (e.g. footsteps + breathing),
/// streaming playback for music tracks, and spatial panning/attenuation.
/// For HRTF-enabled sources, per-entity spatial tracks with binaural effects
/// are created automatically.
///
/// # Audio bus hierarchy
///
/// ```text
/// Main Mixer (master volume)
/// ├── SFX Bus       (category volume)
/// │   ├── non-spatial sounds
/// │   └── spatial sub-tracks
/// ├── Music Bus     (category volume)
/// ├── Ambient Bus   (category volume)
/// └── Voice Bus     (category volume)
/// ```
///
/// Volume changes on the master or category buses affect all currently-playing
/// sounds in real time (no longer just pre-play multiplication).
pub(crate) struct AudioEngine {
    manager: AudioManager,
    /// Active voices keyed by entity UUID. Each entity can have
    /// multiple concurrent voices.
    active_sounds: HashMap<u64, Vec<VoiceEntry>>,
    /// Cached sound data keyed by file path (avoids re-loading from disk).
    /// Only used for static (non-streaming) sounds.
    sound_cache: HashMap<String, StaticSoundData>,
    /// LRU order for sound cache eviction. Most recently used at back.
    cache_order: Vec<String>,
    /// UUIDs for which the sound completion callback should be suppressed
    /// (stop was user-initiated via stop/fade_out, not natural completion).
    suppress_callback: HashSet<u64>,

    // --- Audio bus routing ---
    /// Per-category bus tracks. Non-spatial sounds play on their category's bus;
    /// spatial tracks are created as children of their category bus.
    /// Indexed by [`AudioCategory`] discriminant.
    bus_tracks: [Option<TrackHandle>; AudioCategory::COUNT],

    // --- Spatial audio ---
    /// kira listener handle (one per scene). Position/orientation updated per-frame.
    listener: Option<ListenerHandle>,
    /// Per-entity spatial track state. Created for entities with `spatial: true`.
    spatial_tracks: HashMap<u64, SpatialTrackState>,

    // --- Voice management ---
    /// Maximum simultaneous voices globally.
    max_voices: usize,
    /// Maximum simultaneous voices per entity.
    max_voices_per_entity: usize,
    /// Monotonic counter for voice age tracking.
    next_birth: u64,
}

impl AudioEngine {
    pub fn new() -> Option<Self> {
        match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(mut manager) => {
                // Create a single listener at the origin.
                let listener = match manager
                    .add_listener(glam::Vec3::ZERO, glam::Quat::IDENTITY)
                {
                    Ok(lh) => Some(lh),
                    Err(e) => {
                        log::error!("Failed to create audio listener: {}", e);
                        None
                    }
                };

                // Create per-category bus tracks as sub-tracks of the main mixer.
                let mut bus_tracks: [Option<TrackHandle>; AudioCategory::COUNT] =
                    Default::default();
                for (slot, i) in bus_tracks.iter_mut().zip(0..AudioCategory::COUNT) {
                    match manager.add_sub_track(TrackBuilder::new()) {
                        Ok(track) => *slot = Some(track),
                        Err(e) => {
                            log::error!(
                                "Failed to create audio bus for category {}: {}",
                                AudioCategory::from_index(i)
                                    .map(|c| c.label())
                                    .unwrap_or("?"),
                                e,
                            );
                        }
                    }
                }

                Some(Self {
                    manager,
                    active_sounds: HashMap::new(),
                    sound_cache: HashMap::new(),
                    cache_order: Vec::new(),
                    suppress_callback: HashSet::new(),
                    bus_tracks,
                    listener,
                    spatial_tracks: HashMap::new(),
                    max_voices: DEFAULT_MAX_VOICES,
                    max_voices_per_entity: DEFAULT_MAX_VOICES_PER_ENTITY,
                    next_birth: 0,
                })
            }
            Err(e) => {
                log::error!("Failed to create AudioManager: {}", e);
                None
            }
        }
    }

    // ------------------------------------------------------------------
    // Listener management
    // ------------------------------------------------------------------

    /// Update the kira listener's position and orientation.
    pub fn update_listener(&mut self, position: glam::Vec3, orientation: glam::Quat) {
        if let Some(ref mut lh) = self.listener {
            lh.set_position(position, Tween::default());
            lh.set_orientation(orientation, Tween::default());
        }
    }

    // ------------------------------------------------------------------
    // Spatial track management
    // ------------------------------------------------------------------

    /// Ensure a spatial track exists for the given entity. Creates one if needed.
    ///
    /// The spatial track is created as a child of the entity's category bus so
    /// that category and master volume changes affect it in real time.
    ///
    /// All spatial tracks are created with a `BinauralEffect` so that HRTF can
    /// be toggled at runtime via `BinauralParams::set_enabled()` without
    /// recreating the track (which would interrupt playback).
    pub fn ensure_spatial_track(
        &mut self,
        entity_uuid: u64,
        position: glam::Vec3,
        min_distance: f32,
        max_distance: f32,
        hrtf: bool,
        category: AudioCategory,
    ) {
        if let Some(st) = self.spatial_tracks.get(&entity_uuid) {
            // Toggle the binaural effect on/off to match current hrtf flag.
            if let Some(ref bh) = st.binaural {
                bh.params.set_enabled(hrtf);
            }
            return;
        }

        let listener_id = match &self.listener {
            Some(lh) => lh.id(),
            None => return,
        };

        // Always add the binaural effect so HRTF can be toggled without
        // recreating the track. When hrtf is false, the effect starts disabled.
        let (effect_builder, binaural_handle) = BinauralEffectBuilder::new();
        if !hrtf {
            binaural_handle.params.set_enabled(false);
        }
        // Disable kira's built-in panning — either the binaural effect handles
        // it (hrtf on) or we get simple distance attenuation only (hrtf off).
        let mut builder = SpatialTrackBuilder::new()
            .distances(SpatialTrackDistances {
                min_distance,
                max_distance,
            })
            .persist_until_sounds_finish(true)
            .spatialization_strength(0.0);
        builder.add_effect(effect_builder);

        // Create the spatial track as a child of the category bus (so bus
        // volume/mute propagates), falling back to the main mixer if the bus
        // isn't available.
        let result = if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.add_spatial_sub_track(listener_id, position, builder)
        } else {
            self.manager
                .add_spatial_sub_track(listener_id, position, builder)
        };

        match result {
            Ok(track) => {
                let state = SpatialTrackState {
                    track,
                    binaural: Some(binaural_handle),
                };
                self.spatial_tracks.insert(entity_uuid, state);
            }
            Err(e) => {
                log::error!(
                    "Failed to create spatial track for entity {}: {}",
                    entity_uuid,
                    e
                );
            }
        }
    }

    /// Update the position of an entity's spatial track.
    pub fn update_spatial_position(&mut self, entity_uuid: u64, position: glam::Vec3) {
        if let Some(st) = self.spatial_tracks.get_mut(&entity_uuid) {
            st.track.set_position(position, Tween::default());
        }
    }

    /// Get the binaural params for an entity's HRTF effect.
    pub fn get_binaural_params(&self, entity_uuid: u64) -> Option<&Arc<BinauralParams>> {
        self.spatial_tracks
            .get(&entity_uuid)
            .and_then(|st| st.binaural.as_ref())
            .map(|bh| &bh.params)
    }

    /// Remove and drop the spatial track for an entity.
    #[allow(dead_code)]
    pub fn remove_spatial_track(&mut self, entity_uuid: u64) {
        self.spatial_tracks.remove(&entity_uuid);
    }

    // ------------------------------------------------------------------
    // Sound playback
    // ------------------------------------------------------------------

    /// Play a sound for the given entity. Multiple sounds can overlap.
    /// Finished sounds are automatically pruned.
    ///
    /// `entity_volume` is the entity's own linear volume (0.0–1.0).
    /// Category and master volume are applied by the bus track hierarchy.
    ///
    /// If the global or per-entity voice limit is reached, the lowest-priority
    /// (then quietest, then oldest) voice is stolen to make room.
    /// A new sound can only steal from voices with equal or lower priority.
    #[allow(clippy::too_many_arguments)]
    pub fn play_sound(
        &mut self,
        entity_uuid: u64,
        path: &str,
        entity_volume: f32,
        pitch: f32,
        looping: bool,
        streaming: bool,
        category: AudioCategory,
        priority: u8,
    ) {
        // Prune finished sounds globally.
        self.prune_stopped();

        // Enforce per-entity voice limit.
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            while voices.len() >= self.max_voices_per_entity {
                if !Self::steal_voice_from(voices, priority) {
                    // Cannot steal — all voices are higher priority. Drop the request.
                    log::debug!(
                        "Voice limit per-entity ({}) reached for entity {}; \
                         cannot steal (priority {}).",
                        self.max_voices_per_entity,
                        entity_uuid,
                        priority,
                    );
                    return;
                }
            }
        }

        // Enforce global voice limit.
        while self.total_voice_count() >= self.max_voices {
            if !self.steal_global_voice(priority) {
                log::debug!(
                    "Global voice limit ({}) reached; cannot steal (priority {}).",
                    self.max_voices,
                    priority,
                );
                return;
            }
        }

        if streaming {
            self.play_streaming(entity_uuid, path, entity_volume, pitch, looping, category, priority);
        } else {
            self.play_static(entity_uuid, path, entity_volume, pitch, looping, category, priority);
        }
    }

    /// Play a sound with a fade-in from silence to the entity volume.
    #[allow(clippy::too_many_arguments)]
    pub fn play_sound_fade(
        &mut self,
        entity_uuid: u64,
        path: &str,
        entity_volume: f32,
        pitch: f32,
        looping: bool,
        streaming: bool,
        fade_secs: f32,
        category: AudioCategory,
        priority: u8,
    ) {
        // Play at silence.
        self.play_sound(entity_uuid, path, 0.0, pitch, looping, streaming, category, priority);

        // Immediately tween to target volume.
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let tween = fade_tween(fade_secs);
            let target = Decibels(linear_to_db(entity_volume));
            for voice in voices.iter_mut() {
                if voice.handle.state() == PlaybackState::Playing {
                    voice.handle.set_volume(target, tween);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn play_static(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
        category: AudioCategory,
        priority: u8,
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

        let prepared = sound_data.with_settings(settings);

        // Route to spatial track if one exists, then bus track, then main mixer.
        let result = if let Some(st) = self.spatial_tracks.get_mut(&entity_uuid) {
            st.track.play(prepared)
        } else if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.play(prepared)
        } else {
            self.manager.play(prepared)
        };

        match result {
            Ok(handle) => {
                let birth = self.next_birth;
                self.next_birth += 1;
                self.active_sounds
                    .entry(entity_uuid)
                    .or_default()
                    .push(VoiceEntry {
                        handle: SoundHandle::Static(handle),
                        priority,
                        volume_linear: volume,
                        birth,
                    });
            }
            Err(e) => {
                log::error!("Failed to play sound for entity {}: {}", entity_uuid, e);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn play_streaming(
        &mut self,
        entity_uuid: u64,
        path: &str,
        volume: f32,
        pitch: f32,
        looping: bool,
        category: AudioCategory,
        priority: u8,
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

        // Route to spatial track if one exists, then bus track, then main mixer.
        let result = if let Some(st) = self.spatial_tracks.get_mut(&entity_uuid) {
            st.track.play(data)
        } else if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.play(data)
        } else {
            self.manager.play(data)
        };

        match result {
            Ok(handle) => {
                let birth = self.next_birth;
                self.next_birth += 1;
                self.active_sounds
                    .entry(entity_uuid)
                    .or_default()
                    .push(VoiceEntry {
                        handle: SoundHandle::Streaming(handle),
                        priority,
                        volume_linear: volume,
                        birth,
                    });
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
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            for voice in voices.iter_mut() {
                if voice.handle.state() == PlaybackState::Playing {
                    voice.handle.pause(Tween::default());
                }
            }
        }
    }

    /// Resume all paused sounds for the given entity.
    pub fn resume_sound(&mut self, entity_uuid: u64) {
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            for voice in voices.iter_mut() {
                if voice.handle.state() == PlaybackState::Paused {
                    voice.handle.resume(Tween::default());
                }
            }
        }
    }

    /// Fade in: resume paused sounds with a smooth volume ramp, or play if not active.
    /// Returns `true` if active sounds were found and faded in.
    pub fn fade_in(
        &mut self,
        entity_uuid: u64,
        target_volume_db: f32,
        duration_secs: f32,
    ) -> bool {
        let tween = fade_tween(duration_secs);
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let has_active = voices
                .iter()
                .any(|v| matches!(v.handle.state(), PlaybackState::Playing | PlaybackState::Paused));
            if has_active {
                for voice in voices.iter_mut() {
                    if voice.handle.state() == PlaybackState::Paused {
                        // Set to silence first, then resume with fade.
                        voice.handle.set_volume(Decibels(-80.0), Tween::default());
                        voice.handle.resume(Tween::default());
                    }
                    voice.handle.set_volume(Decibels(target_volume_db), tween);
                }
                return true;
            }
        }
        false
    }

    /// Fade out and stop all sounds on an entity.
    pub fn fade_out_stop(&mut self, entity_uuid: u64, duration_secs: f32) {
        self.suppress_callback.insert(entity_uuid);
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let tween = fade_tween(duration_secs);
            for voice in voices.iter_mut() {
                voice.handle.stop(tween);
            }
        }
    }

    /// Fade volume to a specific level over time.
    pub fn fade_to(&mut self, entity_uuid: u64, target_volume_db: f32, duration_secs: f32) {
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let tween = fade_tween(duration_secs);
            let target = Decibels(target_volume_db);
            for voice in voices.iter_mut() {
                voice.handle.set_volume(target, tween);
            }
        }
    }

    /// Stop all sounds for the given entity.
    pub fn stop_sound(&mut self, entity_uuid: u64) {
        self.suppress_callback.insert(entity_uuid);
        if let Some(voices) = self.active_sounds.remove(&entity_uuid) {
            for mut voice in voices {
                voice.handle.stop(Tween::default());
            }
        }
        // Also remove the spatial track so it's recreated fresh next play.
        self.spatial_tracks.remove(&entity_uuid);
    }

    /// Set volume for all active sounds on an entity.
    /// Volume is in decibels (0.0 = unity gain, -60.0 = near-silent).
    pub fn set_volume(&mut self, entity_uuid: u64, volume_db: f32) {
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let db = Decibels(volume_db);
            for voice in voices.iter_mut() {
                voice.handle.set_volume(db, Tween::default());
            }
        }
    }

    /// Set panning for all active sounds on an entity.
    /// Panning: -1.0 = hard left, 0.0 = center, 1.0 = hard right.
    pub fn set_panning(&mut self, entity_uuid: u64, panning: f32) {
        if let Some(voices) = self.active_sounds.get_mut(&entity_uuid) {
            let p = Panning(panning.clamp(-1.0, 1.0));
            for voice in voices.iter_mut() {
                voice.handle.set_panning(p, Tween::default());
            }
        }
    }

    /// Stop all active sounds.
    pub fn stop_all(&mut self) {
        for (_, voices) in self.active_sounds.drain() {
            for mut voice in voices {
                voice.handle.stop(Tween::default());
            }
        }
        self.spatial_tracks.clear();
    }

    // ------------------------------------------------------------------
    // Bus / mixer control
    // ------------------------------------------------------------------

    /// Set the master volume on kira's main track. Affects all playing sounds.
    pub fn set_master_volume(&mut self, volume_linear: f32) {
        self.manager
            .main_track()
            .set_volume(Decibels(linear_to_db(volume_linear)), Tween::default());
    }

    /// Set volume for a category bus. Affects all sounds routed to this bus.
    pub fn set_bus_volume(&mut self, category: AudioCategory, volume_linear: f32) {
        if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.set_volume(Decibels(linear_to_db(volume_linear)), Tween::default());
        }
    }

    /// Mute a category bus (pauses the track, silencing all routed sounds).
    pub fn mute_bus(&mut self, category: AudioCategory) {
        if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.pause(Tween::default());
        }
    }

    /// Unmute a category bus (resumes the track).
    pub fn unmute_bus(&mut self, category: AudioCategory) {
        if let Some(ref mut bus) = self.bus_tracks[category as usize] {
            bus.resume(Tween::default());
        }
    }

    /// Drain entity UUIDs whose sounds have all finished (naturally, not explicitly stopped).
    /// Call once per frame to detect sound completion for callbacks.
    pub fn drain_completed(&mut self) -> Vec<u64> {
        let mut completed = Vec::new();
        self.active_sounds.retain(|uuid, voices| {
            voices.retain(|v| v.handle.state() != PlaybackState::Stopped);
            if voices.is_empty() {
                if !self.suppress_callback.remove(uuid) {
                    completed.push(*uuid);
                }
                false
            } else {
                true
            }
        });
        // Clean remaining suppress entries for UUIDs no longer tracked.
        self.suppress_callback
            .retain(|uuid| self.active_sounds.contains_key(uuid));
        // Clean up spatial tracks for entities with no active sounds.
        let active = &self.active_sounds;
        self.spatial_tracks.retain(|uuid, _| active.contains_key(uuid));
        completed
    }

    // ------------------------------------------------------------------
    // Voice management
    // ------------------------------------------------------------------

    /// Total number of currently active voices across all entities.
    pub fn total_voice_count(&self) -> usize {
        self.active_sounds.values().map(|v| v.len()).sum()
    }

    /// Set the global maximum number of simultaneous voices.
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max.max(1);
    }

    /// Get the global maximum number of simultaneous voices.
    #[allow(dead_code)]
    pub fn max_voices(&self) -> usize {
        self.max_voices
    }

    /// Set the per-entity maximum number of simultaneous voices.
    pub fn set_max_voices_per_entity(&mut self, max: usize) {
        self.max_voices_per_entity = max.max(1);
    }

    /// Get the per-entity maximum number of simultaneous voices.
    #[allow(dead_code)]
    pub fn max_voices_per_entity(&self) -> usize {
        self.max_voices_per_entity
    }

    /// Prune stopped sounds from all entities.
    fn prune_stopped(&mut self) {
        self.active_sounds.retain(|_, voices| {
            voices.retain(|v| v.handle.state() != PlaybackState::Stopped);
            !voices.is_empty()
        });
    }

    /// Steal the least-important voice from a single entity's voice list.
    /// Returns true if a voice was stolen, false if all voices have higher priority.
    ///
    /// Stealing order: lowest priority → lowest volume → oldest birth.
    fn steal_voice_from(voices: &mut Vec<VoiceEntry>, new_priority: u8) -> bool {
        if voices.is_empty() {
            return false;
        }
        // Find the "weakest" stealable voice (priority <= new_priority).
        let victim = voices
            .iter()
            .enumerate()
            .filter(|(_, v)| v.priority <= new_priority)
            .min_by(|(_, a), (_, b)| {
                a.priority
                    .cmp(&b.priority)
                    .then(a.volume_linear.partial_cmp(&b.volume_linear).unwrap_or(std::cmp::Ordering::Equal))
                    .then(a.birth.cmp(&b.birth))
            })
            .map(|(i, _)| i);

        if let Some(idx) = victim {
            let mut entry = voices.swap_remove(idx);
            entry.handle.stop(Tween::default());
            true
        } else {
            false
        }
    }

    /// Steal the least-important voice globally (across all entities).
    /// Returns true if a voice was stolen, false if all voices have higher priority.
    fn steal_global_voice(&mut self, new_priority: u8) -> bool {
        // Find (entity_uuid, index) of the weakest stealable voice globally.
        let mut best: Option<(u64, usize, u8, f32, u64)> = None; // (uuid, idx, priority, volume, birth)
        for (&uuid, voices) in &self.active_sounds {
            for (i, v) in voices.iter().enumerate() {
                if v.priority > new_priority {
                    continue;
                }
                let dominated = match &best {
                    None => true,
                    Some((_, _, bp, bv, bb)) => {
                        (v.priority, v.volume_linear.to_bits(), v.birth)
                            < (*bp, bv.to_bits(), *bb)
                    }
                };
                if dominated {
                    best = Some((uuid, i, v.priority, v.volume_linear, v.birth));
                }
            }
        }

        if let Some((uuid, idx, ..)) = best {
            if let Some(voices) = self.active_sounds.get_mut(&uuid) {
                let mut entry = voices.swap_remove(idx);
                entry.handle.stop(Tween::default());
                if voices.is_empty() {
                    self.active_sounds.remove(&uuid);
                }
            }
            true
        } else {
            false
        }
    }
}
