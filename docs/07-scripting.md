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
│  │ field_   │ │ field_   │ │ field_   │        │
│  │ overrides│ │ overrides│ │ overrides│        │
│  └──────────┘ └──────────┘ └──────────┘        │
└─────────────────────────────────────────────────┘
```

### Core Types

- **ScriptEngine** (`scene/script_engine.rs`): Owns a single `mlua::Lua` state (LuaJIT). Maintains a `HashMap<u64, LuaRegistryKey>` mapping entity UUIDs to per-entity Lua environments. Also maintains `error_counts: HashMap<(u64, String), u32>` for error throttling. All `Engine.*` bindings are registered on construction via `script_glue::register_all()`. An instruction-count hook (10 million instructions) prevents infinite loops.

- **LuaScriptComponent** (`scene/components.rs`): `script_path: String` + `field_overrides: HashMap<String, ScriptFieldValue>` + `loaded: bool` (runtime-only, resets to `false` on clone — same pattern as physics runtime handles). Serialized to `.ggscene` (script_path + field_overrides only).

- **SceneScriptContext** (`scene/script_glue.rs`): Runtime context set as Lua `app_data` during script execution. Holds raw pointers `scene: *mut Scene` and `input: *const Input`. The `input` pointer is **null** during `on_create` / `on_destroy`; only valid during `on_update` / `on_fixed_update`.

- **ScriptFieldValue** (`scene/script_engine.rs`): Enum representing configurable script values: `Bool(bool)`, `Float(f64)`, `String(String)`. Serialized with `#[serde(untagged)]` for clean YAML output.

## Per-Entity Environment Model

Each entity's script runs in an **isolated Lua environment table**:

1. A new table `env` is created
2. `entity_id = uuid` (u64) is set in the environment
3. A metatable with `__index = _G` is attached — scripts inherit globals (`print`, `math`, `Engine.*`) but local variables stay isolated
4. The script source is loaded and executed in this environment via `set_environment(env)`
5. The environment is stored in the Lua registry keyed by entity UUID
6. The environment is also mirrored into a Lua-side master table (`__gg_entity_envs`) so that cross-entity functions (`get_script_field`, `set_script_field`) can access it without a raw pointer to ScriptEngine

When lifecycle functions are called, `raw_get(name)` is used instead of normal table access. This prevents falling through to `_G` via `__index` — if a function isn't defined in the script, it's treated as a successful no-op (not an error).

## Script Lifecycle

Scripts define up to six lifecycle functions:

```lua
function on_create()
    -- Called once when play mode starts
    -- Input is NOT available here
    -- field_overrides have already been applied to `fields` table
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

function on_collision_enter(other_uuid)
    -- Called when a collision with another entity begins
    -- other_uuid is the UUID of the other entity
end

function on_collision_exit(other_uuid)
    -- Called when a collision with another entity ends
    -- other_uuid is the UUID of the other entity
end

function on_destroy()
    -- Called once when play mode stops
    -- Input is NOT available here
end
```

All six are optional. `entity_id` is always available as a local variable set during environment creation. `on_fixed_update` runs at the physics timestep rate, interleaved with physics steps — use it for deterministic physics interactions. `on_collision_enter` and `on_collision_exit` are called by the physics system when collisions begin or end; the argument is the UUID of the other colliding entity.

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
            │     apply field_overrides to env's fields table
            │     set loaded = true
            ├── Set SceneScriptContext (input = null)
            └── Call on_create() for each entity
         │
         ▼
  Per-frame loop:
    ├── scene.on_update_scripts(dt, input)      ← NativeScript
    ├── scene.on_update_lua_scripts(dt, input)  ← Lua scripts
    │     ├── on_update(dt) for each entity
    │     └── on_fixed_update(dt) at physics rate
    └── scene.on_update_physics(dt)             ← Physics step
         │                                         ├── on_collision_enter(other_uuid)
         │                                         └── on_collision_exit(other_uuid)
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

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `is_key_down` | `(key_name)` | `bool` | `true` while the key is held |
| `is_key_pressed` | `(key_name)` | `bool` | `true` only on the first frame the key is pressed |
| `is_key_released` | `(key_name)` | `bool` | `true` only on the first frame the key is released |
| `is_mouse_button_down` | `(button_name)` | `bool` | `true` while the button is held |
| `is_mouse_button_pressed` | `(button_name)` | `bool` | `true` only on the first frame the button is pressed |
| `get_mouse_position` | `()` | `(x, y)` f64 | Screen-space mouse position |
| `get_mouse_delta` | `()` | `(dx, dy)` f64 | Raw mouse motion delta this frame |
| `get_scroll_delta` | `()` | `(dx, dy)` f64 | Scroll wheel delta this frame |

