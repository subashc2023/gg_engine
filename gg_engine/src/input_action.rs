use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::events::gamepad::{GamepadAxis, GamepadButton, GamepadId};
use crate::events::{KeyCode, MouseButton};
use crate::input::Input;

// ---------------------------------------------------------------------------
// Configuration types (serialized in .ggproject)
// ---------------------------------------------------------------------------

/// Whether an action produces a digital (button) or analog (axis) signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionType {
    Button,
    Axis,
}

fn default_dead_zone() -> f32 {
    0.15
}
fn default_scale() -> f32 {
    1.0
}

/// A physical input source that can trigger or contribute to an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputBinding {
    /// A keyboard key.
    Key(KeyCode),
    /// A mouse button.
    Mouse(MouseButton),
    /// A gamepad button. `gamepad_id: None` means any connected gamepad.
    GamepadButton {
        #[serde(rename = "Button")]
        button: GamepadButton,
        #[serde(rename = "GamepadId", default, skip_serializing_if = "Option::is_none")]
        gamepad_id: Option<GamepadId>,
    },
    /// A gamepad axis treated as a digital button.
    /// Fires when the axis value crosses the threshold (positive or negative).
    GamepadAxisAsButton {
        #[serde(rename = "Axis")]
        axis: GamepadAxis,
        #[serde(rename = "Threshold")]
        threshold: f32,
        #[serde(rename = "GamepadId", default, skip_serializing_if = "Option::is_none")]
        gamepad_id: Option<GamepadId>,
    },
    /// A gamepad axis contributing an analog value to an Axis action.
    GamepadAxis {
        #[serde(rename = "Axis")]
        axis: GamepadAxis,
        #[serde(rename = "DeadZone", default = "default_dead_zone")]
        dead_zone: f32,
        #[serde(rename = "Scale", default = "default_scale")]
        scale: f32,
        #[serde(rename = "GamepadId", default, skip_serializing_if = "Option::is_none")]
        gamepad_id: Option<GamepadId>,
    },
    /// Two keyboard keys forming a -1 / +1 axis.
    KeyComposite {
        #[serde(rename = "Negative")]
        negative: KeyCode,
        #[serde(rename = "Positive")]
        positive: KeyCode,
    },
}

/// One logical input action with its bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAction {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Type")]
    pub action_type: ActionType,
    #[serde(rename = "Bindings")]
    pub bindings: Vec<InputBinding>,
}

/// A collection of input actions. Stored on `ProjectConfig` and serialized
/// in the `.ggproject` file. Uses `transparent` so the YAML is a bare sequence
/// under the `InputActions` key rather than a nested `{ actions: [...] }`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InputActionMap {
    pub actions: Vec<InputAction>,
}

// ---------------------------------------------------------------------------
// Runtime state (per-frame evaluation)
// ---------------------------------------------------------------------------

/// Cached per-frame state for a single action.
#[derive(Debug, Clone, Default)]
pub struct ActionState {
    pub pressed: bool,
    pub prev_pressed: bool,
    pub value: f32,
    pub prev_value: f32,
}

impl ActionState {
    pub fn is_pressed(&self) -> bool {
        self.pressed
    }
    pub fn is_just_pressed(&self) -> bool {
        self.pressed && !self.prev_pressed
    }
    pub fn is_just_released(&self) -> bool {
        !self.pressed && self.prev_pressed
    }
}

/// Per-frame evaluated state for all actions in the action map.
#[derive(Debug, Clone, Default)]
pub struct InputActionState {
    states: HashMap<String, ActionState>,
}

impl InputActionState {
    pub fn is_action_pressed(&self, name: &str) -> bool {
        self.states.get(name).is_some_and(|s| s.is_pressed())
    }

    pub fn is_action_just_pressed(&self, name: &str) -> bool {
        self.states.get(name).is_some_and(|s| s.is_just_pressed())
    }

    pub fn is_action_just_released(&self, name: &str) -> bool {
        self.states.get(name).is_some_and(|s| s.is_just_released())
    }

