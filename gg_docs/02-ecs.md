# ECS & Scene

The Entity Component System lives in `gg_engine/src/scene/` and is built on top of [hecs](https://crates.io/crates/hecs) 0.11 (archetypal ECS storage).

## Scene

**File:** `scene/mod.rs`

`Scene` wraps `hecs::World` and owns all entity/component data.

### Entity Management

```rust
let entity = scene.create_entity();                              // Default: IdComponent + Tag("Entity") + Transform(IDENTITY)
let entity = scene.create_entity_with_tag("Player");             // Custom name
let entity = scene.create_entity_with_uuid(uuid, "Player");      // Known UUID (deserialization)
scene.destroy_entity(entity);
scene.is_alive(entity);
scene.entity_count();
scene.find_entity_by_id(u32) -> Option<Entity>;                  // From hecs entity ID
scene.duplicate_entity(entity);                                   // New UUID, copies all components
```

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

### Utility Methods

| Method | Description |
|--------|-------------|
| `each_entity_with_tag()` | Returns `Vec<(Entity, String)>` sorted by ID |
| `set_primary_camera(entity)` | Clears primary on all others |
| `get_primary_camera_entity()` | Returns `Option<Entity>` |
| `copy(source)` | Deep-copies an entire scene (preserves UUIDs, resets physics) |
| `on_viewport_resize(w, h)` | Updates all non-fixed-aspect-ratio camera projections |

### Rendering

Two render paths:

```rust
// Editor mode — external VP from EditorCamera
scene.on_update_editor(&editor_camera.view_projection(), &mut renderer);

// Runtime mode — finds primary CameraComponent, computes VP
scene.on_update_runtime(&mut renderer);
```

Both iterate `SpriteRendererComponent`, `CircleRendererComponent`, and `TextComponent` entities and submit draw calls.

### Physics Lifecycle

```rust
scene.on_runtime_start();       // Creates rapier2d world, spawns bodies/colliders
scene.on_update_physics(dt);    // Steps simulation, writes back transforms
scene.on_runtime_stop();        // Drops physics world, resets runtime handles
```

### Script Lifecycle

```rust
scene.on_update_scripts(dt, &input);  // Runs all NativeScriptComponent scripts
```

## Entity

**File:** `scene/entity.rs`

Lightweight `Copy` newtype over `hecs::Entity`. No back-reference to Scene — all component operations go through Scene methods.

```rust
entity.id() -> u32  // hecs runtime ID (NOT the UUID)
```

## Built-in Components

**File:** `scene/components.rs`

### IdComponent

```rust
struct IdComponent(pub Uuid);
```

64-bit UUID, spawned on every entity automatically. Used for persistent identification across serialization/deserialization.

### TagComponent

```rust
struct TagComponent {
    pub tag: String,
}
```

Human-readable entity name.

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
- Implements `Clone`

### SpriteRendererComponent

```rust
struct SpriteRendererComponent {
    pub color: Vec4,
    pub texture_handle: Uuid,              // Asset handle (0 = none)
    pub texture: Option<Ref<Texture2D>>,   // Runtime GPU texture
    pub tiling_factor: f32,
}
```

- `new(color)`, `from_rgb(r, g, b)`, `Default` (white)
- Clone via `Arc` sharing for textures
- `texture_handle` links to the asset registry; resolved to `texture` at runtime via `Scene::resolve_texture_handles()`

### CircleRendererComponent

```rust
struct CircleRendererComponent {
    pub color: Vec4,
    pub thickness: f32,  // 1.0 = filled
    pub fade: f32,       // default 0.005
}
```

SDF-based circle rendered on a quad. Fragments with alpha <= 0 are discarded for correct entity picking.

### CameraComponent

```rust
struct CameraComponent {
    pub camera: SceneCamera,
    pub primary: bool,
    pub fixed_aspect_ratio: bool,
}
```

Only the primary camera renders. `SceneCamera` is projection-only (see [Rendering — SceneCamera](rendering.md#scenecamera-ecs)).

### RigidBody2DComponent

```rust
struct RigidBody2DComponent {
    pub body_type: RigidBody2DType,  // Static, Dynamic, Kinematic
    pub fixed_rotation: bool,
    pub runtime_body: Option<RigidBodyHandle>,
}
```

Manual `Clone` resets `runtime_body` to `None`.

### BoxCollider2DComponent

```rust
struct BoxCollider2DComponent {
    pub offset: Vec2,
    pub size: Vec2,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub restitution_threshold: f32,
    pub runtime_fixture: Option<ColliderHandle>,
}
```

Manual `Clone` resets `runtime_fixture` to `None`.

### CircleCollider2DComponent

```rust
struct CircleCollider2DComponent {
    pub offset: Vec2,
    pub radius: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub restitution_threshold: f32,
    pub runtime_fixture: Option<ColliderHandle>,
}
```

Manual `Clone` resets `runtime_fixture` to `None`.

### TextComponent

```rust
struct TextComponent {
    pub text: String,
    pub font_path: String,
    pub font_size: f32,
    pub color: Vec4,
    pub line_spacing: f32,
    pub kerning: f32,
}
```

MSDF text rendered via the batch text pipeline. Fonts loaded via `Scene::load_fonts(renderer)` and cached on the Scene. Serialized to `.ggscene`.

## Native Scripting

**File:** `scene/native_script.rs`

### NativeScript Trait

```rust
trait NativeScript {
    fn on_create(&mut self, entity: Entity, scene: &mut Scene) {}
    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {}
    fn on_destroy(&mut self, entity: Entity, scene: &mut Scene) {}
}
```

All methods have default empty implementations.

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

## Scene Serialization

YAML-based scene persistence (`.ggscene` files) via external serializer pattern. Scene types have no serde derives — `SceneSerializer` handles conversion through intermediate data structs.

**Not serialized:** `NativeScriptComponent` (runtime-only, code-defined), `Texture2D` GPU resources, physics runtime handles, Lua `loaded` flags.

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