All input functions return `false` / `(0, 0)` during `on_create` and `on_destroy` (input pointer is null).

### Gamepad

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `is_gamepad_button_down` | `(gamepad_id, button_name)` | `bool` | `true` while the button is held |
| `is_gamepad_button_pressed` | `(gamepad_id, button_name)` | `bool` | `true` on the first frame the button is pressed |
| `is_gamepad_button_released` | `(gamepad_id, button_name)` | `bool` | `true` on the first frame the button is released |
| `get_gamepad_axis` | `(gamepad_id, axis_name)` | `f32` | Analog axis value, dead-zone filtered (see `set_dead_zone`) |
| `is_gamepad_connected` | `(gamepad_id)` | `bool` | `true` if the gamepad is connected |
| `set_dead_zone` | `(axis_name, value)` | — | Set global dead zone for a gamepad axis (0.0–0.99). Default 0.15 for sticks, 0.0 for triggers |
| `get_dead_zone` | `(axis_name)` | `f32` | Get global dead zone for a gamepad axis |

`gamepad_id` is a zero-based integer identifying the gamepad (0 = first gamepad, etc.).

**Button names** (case-sensitive):

| Name(s) | Button |
|----------|--------|
| `"South"` / `"A"` / `"Cross"` | Face button bottom |
| `"East"` / `"B"` / `"Circle"` | Face button right |
| `"West"` / `"X"` / `"Square"` | Face button left |
| `"North"` / `"Y"` / `"Triangle"` | Face button top |
| `"LeftBumper"` / `"L1"` | Left shoulder button |
| `"RightBumper"` / `"R1"` | Right shoulder button |
| `"LeftTrigger"` / `"L2"` | Left trigger button |
| `"RightTrigger"` / `"R2"` | Right trigger button |
| `"Select"` / `"Back"` / `"Share"` | Select / Back / Share button |
| `"Start"` / `"Options"` | Start / Options button |
| `"Guide"` / `"Home"` / `"PS"` | Guide / Home button |
| `"LeftStick"` / `"L3"` | Left stick click |
| `"RightStick"` / `"R3"` | Right stick click |
| `"DPadUp"` | D-pad up |
| `"DPadDown"` | D-pad down |
| `"DPadLeft"` | D-pad left |
| `"DPadRight"` | D-pad right |

**Axis names** (case-sensitive):

| Name | Axis |
|------|------|
| `"LeftStickX"` | Left stick horizontal |
| `"LeftStickY"` | Left stick vertical |
| `"RightStickX"` | Right stick horizontal |
| `"RightStickY"` | Right stick vertical |
| `"LeftTrigger"` | Left trigger analog |
| `"RightTrigger"` | Right trigger analog |

Unknown button or axis names log a warning and return `false` / `0.0`.

### Input Actions

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `is_action_pressed` | `(action_name)` | `bool` | `true` while the action is active |
| `is_action_just_pressed` | `(action_name)` | `bool` | `true` on the first frame the action becomes active |
| `is_action_just_released` | `(action_name)` | `bool` | `true` on the first frame the action becomes inactive |
| `get_action_value` | `(action_name)` | `f32` | Continuous axis value (e.g. -1.0..1.0 for analog input) |

Input actions are defined in the `.ggproject` file via the editor's Project panel. Actions map logical names to physical keys/buttons, decoupling game logic from specific input bindings.

### Entity Queries

| Function | Signature | Returns |
|----------|-----------|---------|
| `has_component` | `(entity_id, component_name)` | `bool` |
| `find_entity_by_name` | `(name)` | entity UUID (u64) or `nil` |
| `get_entity_name` | `(uuid)` | entity tag name (string) or `nil` |

### Entity Lifecycle

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `create_entity` | `(name)` | entity UUID (u64) | Creates entity with Tag + Transform + Id components |
| `destroy_entity` | `(uuid)` | — | Deferred destruction (queued, not immediate) |
| `get_entity_name` | `(uuid)` | string or `nil` | Returns the entity's TagComponent value |

