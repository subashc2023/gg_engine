# Audio System

The engine integrates [kira](https://docs.rs/kira/0.12) 0.12 for audio playback. Sounds are keyed by entity UUID — each entity can have multiple concurrent sounds (e.g. footsteps + breathing). Supports both static (in-memory) and streaming (from-disk) playback, plus spatial audio with camera-relative panning and distance attenuation. Audio assets are referenced by UUID handle (`AudioSourceComponent`) and resolved to file paths at runtime via the asset manager.

**Files:**
- `gg_engine/src/scene/audio.rs` — `AudioEngine` struct, kira wrapper
- `gg_engine/src/scene/components.rs` — `AudioSourceComponent` definition
- `gg_engine/src/scene/audio_ops.rs` — Scene audio lifecycle (`on_audio_start`, `on_audio_stop`, `resolve_audio_handles`, `play_entity_sound`, `stop_entity_sound`, `set_entity_volume`, `set_entity_panning`, `fade_in_entity_sound`, `fade_out_entity_sound`, `fade_to_entity_volume`, `set_master_volume`, `set_category_volume`, `update_spatial_audio`, `dispatch_sound_finished_events`)
- `gg_engine/src/scene/script_glue.rs` — Lua bindings (`play_sound`, `stop_sound`, `set_volume`, `set_panning`, `fade_in`, `fade_out`, `fade_to`, `set_master_volume`, `get_master_volume`, `set_category_volume`, `get_category_volume`)
- `gg_editor/src/panels/properties/audio.rs` — Editor UI for audio source properties

## AudioEngine

**File:** `gg_engine/src/scene/audio.rs`

Wraps `kira::AudioManager<DefaultBackend>` with entity-keyed playback tracking and file caching. Supports both static (in-memory) and streaming (from-disk) playback via a unified `SoundHandle` enum.

```rust
enum SoundHandle {
    Static(StaticSoundHandle),
    Streaming(StreamingSoundHandle<FromFileError>),
}

pub(crate) struct AudioEngine {
    manager: AudioManager,
    active_sounds: HashMap<u64, Vec<SoundHandle>>,
    sound_cache: HashMap<String, StaticSoundData>,
}
```

| Field | Type | Description |
|-------|------|-------------|
| `manager` | `kira::AudioManager` | Kira audio manager (default backend) |
| `active_sounds` | `HashMap<u64, Vec<SoundHandle>>` | Active sound handles keyed by entity UUID. Each entity can have multiple concurrent sounds |
| `sound_cache` | `HashMap<String, StaticSoundData>` | Cached loaded audio files keyed by absolute file path (static sounds only) |

### SoundHandle

The `SoundHandle` enum unifies `StaticSoundHandle` and `StreamingSoundHandle<FromFileError>`, delegating `state()`, `stop()`, `set_volume()`, and `set_panning()` to the appropriate kira type.

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new()` | `-> Option<Self>` | Creates the kira `AudioManager`. Returns `None` on failure (logged as error) |
| `play_sound` | `(entity_uuid, path, volume, pitch, looping, streaming)` | Prunes finished sounds for the entity, then plays a new sound (static or streaming). Multiple sounds can overlap per entity |
| `stop_sound` | `(entity_uuid)` | Removes and stops all sounds for the given entity (uses `Tween::default()` for fade) |
| `set_volume` | `(entity_uuid, volume)` | Adjusts volume on all active sounds for the entity (uses `kira::Decibels`) |
| `set_panning` | `(entity_uuid, panning)` | Sets panning for all active sounds. -1.0 = hard left, 0.0 = center, 1.0 = hard right (clamped, uses `kira::Panning`) |
| `stop_all` | `()` | Drains and stops all active sounds |

**Sound caching:** The first call to `play_sound` for a static (non-streaming) sound loads `StaticSoundData::from_file()` and caches it. Subsequent plays clone the cached data (avoiding disk I/O). Streaming sounds always read from disk (no caching — that's the point of streaming).

**Static vs Streaming:**
- **Static** (`streaming: false`): Loads the entire file into memory via `StaticSoundData`. Best for short sound effects (footsteps, UI clicks, gunshots). Cached for reuse.
- **Streaming** (`streaming: true`): Decodes gradually from disk via `StreamingSoundData`. Best for long music tracks and ambient audio. Lower memory usage but slightly higher CPU overhead.

**Playback configuration:**
- Volume: passed directly to `StaticSoundSettings::volume()` or `StreamingSoundData::volume()`
- Pitch: converted to `f64`, passed as `playback_rate()`
- Looping: when enabled, sets `loop_region(..)` (loops the entire sound)

**Finished sound pruning:** Each call to `play_sound` automatically removes handles for sounds that have finished playing (`PlaybackState::Stopped`), preventing unbounded handle accumulation.

## AudioSourceComponent

**File:** `gg_engine/src/scene/components.rs`

```rust
pub struct AudioSourceComponent {
    pub audio_handle: Uuid,           // asset registry UUID (0 = none)
    pub volume: f32,                  // 0.0-1.0, default 1.0
    pub pitch: f32,                   // playback rate, 1.0 = normal, default 1.0
    pub looping: bool,                // default false
    pub play_on_start: bool,          // auto-play on entering play mode, default false
    pub streaming: bool,              // stream from disk instead of loading into memory, default false
    pub spatial: bool,                // enable spatial audio (panning + distance attenuation), default false
    pub min_distance: f32,            // distance at which attenuation begins, default 1.0
    pub max_distance: f32,            // distance at which sound is fully attenuated, default 50.0
    pub(crate) resolved_path: Option<String>,  // runtime-only, not serialized
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `audio_handle` | `0` | Asset registry UUID referencing the audio file |
| `volume` | `1.0` | Playback volume (0.0 = silent, 1.0 = full) |
| `pitch` | `1.0` | Playback rate (< 1.0 = slower/lower, > 1.0 = faster/higher) |
| `looping` | `false` | Whether the sound loops continuously |
| `play_on_start` | `false` | Automatically play when entering play mode |
| `streaming` | `false` | Stream from disk instead of loading into memory. Better for long music tracks |
| `spatial` | `false` | Enable spatial audio (panning and distance attenuation based on entity position relative to camera) |
| `min_distance` | `1.0` | Distance from the listener at which attenuation begins (no attenuation within this radius) |
| `max_distance` | `50.0` | Distance at which the sound is fully attenuated (-60 dB) |
| `category` | `SFX` | Sound category for volume mixing (`SFX`, `Music`, `Ambient`, `Voice`). Effective volume = entity × category × master |
| `resolved_path` | `None` | Absolute file path resolved at runtime from asset manager. Not serialized |

**Supported formats:** WAV, OGG, MP3, FLAC (via kira's built-in decoders).

Derives `Clone`. The `resolved_path` is cloned as-is (unlike physics handles which reset to `None` on clone).

## Scene Integration / Lifecycle

**File:** `gg_engine/src/scene/mod.rs`

Audio lifecycle is managed by the Scene and runs only in **Play mode** (not Simulate mode).

### Startup Sequence

Called from `Scene::on_runtime_start()`, after physics and scripting initialization:

```
on_runtime_start()
  1. on_physics_2d_start()
  2. on_lua_scripting_start()     [Play mode only]
  3. on_audio_start()             [Play mode only]
```

### on_audio_start()

1. Creates an `AudioEngine` via `AudioEngine::new()`. If creation fails, returns early (no audio).
2. Queries all entities with `IdComponent` + `AudioSourceComponent`.
3. Filters for entities where `play_on_start == true` AND `resolved_path.is_some()`.
4. Plays each matching entity's sound through the audio engine.

### on_audio_stop()

1. Calls `stop_all()` on the audio engine.
2. Drops the audio engine (`self.audio_engine = None`).

Called from `Scene::on_runtime_stop()`, before scripting and physics teardown.

### resolve_audio_handles()

```rust
pub fn resolve_audio_handles(&mut self, asset_manager: &mut EditorAssetManager)
```

Resolves asset UUIDs to absolute file paths:

1. Queries all entities with `AudioSourceComponent` where `audio_handle != 0` and `resolved_path.is_none()`.
2. For each, looks up the absolute path via `asset_manager.get_absolute_path()`.
3. If the path exists on disk, stores it in `resolved_path`.

This must be called before `on_runtime_start()` for `play_on_start` sounds to work. Typically called alongside `resolve_texture_handles()` in the editor/player startup path.

### Simulate vs Play

| Mode | Physics | Lua Scripts | Audio |
|------|---------|-------------|-------|
| Play | Yes | Yes | Yes |
| Simulate | Yes | No | No |

`on_simulation_start()` calls only `on_physics_2d_start()` — no audio engine is created.

### Scene::copy()

The `AudioEngine` is **not** copied during `Scene::copy()`. A fresh engine is created on the next `on_runtime_start()`. The `AudioSourceComponent` itself is cloned normally (including `resolved_path`).

## Spatial Audio

**File:** `gg_engine/src/scene/mod.rs` — `update_spatial_audio()`

When `AudioSourceComponent::spatial` is `true`, the engine automatically computes panning and distance attenuation each frame based on the entity's position relative to the primary camera.

### How it works

`Scene::update_spatial_audio()` is called each frame during Play and Simulate modes (after `on_update_animations`):

1. **Find the listener**: queries for the primary `CameraComponent` + `TransformComponent`, uses the camera's 2D position as the listener
2. **Collect spatial updates**: for each entity with `spatial: true`, computes:
   - **Panning**: `delta.x / max_distance`, clamped to [-1.0, 1.0]. Gives natural stereo spread proportional to horizontal offset
   - **Distance attenuation**: linear dB falloff from 0 dB at `min_distance` to -60 dB at `max_distance`. Sounds within `min_distance` are at full volume; sounds beyond `max_distance` are effectively silent
   - **Effective volume**: `component_volume + attenuation_db` (additive in dB space)
3. **Apply**: sets panning and volume on all active sound handles for each entity

Uses a collect-then-apply pattern (collects updates into a `Vec`, then applies via `audio_engine`) to avoid borrow checker issues with simultaneous ECS queries and audio engine mutation.

## Runtime API

**File:** `gg_engine/src/scene/mod.rs`

These methods are used by Lua scripts (via script glue) to control audio at runtime.

| Method | Description |
|--------|-------------|
| `play_entity_sound(entity)` | Reads the entity's `AudioSourceComponent` (volume, pitch, looping, streaming, resolved_path), plays via `AudioEngine::play_sound()` |
| `stop_entity_sound(entity)` | Looks up the entity's UUID, calls `AudioEngine::stop_sound()` |
| `set_entity_volume(entity, volume)` | Looks up the entity's UUID, calls `AudioEngine::set_volume()` |
| `set_entity_panning(entity, panning)` | Looks up the entity's UUID, calls `AudioEngine::set_panning()` |
| `fade_in_entity_sound(entity, secs)` | Fade in from silence (resumes paused sounds or plays new) |
| `fade_out_entity_sound(entity, secs)` | Fade to silence and stop |
| `fade_to_entity_volume(entity, vol, secs)` | Fade to target volume over time |
| `set_master_volume(volume)` | Set global master volume (0.0–1.0) |
| `get_master_volume()` | Get global master volume |
| `set_category_volume(cat, volume)` | Set per-category volume (0.0–1.0) |
| `get_category_volume(cat)` | Get per-category volume |
| `update_spatial_audio()` | Updates panning and volume for all spatial audio sources based on listener position; also drains completed sounds for `on_sound_finished` callbacks |

All methods are no-ops if the entity lacks the required components, has no resolved path, or the audio engine is `None`.

## Lua Scripting API

**File:** `gg_engine/src/scene/script_glue.rs`

Registered on the `Engine` global table:

| Lua Function | Signature | Description |
|-------------|-----------|-------------|
| `Engine.play_sound(entity_id)` | `(u64) -> ()` | Play the entity's audio source |
| `Engine.stop_sound(entity_id)` | `(u64) -> ()` | Stop the entity's audio playback |
| `Engine.pause_sound(entity_id)` | `(u64) -> ()` | Pause the entity's audio (resumable) |
| `Engine.resume_sound(entity_id)` | `(u64) -> ()` | Resume paused audio |
| `Engine.set_volume(entity_id, volume)` | `(u64, f32) -> ()` | Adjust volume at runtime (linear 0.0–1.0) |
| `Engine.set_panning(entity_id, panning)` | `(u64, f32) -> ()` | Set stereo panning (-1.0 = left, 0.0 = center, 1.0 = right) |
| `Engine.fade_in(entity_id, secs)` | `(u64, f32) -> ()` | Fade in from silence (play or resume) |
| `Engine.fade_out(entity_id, secs)` | `(u64, f32) -> ()` | Fade to silence and stop |
| `Engine.fade_to(entity_id, vol, secs)` | `(u64, f32, f32) -> ()` | Fade to target volume |
| `Engine.set_master_volume(volume)` | `(f32) -> ()` | Set global master volume (0.0–1.0) |
| `Engine.get_master_volume()` | `() -> f32` | Get global master volume |
| `Engine.set_category_volume(cat, vol)` | `(string, f32) -> ()` | Set category volume ("sfx"/"music"/"ambient"/"voice") |
| `Engine.get_category_volume(cat)` | `(string) -> f32` | Get category volume |

All bindings access the scene through `SceneScriptContext` (the standard take-modify-replace pattern). Entity is looked up by UUID via `scene.find_entity_by_uuid()`. No-op if context is unavailable or entity not found.

### Lua Example

```lua
-- Play a sound on key press
function on_update(dt)
    local self_id = Engine.get_uuid()

    if Engine.is_key_pressed("Space") then
        Engine.play_sound(self_id)
    end

    if Engine.is_key_pressed("S") then
        Engine.stop_sound(self_id)
    end

    -- Adjust volume and panning at runtime
    Engine.set_volume(self_id, 0.5)
    Engine.set_panning(self_id, -0.5)  -- slightly to the left
end
```

### Sound Completion Callback

When all sounds on an entity finish playing naturally, the `on_sound_finished()` callback is invoked on the entity's Lua script. This does **not** fire when sounds are explicitly stopped via `Engine.stop_sound()` or `Engine.fade_out()`.

```lua
function on_sound_finished()
    -- Play next track or transition
    Engine.play_sound(Engine.get_uuid())
end
```

### Volume Mixing

Effective volume = `entity_volume × category_volume × master_volume` (all linear 0.0–1.0).

```lua
function on_create()
    -- Set up volume levels
    Engine.set_master_volume(0.8)
    Engine.set_category_volume("music", 0.6)
    Engine.set_category_volume("sfx", 1.0)
end
```

### Fade Examples

```lua
-- Fade in music over 2 seconds
Engine.fade_in(music_entity_id, 2.0)

-- Fade out and stop over 1.5 seconds
Engine.fade_out(music_entity_id, 1.5)

-- Crossfade: fade out old, fade in new
Engine.fade_out(old_music_id, 2.0)
Engine.fade_in(new_music_id, 2.0)

-- Fade volume to 50% over 1 second
Engine.fade_to(entity_id, 0.5, 1.0)
```

## Editor UI

**File:** `gg_editor/src/panels/properties/audio.rs`

The audio source properties panel (`draw_audio_source_component`) renders inside the entity properties panel as a collapsible "Audio Source" section.

### Controls

| Control | Type | Details |
|---------|------|---------|
| Audio File | Button + file dialog | Opens file picker filtered to `assets/audio/` directory. Accepts WAV, OGG, MP3, FLAC |
| Drag-and-drop | `ContentBrowserPayload` | Accepts audio files dragged from the content browser (validated by extension) |
| Clear (X) | Small button | Removes the asset reference (sets `audio_handle` to 0). Only shown when an asset is assigned |
| Volume | Slider | Range 0.0 to 1.0 |
| Pitch | DragValue | Range 0.1 to 4.0, step 0.01 |
| Looping | Checkbox | Toggle looping playback |
| Play On Start | Checkbox | Toggle auto-play on entering play mode |
| Streaming | Checkbox | Stream from disk instead of loading into memory (tooltip: "Better for long music tracks") |
| Spatial Audio | Checkbox | Enable camera-relative panning and distance attenuation (tooltip explains behavior) |
| Min Distance | DragValue | Only shown when spatial is enabled. Distance at which attenuation begins (range 0.0 to max_distance) |
| Max Distance | DragValue | Only shown when spatial is enabled. Distance at which sound is fully attenuated (range min_distance to 1000.0) |

The audio file button displays the asset filename (resolved from the asset registry) or "None" if no asset is assigned. A blue highlight stroke appears when hovering with a valid drag-and-drop payload. The component can be removed via right-click context menu on the header.

## Serialization

**File:** `gg_engine/src/scene/scene_serializer.rs`

Uses an intermediate `AudioSourceData` struct for serde:

```rust
struct AudioSourceData {
    audio_handle: u64,    // "AudioHandle", skipped if 0
    volume: f32,          // "Volume", default 1.0
    pitch: f32,           // "Pitch", default 1.0
    looping: bool,        // "Looping", default false
    play_on_start: bool,  // "PlayOnStart", default false
    streaming: bool,      // "Streaming", default false
    spatial: bool,        // "Spatial", default false
    min_distance: f32,    // "MinDistance", default 1.0, skipped if default
    max_distance: f32,    // "MaxDistance", default 50.0, skipped if default
}
```

| YAML Key | Type | Default | Notes |
|----------|------|---------|-------|
| `AudioHandle` | `u64` | `0` | Skipped in output if zero |
| `Volume` | `f32` | `1.0` | |
| `Pitch` | `f32` | `1.0` | |
| `Looping` | `bool` | `false` | |
| `PlayOnStart` | `bool` | `false` | |
| `Streaming` | `bool` | `false` | |
| `Spatial` | `bool` | `false` | |
| `MinDistance` | `f32` | `1.0` | Skipped in output if default |
| `MaxDistance` | `f32` | `50.0` | Skipped in output if default |

All new fields use `#[serde(default)]` for backward compatibility with existing `.ggscene` files.

`resolved_path` is **never** serialized — it is runtime-only, populated by `resolve_audio_handles()` after deserialization.

### YAML Example

```yaml
- Entity: 42
  TagComponent:
    Tag: BGM
  TransformComponent:
    Translation: [0.0, 0.0, 0.0]
    Rotation: [0.0, 0.0, 0.0]
    Scale: [1.0, 1.0, 1.0]
  AudioSourceComponent:
    AudioHandle: 77777
    Volume: 0.75
    Pitch: 1.2
    Looping: true
    PlayOnStart: true
    Streaming: true
    Spatial: true
    MinDistance: 2.0
    MaxDistance: 30.0
```
