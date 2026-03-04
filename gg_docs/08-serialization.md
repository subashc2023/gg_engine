# Scene Serialization

**File:** `scene/scene_serializer.rs`

YAML-based scene persistence using `serde` + `serde_yaml`. File extension: `.ggscene`.

## Design

Scene types (`TransformComponent`, `CameraComponent`, etc.) have **no serde derives**. The `SceneSerializer` owns all serialization logic via intermediate data structs (`SceneData`, `EntityData`, etc.), decoupling scene implementation from serialization concerns.

## API

```rust
// File I/O
SceneSerializer::serialize(&scene, "path/to/scene.ggscene")            -> bool
SceneSerializer::deserialize(&mut scene, "path/to/scene.ggscene")      -> bool

// In-memory snapshots
SceneSerializer::serialize_to_string(&scene)                           -> Option<String>
SceneSerializer::deserialize_from_string(&mut scene, &yaml_string)     -> bool
```

All methods return `bool` (or `Option`) — errors are logged, not panicked. `deserialize` **appends** entities to the scene; callers should provide a fresh scene for a clean load.

## YAML Format

```yaml
Scene: Untitled
Entities:
- Entity: 100                          # UUID as u64
  TagComponent:
    Tag: Camera
  TransformComponent:
    Translation: [0.0, 5.0, 0.0]
    Rotation: [0.0, 0.0, 0.0]         # radians (Euler XYZ)
    Scale: [1.0, 1.0, 1.0]
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
    TextureHandle: 12345678901234       # Asset UUID (0 = none)
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
    RestitutionThreshold: 0.5
  CircleCollider2DComponent:
    Offset: [0.0, 0.0]
    Radius: 0.5
    Density: 1.0
    Friction: 0.5
    Restitution: 0.0
    RestitutionThreshold: 0.5
  TextComponent:
    Text: Hello World
    FontPath: assets/fonts/JetBrainsMono-Regular.ttf
    FontSize: 1.0
    Color: [1.0, 1.0, 1.0, 1.0]
    LineSpacing: 0.0
    Kerning: 0.0
  LuaScriptComponent:
    ScriptPath: assets/scripts/camera_controller.lua
```

Optional components are omitted from YAML when not present (`skip_serializing_if = "Option::is_none"`). Missing components default to `None` on load (`default` attribute).

## Intermediate Serde Structs

| Struct | YAML Key | Fields |
|--------|----------|--------|
| `SceneData` | (root) | `name` ("Untitled"), `entities: Vec<EntityData>` |
| `EntityData` | `Entity: <uuid>` | UUID + all optional component data |
| `TagData` | `TagComponent` | `tag: String` |
| `TransformData` | `TransformComponent` | `translation: [f32;3]`, `rotation: [f32;3]`, `scale: [f32;3]` |
| `CameraData` | `CameraComponent` | `camera: SceneCameraData`, `primary: bool`, `fixed_aspect_ratio: bool` |
| `SceneCameraData` | `Camera` | `projection_type: u32`, perspective FOV/near/far, orthographic size/near/far |
| `SpriteData` | `SpriteRendererComponent` | `color: [f32;4]`, `tiling_factor: f32` (default 1.0), `texture_handle: u64` (asset UUID, 0 = none) |
| `CircleData` | `CircleRendererComponent` | `color: [f32;4]`, `thickness: f32` (default 1.0), `fade: f32` (default 0.005) |
| `RigidBody2DData` | `RigidBody2DComponent` | `body_type: String`, `fixed_rotation: bool` |
| `BoxCollider2DData` | `BoxCollider2DComponent` | `offset: [f32;2]`, `size: [f32;2]`, density/friction/restitution/threshold |
| `CircleCollider2DData` | `CircleCollider2DComponent` | `offset: [f32;2]`, `radius: f32`, density/friction/restitution/threshold |
| `TextData` | `TextComponent` | `text: String`, `font_path: String`, `font_size: f32`, `color: [f32;4]`, `line_spacing: f32`, `kerning: f32` |
| `LuaScriptData` | `LuaScriptComponent` | `script_path: String` |

All field names use PascalCase in YAML via `#[serde(rename)]`.

## Serialized vs Runtime-Only