### Entity Hierarchy

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_parent` | `(child_id, parent_id)` | `bool` | `false` if entity not found or cycle detected. Preserves world transform |
| `detach_from_parent` | `(entity_id)` | — | Makes entity a root entity. Preserves world transform |
| `get_parent` | `(entity_id)` | UUID (u64) or `nil` | `nil` if the entity is a root entity |
| `get_children` | `(entity_id)` | table | 1-indexed Lua array of child UUIDs |

### Cross-Entity Field Access

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `get_script_field` | `(entity_id, field_name)` | value or `nil` | Reads from another entity's `fields` table |
| `set_script_field` | `(entity_id, field_name, value)` | — | Only Bool, Integer, Number, and String values accepted |

These functions access entity environments directly from the Lua-side registry table (`__gg_entity_envs`), requiring no ScriptEngine pointer.

### Animation

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `play_animation` | `(entity_id, clip_name)` | `bool` | Requires `SpriteAnimatorComponent`. Returns `true` if clip found |
| `stop_animation` | `(entity_id)` | — | Stops the currently playing animation |
| `is_animation_playing` | `(entity_id)` | `bool` | Returns `true` if an animation is currently playing |
| `get_current_animation` | `(entity_id)` | `string` or `nil` | Current clip name |
| `get_animation_frame` | `(entity_id)` | `u32` | Current frame index |
| `set_animation_speed` | `(entity_id, speed)` | — | Set `speed_scale` (1.0 = normal) |
| `play_instanced_animation` | `(entity_id, clip_name)` | `bool` | GPU-driven animation via `InstancedSpriteAnimator` |
| `stop_instanced_animation` | `(entity_id)` | — | Stop instanced animation |
| `get_instanced_animation` | `(entity_id)` | `string` or `nil` | Current instanced clip name |
| `set_anim_param` | `(entity_id, name, value)` | — | Set animation controller parameter (auto-detects bool/float) |
| `get_anim_param` | `(entity_id, name)` | value or `nil` | Get animation controller parameter |

### Timers

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_timeout` | `(ms, callback)` | `timer_id` (integer) | One-shot timer, fires `callback()` after `ms` milliseconds |
| `set_interval` | `(ms, callback)` | `timer_id` (integer) | Repeating timer, fires `callback()` every `ms` milliseconds |
| `clear_timer` | `(timer_id)` | — | Cancel a timer by ID |

### Coroutines

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `start_coroutine` | `(fn)` | — | Start a coroutine that runs `fn`. The function can yield to pause |
| `wait` | `(seconds)` | — | Pause the coroutine for `seconds` (call inside a coroutine only) |
| `wait_frame` | `()` | — | Pause the coroutine until the next frame (call inside a coroutine only) |
| `stop_all_coroutines` | `()` | — | Cancel all coroutines for the calling entity |

`Engine.wait()` and `Engine.wait_frame()` are Lua-side wrappers that call `coroutine.yield()`. You can also use `coroutine.yield()` directly to pause until the next frame. Coroutines are resumed each frame after `on_update`, timers, and before the event bus. Dead or errored coroutines are automatically cleaned up.

```lua
function on_create()
    Engine.start_coroutine(function()
        Engine.log("Starting countdown...")
        Engine.wait(1.0)       -- pause 1 second
        Engine.log("1...")
        Engine.wait(1.0)
        Engine.log("2...")
        Engine.wait(1.0)
        Engine.log("Go!")
    end)
end
```

### Event Bus

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `emit` | `(event_name, data?)` | — | Broadcast an event to all listeners. `data` is an optional value (typically a table) |
| `on` | `(event_name, callback)` | — | Register a listener for an event. Tied to the calling entity. One listener per entity per event (last wins) |
| `off` | `(event_name)` | — | Unregister the calling entity's listener for an event |

Events are dispatched after `on_update` + timers + coroutines (same frame). Cascading emits (events that trigger new emits) are handled via a drain loop with a 100-round safety limit. Listeners are automatically cleaned up when their entity is destroyed.

```lua
-- listener.lua
function on_create()
    Engine.on("player_died", function(data)
        Engine.log("Player died at position: " .. data.x .. ", " .. data.y)
    end)
end

-- player.lua
function on_update(dt)
    if health <= 0 then
        local x, y, z = Engine.get_translation(entity_id)
        Engine.emit("player_died", { x = x, y = y })
    end
end
```

