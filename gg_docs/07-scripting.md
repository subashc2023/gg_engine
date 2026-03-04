# Lua Scripting (LuaJIT)

**Files:** `scene/script_engine.rs`, `scene/script_glue.rs`

Feature-gated (`lua-scripting`, default on). Uses `mlua 0.10` with vendored LuaJIT backend. Kept enabled in dist builds (`--features lua-scripting`).

## Architecture

```
┌─────────────────────────────────────────────────┐
│  Scene                                          │
│  ┌─────────────────────────────────────────┐    │
│  │  ScriptEngine (Option<ScriptEngine>)    │    │
│  │  ┌───────────────┐  ┌───────────────┐   │    │
│  │  │  Lua state     │  │  entity_envs  │   │    │
│  │  │  (LuaJIT)      │  │  HashMap<u64, │   │    │
│  │  │                │  │   RegistryKey> │   │    │
│  │  └───────────────┘  └───────────────┘   │    │
│  └─────────────────────────────────────────┘    │
│                                                  │
│  Entities with LuaScriptComponent:               │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐        │
│  │ script_  │ │ script_  │ │ script_  │        │
│  │ path     │ │ path     │ │ path     │        │
│  │ loaded   │ │ loaded   │ │ loaded   │        │
│  └──────────┘ └──────────┘ └──────────┘        │
└─────────────────────────────────────────────────┘
```

### Core Types

- **ScriptEngine** (`scene/script_engine.rs`): Owns a single `mlua::Lua` state (LuaJIT). Maintains a `HashMap<u64, LuaRegistryKey>` mapping entity UUIDs to per-entity Lua environments. All `Engine.*` bindings are registered on construction via `script_glue::register_all()`.

- **LuaScriptComponent** (`scene/components.rs`): `script_path: String` + `loaded: bool` (runtime-only, resets to `false` on clone — same pattern as physics runtime handles). Serialized to `.ggscene` (script_path only).

- **SceneScriptContext** (`scene/script_glue.rs`): Runtime context set as Lua `app_data` during script execution. Holds raw pointers `scene: *mut Scene` and `input: *const Input`. The `input` pointer is **null** during `on_create` / `on_destroy`; only valid during `on_update`.

## Per-Entity Environment Model

Each entity's script runs in an **isolated Lua environment table**:

1. A new table `env` is created
2. `entity_id = uuid` (u64) is set in the environment
3. A metatable with `__index = _G` is attached — scripts inherit globals (`print`, `math`, `Engine.*`) but local variables stay isolated
4. The script source is loaded and executed in this environment via `set_environment(env)`
5. The environment is stored in the Lua registry keyed by entity UUID

When lifecycle functions are called, `raw_get(name)` is used instead of normal table access. This prevents falling through to `_G` via `__index` — if a function isn't defined in the script, it's treated as a successful no-op (not an error).

## Script Lifecycle

Scripts define up to three lifecycle functions:

```lua
function on_create()
    -- Called once when play mode starts
    -- Input is NOT available here
end

function on_update(dt)
    -- Called every frame with delta time (seconds)
    -- Input IS available here
end

function on_fixed_update(dt)
    -- Called at physics timestep (deterministic rate)
    -- Use for physics code (apply_impulse, set_velocity, etc.)
    -- Input IS available here
end

function on_destroy()
    -- Called once when play mode stops
    -- Input is NOT available here
end
```

All four are optional. `entity_id` is always available as a local variable set during environment creation. `on_fixed_update` runs at the physics timestep rate, interleaved with physics steps — use it for deterministic physics interactions.

### Integration with Scene

