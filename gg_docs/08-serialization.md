# Scene Serialization

**File:** `scene/scene_serializer.rs`

YAML-based scene persistence using `serde` + `serde_yml`. File extension: `.ggscene`.

## Design

Scene types (`TransformComponent`, `CameraComponent`, etc.) have **no serde derives**. The `SceneSerializer` owns all serialization logic via intermediate data structs (`SceneData`, `EntityData`, etc.), decoupling scene implementation from serialization concerns.

## API

```rust
// File I/O
SceneSerializer::serialize(&scene, "path/to/scene.ggscene")            -> bool
SceneSerializer::deserialize(&mut scene, "path/to/scene.ggscene")      -> bool

// In-memory snapshots (auto-save support)
SceneSerializer::serialize_to_string(&scene)                           -> Option<String>
SceneSerializer::deserialize_from_string(&mut scene, &yaml_string)     -> bool
```

All methods return `bool` (or `Option`) — errors are logged, not panicked. `deserialize` **appends** entities to the scene; callers should provide a fresh scene for a clean load.

The in-memory variants (`serialize_to_string` / `deserialize_from_string`) enable auto-save and recovery without file I/O. The editor uses these to snapshot scene state periodically, and can restore from the YAML string on crash recovery.

## YAML Format

```yaml
Version: 1
Scene: Untitled
Entities:
- Entity: 100                          # UUID as u64
  TagComponent:
    Tag: Camera
  TransformComponent:
    Translation: [0.0, 5.0, 0.0]
    Rotation: [0.0, 0.0, 0.0]         # radians (Euler XYZ)
    Scale: [1.0, 1.0, 1.0]
  RelationshipComponent:
    Parent: 200                        # parent entity UUID (omitted if none)
    Children: [300, 400]               # child entity UUIDs (omitted if empty)
  CameraComponent:
    Camera:
      ProjectionType: 1               # 0 = Perspective, 1 = Orthographic
      PerspectiveFOV: 0.7853982
      PerspectiveNear: 0.01
      PerspectiveFar: 1000.0
      OrthographicSize: 10.0
      OrthographicNear: -1.0
      OrthographicFar: 1.0
    Primary: true
    FixedAspectRatio: false
  SpriteRendererComponent:
    Color: [0.2, 0.3, 0.8, 1.0]       # RGBA
    TilingFactor: 1.0
    TextureHandle: 12345678901234       # Asset UUID (0 = none, skipped)
  CircleRendererComponent:
    Color: [1.0, 0.5, 0.0, 1.0]
    Thickness: 1.0                     # 1.0 = filled, <1.0 = ring
    Fade: 0.005
  RigidBody2DComponent:
    BodyType: Dynamic                  # Static | Dynamic | Kinematic
    FixedRotation: false
  BoxCollider2DComponent:
    Offset: [0.0, 0.0]
    Size: [0.5, 0.5]                   # half-extents
    Density: 1.0
    Friction: 0.5
    Restitution: 0.0
  CircleCollider2DComponent:
    Offset: [0.0, 0.0]
    Radius: 0.5
    Density: 1.0
    Friction: 0.5
    Restitution: 0.0
  TextComponent:
    Text: Hello World
    FontPath: assets/fonts/JetBrainsMono-Regular.ttf
    FontSize: 1.0
    Color: [1.0, 1.0, 1.0, 1.0]
    LineSpacing: 0.0
    Kerning: 0.0
  LuaScriptComponent:
    ScriptPath: assets/scripts/player.lua
    Fields:                            # field overrides (omitted if empty)
      speed: 5.0
      maxHealth: 100
      playerName: Hero
  SpriteAnimatorComponent:
    CellSize: [16.0, 16.0]
    Columns: 8
    Clips:
    - Name: idle
      StartFrame: 0
      EndFrame: 3
      FPS: 12.0                        # default: 12.0
      Looping: true                    # default: true
    - Name: run
      StartFrame: 4
      EndFrame: 11
      FPS: 16.0
      Looping: true
  AudioSourceComponent:
    AudioHandle: 56789012345678        # Asset UUID (0 = none, skipped)
    Volume: 1.0                        # default: 1.0
    Pitch: 1.0                         # default: 1.0
    Looping: false
    PlayOnStart: false
  TilemapComponent:
    Width: 10
    Height: 10
    TileSize: [1.0, 1.0]
    TextureHandle: 99887766554433      # Asset UUID (0 = none, skipped)
    TilesetColumns: 8                  # default: 1
    CellSize: [16.0, 16.0]
    Spacing: [1.0, 1.0]               # default: [0,0], skipped if zero
    Margin: [2.0, 2.0]                # default: [0,0], skipped if zero
    Tiles: [0, 1, 2, -1, 3, 4, ...]   # -1 = empty, supports TILE_FLIP_H/V flags
```