### Audio

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `play_sound` | `(entity_id)` | — | Plays the entity's `AudioSourceComponent` |
| `stop_sound` | `(entity_id)` | — | Stops audio playback |
| `pause_sound` | `(entity_id)` | — | Pause audio playback (can be resumed) |
| `resume_sound` | `(entity_id)` | — | Resume previously paused audio |
| `set_volume` | `(entity_id, volume)` | — | `volume` is `f32` (0.0 = silent, 1.0 = full) |
| `set_panning` | `(entity_id, panning)` | — | `panning` is `f32` (-1.0 = left, 0.0 = center, 1.0 = right) |
| `fade_in` | `(entity_id, duration_secs)` | — | Play (or resume) with fade from silence over `duration_secs` |
| `fade_out` | `(entity_id, duration_secs)` | — | Fade to silence and stop over `duration_secs` |
| `fade_to` | `(entity_id, volume, duration_secs)` | — | Fade to target `volume` over `duration_secs` |
| `set_master_volume` | `(volume)` | — | Set global master volume (0.0–1.0) |
| `get_master_volume` | `()` | `f32` | Get global master volume |
| `set_category_volume` | `(category, volume)` | — | Set volume for a category. Category: `"sfx"`, `"music"`, `"ambient"`, `"voice"` (case-insensitive) |
| `get_category_volume` | `(category)` | `f32` | Get volume for a category |
| `mute_bus` | `(category)` | — | Mute a sound category bus. Category: `"sfx"`, `"music"`, `"ambient"`, `"voice"` (case-insensitive) |
| `unmute_bus` | `(category)` | — | Unmute a sound category bus |
| `is_bus_muted` | `(category)` | `bool` | Check if a sound category bus is muted |
| `set_hrtf` | `(entity_id, enabled)` | — | Enable/disable HRTF binaural processing on an entity's `AudioSourceComponent` |
| `get_hrtf` | `(entity_id)` | `bool` | Get whether HRTF is enabled on an entity's audio source |

### Voice Management

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_max_voices` | `(count)` | — | Set global voice limit (default 32). When exceeded, lowest-priority voice is stolen |
| `get_max_voices` | `()` | `int` | Get global voice limit |
| `set_max_voices_per_entity` | `(count)` | — | Set per-entity voice limit (default 4) |
| `get_max_voices_per_entity` | `()` | `int` | Get per-entity voice limit |
| `get_active_voice_count` | `()` | `int` | Get current number of active voices across all entities |

### Cursor & Window

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_cursor_mode` | `(mode)` | — | `"normal"`, `"confined"`, or `"locked"` |
| `get_cursor_mode` | `()` | `string` | Current cursor mode |
| `get_window_size` | `()` | `(width, height)` | Physical pixel dimensions |
| `set_window_size` | `(width, height)` | — | Request window resize |