```
Editor Play button pressed
         │
         ▼
  on_runtime_start()
    ├── on_physics_2d_start()
    └── on_lua_scripting_start()
            ├── Create ScriptEngine (fresh Lua state)
            ├── For each LuaScriptComponent:
            │     create_entity_env(uuid, script_path)
            │     set loaded = true
            ├── Set SceneScriptContext (input = null)
            └── Call on_create() for each entity
         │
         ▼
  Per-frame loop:
    ├── scene.on_update_scripts(dt, input)      ← NativeScript
    ├── scene.on_update_lua_scripts(dt, input)  ← Lua scripts
    └── scene.on_update_physics(dt)             ← Physics step
         │
         ▼
  on_runtime_stop()
    ├── on_lua_scripting_stop()
    │     ├── Set SceneScriptContext (input = null)
    │     ├── Call on_destroy() for each entity
    │     ├── Drop ScriptEngine
    │     └── Reset loaded = false on all LuaScriptComponents
    └── on_physics_2d_stop()
```

**Simulation mode** runs physics only — no Lua scripts are loaded or executed.

### Take-Modify-Replace Pattern

The ScriptEngine is stored as `Option<ScriptEngine>` on Scene. During script execution:

1. Engine is `Option::take()`-ed from Scene
2. `SceneScriptContext` (raw pointers to scene/input) is set as Lua `app_data`
3. Script functions execute, calling `Engine.*` functions that access the scene via the raw pointer
4. `app_data` is cleared
5. Engine is put back into `self.script_engine`

This ensures exclusive access — no aliasing between the Lua state and Scene ownership.

## Engine API Reference

All functions are registered under the global `Engine` table. Scripts call them as `Engine.function_name(args)`.

### Transform

| Function | Signature | Returns |
|----------|-----------|---------|
| `get_translation` | `(entity_id)` | `(x, y, z)` or `(0, 0, 0)` |
| `set_translation` | `(entity_id, x, y, z)` | — |
| `get_rotation` | `(entity_id)` | `(rx, ry, rz)` radians or `(0, 0, 0)` |
| `set_rotation` | `(entity_id, rx, ry, rz)` | — |
| `get_scale` | `(entity_id)` | `(sx, sy, sz)` or `(1, 1, 1)` |
| `set_scale` | `(entity_id, sx, sy, sz)` | — |

### Input

| Function | Signature | Returns |
|----------|-----------|---------|
| `is_key_down` | `(key_name)` | `bool` |

### Entity Queries

| Function | Signature | Returns |
|----------|-----------|---------|
| `has_component` | `(entity_id, component_name)` | `bool` |
| `find_entity_by_name` | `(name)` | entity UUID (u64) or `nil` |

### Physics

| Function | Signature | Returns |
|----------|-----------|---------|
| `apply_impulse` | `(entity_id, ix, iy)` | — |
| `apply_impulse_at_point` | `(entity_id, ix, iy, px, py)` | — |
| `apply_force` | `(entity_id, fx, fy)` | — |
| `get_linear_velocity` | `(entity_id)` | `(vx, vy)` or `(0, 0)` |
| `set_linear_velocity` | `(entity_id, vx, vy)` | — |
| `get_angular_velocity` | `(entity_id)` | `omega` (rad/s) or `0` |
| `set_angular_velocity` | `(entity_id, omega)` | — |

### Math Utilities

| Function | Signature | Returns |
|----------|-----------|---------|
| `vector_dot` | `(x1, y1, z1, x2, y2, z2)` | `f32` |
| `vector_cross` | `(x1, y1, z1, x2, y2, z2)` | `(x, y, z)` |
| `vector_normalize` | `(x, y, z)` | `(x, y, z)` |

### Debug

| Function | Signature | Description |
|----------|-----------|-------------|
| `rust_function` | `()` | Logs a message proving Rust was called |
| `native_log` | `(text, number)` | Logs text + number |
| `native_log_vector` | `(x, y, z)` | Logs 3 floats as Vec3 |

## Key Name Reference

Accepted names for `Engine.is_key_down()` (case-sensitive):

