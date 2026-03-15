# Physics (2D & 3D)

The engine integrates [rapier2d](https://rapier.rs/) 0.22 for 2D and [rapier3d](https://rapier.rs/) 0.22 for 3D rigid body physics.

**Files:**
- `gg_engine/src/scene/physics_2d.rs` — `PhysicsWorld2D`, collision collector, fixed timestep
- `gg_engine/src/scene/physics_3d.rs` — `PhysicsWorld3D`, 3D collision collector
- `gg_engine/src/scene/physics_3d_ops.rs` — 3D physics operations (impulse, force, velocity, raycast)
- `gg_engine/src/scene/physics_common.rs` — Shared `PhysicsTimestep` accumulator
- `gg_engine/src/scene/mod.rs` — Scene physics lifecycle, validation, interpolation writeback
- `gg_engine/src/scene/script_engine.rs` — Lua collision callbacks
- `gg_engine/src/scene/components.rs` — Physics component definitions (2D and 3D)

## PhysicsWorld2D

Wraps rapier2d pipeline components:
- Gravity (default: `(0.0, -9.81)`)
- Integration parameters (dt set to `FIXED_TIMESTEP`)
- Physics pipeline
- Island manager
- Broad phase / Narrow phase
- Impulse joint set / Multibody joint set
- Rigid body set / Collider set
- CCD solver
- Query pipeline
- Fixed timestep accumulator
- Previous transforms for interpolation
- Collider-to-UUID mapping for collision event dispatch
- Collision event collector

## Components

### RigidBody2DComponent

```rust
struct RigidBody2DComponent {
    pub body_type: RigidBody2DType,
    pub fixed_rotation: bool,
    pub linear_damping: f32,       // default 0.0
    pub angular_damping: f32,      // default 0.0
    pub gravity_scale: f32,        // default 1.0
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
    pub runtime_fixture: Option<ColliderHandle>,
}
```

Half-extents are `size * entity_scale`. The collider is created as a cuboid via `ColliderBuilder::cuboid(half_x, half_y)`.

### CircleCollider2DComponent

```rust
struct CircleCollider2DComponent {
    pub offset: Vec2,
    pub radius: f32,         // default 0.5
    pub density: f32,        // default 1.0
    pub friction: f32,       // default 0.5
    pub restitution: f32,    // default 0.0
    pub runtime_fixture: Option<ColliderHandle>,
}
```

The radius is scaled by `max(scale.x.abs(), scale.y.abs())` at runtime. The collider is created as a ball via `ColliderBuilder::ball(scaled_radius)`.

Manual `Clone` impl resets `runtime_fixture` to `None` (same pattern as `BoxCollider2DComponent`).

## Physics Property Validation

Before creating colliders, physics properties are validated via `validate_physics_value()`:

- **density**: clamped to minimum `0.0`
- **friction**: clamped to minimum `0.0`
- **restitution**: clamped to minimum `0.0`

When a value is below the minimum, a warning is logged:
```
Entity {uuid}: negative {property} ({value:.3}), clamped to {min}
```

Zero-size box colliders (`half_x <= 0` or `half_y <= 0`) and zero-radius circle colliders are skipped entirely with a warning.

## Fixed Timestep System

Physics runs at a fixed rate decoupled from the render frame rate.

**Constants:**
- `FIXED_TIMESTEP` = `1.0 / 60.0` (16.67ms per physics step)
- `MAX_FRAME_DT` = `0.25` (250ms cap to prevent spiral of death after long hitches)

**Accumulator model:**

| Method | Description |
|--------|-------------|
| `accumulate(dt)` | Adds `dt.min(MAX_FRAME_DT)` to the accumulator |
| `can_step()` | Returns `true` if `accumulator >= FIXED_TIMESTEP` |
| `step_once()` | Executes one rapier pipeline step, drains `FIXED_TIMESTEP` from accumulator |
| `fixed_timestep()` | Returns the `FIXED_TIMESTEP` constant |
| `alpha()` | Returns `accumulator / FIXED_TIMESTEP` for interpolation (0.0..1.0) |

Multiple physics steps can execute per frame when the frame time exceeds one timestep. The `MAX_FRAME_DT` cap prevents runaway stepping after long pauses.

## Transform Interpolation

To decouple visual smoothness from the fixed physics rate, body transforms are interpolated between the previous and current physics state.

**Storage:** `prev_transforms: HashMap<RigidBodyHandle, (f32, f32, f32)>` — stores `(position_x, position_y, angle)` before each step.

| Method | Description |
|--------|-------------|
| `snapshot_transforms()` | Captures all body positions/angles into `prev_transforms`. Called before `step_once()` |
| `prev_transform(handle)` | Returns the pre-step `(x, y, angle)` for a given body handle |
| `alpha()` | Interpolation fraction: how far through the next timestep we are |

**Writeback after the step loop** (in `Scene::on_update_physics`):

```
interpolated_x = prev_x + (cur_x - prev_x) * alpha
interpolated_y = prev_y + (cur_y - prev_y) * alpha
interpolated_angle = prev_angle + shortest_path_diff * alpha
```

Angle interpolation uses shortest-path wrapping to avoid flipping through the wrong direction on `TAU` boundaries. On the first frame (no previous snapshot), current values are used directly.

## Collision Events

### CollisionCollector

Rapier dispatches collision events through its `EventHandler` trait. `CollisionCollector` implements this trait, collecting events into a `Mutex<Vec<(ColliderHandle, ColliderHandle, bool)>>`.

```rust
struct CollisionCollector {
    events: Mutex<Vec<(ColliderHandle, ColliderHandle, bool)>>,
}
```

The `Mutex` is required because rapier's `EventHandler` trait requires `Sync`. Events are accumulated during `step_once()` and drained afterward.

**Event types:**
- `CollisionEvent::Started(h1, h2, _)` — two colliders began touching (`started = true`)
- `CollisionEvent::Stopped(h1, h2, _)` — two colliders separated (`started = false`)

The `handle_contact_force_event` callback is a no-op.

### Collider-to-UUID Mapping

Each collider is mapped to its owning entity's UUID via:

```rust
collider_to_uuid: HashMap<ColliderHandle, u64>
```

Registration happens in `on_physics_2d_start()` immediately after inserting each collider:
```rust
physics.register_collider(collider_handle, entity_uuid);
```

### Draining Events

`PhysicsWorld2D::drain_collision_events()` resolves raw collider handles to entity UUIDs:

1. Locks the collector's `Mutex<Vec<...>>`
2. Drains all `(ColliderHandle, ColliderHandle, bool)` tuples
3. Looks up each handle in `collider_to_uuid`
4. Returns `Vec<(uuid_a, uuid_b, started)>` — only pairs where both handles resolve

### Lua Collision Callbacks

In play mode, after each physics step, `Scene::dispatch_collision_events()` drains the events and calls Lua callbacks on both entities in each pair:

- `on_collision_enter(other_uuid)` — called when `started = true`
- `on_collision_exit(other_uuid)` — called when `started = false`

Both entities are notified (entity A gets `other_uuid = B`, entity B gets `other_uuid = A`). Collision callbacks only fire in play mode (not simulate mode, which runs physics without Lua).

## Scene Physics Lifecycle

The physics lifecycle is driven by the editor's play/stop state:

```
+--------------------------------------------------------------+
| Edit Mode                                                    |
|  (no physics simulation)                                     |
+----------------------------+---------------------------------+
                             | Play / Simulate button
                             v
+--------------------------------------------------------------+
| on_runtime_start() / on_simulation_start()                   |
|  1. Create PhysicsWorld2D(0.0, -9.81)                        |
|  2. Iterate RigidBody2DComponent entities                    |
|     -> Spawn rapier rigid bodies                             |
|     -> Store RigidBodyHandle in runtime_body                 |
|  3. Iterate BoxCollider2DComponent entities                   |
|     -> Validate density/friction/restitution (clamp >= 0)    |
|     -> Attach colliders to bodies                            |
|     -> Store ColliderHandle in runtime_fixture               |
|     -> Register collider -> entity UUID mapping              |
|  4. Iterate CircleCollider2DComponent entities                |
|     -> Scale radius by max(|scale.x|, |scale.y|)            |
|     -> Validate density/friction/restitution (clamp >= 0)    |
|     -> Attach colliders to bodies                            |
|     -> Store ColliderHandle in runtime_fixture               |
|     -> Register collider -> entity UUID mapping              |
|  5. [Play only] Initialize Lua scripting + audio             |
+----------------------------+---------------------------------+
                             |
                             v
+--------------------------------------------------------------+
| on_update_physics(dt, input)  [called each frame]            |
|                                                              |
|  1. accumulate(dt)  (capped at MAX_FRAME_DT = 250ms)        |
|                                                              |
|  2. Fixed-step loop (while accumulator >= FIXED_TIMESTEP):   |
|     a. [Play mode] call_lua_fixed_update(FIXED_DT)           |
|        -> Lua on_fixed_update(dt) on all script entities     |
|        -> Scripts apply impulses/forces at physics rate       |
|     b. snapshot_transforms()                                 |
|        -> Store pre-step (x, y, angle) for each body         |
|     c. step_once()                                           |
|        -> Rapier pipeline step (one FIXED_TIMESTEP)          |
|        -> CollisionCollector gathers collision events         |
|     d. [Play mode] dispatch_collision_events()               |
|        -> Drain events, resolve to UUIDs                     |
|        -> Call on_collision_enter / on_collision_exit in Lua  |
|        -> flush_pending_destroys()                           |
|                                                              |
|  3. Interpolated writeback:                                  |
|     alpha = accumulator / FIXED_TIMESTEP                     |
|     For each body:                                           |
|       pos = lerp(prev_pos, cur_pos, alpha)                   |
|       angle = shortest_path_lerp(prev_angle, cur_angle, a)   |
|     -> Write to TransformComponent (x, y, rotation.z)        |
+----------------------------+---------------------------------+
                             | Stop button
                             v
+--------------------------------------------------------------+
| on_runtime_stop() / on_simulation_stop()                     |
|  1. Drop PhysicsWorld2D                                      |
|  2. Reset all runtime_body handles to None                   |
|  3. Reset all runtime_fixture handles to None                |
|     (both BoxCollider2D and CircleCollider2D)                |
|  4. [Play only] Tear down Lua scripting + audio              |
+--------------------------------------------------------------+
```

### Simulate vs Play Mode

- **Play mode**: Physics + Lua scripts + audio. `on_fixed_update(dt)` and collision callbacks run.
- **Simulate mode**: Physics only. No Lua scripts, no collision dispatch, no audio.

Both modes share `on_physics_2d_start()` / `on_physics_2d_stop()` for the physics world setup/teardown.

## Runtime Handle Management

`RigidBody2DComponent`, `BoxCollider2DComponent`, and `CircleCollider2DComponent` all have `runtime_*` fields that hold rapier handles:

- **Set** during `on_physics_2d_start()` — stores the rapier handle for the corresponding body/collider
- **Used** during `on_update_physics(dt)` — to read back simulated positions and resolve collision events
- **Reset** to `None` during `on_physics_2d_stop()` or when cloned (manual `Clone` impl)

This separation ensures physics state is purely runtime and doesn't leak into serialized scene data or scene copies.

## Collision Layers and Masks

Both `BoxCollider2DComponent` and `CircleCollider2DComponent` support collision filtering via bitmasks:

| Field | Default | Description |
|-------|---------|-------------|
| `collision_layer` | `0x0001` | Which group(s) this collider belongs to |
| `collision_mask` | `0xFFFF` | Which group(s) this collider can collide with |

These are mapped to rapier `InteractionGroups(layer, mask)`. Two colliders collide only when `(a.layer & b.mask) != 0 && (b.layer & a.mask) != 0`.

When `friction == 0.0` on either collider, the Min combine rule is used (zero wins against any surface).

## Active Events

Both `BoxCollider2DComponent` and `CircleCollider2DComponent` colliders are built with `ActiveEvents::COLLISION_EVENTS` enabled. This tells rapier to generate `CollisionEvent::Started` / `CollisionEvent::Stopped` events for the collision collector.

## Usage Example

```rust
// Create a dynamic entity with a box collider
let entity = scene.create_entity_with_tag("Box");
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

// Create a dynamic entity with a circle collider
let ball = scene.create_entity_with_tag("Ball");
scene.add_component(ball, RigidBody2DComponent {
    body_type: RigidBody2DType::Dynamic,
    ..Default::default()
});

scene.add_component(ball, CircleCollider2DComponent {
    radius: 0.5,
    density: 1.0,
    friction: 0.3,
    restitution: 0.9,
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

### Lua Collision Script Example

```lua
function on_collision_enter(other_uuid)
    -- Called when this entity starts touching another
    local other_name = Engine.get_tag(other_uuid)
    print("Collided with: " .. other_name)
end

function on_collision_exit(other_uuid)
    -- Called when this entity stops touching another
    print("Separated from entity " .. other_uuid)
end

function on_fixed_update(dt)
    -- Called once per physics step (1/60s), not per render frame
    -- Apply forces/impulses here for deterministic physics
    if Engine.is_key_down("W") then
        Engine.apply_force(0.0, 10.0)
    end
end
```

## Serialization

All physics components are serialized to `.ggscene` files:
- `RigidBody2DComponent`: body_type, fixed_rotation, linear_damping, angular_damping, gravity_scale
- `BoxCollider2DComponent`: offset, size, density, friction, restitution, collision_layer, collision_mask, is_sensor
- `CircleCollider2DComponent`: offset, radius, density, friction, restitution, collision_layer, collision_mask, is_sensor
- `RigidBody3DComponent`: body_type, lock_rotation_x/y/z, linear_damping, angular_damping, gravity_scale
- `BoxCollider3DComponent`: offset, size, density, friction, restitution, collision_layer, collision_mask, is_sensor
- `SphereCollider3DComponent`: offset, radius, density, friction, restitution, collision_layer, collision_mask, is_sensor
- `CapsuleCollider3DComponent`: offset, half_height, radius, density, friction, restitution, collision_layer, collision_mask, is_sensor

Runtime handles (`runtime_body`, `runtime_fixture`) are **never** serialized.

---

## 3D Physics (PhysicsWorld3D)

The 3D physics system mirrors the 2D architecture, using rapier3d 0.22.

**File:** `gg_engine/src/scene/physics_3d.rs`

`PhysicsWorld3D` wraps rapier3d pipeline components with the same fixed-timestep accumulator (shared `PhysicsTimestep` type from `physics_common.rs`). Default gravity: `(0, -9.81, 0)`.

### 3D Components

#### RigidBody3DComponent

```rust
struct RigidBody3DComponent {
    pub body_type: RigidBody3DType,    // Static, Dynamic, Kinematic
    pub lock_rotation_x: bool,         // per-axis rotation locks (unlike 2D's single fixed_rotation)
    pub lock_rotation_y: bool,
    pub lock_rotation_z: bool,
    pub gravity_scale: f32,            // default 1.0
    pub linear_damping: f32,           // default 0.0
    pub angular_damping: f32,          // default 0.0
    pub runtime_body: Option<RigidBodyHandle>,
}
```

#### BoxCollider3DComponent

```rust
struct BoxCollider3DComponent {
    pub offset: Vec3,
    pub size: Vec3,                    // half-extents, default (0.5, 0.5, 0.5)
    pub density: f32,                  // default 1.0
    pub friction: f32,                 // default 0.5
    pub restitution: f32,              // default 0.0
    pub collision_layer: u32,          // default 0x0001
    pub collision_mask: u32,           // default 0xFFFF
    pub is_sensor: bool,
    pub runtime_fixture: Option<ColliderHandle>,
}
```

#### SphereCollider3DComponent

```rust
struct SphereCollider3DComponent {
    pub offset: Vec3,
    pub radius: f32,                   // default 0.5
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub collision_layer: u32,
    pub collision_mask: u32,
    pub is_sensor: bool,
    pub runtime_fixture: Option<ColliderHandle>,
}
```

#### CapsuleCollider3DComponent

```rust
struct CapsuleCollider3DComponent {
    pub offset: Vec3,
    pub half_height: f32,              // excluding hemisphere caps, default 0.5
    pub radius: f32,                   // hemisphere radius, default 0.25
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub collision_layer: u32,
    pub collision_mask: u32,
    pub is_sensor: bool,
    pub runtime_fixture: Option<ColliderHandle>,
}
```

### Key Differences from 2D Physics

| Aspect | 2D | 3D |
|--------|----|----|
| Rotation lock | Single `fixed_rotation` bool | Per-axis `lock_rotation_x/y/z` |
| Collider types | Box, Circle | Box, Sphere, Capsule |
| Gravity | `(0, -9.81)` | `(0, -9.81, 0)` |
| Interpolation storage | `(x, y, angle)` | `(Vector3, UnitQuaternion)` |
| Rapier crate | rapier2d 0.22 | rapier3d 0.22 |

### 3D Lua API

| Function | Description |
|----------|-------------|
| `Engine.apply_impulse_3d(entity_id, x, y, z)` | Apply impulse to 3D body |
| `Engine.apply_force_3d(entity_id, x, y, z)` | Apply force to 3D body |
| `Engine.get_linear_velocity_3d(entity_id)` | Returns `(x, y, z)` |
| `Engine.set_linear_velocity_3d(entity_id, x, y, z)` | Set velocity |
| `Engine.raycast_3d(ox, oy, oz, dx, dy, dz, max_dist)` | Returns `(entity_id, x, y, z, nx, ny, nz)` or nil |
| `Engine.get_gravity_3d()` | Returns `(x, y, z)` |
| `Engine.set_gravity_3d(x, y, z)` | Set 3D world gravity |

## Runtime Body Type Changes

Body type can be changed at runtime via Scene methods or Lua scripts.

**Scene API:**

| Method | Description |
|--------|-------------|
| `set_body_type(entity, body_type)` | Change 2D body type (updates both rapier body and component) |
| `get_body_type(entity) -> Option<RigidBody2DType>` | Read current 2D body type |
| `set_body_type_3d(entity, body_type)` | Change 3D body type |
| `get_body_type_3d(entity) -> Option<RigidBody3DType>` | Read current 3D body type |

**Lua API:**

| Function | Description |
|----------|-------------|
| `Engine.set_body_type(entity_id, type_str)` | Set 2D body type: `"static"`, `"dynamic"`, `"kinematic"` |
| `Engine.get_body_type(entity_id) -> string\|nil` | Get current 2D body type |
| `Engine.set_body_type_3d(entity_id, type_str)` | Set 3D body type |
| `Engine.get_body_type_3d(entity_id) -> string\|nil` | Get current 3D body type |

Both `from_str_loose()` parsers accept `"fixed"` as an alias for `"static"` (matching rapier's terminology).

## Shape Overlap Queries

Query the physics world for colliders at a point, within an AABB, or overlapping a shape.

**Scene API (2D):**

| Method | Description |
|--------|-------------|
| `point_query(point) -> Vec<u64>` | Find entities whose colliders contain a point |
| `aabb_query(min, max) -> Vec<u64>` | Find entities whose collider AABBs overlap a box |
| `overlap_circle(center, radius, exclude) -> Vec<u64>` | Circle shape overlap test |
| `overlap_box(center, half_extents, exclude) -> Vec<u64>` | Box shape overlap test |

**Scene API (3D):**

| Method | Description |
|--------|-------------|
| `point_query_3d(point) -> Vec<u64>` | Find entities at a 3D point |
| `aabb_query_3d(min, max) -> Vec<u64>` | Find entities in a 3D AABB |
| `overlap_sphere(center, radius, exclude) -> Vec<u64>` | Sphere shape overlap test |
| `overlap_box_3d(center, half_extents, exclude) -> Vec<u64>` | 3D box shape overlap test |

**Lua API:**

| Function | Returns |
|----------|---------|
| `Engine.point_query(x, y)` | Table of entity IDs |
| `Engine.aabb_query(min_x, min_y, max_x, max_y)` | Table of entity IDs |
| `Engine.overlap_circle(cx, cy, radius, exclude_id?)` | Table of entity IDs |
| `Engine.overlap_box(cx, cy, half_w, half_h, exclude_id?)` | Table of entity IDs |
| `Engine.point_query_3d(x, y, z)` | Table of entity IDs |
| `Engine.aabb_query_3d(min_x, min_y, min_z, max_x, max_y, max_z)` | Table of entity IDs |
| `Engine.overlap_sphere(cx, cy, cz, radius, exclude_id?)` | Table of entity IDs |
| `Engine.overlap_box_3d(cx, cy, cz, hx, hy, hz, exclude_id?)` | Table of entity IDs |

Uses rapier's `QueryPipeline` methods: `intersections_with_point`, `colliders_with_aabb_intersecting_aabb`, and `intersections_with_shape`.

## Joints API

Create joints between entities at runtime. Joints connect two rigid bodies with a constraint.

**Joint types (2D):**

| Joint | Description |
|-------|-------------|
| Revolute | Hinge — bodies rotate around anchor points |
| Fixed | Locks relative position and rotation |
| Prismatic | Slider — bodies translate along an axis |

**Joint types (3D):**

| Joint | Description |
|-------|-------------|
| Revolute | Hinge around a given axis |
| Fixed | Locks relative transform |
| Ball (Spherical) | Point-to-point — free rotation around anchors |
| Prismatic | Slider along a given axis |

**Scene API (2D):**

| Method | Returns |
|--------|---------|
| `create_revolute_joint(entity_a, entity_b, anchor_a, anchor_b) -> Option<u64>` | Joint ID |
| `create_fixed_joint(entity_a, entity_b, anchor_a, anchor_b) -> Option<u64>` | Joint ID |
| `create_prismatic_joint(entity_a, entity_b, anchor_a, anchor_b, axis) -> Option<u64>` | Joint ID |
| `remove_joint(joint_id)` | — |

**Scene API (3D):**

| Method | Returns |
|--------|---------|
| `create_revolute_joint_3d(ea, eb, anchor_a, anchor_b, axis) -> Option<u64>` | Joint ID |
| `create_fixed_joint_3d(ea, eb, anchor_a, anchor_b) -> Option<u64>` | Joint ID |
| `create_ball_joint_3d(ea, eb, anchor_a, anchor_b) -> Option<u64>` | Joint ID |
| `create_prismatic_joint_3d(ea, eb, anchor_a, anchor_b, axis) -> Option<u64>` | Joint ID |
| `remove_joint_3d(joint_id)` | — |

**Lua API:**

| Function | Returns |
|----------|---------|
| `Engine.create_revolute_joint(ea, eb, ax, ay, bx, by)` | joint_id or nil |
| `Engine.create_fixed_joint(ea, eb, ax, ay, bx, by)` | joint_id or nil |
| `Engine.create_prismatic_joint(ea, eb, ax, ay, bx, by, dx, dy)` | joint_id or nil |
| `Engine.remove_joint(joint_id)` | — |
| `Engine.create_revolute_joint_3d(ea, eb, ax, ay, az, bx, by, bz, dx, dy, dz)` | joint_id or nil |
| `Engine.create_fixed_joint_3d(ea, eb, ax, ay, az, bx, by, bz)` | joint_id or nil |
| `Engine.create_ball_joint_3d(ea, eb, ax, ay, az, bx, by, bz)` | joint_id or nil |
| `Engine.create_prismatic_joint_3d(ea, eb, ax, ay, az, bx, by, bz, dx, dy, dz)` | joint_id or nil |
| `Engine.remove_joint_3d(joint_id)` | — |

**Joint handle encoding:** Rapier arena `(index, generation)` packed into u64 as `(generation << 32) | index`. Anchors are in local space of each body. Axis vectors are auto-normalized (near-zero falls back to a default axis with a warning).