### UI Anchors

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_ui_anchor` | `(entity_id, ax, ay, ox, oy)` | — | Adds/updates UIAnchorComponent. Anchor 0-1, offset in world units |
| `get_ui_anchor` | `(entity_id)` | `(ax, ay, ox, oy)` or `nil` | Returns anchor and offset values |

### Component Manipulation

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `add_component` | `(entity_id, name, ...)` | `bool` | Add a component to an entity at runtime. Returns `true` on success. Extra args depend on component type |
| `remove_component` | `(entity_id, name)` | `bool` | Remove a component from an entity at runtime. Returns `true` if removed |

**Supported component names for `add_component`:**

| Name | Extra Arguments | Notes |
|------|-----------------|-------|
| `"SpriteRenderer"` | — | Default white sprite |
| `"CircleRenderer"` | — | Default white circle |
| `"Text"` | `text_string` | Creates text component with given string |
| `"AudioSource"` | — | Default audio source |
| `"AudioListener"` | — | Default audio listener |
| `"ParticleEmitter"` | — | Default particle emitter |
| `"UIAnchor"` | `ax, ay, ox, oy` | Creates UI anchor with anchor and offset values |
| `"UIRect"` | `width, height` | Creates UI rect with size (default 100x100) |
| `"UIImage"` | — | Default UI image |
| `"UIInteractable"` | — | Default UI interactable |
| `"UILayout"` | — | Default UI layout |
| `"Camera"` | — | Default camera |
| `"RigidBody2D"` | — | Default dynamic body |
| `"BoxCollider2D"` | — | Default 1x1 box collider |
| `"CircleCollider2D"` | — | Default radius 0.5 circle collider |
| `"SpriteAnimator"` | — | Default sprite animator |

**Supported component names for `remove_component`:** `"SpriteRenderer"`, `"CircleRenderer"`, `"Text"`, `"AudioSource"`, `"AudioListener"`, `"ParticleEmitter"`, `"UIAnchor"`, `"UIRect"`, `"UIImage"`, `"UIInteractable"`, `"UILayout"`, `"Camera"`, `"SpriteAnimator"`, `"RigidBody2D"`, `"BoxCollider2D"`, `"CircleCollider2D"`.

If the entity already has the component, `add_component` updates it (for types that accept extra args) or is a no-op. Unknown component names log a warning and return `false`.

### Runtime Settings

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `get_vsync` | `()` | `bool` | Current VSync state |
| `set_vsync` | `(enabled)` | — | Request VSync toggle (Fifo/Mailbox) |
| `get_fullscreen` | `()` | `string` | `"windowed"`, `"borderless"`, or `"exclusive"` |
| `set_fullscreen` | `(mode)` | — | `"windowed"`, `"borderless"`, or `"exclusive"` |
| `get_shadow_quality` | `()` | `int` | 0=Low, 1=Medium, 2=High, 3=Ultra |
| `set_shadow_quality` | `(level)` | — | 0-3, errors on out of range |
| `get_gui_scale` | `()` | `number` | Current GUI scale factor |
| `set_gui_scale` | `(factor)` | — | Set GUI scale (affects UI anchors) |
| `quit` | `()` | — | Request application exit |
| `load_scene` | `(path)` | — | Request scene transition (deferred) |

### Save/Load

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `save_data` | `(slot_name, table)` | `bool` | Serialize a Lua table to `<project>/saves/<slot>.json`. Returns `true` on success. Uses atomic write |
| `load_data` | `(slot_name)` | table or `nil` | Load a Lua table from `<project>/saves/<slot>.json`. Returns `nil` if not found or parse error |
| `delete_save` | `(slot_name)` | `bool` | Delete a save file. Returns `true` if deleted, `false` if not found |
| `save_exists` | `(slot_name)` | `bool` | Check if a save file exists |
| `list_saves` | `()` | table | Returns a 1-indexed array of all save slot name strings |

Slot names are sanitized against path traversal — `..`, `/`, and `\` are rejected. Empty slot names are also rejected. The save directory is configured by the editor/player from the project directory.

```lua
-- Save game state
Engine.save_data("checkpoint_1", {
    level = 7,
    player_name = "Hero",
    items = {"axe", "bow"},
    position = { x = 10.5, y = 3.2 },
})

-- Load game state
local data = Engine.load_data("checkpoint_1")
if data then
    Engine.log("Loaded level " .. data.level)
end

-- Check and list saves
if Engine.save_exists("checkpoint_1") then
    Engine.log("Save exists!")
