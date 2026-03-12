# ECS & Scene

The Entity Component System lives in `gg_engine/src/scene/` and is built on top of [hecs](https://crates.io/crates/hecs) 0.11 (archetypal ECS storage).

## Scene

**File:** `scene/mod.rs`

`Scene` wraps `hecs::World` and owns all entity/component data. It also owns the physics world (`PhysicsWorld2D`), the Lua script engine (`ScriptEngine`), the audio engine (`AudioEngine`), and internal caches for UUID and name lookups.

### Entity Management

```rust
let entity = scene.create_entity();                              // Default: IdComponent + Tag("Entity") + Transform(IDENTITY) + RelationshipComponent
let entity = scene.create_entity_with_tag("Player");             // Custom name
let entity = scene.create_entity_with_uuid(uuid, "Player");      // Known UUID (deserialization)
scene.destroy_entity(entity);                                    // Recursive — destroys all children, detaches from parent
scene.queue_entity_destroy(uuid);                                // Deferred destruction (by UUID) — for use during script callbacks
scene.flush_pending_destroys();                                  // Flush the deferred destruction queue
scene.is_alive(entity);
scene.entity_count();
scene.find_entity_by_id(u32) -> Option<Entity>;                  // From hecs entity ID
scene.find_entity_by_uuid(u64) -> Option<Entity>;                // O(1) UUID lookup via internal cache
scene.find_entity_by_name(&str) -> Option<(Entity, u64)>;        // Lazy name cache, returns entity + UUID
scene.duplicate_entity(entity);                                   // New UUID, copies all components, resets relationship to root
```

**Note:** `create_entity` (all variants) now auto-adds four components: `IdComponent`, `TagComponent`, `TransformComponent`, and `RelationshipComponent`.

### Component Operations

```rust
scene.add_component(entity, MyComponent { ... });
scene.get_component::<T>(entity)     -> Option<hecs::Ref<T>>
scene.get_component_mut::<T>(entity) -> Option<hecs::RefMut<T>>
scene.has_component::<T>(entity)     -> bool
scene.remove_component::<T>(entity);
```

**Note:** `add_component` auto-calls `on_component_added` — for `CameraComponent`, this initializes viewport size.

### Queries

```rust
// Direct hecs queries via escape hatches
let world = scene.world();
for (entity, (transform, sprite)) in world.query::<(hecs::Entity, &TransformComponent, &SpriteRendererComponent)>().iter() {
    // ...
}
```

**hecs 0.11 query API:** `query::<Q>().iter()` yields `Q::Item` directly (NOT `(Entity, Q::Item)`). To get entity IDs, include `hecs::Entity` as a query component.

### Hierarchy Operations

Parent-child relationships are tracked via `RelationshipComponent` using entity UUIDs. These methods manage the hierarchy:

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_parent` | `(child, parent, preserve_world_transform) -> bool` | Parent `child` under `parent`. Returns `false` if it would create a cycle or self-parenting. When `preserve_world_transform` is `true`, the child's local transform is adjusted so its world position stays the same. |
| `detach_from_parent` | `(entity, preserve_world_transform)` | Remove entity from its parent, making it a root entity. Optionally preserves world-space position. |
| `get_parent` | `(entity) -> Option<u64>` | Returns the parent UUID, or `None` if root. |
| `get_children` | `(entity) -> Vec<u64>` | Returns ordered list of child UUIDs. |
| `reorder_child` | `(child_uuid, new_index)` | Move a child to a specific index within its parent's children list. |
| `is_ancestor_of` | `(ancestor_uuid, entity_uuid) -> bool` | Walk parent chain to check ancestry. Used for cycle detection. |
| `root_entities` | `() -> Vec<(Entity, String)>` | All entities without a parent, sorted by entity ID. |
| `get_world_transform` | `(entity) -> Mat4` | Compute world-space transform by walking the parent chain. No caching — O(depth) per call. |

**World-transform preservation:** When `preserve_world_transform` is `true` in `set_parent` or `detach_from_parent`, the entity's local transform is decomposed and rewritten so that `parent_world * local == original_world`. This ensures the entity doesn't jump visually when reparented.

**Cycle prevention:** `set_parent` checks `is_ancestor_of(child, parent)` before establishing the relationship. Self-parenting is also rejected.

### Utility Methods

| Method | Description |
|--------|-------------|
| `each_entity_with_tag()` | Returns `Vec<(Entity, String)>` sorted by ID |
| `set_primary_camera(entity)` | Clears primary on all others |
| `get_primary_camera_entity()` | Returns `Option<Entity>` |
| `copy(source)` | Deep-copies an entire scene (preserves UUIDs, resets physics/script handles) |
| `on_viewport_resize(w, h)` | Updates all non-fixed-aspect-ratio camera projections |
| `find_asset_references(asset_handle)` | Returns `Vec<(String, &str)>` of `(entity_name, component_kind)` pairs referencing the asset. Scans `SpriteRendererComponent::texture_handle`, `TilemapComponent::texture_handle`, and `AudioSourceComponent::audio_handle`. |

### Runtime Settings

Scene exposes a request/take pattern for dynamic settings controlled by Lua scripts:

| Method | Description |
|--------|-------------|
| `request_window_size(w, h)` | Request window resize (physical pixels) |
| `take_requested_window_size()` | Consume pending resize request |
| `request_vsync(bool)` | Request VSync toggle |
| `request_fullscreen(mode)` | Request fullscreen mode change |
| `request_shadow_quality(0-3)` | Request shadow quality tier |
| `set_gui_scale(factor)` / `gui_scale()` | Set/get global GUI scale (affects UI anchors) |
| `set_cursor_mode(mode)` / `cursor_mode()` | Set/get cursor mode (Normal/Confined/Locked) |
| `request_quit()` | Request application exit |
| `request_load_scene(path)` | Request scene transition (deferred) |

`FullscreenMode` enum: `Windowed`, `Borderless`, `Exclusive`.

### Rendering

Three render paths:

```rust
// Editor mode — external VP from EditorCamera
scene.on_update_editor(&editor_camera.view_projection(), &mut renderer);

// Runtime mode — finds primary CameraComponent, computes VP
scene.on_update_runtime(&mut renderer);

// Simulation mode — external camera, physics-only (no scripts)
scene.on_update_simulation(&editor_camera.view_projection(), &mut renderer);
```

All paths call the shared `render_scene()` which iterates `SpriteRendererComponent` (with optional `SpriteAnimatorComponent` for animated sprites), `CircleRendererComponent`, `TextComponent`, and `TilemapComponent` entities and submits draw calls. World transforms are pre-computed via `build_world_transform_cache()` for hierarchy-aware rendering.

### Animation Lifecycle

```rust
scene.on_update_animations(dt);  // Advances all SpriteAnimatorComponent timers
```

Call each frame before rendering. This only updates animator state (current frame, timer). The renderer reads the current frame to compute UV coordinates from the sprite sheet.

### Texture & Font Loading

```rust
scene.resolve_texture_handles(&mut asset_manager, &renderer);       // Sync: load SpriteRenderer + Tilemap textures
scene.resolve_texture_handles_async(&mut asset_manager);             // Async: request background loads
scene.resolve_audio_handles(&mut asset_manager);                     // Resolve AudioSourceComponent handles to file paths
scene.load_fonts(&renderer);                                         // Load MSDF fonts for TextComponent entities
```

### Audio Lifecycle

Audio playback is managed internally. The audio engine starts/stops with runtime mode. API for scripts:

```rust
scene.play_entity_sound(entity);         // Play audio for an entity's AudioSourceComponent
scene.stop_entity_sound(entity);         // Stop audio for an entity
scene.set_entity_volume(entity, vol);    // Adjust volume at runtime
```

### Physics Lifecycle

```rust
scene.on_runtime_start();       // Creates rapier2d world, spawns bodies/colliders, starts scripts + audio
scene.on_update_physics(dt);    // Steps simulation (with optional Lua on_fixed_update), writes back interpolated transforms
scene.on_runtime_stop();        // Drops physics world, scripts, audio — resets all runtime handles

scene.on_simulation_start();    // Physics only (no scripts)
scene.on_simulation_stop();
```

Physics scripting API (used by both native + Lua scripts):

| Method | Description |
|--------|-------------|
| `apply_impulse(entity, Vec2)` | Instant velocity change |
| `apply_impulse_at_point(entity, impulse, point)` | Impulse at world-space point (can cause torque) |
| `apply_force(entity, Vec2)` | Continuous force (accumulated per physics step) |
| `get_linear_velocity(entity) -> Option<Vec2>` | Current linear velocity |
| `set_linear_velocity(entity, Vec2)` | Override linear velocity |
| `get_angular_velocity(entity) -> Option<f32>` | Current angular velocity (rad/s) |
| `set_angular_velocity(entity, f32)` | Override angular velocity |

### Script Lifecycle

```rust
scene.on_update_scripts(dt, &input);       // Runs all NativeScriptComponent scripts
scene.on_update_lua_scripts(dt, &input);   // Runs all LuaScriptComponent scripts (play mode)
scene.reload_lua_scripts();                // Hot-reload all Lua scripts from disk mid-play
```

## Entity

**File:** `scene/entity.rs`

Lightweight `Copy` newtype over `hecs::Entity`. No back-reference to Scene — all component operations go through Scene methods.

```rust
entity.id() -> u32      // hecs runtime ID (NOT the UUID)
entity.handle() -> hecs::Entity  // underlying hecs handle
```

## Built-in Components

**File:** `scene/components.rs`, `scene/animation.rs`

Every entity created via `Scene::create_entity` automatically receives: `IdComponent`, `TagComponent`, `TransformComponent`, and `RelationshipComponent`.

### IdComponent

```rust
struct IdComponent {
    pub id: Uuid,
}
```

64-bit UUID, spawned on every entity automatically. Used for persistent identification across serialization/deserialization and parent-child relationships.

### TagComponent

```rust
struct TagComponent {
    pub tag: String,
}
```

Human-readable entity name. Default: `"Entity"`.

### TransformComponent

```rust
struct TransformComponent {
    pub translation: Vec3,
    pub rotation: Vec3,    // Euler angles in radians, XYZ order
    pub scale: Vec3,
}
```

- `new(Vec3)` sets translation, scale defaults to `Vec3::ONE`
- `get_transform() -> Mat4` builds combined TRS matrix via `Mat4::from_scale_rotation_translation`
- Default: translation `ZERO`, rotation `ZERO`, scale `ONE`
- Implements `Clone`

### RelationshipComponent

```rust
struct RelationshipComponent {
    /// Parent entity UUID. `None` = root entity.
    pub parent: Option<u64>,
    /// Ordered list of child entity UUIDs.
    pub children: Vec<u64>,
}
```

Tracks parent-child hierarchy between entities. Auto-added on every entity creation with default values (no parent, no children). Parent and children are stored as UUIDs (from `IdComponent`) so relationships survive scene copy and serialization.

- `has_relationships() -> bool` — returns `true` if this entity has a parent or children
- Default: `parent: None`, `children: []`
- Implements `Clone`

Hierarchy operations (parenting, detaching, reordering) are performed via `Scene` methods — see [Hierarchy Operations](#hierarchy-operations).

### SpriteRendererComponent

```rust
struct SpriteRendererComponent {
    pub color: Vec4,
    pub texture_handle: Uuid,              // Asset handle (0 = none)
    pub texture: Option<Ref<Texture2D>>,   // Runtime GPU texture (not serialized)
    pub tiling_factor: f32,
    pub sorting_layer: i32,                // Render ordering: higher layers draw on top
    pub order_in_layer: i32,               // Ordering within a sorting layer
    pub atlas_min: Option<Vec2>,           // Optional atlas sub-region UV min
    pub atlas_max: Option<Vec2>,           // Optional atlas sub-region UV max
}
```

- `new(color)`, `from_rgb(r, g, b)`, `Default` (white, tiling_factor 1.0, sorting_layer 0, order_in_layer 0)
- Clone via `Arc` sharing for textures
- `texture_handle` links to the asset registry; resolved to `texture` at runtime via `Scene::resolve_texture_handles()`
- `sorting_layer` and `order_in_layer` control render order (sorted before draw calls)

### CircleRendererComponent

```rust
struct CircleRendererComponent {
    pub color: Vec4,
    pub thickness: f32,  // 1.0 = filled, lower = ring/outline
    pub fade: f32,       // default 0.005, higher = softer edges
}
```

SDF-based circle rendered on a quad. Size controlled by entity's `TransformComponent` scale. Fragments with alpha <= 0 are discarded for correct entity picking.

### CameraComponent

```rust
struct CameraComponent {
    pub camera: SceneCamera,
    pub primary: bool,
    pub fixed_aspect_ratio: bool,
}
```

Only the primary camera renders. `SceneCamera` is projection-only (see [Rendering — SceneCamera](rendering.md#scenecamera-ecs)). When `fixed_aspect_ratio` is `false` (default), the projection is recalculated on viewport resize.

### TextComponent

```rust
struct TextComponent {
    pub text: String,
    pub font_path: String,
    pub font: Option<Ref<Font>>,   // Runtime-only, not serialized
    pub font_size: f32,
    pub color: Vec4,
    pub line_spacing: f32,
    pub kerning: f32,
}
```

MSDF text rendered via the batch text pipeline. The `font_path` points to a `.ttf` file. Fonts loaded via `Scene::load_fonts(renderer)` and cached. Default: `font_size` 1.0, `color` white, `line_spacing` 1.0, `kerning` 0.0.

### AudioSourceComponent

```rust
struct AudioSourceComponent {
    /// Asset handle referencing an audio file (wav/ogg/mp3/flac). 0 = none.
    pub audio_handle: Uuid,
    /// Playback volume (0.0–1.0). Default: 1.0.
    pub volume: f32,
    /// Playback rate/pitch (1.0 = normal speed). Default: 1.0.
    pub pitch: f32,
    /// Whether the sound loops. Default: false.
    pub looping: bool,
    /// If true, sound plays automatically when entering play mode. Default: false.
    pub play_on_start: bool,
    // (runtime-only) resolved_path: Option<String> — not serialized
}
```

Asset-handle based audio source. The `audio_handle` references an audio asset in the registry. At runtime, `Scene::resolve_audio_handles()` resolves the handle to a file path. Sounds with `play_on_start` play automatically when entering play mode. Runtime playback controlled via `Scene::play_entity_sound()`, `stop_entity_sound()`, `set_entity_volume()`.

> **See also:** [Audio](10-audio.md) for AudioEngine architecture, kira integration, lifecycle, and Lua API.

### TilemapComponent

```rust
struct TilemapComponent {
    pub width: u32,                    // Number of columns in the grid
    pub height: u32,                   // Number of rows in the grid
    pub tile_size: Vec2,               // World-space size per tile
    pub texture_handle: Uuid,          // Asset handle for tileset texture (0 = none)
    pub texture: Option<Ref<Texture2D>>,  // Runtime-only, not serialized
    pub tileset_columns: u32,          // Number of columns in the tileset image
    pub cell_size: Vec2,               // Pixel size per cell in the tileset image
    pub spacing: Vec2,                 // Spacing between tiles in tileset (pixels)
    pub margin: Vec2,                  // Margin from tileset edge (pixels)
    pub tiles: Vec<i32>,              // Tile IDs, row-major. -1 = empty.
}
```

Grid-based tile map renderer for 2D levels. Each tile ID maps to a sub-region of the tileset texture. Tile values may include flip flags in the high bits:

| Constant | Value | Description |
|----------|-------|-------------|
| `TILE_FLIP_H` | `0x4000_0000` (bit 30) | Horizontal flip |
| `TILE_FLIP_V` | `0x2000_0000` (bit 29) | Vertical flip |
| `TILE_ID_MASK` | `0x1FFF_FFFF` (lower 29 bits) | Actual tile ID |

Methods:

| Method | Signature | Description |
|--------|-----------|-------------|
| `get_tile` | `(x: u32, y: u32) -> i32` | Get tile ID at grid position. Returns -1 if out of bounds. |
| `set_tile` | `(x: u32, y: u32, id: i32)` | Set tile ID at grid position. No-op if out of bounds. |
| `resize` | `(new_width: u32, new_height: u32)` | Resize the grid, preserving existing data. New cells filled with -1. |

Default: 10x10 grid, `tile_size` (1.0, 1.0), `cell_size` (32.0, 32.0), all tiles empty (-1).

The tilemap's world position comes from the entity's `TransformComponent`. At render time, each non-empty tile computes UV coordinates from the tileset image using `cell_size`, `spacing`, and `margin`, and submits a sub-textured quad draw call.

> **See also:** [Rendering — Tilemap Rendering](06-rendering.md#tilemap-rendering) for the rendering pipeline, [Editor — Tilemap Painting](03-editor.md#tilemap-painting) for the editor paint tools, [Scripting — Tilemap](07-scripting.md#tilemap) for the Lua API.

### SpriteAnimatorComponent

**File:** `scene/animation.rs`

```rust
struct SpriteAnimatorComponent {
    /// Pixel size of each cell in the sprite sheet.
    pub cell_size: Vec2,
    /// Number of columns in the sprite sheet grid.
    pub columns: u32,
    /// Animation clips defined for this sprite sheet.
    pub clips: Vec<AnimationClip>,
    /// Default clip index to play when a non-looping clip finishes.
    pub default_clip: Option<usize>,
    /// Playback speed multiplier (1.0 = normal, 0.5 = half, 2.0 = double).
    pub speed_scale: f32,

    // Runtime state (managed internally):
    // current_clip_index: Option<usize>
    // frame_timer: f32
    // current_frame: u32
    // playing: bool
}
```

Sprite sheet animation component. Requires a `SpriteRendererComponent` on the same entity with a loaded texture (the sprite sheet). The animator divides the texture into a grid of `columns` x N rows, each cell being `cell_size` pixels. Frame indices are 0-based and row-major.

At runtime, `Scene::on_update_animations(dt)` advances the frame timer. During rendering, the current frame's UV region is used instead of the full texture.

Public methods:

| Method | Signature | Description |
|--------|-----------|-------------|
| `play` | `(name: &str) -> bool` | Play a clip by name. Returns `false` if not found. Only resets frame if switching clips. |
| `stop` | `()` | Stop playback. |
| `is_playing` | `() -> bool` | Whether the animator is currently playing. |
| `current_grid_coords` | `() -> Option<(u32, u32)>` | Current frame's (column, row) in the sprite sheet grid. |

Default: `cell_size` (32.0, 32.0), `columns` 1, no clips.

### AnimationClip

**File:** `scene/animation.rs`

```rust
struct AnimationClip {
    /// Human-readable name (e.g. "idle", "walk", "run").
    pub name: String,
    /// First frame index (0-based, row-major in grid).
    pub start_frame: u32,
    /// Last frame index (inclusive).
    pub end_frame: u32,
    /// Playback speed in frames per second.
    pub fps: f32,
    /// Whether the clip loops when it reaches the end.
    pub looping: bool,
}
```

Named animation range within a sprite sheet. Frame indices map to grid cells: frame N is at column `N % columns`, row `N / columns`. Looping clips wrap from `end_frame` back to `start_frame`. Non-looping clips stop at `end_frame`.

Default: `fps` 12.0, `looping` true.

> **See also:** [Rendering — Animation Rendering](06-rendering.md#animation-rendering) for how animated sprites are rendered, [Scripting — Animation](07-scripting.md#animation) for the Lua API.

### RigidBody2DComponent

```rust
struct RigidBody2DComponent {
    pub body_type: RigidBody2DType,  // Static, Dynamic, Kinematic
    pub fixed_rotation: bool,
    pub linear_damping: f32,         // Velocity damping (default: 0.0)
    pub angular_damping: f32,        // Rotation damping (default: 0.0)
    pub gravity_scale: f32,          // Gravity multiplier (default: 1.0)
    // (runtime-only) runtime_body: Option<RigidBodyHandle> — not serialized
}
```

2D rigid body for physics simulation. Requires a `TransformComponent`. At runtime start, the scene creates a rapier rigid body from this component's settings.

`RigidBody2DType` enum: `Static` (fixed in place), `Dynamic` (fully simulated), `Kinematic` (position-based movement).

Manual `Clone` resets `runtime_body` to `None`.

### BoxCollider2DComponent

```rust
struct BoxCollider2DComponent {
    pub offset: Vec2,
    pub size: Vec2,           // Half-extents (default 0.5 x 0.5)
    pub density: f32,         // Default: 1.0
    pub friction: f32,        // Default: 0.5
    pub restitution: f32,     // Default: 0.0
    pub collision_layer: u32, // Collision group membership (bitmask, default: 0x0001)
    pub collision_mask: u32,  // Which groups to collide with (bitmask, default: 0xFFFF)
    // (runtime-only) runtime_fixture: Option<ColliderHandle> — not serialized
}
```

2D box collider. Requires a `RigidBody2DComponent` on the same entity. Half-extents are scaled by the entity's transform scale. `collision_layer` and `collision_mask` map to rapier `InteractionGroups` for filtering.

Manual `Clone` resets `runtime_fixture` to `None`.

### CircleCollider2DComponent

```rust
struct CircleCollider2DComponent {
    pub offset: Vec2,
    pub radius: f32,          // Default: 0.5
    pub density: f32,         // Default: 1.0
    pub friction: f32,        // Default: 0.5
    pub restitution: f32,     // Default: 0.0
    pub collision_layer: u32, // Collision group membership (bitmask, default: 0x0001)
    pub collision_mask: u32,  // Which groups to collide with (bitmask, default: 0xFFFF)
    // (runtime-only) runtime_fixture: Option<ColliderHandle> — not serialized
}
```

2D circle collider. Requires a `RigidBody2DComponent` on the same entity. Radius scaled by `max(scale.x, scale.y)`.

Manual `Clone` resets `runtime_fixture` to `None`.

### AudioListenerComponent

Empty marker component designating which entity acts as the spatial audio listener. The primary camera entity typically has this component.

### ParticleEmitterComponent

```rust
struct ParticleEmitterComponent {
    pub emission_rate: f32,
    pub lifetime_range: (f32, f32),
    pub particle_props: ParticleProps,   // Template for emitted particles
}
```

CPU particle emitter attached to an entity. Spawns particles at the entity's position using the configured `ParticleProps` template.

### InstancedSpriteAnimator

```rust
struct InstancedSpriteAnimator {
    pub clips: Vec<AnimationClip>,
    pub cell_size: Vec2,
    pub columns: u32,
    pub default_clip: Option<usize>,
    // Runtime state: current_clip, start_time, playing
}
```

GPU-driven stateless animation. Frame computation happens entirely in the vertex shader: `frame = start_frame + floor((global_time - start_time) * fps * speed_scale) % frame_count`. Zero per-frame CPU cost while playing. Non-looping clips transition to `default_clip` on completion.

### AnimationControllerComponent

```rust
struct AnimationControllerComponent {
    pub clips: Vec<String>,                           // Clip names
    pub parameters: HashMap<String, AnimParamValue>,  // Bool or Float parameters
    pub transitions: Vec<AnimationTransition>,         // State machine transitions
}
```

Data-driven animation state machine. Evaluates transitions each frame after animation updates. Conditions: `OnFinished` (current clip ended), `ParamBool(name, value)`, `ParamFloat(name, ordering, threshold)`. First matching transition wins, calling `play()` on the entity's `SpriteAnimatorComponent`.

### NativeScriptComponent

```rust
struct NativeScriptComponent {
    // (internal) instance: Option<Box<dyn NativeScript>>
    // (internal) instantiate_fn: fn() -> Box<dyn NativeScript>
    // (internal) created: bool
}
```

Attaches a `NativeScript` to an entity. Created via `NativeScriptComponent::bind::<T>()` where `T: NativeScript + Default`. Lazy instantiation on first `on_update_scripts` call. **Not serialized** (runtime-only, code-defined). Not `Clone` — manually copied in `Scene::copy()` and `duplicate_entity()`.

### LuaScriptComponent

```rust
#[cfg(feature = "lua-scripting")]
struct LuaScriptComponent {
    pub script_path: String,
    pub field_overrides: HashMap<String, ScriptFieldValue>,
    // (runtime-only) loaded: bool — reset on clone, not serialized
}
```

Lua script attached to an entity. Feature-gated behind `lua-scripting`. The `script_path` points to a `.lua` file relative to the project root. `field_overrides` stores editor-set values applied before `on_create()`. The `loaded` flag is reset on clone (same pattern as physics handles).

### UIAnchorComponent

```rust
struct UIAnchorComponent {
    /// Normalized anchor point on screen. (0,0) = top-left, (1,1) = bottom-right.
    pub anchor: Vec2,
    /// Offset from the anchor point in world units.
    pub offset: Vec2,
}
```

Screen-relative positioning component for UI elements (HUD, menus, health bars). The anchor point maps to a position on the primary camera's viewport, and the offset displaces from that point in world units. `Scene::apply_ui_anchors()` repositions entities each frame based on the current camera.

Default: `anchor` (0.5, 0.5) = center, `offset` (0, 0).

Editor preset buttons: TL (0,0), TC (0.5,0), TR (1,0), CL (0,1), C (0.5,0.5), CR (1,0.5), BL (0,1), BC (0.5,1), BR (1,1).

GUI scale (`Scene::gui_scale()`) multiplies offsets for resolution-independent sizing.

## Native Scripting

**File:** `scene/native_script.rs`

### NativeScript Trait

```rust
trait NativeScript: Send + Sync + 'static {
    fn on_create(&mut self, entity: Entity, scene: &mut Scene) {}
    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {}
    fn on_destroy(&mut self, entity: Entity, scene: &mut Scene) {}
}
```

All methods have default empty implementations. The trait requires `Send + Sync + 'static`.