Optional components are omitted from YAML when not present (`skip_serializing_if = "Option::is_none"`). Missing components default to `None` on load (`default` attribute). `RelationshipComponent` uses a custom `has_no_relationships()` skip function — it is omitted when both parent is `None` and children is empty. `RestitutionThreshold` is a legacy field on colliders: deserialized (for backwards compat) but no longer serialized.

## Intermediate Serde Structs

| Struct | YAML Key | Fields |
|--------|----------|--------|
| `SceneData` | (root) | `version: u32` (default 1), `name` ("Untitled"), `entities: Vec<EntityData>` |
| `EntityData` | `Entity: <uuid>` | UUID + all optional component data |
| `TagData` | `TagComponent` | `tag: String` |
| `TransformData` | `TransformComponent` | `translation: [f32;3]`, `rotation: [f32;3]`, `scale: [f32;3]` |
| `CameraData` | `CameraComponent` | `camera: SceneCameraData`, `primary: bool`, `fixed_aspect_ratio: bool` |
| `SceneCameraData` | `Camera` | `projection_type: u32`, perspective FOV/near/far, orthographic size/near/far |
| `SpriteData` | `SpriteRendererComponent` | `color: [f32;4]`, `tiling_factor: f32` (default 1.0), `texture_handle: u64` (asset UUID, 0 = none) |
| `CircleData` | `CircleRendererComponent` | `color: [f32;4]`, `thickness: f32` (default 1.0), `fade: f32` (default 0.005) |
| `RigidBody2DData` | `RigidBody2DComponent` | `body_type: String`, `fixed_rotation: bool` |
| `BoxCollider2DData` | `BoxCollider2DComponent` | `offset: [f32;2]`, `size: [f32;2]`, density/friction/restitution (+ legacy `_restitution_threshold`, skip_serializing) |
| `CircleCollider2DData` | `CircleCollider2DComponent` | `offset: [f32;2]`, `radius: f32`, density/friction/restitution (+ legacy `_restitution_threshold`, skip_serializing) |
| `TextData` | `TextComponent` | `text: String`, `font_path: String`, `font_size: f32`, `color: [f32;4]`, `line_spacing: f32`, `kerning: f32` |
| `LuaScriptData` | `LuaScriptComponent` | `script_path: String`, `fields: Option<HashMap<String, ScriptFieldValue>>` (omitted if empty) |
| `RelationshipData` | `RelationshipComponent` | `parent: Option<u64>`, `children: Vec<u64>` |
| `SpriteAnimatorData` | `SpriteAnimatorComponent` | `cell_size: [f32;2]`, `columns: u32`, `clips: Vec<AnimationClipData>` |
| `AnimationClipData` | (nested in Clips) | `name: String`, `start_frame: u32`, `end_frame: u32`, `fps: f32` (default 12.0), `looping: bool` (default true) |
| `AudioSourceData` | `AudioSourceComponent` | `audio_handle: u64`, `volume: f32` (default 1.0), `pitch: f32` (default 1.0), `looping: bool`, `play_on_start: bool` |
| `TilemapData` | `TilemapComponent` | `width: u32`, `height: u32`, `tile_size: [f32;2]`, `texture_handle: u64`, `tileset_columns: u32` (default 1), `cell_size: [f32;2]`, `spacing: [f32;2]` (default [0,0]), `margin: [f32;2]` (default [0,0]), `tiles: Vec<i32>` |

All field names use PascalCase in YAML via `#[serde(rename)]`.

### Default Value Functions

Several fields use custom default functions for backwards-compatible deserialization:

