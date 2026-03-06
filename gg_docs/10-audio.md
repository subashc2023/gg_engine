# Audio System

The engine integrates [kira](https://docs.rs/kira/0.12) 0.12 for audio playback. Sounds are keyed by entity UUID — each entity can have at most one active sound at a time. Audio assets are referenced by UUID handle (`AudioSourceComponent`) and resolved to file paths at runtime via the asset manager.

**Files:**
- `gg_engine/src/scene/audio.rs` — `AudioEngine` struct, kira wrapper
- `gg_engine/src/scene/components.rs` — `AudioSourceComponent` definition
- `gg_engine/src/scene/mod.rs` — Scene audio lifecycle (`on_audio_start`, `on_audio_stop`, `resolve_audio_handles`, `play_entity_sound`, `stop_entity_sound`, `set_entity_volume`)
- `gg_engine/src/scene/script_glue.rs` — Lua bindings (`play_sound`, `stop_sound`, `set_volume`)
- `gg_editor/src/panels/properties/audio.rs` — Editor UI for audio source properties

## AudioEngine

**File:** `gg_engine/src/scene/audio.rs`

Wraps `kira::AudioManager<DefaultBackend>` with entity-keyed playback tracking and file caching.

```rust
pub(crate) struct AudioEngine {
    manager: AudioManager,
    active_sounds: HashMap<u64, StaticSoundHandle>,
    sound_cache: HashMap<String, StaticSoundData>,
}
```

| Field | Type | Description |
|-------|------|-------------|
| `manager` | `kira::AudioManager` | Kira audio manager (default backend) |
| `active_sounds` | `HashMap<u64, StaticSoundHandle>` | Active sound handles keyed by entity UUID |
| `sound_cache` | `HashMap<String, StaticSoundData>` | Cached loaded audio files keyed by absolute file path |

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new()` | `-> Option<Self>` | Creates the kira `AudioManager`. Returns `None` on failure (logged as error) |
| `play_sound` | `(entity_uuid, path, volume, pitch, looping)` | Stops any existing sound for the entity, loads (or retrieves cached) audio data, configures volume/pitch/looping, plays via kira. Inserts handle into `active_sounds` |
| `stop_sound` | `(entity_uuid)` | Removes and stops the sound for the given entity (uses `Tween::default()` for fade) |
| `set_volume` | `(entity_uuid, volume)` | Adjusts volume on an active sound handle (uses `Tween::default()`) |
| `stop_all` | `()` | Drains and stops all active sounds |

**Sound caching:** The first call to `play_sound` for a given file path loads `StaticSoundData::from_file()` and caches it. Subsequent plays of the same file clone the cached data (avoiding disk I/O).

**Playback configuration:**
- Volume: passed directly to `StaticSoundSettings::volume()`
- Pitch: converted to `f64`, passed to `StaticSoundSettings::playback_rate()`
- Looping: when enabled, sets `StaticSoundSettings::loop_region(..)` (loops the entire sound)

## AudioSourceComponent

**File:** `gg_engine/src/scene/components.rs`

```rust
pub struct AudioSourceComponent {
    pub audio_handle: Uuid,           // asset registry UUID (0 = none)
    pub volume: f32,                  // 0.0-1.0, default 1.0
    pub pitch: f32,                   // playback rate, 1.0 = normal, default 1.0
    pub looping: bool,                // default false
    pub play_on_start: bool,          // auto-play on entering play mode, default false
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

## Runtime API

**File:** `gg_engine/src/scene/mod.rs`

These methods are used by Lua scripts (via script glue) to control audio at runtime.

| Method | Description |
|--------|-------------|
| `play_entity_sound(entity)` | Reads the entity's `AudioSourceComponent` (volume, pitch, looping, resolved_path), plays via `AudioEngine::play_sound()` |
| `stop_entity_sound(entity)` | Looks up the entity's UUID, calls `AudioEngine::stop_sound()` |
| `set_entity_volume(entity, volume)` | Looks up the entity's UUID, calls `AudioEngine::set_volume()` |

All three methods are no-ops if the entity lacks the required components, has no resolved path, or the audio engine is `None`.

## Lua Scripting API

**File:** `gg_engine/src/scene/script_glue.rs`

Registered on the `Engine` global table:

| Lua Function | Rust Binding | Signature | Description |
|-------------|--------------|-----------|-------------|
| `Engine.play_sound(entity_id)` | `lua_play_sound` | `(u64) -> ()` | Play the entity's audio source |
| `Engine.stop_sound(entity_id)` | `lua_stop_sound` | `(u64) -> ()` | Stop the entity's audio playback |
| `Engine.set_volume(entity_id, volume)` | `lua_set_volume` | `(u64, f32) -> ()` | Adjust volume at runtime |

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

    -- Fade volume based on distance or other logic
    Engine.set_volume(self_id, 0.5)
end
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
}
```

| YAML Key | Type | Default | Notes |
|----------|------|---------|-------|
| `AudioHandle` | `u64` | `0` | Skipped in output if zero |
| `Volume` | `f32` | `1.0` | |
| `Pitch` | `f32` | `1.0` | |
| `Looping` | `bool` | `false` | |
| `PlayOnStart` | `bool` | `false` | |

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
```