### NativeScriptComponent

```rust
// Bind a script to an entity
scene.add_component(entity, NativeScriptComponent::bind::<MyScript>());
```

- Lazy instantiation: script instance created on first `on_update_scripts` call
- Uses take-modify-replace pattern: `Option::take()` script out of component, call methods with `&mut Scene`, put back
- **Not serialized** (runtime-only, code-defined)

### Usage

```rust
struct CameraController;

impl NativeScript for CameraController {
    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {
        if let Some(mut transform) = scene.get_component_mut::<TransformComponent>(entity) {
            let speed = 5.0 * dt.seconds();
            if input.is_key_pressed(KeyCode::KeyW) {
                transform.translation.y += speed;
            }
        }
    }
}

// In Application::on_update, before rendering:
scene.on_update_scripts(dt, &input);
```

## Scene Copy & Duplication

`Scene::copy(source)` creates a deep clone of the entire scene:

- All entities recreated with their original UUIDs (preserves hierarchy, script references, etc.)
- All cloneable components copied via `for_each_cloneable_component!` macro
- `NativeScriptComponent` manually copied (not `Clone` — only `instantiate_fn` is carried over)
- `LuaScriptComponent` cloned (resets `loaded` flag)
- Runtime-only handles (physics bodies, colliders, Lua loaded flags) reset to `None`/`false`
- Used by the editor for play/stop snapshot-restore cycle