end
local all_saves = Engine.list_saves()  -- e.g. {"checkpoint_1", "autosave"}
```

### Loading Screen

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `set_loading_screen_color` | `(r, g, b)` | — | Set the background color shown during scene transitions. RGB values 0.0–1.0. Default is black |
| `get_loading_screen_color` | `()` | `(r, g, b)` | Get current loading screen color |

Set before calling `load_scene()` for the color to take effect on the next transition. The color is propagated across scene transitions and `Scene::copy()`.

### Tilemap

| Function / Constant | Signature | Returns | Notes |
|----------------------|-----------|---------|-------|
| `set_tile` | `(entity_id, x, y, tile_id)` | — | Sets tile at grid position. Requires `TilemapComponent` |
| `get_tile` | `(entity_id, x, y)` | `i32` | Returns tile ID at grid position, `-1` if empty/OOB |
| `TILE_FLIP_H` | — | `0x4000_0000` | Bit flag for horizontal flip (bit 30). OR with tile ID |
| `TILE_FLIP_V` | — | `0x2000_0000` | Bit flag for vertical flip (bit 29). OR with tile ID |
| `TILE_ID_MASK` | — | `0x1FFF_FFFF` | Mask to extract raw tile ID (lower 29 bits) |

Tile IDs support flip flags in the high bits. Combine via bitwise OR: `Engine.set_tile(entity_id, x, y, bit.bor(tile_id, Engine.TILE_FLIP_H))`.

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

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `vector_dot` | `(x1, y1, z1, x2, y2, z2)` | `f32` | Dot product of two 3D vectors |
| `vector_cross` | `(x1, y1, z1, x2, y2, z2)` | `(x, y, z)` | Cross product of two 3D vectors |
| `vector_normalize` | `(x, y, z)` | `(x, y, z)` | Normalize a 3D vector to unit length |
| `vector_length` | `(x, y, z)` | `f32` | Length (magnitude) of a 3D vector |
| `distance` | `(x1, y1, z1, x2, y2, z2)` | `f32` | Euclidean distance between two 3D points |
| `distance_2d` | `(x1, y1, x2, y2)` | `f32` | Euclidean distance between two 2D points |
| `lerp` | `(a, b, t)` | `f32` | Linear interpolation between two scalars |
| `lerp_vec3` | `(x1, y1, z1, x2, y2, z2, t)` | `(x, y, z)` | Component-wise lerp of two 3D vectors |
| `slerp` | `(x1, y1, z1, w1, x2, y2, z2, w2, t)` | `(x, y, z, w)` | Spherical interpolation between two quaternions |
| `clamp` | `(value, min, max)` | `f32` | Clamp a value to the range [min, max] |
| `move_toward` | `(current, target, max_delta)` | `f32` | Move scalar toward target by at most `max_delta` |
| `move_toward_vec3` | `(x1, y1, z1, x2, y2, z2, max_delta)` | `(x, y, z)` | Move 3D point toward target by at most `max_delta` distance |

### Utility

| Function | Signature | Returns | Notes |
|----------|-----------|---------|-------|
| `log` | `(...)` | — | Variadic logging to the engine console at Info level. All arguments are converted to strings and concatenated with tabs |
| `get_time` | `()` | `f64` | Scene elapsed time in seconds (monotonic) |
| `delta_time` | `()` | `f64` | Frame delta time in seconds |

### Debug

| Function | Signature | Description |
|----------|-----------|-------------|
| `rust_function` | `()` | Logs a message proving Rust was called |
| `native_log` | `(text, number)` | Logs text + number |
| `native_log_vector` | `(x, y, z)` | Logs 3 floats as Vec3 |

## Module System

Scripts can import shared modules using the standard Lua `require()` function:

```lua
local utils = require("utils.math")
local result = utils.add(1, 2)
```

Module names use dot-separated paths: `require("utils.math")` loads `<script_module_path>/utils/math.lua`. The search path is set from the project's `ScriptModulePath` setting.

**Key behaviors:**

- **Cached**: Each module file is loaded and executed at most once. Subsequent `require()` calls return the cached result
- **Sandboxed**: Modules execute in their own environment. A module returns a value (typically a table of functions) that becomes the cached result
- **No path traversal**: Module names containing `..`, leading `/` or `\` are rejected with an error
- **Path validation**: The resolved file path must be within the search directory — attempts to escape are blocked
- **Return value**: If a module returns `nil`/nothing, `true` is cached as a sentinel so the file is not re-executed

```lua
-- utils/math.lua (module file)
local M = {}
function M.add(a, b) return a + b end
function M.clamp(val, lo, hi) return math.max(lo, math.min(hi, val)) end
return M
```

## Key Name Reference

Accepted names for `Engine.is_key_down()`, `Engine.is_key_pressed()`, and `Engine.is_key_released()` (case-sensitive):

| Category | Names |
|----------|-------|
| Letters | `"A"` / `"KeyA"` through `"Z"` / `"KeyZ"` |
| Digits | `"0"` / `"Num0"` through `"9"` / `"Num9"` |
| Arrows | `"Up"` / `"ArrowUp"`, `"Down"` / `"ArrowDown"`, `"Left"` / `"ArrowLeft"`, `"Right"` / `"ArrowRight"` |
| Common | `"Space"`, `"Enter"` / `"Return"`, `"Escape"` / `"Esc"`, `"Tab"`, `"Backspace"`, `"Delete"`, `"Insert"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"` |
| Modifiers | `"Shift"` / `"LeftShift"`, `"RightShift"`, `"Ctrl"` / `"Control"` / `"LeftCtrl"`, `"RightCtrl"`, `"Alt"` / `"LeftAlt"`, `"RightAlt"` |
| Function | `"F1"` through `"F12"` |

Unknown key names log a warning and return `false`.

## Mouse Button Name Reference

Accepted names for `Engine.is_mouse_button_down()` and `Engine.is_mouse_button_pressed()` (case-sensitive):