| Function | Returns | Used By |
|----------|---------|---------|
| `default_scene_version()` | `1` (u32) | `SceneData::version` |
| `default_tiling_factor()` | `1.0` (f32) | `SpriteData::tiling_factor` |
| `default_thickness()` | `1.0` (f32) | `CircleData::thickness` |
| `default_fade()` | `0.005` (f32) | `CircleData::fade` |
| `default_font_size()` | `1.0` (f32) | `TextData::font_size` |
| `default_line_spacing()` | `1.0` (f32) | `TextData::line_spacing` |
| `default_animation_fps()` | `12.0` (f32) | `AnimationClipData::fps` |
| `default_true()` | `true` (bool) | `AnimationClipData::looping` |
| `default_volume()` | `1.0` (f32) | `AudioSourceData::volume` |
| `default_pitch()` | `1.0` (f32) | `AudioSourceData::pitch` |
| `default_tileset_columns()` | `1` (u32) | `TilemapData::tileset_columns` |
| `default_zero_vec2()` | `[0.0, 0.0]` | `TilemapData::spacing`, `TilemapData::margin` |

### ScriptFieldValue (Lua Field Overrides)

`ScriptFieldValue` is an `#[serde(untagged)]` enum enabling clean YAML output without type tags:

```rust
#[serde(untagged)]
pub enum ScriptFieldValue {
    Bool(bool),     // must be first — prevents true/false being parsed as strings
    Float(f64),
    String(String),
}
```

This produces YAML like `speed: 5.0` rather than `speed: !Float 5.0`. When `lua-scripting` is disabled, fields are preserved as opaque `serde_yaml_ng::Value` to avoid data loss on round-trip.

## Serialized vs Runtime-Only

| Component | Serialized | Runtime-Only Fields |
|-----------|:----------:|---------------------|
| `IdComponent` | UUID (as `Entity: <u64>` in YAML) | -- |
| `TagComponent` | tag | -- |
| `TransformComponent` | translation, rotation, scale | -- |
| `CameraComponent` | all projection params, primary, fixed_aspect_ratio | -- |
| `SpriteRendererComponent` | color, tiling_factor, texture_handle (UUID) | `texture: Option<Arc<Texture2D>>` (resolved at runtime via asset manager) |
| `CircleRendererComponent` | color, thickness, fade | -- |
| `RigidBody2DComponent` | body_type, fixed_rotation | `runtime_body: Option<RigidBodyHandle>` |
| `BoxCollider2DComponent` | offset, size, density, friction, restitution | `runtime_fixture: Option<ColliderHandle>` |
| `CircleCollider2DComponent` | offset, radius, density, friction, restitution | `runtime_fixture: Option<ColliderHandle>` |
| `TextComponent` | text, font_path, font_size, color, line_spacing, kerning | Font GPU resources (cached on Scene) |
| `LuaScriptComponent` | script_path, field_overrides | `loaded: bool` |
| `RelationshipComponent` | parent, children | -- |
| `SpriteAnimatorComponent` | cell_size, columns, clips | `current_clip_index`, `frame_timer`, `current_frame`, `playing` |
| `AudioSourceComponent` | audio_handle, volume, pitch, looping, play_on_start | `resolved_path: Option<String>` |
| `TilemapComponent` | width, height, tile_size, texture_handle, tileset_columns, cell_size, spacing, margin, tiles | `texture: Option<Arc<Texture2D>>` |
| `NativeScriptComponent` | **NOT serialized** (code-defined) | `instance`, `instantiate_fn`, `created` |

## Deserialization Flow

```
YAML file
    |  fs::read_to_string()
    v
YAML string
    |  serde_yml::from_str()
    v
SceneData (intermediate structs)
    |  data_to_scene()
    v
For each EntityData:
    1. Extract tag name (default "Entity")
    2. create_entity_with_uuid(Uuid::from_raw(id), name)
       -> spawns IdComponent + TagComponent + TransformComponent(IDENTITY)
    3. Update TransformComponent with deserialized values
    4. Add optional components if present in YAML:
       - CameraComponent (reconstructs SceneCamera, sets projection type)
       - SpriteRendererComponent (color, tiling, texture_handle -- resolved later via asset manager)
       - CircleRendererComponent
       - TextComponent (font = None, loaded later)
       - RigidBody2DComponent (runtime_body = None)
       - BoxCollider2DComponent (runtime_fixture = None)
       - CircleCollider2DComponent (runtime_fixture = None)
       - LuaScriptComponent (loaded = false, field_overrides restored from YAML)
       - SpriteAnimatorComponent (clips restored, runtime state default: not playing)
       - RelationshipComponent (parent/children UUIDs restored)
       - AudioSourceComponent (resolved_path = None)
       - TilemapComponent (texture = None, resolved later via asset manager)
```