    pub fn action_value(&self, name: &str) -> f32 {
        self.states.get(name).map_or(0.0, |s| s.value)
    }

    /// Evaluate all actions against the current raw input state.
    /// Called once per frame before `on_update`.
    pub fn update(&mut self, map: &InputActionMap, input: &Input) {
        for action in &map.actions {
            let state = self.states.entry(action.name.clone()).or_default();

            // Snapshot previous frame.
            state.prev_pressed = state.pressed;
            state.prev_value = state.value;

            match action.action_type {
                ActionType::Button => {
                    state.pressed = action
                        .bindings
                        .iter()
                        .any(|b| evaluate_button_binding(b, input));
                    state.value = if state.pressed { 1.0 } else { 0.0 };
                }
                ActionType::Axis => {
                    let mut keyboard_value = 0.0f32;
                    let mut gamepad_value = 0.0f32;
                    let mut has_keyboard = false;

                    for binding in &action.bindings {
                        match binding {
                            InputBinding::KeyComposite { negative, positive } => {
                                let mut v = 0.0f32;
                                if input.is_key_pressed(*negative) {
                                    v -= 1.0;
                                }
                                if input.is_key_pressed(*positive) {
                                    v += 1.0;
                                }
                                if v.abs() > keyboard_value.abs() {
                                    keyboard_value = v;
                                }
                                has_keyboard = true;
                            }
                            InputBinding::Key(key) => {
                                if input.is_key_pressed(*key) {
                                    if 1.0 > keyboard_value.abs() {
                                        keyboard_value = 1.0;
                                    }
                                    has_keyboard = true;
                                }
                            }
                            InputBinding::GamepadAxis {
                                axis,
                                dead_zone,
                                scale,
                                gamepad_id,
                            } => {
                                let gid = gamepad_id.unwrap_or(0);
                                let raw = input.gamepad_axis(gid, *axis);
                                let processed = apply_dead_zone(raw, *dead_zone) * scale;
                                if processed.abs() > gamepad_value.abs() {
                                    gamepad_value = processed;
                                }
                            }
                            // Button-style bindings on an axis action: digital ±1.
                            other => {
                                if evaluate_button_binding(other, input) {
                                    keyboard_value = 1.0;
                                    has_keyboard = true;
                                }
                            }
                        }
                    }

                    // Keyboard overrides gamepad when non-zero.
                    state.value = if has_keyboard && keyboard_value.abs() > 0.001 {
                        keyboard_value
                    } else {
                        gamepad_value
                    };
                    state.value = state.value.clamp(-1.0, 1.0);
                    state.pressed = state.value.abs() > 0.001;
                }
            }
        }
    }