| Name | Button |
|------|--------|
| `"Left"` | Left mouse button |
| `"Right"` | Right mouse button |
| `"Middle"` | Middle mouse button (scroll wheel click) |
| `"Back"` | Back side button |
| `"Forward"` | Forward side button |

Unknown button names log a warning and return `false`.

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
| `"Tilemap"` | `TilemapComponent` |
| `"AudioSource"` / `"Audio"` | `AudioSourceComponent` |
| `"UIAnchor"` | `UIAnchorComponent` |
| `"MeshRenderer"` | `MeshRendererComponent` |
| `"RigidBody3D"` | `RigidBody3DComponent` |
| `"BoxCollider3D"` | `BoxCollider3DComponent` |
| `"SphereCollider3D"` | `SphereCollider3DComponent` |
| `"CapsuleCollider3D"` | `CapsuleCollider3DComponent` |
| `"DirectionalLight"` | `DirectionalLightComponent` |
| `"PointLight"` | `PointLightComponent` |
| `"AmbientLight"` | `AmbientLightComponent` |
| `"ParticleEmitter"` | `ParticleEmitterComponent` |

## Field Override System

Scripts can declare a `fields` table to expose configurable values to the editor:

```lua
-- player.lua
fields = {
    speed = 5.0,
    jump_force = 10.0,
    is_grounded = false,
    player_name = "Hero",
}

function on_update(dt)
    -- Access via fields.speed, fields.jump_force, etc.
    local vx = 0
    if Engine.is_key_down("D") then vx = fields.speed end
    if Engine.is_key_down("A") then vx = -fields.speed end
    Engine.set_linear_velocity(entity_id, vx, 0)
end
```

### ScriptFieldValue Types

The `fields` table supports three value types, represented by the `ScriptFieldValue` enum:

| Variant | Lua Type | Editor Widget |
|---------|----------|---------------|
| `Bool(bool)` | `boolean` | Checkbox |
| `Float(f64)` | `number` / `integer` | Drag float |
| `String(String)` | `string` | Text input |

### How It Works

1. **Discovery**: In edit mode, `ScriptEngine::discover_fields(script_path)` executes the script in a temporary Lua state and reads its `fields` table. Returns `(name, default_value)` pairs sorted alphabetically for stable UI order.

2. **Editor overrides**: The Properties panel displays discovered fields. Per-entity overrides are stored in `LuaScriptComponent.field_overrides: HashMap<String, ScriptFieldValue>`.

3. **Runtime application**: When play mode starts, after `create_entity_env()` loads the script, field overrides are applied to the entity's `fields` table via `ScriptEngine::set_entity_field()` — this happens **before** `on_create()` is called.

4. **Live editing**: During play mode, field values can be edited in the Properties panel. Changes are applied immediately to the running Lua environment via `set_entity_field()`.

5. **Serialization**: `field_overrides` are serialized to `.ggscene` alongside `script_path`. Clean YAML output via `#[serde(untagged)]` (e.g., `speed: 5.0` instead of `speed: !Float 5.0`).

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

UUIDs are masked to 53 bits (`UUID_SAFE_MASK = (1 << 53) - 1`) so they survive **lossless round-trips through Lua/f64**. IEEE 754 doubles have 53 bits of mantissa — values above 2^53 lose precision. The masking ensures `entity_id` passed from Rust -> Lua -> Rust is always exact. 2^53 ~ 9 quadrillion possible values.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| File I/O error | Logged, `create_entity_env` returns `false` |
| Lua execution error | Logged, function returns `false` |
| Missing lifecycle function | NOT an error — treated as successful no-op |
| Unknown key name | Logged as warning, returns `false` |
| Unknown mouse button name | Logged as warning, returns `false` |
| No `SceneScriptContext` | Functions return safe defaults (`0.0`, `false`, `(1,1,1)` for scale) |
| Instruction limit exceeded | `RuntimeError` — prevents infinite loops (10M instruction limit) |

### Error Throttling

Scripts that repeatedly fail are automatically disabled to prevent log spam:

- **Threshold**: `MAX_SCRIPT_ERRORS = 10` consecutive errors in the same callback
- **Tracking**: Error counts are tracked per `(entity UUID, callback name)` pair — errors in `on_update` do not affect the count for `on_fixed_update` on the same entity
- **Reset**: A successful call to a callback resets its error count to zero
- **Disabled behavior**: Once disabled, the callback silently returns `false` without executing — no further log output

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
