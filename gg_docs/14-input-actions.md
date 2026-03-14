# Input Action Mapping

Data-driven input abstraction layer. Define named actions (button or axis) with multiple physical bindings. Query actions by name instead of raw keys/buttons.

**Files:**
- `gg_engine/src/input_action.rs` — Configuration types, runtime evaluation, dead zone remapping
- `gg_engine/src/input.rs` — `Input` struct holds `InputActionState`, exposes query API
- `gg_engine/src/application.rs` — `Application::input_action_map()` trait method
- `gg_engine/src/project.rs` — `ProjectConfig` stores `InputActionMap` (`.ggproject` YAML, schema v2+)
- `gg_engine/src/scene/script_glue.rs` — Lua bindings for action queries
- `gg_editor/src/panels/project.rs` — Editor UI for defining actions

## Configuration Types

### ActionType

Whether an action produces a digital or analog signal.

| Variant | Description |
|---------|-------------|
| `Button` | Digital on/off. `value` is `1.0` when any binding is active, `0.0` otherwise |
| `Axis` | Continuous `-1.0..1.0`. Supports dead zone remapping and keyboard override |

### InputBinding

A physical input source that can trigger or contribute to an action. Six variants:

| Variant | Fields | Description |
|---------|--------|-------------|
| `Key(KeyCode)` | — | A keyboard key |
| `Mouse(MouseButton)` | — | A mouse button |
| `GamepadButton` | `button`, `gamepad_id: Option<GamepadId>` | A gamepad button. `None` = any connected gamepad (defaults to gamepad 0) |
| `GamepadAxisAsButton` | `axis`, `threshold`, `gamepad_id: Option` | Analog axis treated as digital. Fires when the axis value crosses the threshold |
| `GamepadAxis` | `axis`, `dead_zone`, `scale`, `gamepad_id: Option` | Analog axis contributing a continuous value. Default dead zone: `0.15`, default scale: `1.0` |
| `KeyComposite` | `negative: KeyCode`, `positive: KeyCode` | Two keys forming a `-1 / +1` axis |

### InputAction

One logical input action with its bindings.

```rust
struct InputAction {
    pub name: String,
    pub action_type: ActionType,
    pub bindings: Vec<InputBinding>,
}
```

### InputActionMap

A collection of input actions. Uses `#[serde(transparent)]` so the YAML serializes as a bare sequence under the `InputActions` key.

```rust
#[serde(transparent)]
struct InputActionMap {
    pub actions: Vec<InputAction>,
}
```

Stored on `ProjectConfig` and serialized in the `.ggproject` file (schema v2+).

## Runtime Evaluation

### ActionState

Per-action cached state, updated once per frame.

| Field | Type | Description |
|-------|------|-------------|
| `pressed` | `bool` | Whether the action is currently active |
| `prev_pressed` | `bool` | Whether the action was active last frame |
| `value` | `f32` | Current continuous value |
| `prev_value` | `f32` | Previous frame's continuous value |

| Method | Returns | Description |
|--------|---------|-------------|
| `is_pressed()` | `bool` | `true` while the action is active |
| `is_just_pressed()` | `bool` | `true` only on the first frame the action becomes active |
| `is_just_released()` | `bool` | `true` only on the first frame the action becomes inactive |

### InputActionState

Holds a `HashMap<String, ActionState>` for all actions. Evaluated via `update(map, input)` once per frame before `on_update`.

**Button evaluation:**
- `pressed = true` if ANY binding is active (OR logic)
- `value = 1.0` when pressed, `0.0` otherwise

**Axis evaluation:**
1. Keyboard bindings and gamepad bindings are evaluated separately
2. Keyboard overrides gamepad when the keyboard value is non-zero (`abs > 0.001`)
3. Dead zone remapping is applied to gamepad axis values
4. Scale multiplication is applied after dead zone
5. Final value is clamped to `[-1.0, 1.0]`
6. `pressed = true` when `abs(value) > 0.001`

**State lifecycle:**
- `update(map, input)` — snapshots previous state, evaluates all bindings
- `clear()` — resets all state to zero (called on window focus loss)

### Dead Zone Remapping

Prevents small stick drift from registering as input. Values within the dead zone become zero; values outside are smoothly remapped to avoid a sudden jump at the threshold.

```
if |value| < dead_zone:
    result = 0.0
else:
    result = sign(value) * ((|value| - dead_zone) / (1.0 - dead_zone))
    result = clamp(result, 0.0, 1.0)
```

Default dead zone: `0.15`.

## Application Integration

The `Application` trait provides an optional method to supply an action map:

```rust
fn input_action_map(&self) -> Option<InputActionMap> {
    None  // default: no action mapping
}
```

This is called once after `on_attach` to configure the input system. For the editor, it is also polled each frame to support hot-reload when a project is loaded — but action map updates are only applied when the action count changes (avoids per-frame allocation).

### Split Borrow Pattern

`Input::update_actions()` needs `&Input` for raw queries while mutating `action_state`. This is solved by temporarily moving the state out:

```rust
pub(crate) fn update_actions(&mut self) {
    if let Some(ref map) = self.action_map {
        let map = map.clone();
        let mut action_state = std::mem::take(&mut self.action_state);
        action_state.update(&map, self);
        self.action_state = action_state;
    }
}
```

### Input Query API

The `Input` struct exposes action queries alongside raw input:

| Method | Returns | Description |
|--------|---------|-------------|
| `is_action_pressed(name)` | `bool` | `true` while the action is active |
| `is_action_just_pressed(name)` | `bool` | `true` only on the first frame |
| `is_action_just_released(name)` | `bool` | `true` only on the release frame |
| `action_value(name)` | `f32` | Continuous value (`-1.0..1.0` for axes, `0.0`/`1.0` for buttons) |

Unknown action names return `false` / `0.0` (no error, no warning).

## Lua API

All action functions are registered under the global `Engine` table. They delegate to `Input` via the `SceneScriptContext` pointer. Returns safe defaults (`false` / `0.0`) when input context is unavailable (e.g., during `on_create` / `on_destroy`).

| Function | Signature | Returns |
|----------|-----------|---------|
| `Engine.is_action_pressed` | `(name)` | `bool` |
| `Engine.is_action_just_pressed` | `(name)` | `bool` |
| `Engine.is_action_just_released` | `(name)` | `bool` |
| `Engine.get_action_value` | `(name)` | `f32` |

## Project File Format

Input actions are stored in the `.ggproject` YAML file under the `InputActions` key. This was introduced in schema version 2 (`CURRENT_SCHEMA_VERSION = 2`). Projects at schema v1 get an empty action map via serde default.

```yaml
Project:
  SchemaVersion: 2
  Name: MyGame
  AssetDirectory: assets
  ScriptModulePath: assets/scripts
  StartScene: scenes/main.ggscene
  InputActions:
    - Name: Jump
      Type: Button
      Bindings:
        - Key: Space
        - GamepadButton:
            Button: South
    - Name: MoveX
      Type: Axis
      Bindings:
        - KeyComposite:
            Negative: A
            Positive: D
        - GamepadAxis:
            Axis: LeftStickX
            DeadZone: 0.15
            Scale: 1.0
    - Name: Accelerate
      Type: Button
      Bindings:
        - GamepadAxisAsButton:
            Axis: RightTrigger
            Threshold: 0.5
```

`GamepadId` is omitted from serialization when `None` (any gamepad). `DeadZone` defaults to `0.15` and `Scale` defaults to `1.0` when absent.

## Editor Integration

**File:** `gg_editor/src/panels/project.rs`

The Project panel includes a collapsible input action editor:

- **Add/remove actions**: "+" button to add, "X" button per action to remove
- **Action type selector**: `ComboBox` for `Button` / `Axis`
- **Binding list**: per-action binding list with add/remove and type selectors
- **Auto-save**: changes are written to the `.ggproject` file automatically

The editor's `input_action_map()` implementation returns the current project's action map, which is synced to the `Input` system each frame when the action count changes.

## Example

A practical Lua script using input actions for a platformer character:

```lua
fields = {
    move_speed = 5.0,
    jump_force = 10.0,
}

function on_update(dt)
    local move_x = Engine.get_action_value("MoveX")
    local move_y = Engine.get_action_value("MoveY")

    if Engine.is_action_just_pressed("Jump") then
        Engine.apply_impulse(entity_id, 0, fields.jump_force)
    end

    Engine.set_linear_velocity(entity_id, move_x * fields.move_speed, 0)
end
```

This script works identically whether the player uses WASD, arrow keys, or a gamepad — the action map handles the translation from physical inputs to logical actions.

### Corresponding .ggproject Actions

```yaml
InputActions:
  - Name: MoveX
    Type: Axis
    Bindings:
      - KeyComposite:
          Negative: A
          Positive: D
      - KeyComposite:
          Negative: Left
          Positive: Right
      - GamepadAxis:
          Axis: LeftStickX
          DeadZone: 0.15
          Scale: 1.0
  - Name: MoveY
    Type: Axis
    Bindings:
      - KeyComposite:
          Negative: S
          Positive: W
      - GamepadAxis:
          Axis: LeftStickY
          DeadZone: 0.15
          Scale: 1.0
  - Name: Jump
    Type: Button
    Bindings:
      - Key: Space
      - GamepadButton:
          Button: South
```

## Design Notes

- **Keyboard priority**: when both keyboard and gamepad provide axis values, keyboard wins if non-zero. This prevents stick drift from overriding deliberate keyboard input.
- **GamepadId `None`**: defaults to gamepad 0 at evaluation time. Multi-gamepad games should specify explicit IDs.
- **No per-frame allocation**: the `InputActionState` reuses its `HashMap` across frames. Action map sync in the editor only clones when the action count changes.
- **Axis as button**: `GamepadAxisAsButton` fires when the axis crosses the threshold. Positive threshold = fires on push forward; negative threshold = fires on pull back.