    /// Reset all action state (e.g. on window focus loss).
    pub fn clear(&mut self) {
        for state in self.states.values_mut() {
            state.pressed = false;
            state.prev_pressed = false;
            state.value = 0.0;
            state.prev_value = 0.0;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Evaluate whether a single binding is currently "pressed" (digital query).
fn evaluate_button_binding(binding: &InputBinding, input: &Input) -> bool {
    match binding {
        InputBinding::Key(key) => input.is_key_pressed(*key),
        InputBinding::Mouse(btn) => input.is_mouse_button_pressed(*btn),
        InputBinding::GamepadButton { button, gamepad_id } => {
            let gid = gamepad_id.unwrap_or(0);
            input.is_gamepad_button_pressed(gid, *button)
        }
        InputBinding::GamepadAxisAsButton {
            axis,
            threshold,
            gamepad_id,
        } => {
            let gid = gamepad_id.unwrap_or(0);
            let val = input.gamepad_axis(gid, *axis);
            if *threshold > 0.0 {
                val > *threshold
            } else {
                val < *threshold
            }
        }
        InputBinding::GamepadAxis {
            axis,
            dead_zone,
            gamepad_id,
            ..
        } => {
            let gid = gamepad_id.unwrap_or(0);
            apply_dead_zone(input.gamepad_axis(gid, *axis), *dead_zone).abs() > 0.001
        }
        InputBinding::KeyComposite { negative, positive } => {
            input.is_key_pressed(*negative) || input.is_key_pressed(*positive)
        }
    }
}

/// Apply dead zone remapping: values within the dead zone become 0,
/// values outside are remapped to 0.0..1.0 so there's no sudden jump.
fn apply_dead_zone(value: f32, dead_zone: f32) -> f32 {
    if value.abs() < dead_zone {
        0.0
    } else {
        let sign = value.signum();
        let normalized = (value.abs() - dead_zone) / (1.0 - dead_zone);
        sign * normalized.clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input() -> Input {
        Input::new()
    }

    #[test]
    fn button_action_key_binding() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "jump".to_string(),
                action_type: ActionType::Button,
                bindings: vec![InputBinding::Key(KeyCode::Space)],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();

        // Not pressed.
        state.update(&map, &input);
        assert!(!state.is_action_pressed("jump"));
        assert!(!state.is_action_just_pressed("jump"));

        // Press space.
        input.press_key(KeyCode::Space);
        state.update(&map, &input);
        assert!(state.is_action_pressed("jump"));
        assert!(state.is_action_just_pressed("jump"));
        assert!(!state.is_action_just_released("jump"));

        // Still held — just_pressed should be false now.
        state.update(&map, &input);
        assert!(state.is_action_pressed("jump"));
        assert!(!state.is_action_just_pressed("jump"));

        // Release.
        input.release_key(KeyCode::Space);
        state.update(&map, &input);
        assert!(!state.is_action_pressed("jump"));
        assert!(state.is_action_just_released("jump"));

        // Next frame: just_released should be false.
        state.update(&map, &input);
        assert!(!state.is_action_just_released("jump"));
    }

    #[test]
    fn button_action_multiple_bindings() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "fire".to_string(),
                action_type: ActionType::Button,
                bindings: vec![
                    InputBinding::Key(KeyCode::Space),
                    InputBinding::Mouse(MouseButton::Left),
                ],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();

        // Press mouse only.
        input.press_mouse_button(MouseButton::Left);
        state.update(&map, &input);
        assert!(state.is_action_pressed("fire"));

        // Release mouse, press key.
        input.release_mouse_button(MouseButton::Left);
        input.press_key(KeyCode::Space);
        state.update(&map, &input);
        assert!(state.is_action_pressed("fire"));
    }

    #[test]
    fn axis_action_key_composite() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "move_h".to_string(),
                action_type: ActionType::Axis,
                bindings: vec![InputBinding::KeyComposite {
                    negative: KeyCode::A,
                    positive: KeyCode::D,
                }],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();

        // Press D → +1.
        input.press_key(KeyCode::D);
        state.update(&map, &input);
        assert!((state.action_value("move_h") - 1.0).abs() < 0.001);

        // Press A too → cancels to 0.
        input.press_key(KeyCode::A);
        state.update(&map, &input);
        assert!(state.action_value("move_h").abs() < 0.001);

        // Release D, A still held → -1.
        input.release_key(KeyCode::D);
        state.update(&map, &input);
        assert!((state.action_value("move_h") + 1.0).abs() < 0.001);
    }

    #[test]
    fn axis_action_gamepad_dead_zone() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "look".to_string(),
                action_type: ActionType::Axis,
                bindings: vec![InputBinding::GamepadAxis {
                    axis: GamepadAxis::LeftStickX,
                    dead_zone: 0.2,
                    scale: 1.0,
                    gamepad_id: Some(0),
                }],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();
        input.gamepad_connect(0);

        // Within dead zone → 0.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.1);
        state.update(&map, &input);
        assert!(state.action_value("look").abs() < 0.001);

        // Just outside dead zone → small positive value.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.3);
        state.update(&map, &input);
        let val = state.action_value("look");
        assert!(val > 0.0 && val < 0.5);

        // Full deflection → ~1.0.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 1.0);
        state.update(&map, &input);
        assert!((state.action_value("look") - 1.0).abs() < 0.001);
    }

