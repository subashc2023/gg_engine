# 2D Physics

The engine integrates [rapier2d](https://rapier.rs/) 0.22 for 2D rigid body physics.

**File:** `gg_engine/src/scene/physics_2d.rs`

## PhysicsWorld2D

Wraps rapier2d pipeline components:
- Gravity (default: `(0.0, -9.81)`)
- Integration parameters
- Physics pipeline
- Island manager
- Broad phase / Narrow phase
- Impulse joint set / Multibody joint set
- Rigid body set / Collider set
- CCD solver
- Query pipeline

## Components

### RigidBody2DComponent

```rust
struct RigidBody2DComponent {
    pub body_type: RigidBody2DType,
    pub fixed_rotation: bool,
    pub runtime_body: Option<RigidBodyHandle>,
}
```

`RigidBody2DType` enum:

| Variant | Value | Description |
|---------|-------|-------------|
| `Static` | 0 | Immovable |
| `Dynamic` | 1 | Fully simulated |
| `Kinematic` | 2 | Programmatically moved |

`to_rapier()` converts to `rapier2d::dynamics::RigidBodyType`.

### BoxCollider2DComponent

```rust
struct BoxCollider2DComponent {
    pub offset: Vec2,
    pub size: Vec2,          // default (0.5, 0.5)
    pub density: f32,        // default 1.0
    pub friction: f32,       // default 0.5
    pub restitution: f32,    // default 0.0
    pub restitution_threshold: f32,  // default 0.5
    pub runtime_fixture: Option<ColliderHandle>,
}
```

## Scene Physics Lifecycle

The physics lifecycle is driven by the editor's play/stop state:

```
┌──────────────────────────────────────────────────────┐
│ Edit Mode                                            │
│  (no physics simulation)                             │
└───────────────────┬──────────────────────────────────┘
                    │ Play button
                    ▼
┌──────────────────────────────────────────────────────┐
│ on_runtime_start()                                   │
│  1. Create PhysicsWorld2D                            │
│  2. Iterate RigidBody2DComponent entities            │
│     → Spawn rapier rigid bodies                      │
│     → Store RigidBodyHandle in runtime_body          │
│  3. Iterate BoxCollider2DComponent entities           │
│     → Attach colliders to bodies                     │
│     → Store ColliderHandle in runtime_fixture        │
└───────────────────┬──────────────────────────────────┘
                    │
                    ▼
┌──────────────────────────────────────────────────────┐
│ on_update_physics(dt)  [called each frame]           │
│  1. Step rapier pipeline by dt                       │
│  2. Write back body positions → TransformComponent   │
│     (translation.x, translation.y, rotation.z)       │
└───────────────────┬──────────────────────────────────┘
                    │ Stop button
                    ▼
┌──────────────────────────────────────────────────────┐
│ on_runtime_stop()                                    │
│  1. Drop PhysicsWorld2D                              │
│  2. Reset all runtime_body handles to None           │
│  3. Reset all runtime_fixture handles to None        │
└──────────────────────────────────────────────────────┘
```

## Runtime Handle Management

Both `RigidBody2DComponent` and `BoxCollider2DComponent` have `runtime_*` fields that hold rapier handles:

- **Set** during `on_runtime_start()` — stores the rapier handle for the corresponding body/collider
- **Used** during `on_update_physics(dt)` — to read back simulated positions
- **Reset** to `None` during `on_runtime_stop()` or when cloned (manual `Clone` impl)

This separation ensures physics state is purely runtime and doesn't leak into serialized scene data or scene copies.

## Usage Example

```rust
// Create a dynamic entity with physics
let entity = scene.create_entity_with_tag("Ball");
scene.add_component(entity, SpriteRendererComponent::from_rgb(1.0, 0.0, 0.0));

scene.add_component(entity, RigidBody2DComponent {
    body_type: RigidBody2DType::Dynamic,
    fixed_rotation: false,
    runtime_body: None,
});

scene.add_component(entity, BoxCollider2DComponent {
    size: Vec2::new(0.5, 0.5),
    density: 1.0,
    friction: 0.3,
    restitution: 0.7,
    ..Default::default()
});

// Create a static floor
let floor = scene.create_entity_with_tag("Floor");
{
    let mut transform = scene.get_component_mut::<TransformComponent>(floor).unwrap();
    transform.translation.y = -3.0;
    transform.scale = Vec3::new(10.0, 0.5, 1.0);
}
scene.add_component(floor, RigidBody2DComponent {
    body_type: RigidBody2DType::Static,
    ..Default::default()
});
scene.add_component(floor, BoxCollider2DComponent::default());
```

## Serialization

Both physics components are serialized to `.ggscene` files:
- `RigidBody2DComponent`: body_type, fixed_rotation
- `BoxCollider2DComponent`: offset, size, density, friction, restitution, restitution_threshold

Runtime handles (`runtime_body`, `runtime_fixture`) are **never** serialized.