`Scene::duplicate_entity(entity)` creates a copy within the same scene:

- Fresh UUID assigned
- All components cloned
- Relationship reset to root (no parent, no children)

## Scene Serialization

YAML-based scene persistence (`.ggscene` files) via external serializer pattern. Scene types have no serde derives — `SceneSerializer` handles conversion through intermediate data structs.

**Not serialized:** `NativeScriptComponent` (runtime-only, code-defined), `Texture2D` / `Font` GPU resources, physics runtime handles, Lua `loaded` flags, `AudioSourceComponent::resolved_path`, `TilemapComponent::texture`.

See [Scene Serialization](08-serialization.md) for full details: YAML format, intermediate structs, deserialization flow, `Scene::copy()`, UUID system, and editor file operations.

## hecs Tips & Patterns

```rust
// hecs is re-exported at crate root
use gg_engine::hecs;

// Query API: yields Q::Item directly (NOT (Entity, Q::Item))
for transform in world.query::<&TransformComponent>().iter() { ... }

// To get entity IDs: use hecs::Entity as a query component
for (entity, transform) in world.query::<(hecs::Entity, &TransformComponent)>().iter() { ... }

// hecs::Ref implements Clone returning Ref (not inner T)
// Access fields via Deref, don't call .clone() on Ref directly

// Multi-component queries
for (entity, (transform, sprite)) in world.query::<(hecs::Entity, &TransformComponent, &SpriteRendererComponent)>().iter() { ... }
```