    #[test]
    fn dead_zone_remapping() {
        assert!(apply_dead_zone(0.0, 0.15).abs() < 0.001);
        assert!(apply_dead_zone(0.1, 0.15).abs() < 0.001);
        assert!(apply_dead_zone(-0.1, 0.15).abs() < 0.001);
        assert!((apply_dead_zone(1.0, 0.15) - 1.0).abs() < 0.001);
        assert!((apply_dead_zone(-1.0, 0.15) + 1.0).abs() < 0.001);

        // Mid-range should be proportional.
        let mid = apply_dead_zone(0.575, 0.15);
        assert!((mid - 0.5).abs() < 0.01);
    }

    #[test]
    fn unknown_action_returns_defaults() {
        let state = InputActionState::default();
        assert!(!state.is_action_pressed("nonexistent"));
        assert!(!state.is_action_just_pressed("nonexistent"));
        assert!(!state.is_action_just_released("nonexistent"));
        assert!((state.action_value("nonexistent")).abs() < 0.001);
    }

    #[test]
    fn clear_resets_all_state() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "test".to_string(),
                action_type: ActionType::Button,
                bindings: vec![InputBinding::Key(KeyCode::W)],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();
        input.press_key(KeyCode::W);
        state.update(&map, &input);
        assert!(state.is_action_pressed("test"));

        state.clear();
        assert!(!state.is_action_pressed("test"));
        assert!(!state.is_action_just_released("test"));
    }

    #[test]
    fn serialization_round_trip() {
        let map = InputActionMap {
            actions: vec![
                InputAction {
                    name: "jump".to_string(),
                    action_type: ActionType::Button,
                    bindings: vec![
                        InputBinding::Key(KeyCode::Space),
                        InputBinding::GamepadButton {
                            button: GamepadButton::South,
                            gamepad_id: None,
                        },
                    ],
                },
                InputAction {
                    name: "move_h".to_string(),
                    action_type: ActionType::Axis,
                    bindings: vec![
                        InputBinding::KeyComposite {
                            negative: KeyCode::A,
                            positive: KeyCode::D,
                        },
                        InputBinding::GamepadAxis {
                            axis: GamepadAxis::LeftStickX,
                            dead_zone: 0.15,
                            scale: 1.0,
                            gamepad_id: None,
                        },
                    ],
                },
            ],
        };

        let yaml = serde_yaml_ng::to_string(&map).expect("serialize");
        let loaded: InputActionMap = serde_yaml_ng::from_str(&yaml).expect("deserialize");
        assert_eq!(loaded.actions.len(), 2);
        assert_eq!(loaded.actions[0].name, "jump");
        assert_eq!(loaded.actions[1].name, "move_h");
        assert_eq!(loaded.actions[0].bindings.len(), 2);
        assert_eq!(loaded.actions[1].bindings.len(), 2);
    }

    #[test]
    fn gamepad_axis_as_button() {
        let map = InputActionMap {
            actions: vec![InputAction {
                name: "accelerate".to_string(),
                action_type: ActionType::Button,
                bindings: vec![InputBinding::GamepadAxisAsButton {
                    axis: GamepadAxis::RightTrigger,
                    threshold: 0.5,
                    gamepad_id: Some(0),
                }],
            }],
        };

        let mut state = InputActionState::default();
        let mut input = make_input();
        input.gamepad_connect(0);

        // Below threshold.
        input.set_gamepad_axis(0, GamepadAxis::RightTrigger, 0.3);
        state.update(&map, &input);
        assert!(!state.is_action_pressed("accelerate"));

        // Above threshold.
        input.set_gamepad_axis(0, GamepadAxis::RightTrigger, 0.8);
        state.update(&map, &input);
        assert!(state.is_action_pressed("accelerate"));
        assert!(state.is_action_just_pressed("accelerate"));
    }
}