| Component | Serialized | Runtime-Only Fields |
|-----------|:----------:|---------------------|
| `IdComponent` | UUID (as `Entity: <u64>` in YAML) | — |
| `TagComponent` | tag | — |
| `TransformComponent` | translation, rotation, scale | — |
| `CameraComponent` | all projection params, primary, fixed_aspect_ratio | — |
| `SpriteRendererComponent` | color, tiling_factor, texture_handle (UUID) | `texture: Option<Arc<Texture2D>>` (resolved at runtime via asset manager) |
| `CircleRendererComponent` | color, thickness, fade | — |
| `RigidBody2DComponent` | body_type, fixed_rotation | `runtime_body: Option<RigidBodyHandle>` |
| `BoxCollider2DComponent` | offset, size, density, friction, restitution, threshold | `runtime_fixture: Option<ColliderHandle>` |
| `CircleCollider2DComponent` | offset, radius, density, friction, restitution, threshold | `runtime_fixture: Option<ColliderHandle>` |
| `TextComponent` | text, font_path, font_size, color, line_spacing, kerning | Font GPU resources (cached on Scene) |
| `LuaScriptComponent` | script_path | `loaded: bool` |
| `NativeScriptComponent` | **NOT serialized** (code-defined) | `instance`, `instantiate_fn`, `created` |

## Deserialization Flow

```
YAML file
    │  fs::read_to_string()
    ▼
YAML string
    │  serde_yaml::from_str()
    ▼
SceneData (intermediate structs)
    │  data_to_scene()
    ▼
For each EntityData:
    1. Extract tag name (default "Entity")
    2. create_entity_with_uuid(Uuid::from_raw(id), name)
       → spawns IdComponent + TagComponent + TransformComponent(IDENTITY)
    3. Update TransformComponent with deserialized values
    4. Add optional components if present in YAML:
       - CameraComponent (reconstructs SceneCamera, sets projection type)
       - SpriteRendererComponent (color, tiling, texture_handle — resolved later via asset manager)
       - CircleRendererComponent
       - RigidBody2DComponent (runtime_body = None)
       - BoxCollider2DComponent (runtime_fixture = None)
       - CircleCollider2DComponent (runtime_fixture = None)
       - LuaScriptComponent (loaded = false)
```

Physics handles and Lua scripts are initialized later when entering play mode via `on_runtime_start()`.

## Scene Copy

`Scene::copy(source: &Scene) -> Scene` creates a deep copy with UUID preservation. Used by the editor for play/simulate mode instead of serialization snapshots.

### Phase 1: Entity Creation

1. Create empty destination scene, copy viewport dimensions
2. Query all source entities for `(hecs::Entity, IdComponent, TagComponent)`
3. Sort by hecs entity ID to preserve Scene Hierarchy ordering
4. Create each entity in destination via `create_entity_with_uuid(uuid, tag)`
5. Build `HashMap<source_handle, destination_entity>` for component copying

### Phase 2: Component Cloning

```rust
// All Clone-able components copied via generic helper:
copy_component_if_has::<TransformComponent>(&source.world, &mut new_scene, &entity_map);
// ... same for Camera, Sprite, Circle, RigidBody2D, BoxCollider2D, CircleCollider2D, LuaScript

// NativeScriptComponent — manual copy (not Clone):
//   Copies instantiate_fn pointer, sets instance = None, created = false
```

### Key Behaviors

- **UUIDs preserved**: Entities keep their original `IdComponent` UUIDs
- **Physics handles reset**: `runtime_body` / `runtime_fixture` → `None` (recreated at runtime)
- **Lua scripts reset**: `loaded` → `false` (re-loaded when play starts)
- **NativeScript re-instantiated**: `instance` dropped, `instantiate_fn` preserved for lazy creation
- **Textures shared**: `Arc<Texture2D>` reference count incremented (no GPU re-upload)
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
| Open | Ctrl+O | Native file dialog → `SceneSerializer::deserialize()`, sets `editor_scene_path` |
| Save | Ctrl+S | If path set: saves directly. Otherwise: opens Save As |
| Save As | Ctrl+Shift+S | Native file dialog → `SceneSerializer::serialize()`, sets `editor_scene_path` |

All file shortcuts stop playback first if in play/simulate mode. Native file dialogs via `rfd` crate (filter: `*.ggscene`).