| Category | Names |
|----------|-------|
| Letters | `"A"` / `"KeyA"` through `"Z"` / `"KeyZ"` |
| Digits | `"0"` / `"Num0"` through `"9"` / `"Num9"` |
| Arrows | `"Up"` / `"ArrowUp"`, `"Down"` / `"ArrowDown"`, `"Left"` / `"ArrowLeft"`, `"Right"` / `"ArrowRight"` |
| Common | `"Space"`, `"Enter"` / `"Return"`, `"Escape"` / `"Esc"`, `"Tab"`, `"Backspace"`, `"Delete"`, `"Insert"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"` |
| Modifiers | `"Shift"` / `"LeftShift"`, `"RightShift"`, `"Ctrl"` / `"Control"` / `"LeftCtrl"`, `"RightCtrl"`, `"Alt"` / `"LeftAlt"`, `"RightAlt"` |
| Function | `"F1"` through `"F12"` |

Unknown key names log a warning and return `false`.

## Component Name Reference

Accepted names for `Engine.has_component()` (case-sensitive):

| Name | Component |
|------|-----------|
| `"Transform"` | `TransformComponent` |
| `"Camera"` | `CameraComponent` |
| `"SpriteRenderer"` | `SpriteRendererComponent` |
| `"CircleRenderer"` | `CircleRendererComponent` |
| `"RigidBody2D"` | `RigidBody2DComponent` |
| `"BoxCollider2D"` | `BoxCollider2DComponent` |
| `"CircleCollider2D"` | `CircleCollider2DComponent` |
| `"NativeScript"` | `NativeScriptComponent` |
| `"LuaScript"` | `LuaScriptComponent` |

## Writing Scripts

Minimal example — a camera panner controlled by arrow keys:

```lua
-- camera_controller.lua
local speed = 5.0

function on_create()
    Engine.native_log("Camera controller created on entity", entity_id)
end

function on_update(dt)
    local x, y, z = Engine.get_translation(entity_id)

    if Engine.is_key_down("Left")  then x = x - speed * dt end
    if Engine.is_key_down("Right") then x = x + speed * dt end
    if Engine.is_key_down("Up")    then y = y + speed * dt end
    if Engine.is_key_down("Down")  then y = y - speed * dt end

    Engine.set_translation(entity_id, x, y, z)
end
```

Attach to an entity by adding a `LuaScriptComponent` with the script path:

```rust
scene.add_component(entity, LuaScriptComponent::new("assets/scripts/camera_controller.lua"));
```

Example scripts in `assets/scripts/`:

| Script | Description |
|--------|-------------|
| `physics_player.lua` | WASD velocity movement + space to jump via `on_fixed_update` (needs RigidBody2D + BoxCollider2D) |
| `camera_controller.lua` | Arrow key camera panning |
| `camera_follow.lua` | Smooth camera follow using `find_entity_by_name` to track another entity |
| `spinner.lua` | Continuous Z-axis rotation at 2.0 rad/s |
| `force_block.lua` | Sustained force (F), torque via impulse-at-point (Q/E), scale (Z/X) |

## UUID Safety

UUIDs are masked to 53 bits (`UUID_SAFE_MASK = (1 << 53) - 1`) so they survive **lossless round-trips through Lua/f64**. IEEE 754 doubles have 53 bits of mantissa — values above 2^53 lose precision. The masking ensures `entity_id` passed from Rust → Lua → Rust is always exact. 2^53 ≈ 9 quadrillion possible values.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| File I/O error | Logged, `create_entity_env` returns `false` |
| Lua execution error | Logged, function returns `false` |
| Missing lifecycle function | NOT an error — treated as successful no-op |
| Unknown key name | Logged as warning, returns `false` |
| No `SceneScriptContext` | Functions return safe defaults (`0.0`, `false`, `(1,1,1)` for scale) |

## Feature Gating

All Lua code is behind `#[cfg(feature = "lua-scripting")]`:

```toml
# gg_engine/Cargo.toml
[features]
default = ["profiling", "lua-scripting"]
lua-scripting = ["mlua"]

[dependencies]
mlua = { version = "0.10", features = ["luajit", "vendored"], optional = true }
```

Feature-gated items: `ScriptEngine` module, `LuaScriptComponent`, Scene lifecycle methods (`on_lua_scripting_start/stop`, `on_update_lua_scripts`), serialization support. The feature chain flows: `gg_editor`/`gg_sandbox` forward `lua-scripting` to `gg_engine`.