Physics handles and Lua scripts are initialized later when entering play mode via `on_runtime_start()`. Texture handles (on SpriteRendererComponent, TilemapComponent) are resolved to GPU textures via `Scene::resolve_texture_handles(asset_manager, renderer)`. Audio handles are resolved to file paths at runtime start.

## Version Field

The `SceneData` root includes a `Version` field (default `1`, constant `SCENE_VERSION`). On deserialization, if the file's version is higher than the current `SCENE_VERSION`, a warning is logged. This enables future format migrations.

## Scene Copy

`Scene::copy(source: &Scene) -> Scene` creates a deep copy with UUID preservation. Used by the editor for play/simulate mode instead of serialization snapshots.

### Phase 1: Entity Creation

1. Create empty destination scene, copy viewport dimensions
2. Query all source entities for `(hecs::Entity, IdComponent, TagComponent)`
3. Sort by hecs entity ID to preserve Scene Hierarchy ordering
4. Create each entity in destination via `create_entity_with_uuid(uuid, tag)`
5. Build `HashMap<source_handle, destination_entity>` for component copying

### Phase 2: Component Cloning

All `Clone`-able components are copied via the `for_each_cloneable_component!` macro and the `copy_component_if_has::<T>` generic helper:

```rust
// for_each_cloneable_component! expands to copy all of these:
//   TransformComponent, CameraComponent, SpriteRendererComponent,
//   CircleRendererComponent, TextComponent, RigidBody2DComponent,
//   BoxCollider2DComponent, CircleCollider2DComponent,
//   RelationshipComponent, SpriteAnimatorComponent,
//   TilemapComponent, AudioSourceComponent

// NativeScriptComponent -- manual copy (not Clone):
//   Copies instantiate_fn pointer, sets instance = None, created = false
```

Adding a new cloneable component to the `for_each_cloneable_component!` macro in `scene/mod.rs` automatically includes it in both scene copy and entity duplication.

### Key Behaviors

- **UUIDs preserved**: Entities keep their original `IdComponent` UUIDs
- **Physics handles reset**: `runtime_body` / `runtime_fixture` -> `None` (recreated at runtime)
- **Lua scripts reset**: `loaded` -> `false` (re-loaded when play starts)
- **NativeScript re-instantiated**: `instance` dropped, `instantiate_fn` preserved for lazy creation
- **Textures shared**: `Arc<Texture2D>` reference count incremented (no GPU re-upload)
- **Animator state reset**: `current_clip_index`, `frame_timer`, `current_frame`, `playing` reset to defaults
- **Audio resolved_path reset**: `resolved_path` -> `None` (re-resolved at runtime)
- **Order preserved**: Destination entities spawned in same relative order as source

## UUID System

**File:** `uuid.rs`

```rust
pub struct Uuid(u64);

const UUID_SAFE_MASK: u64 = (1u64 << 53) - 1;

impl Uuid {
    pub fn new() -> Self { Self(rand::rng().random::<u64>() & UUID_SAFE_MASK) }
    pub fn from_raw(value: u64) -> Self { Self(value) }
    pub fn raw(&self) -> u64 { self.0 }
}
```

- 53-bit random values for lossless round-trip through Lua/JavaScript f64
- `0` is reserved as "uninitialized" / null
- Serialized as `u64` in YAML, restored via `Uuid::from_raw()`
- `IdComponent` wraps `Uuid`, spawned on every entity automatically

## Editor File Operations

| Operation | Shortcut | Behavior |
|-----------|----------|----------|
| New Scene | Ctrl+N | Fresh empty scene, clears `editor_scene_path` |
| Open | Ctrl+O | Native file dialog -> `SceneSerializer::deserialize()`, sets `editor_scene_path` |
| Save | Ctrl+S | If path set: saves directly. Otherwise: opens Save As |
| Save As | Ctrl+Shift+S | Native file dialog -> `SceneSerializer::serialize()`, sets `editor_scene_path` |

All file shortcuts stop playback first if in play/simulate mode. Native file dialogs via `rfd` crate (filter: `*.ggscene`).
