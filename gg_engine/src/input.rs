use std::collections::{HashMap, HashSet};

use crate::events::gamepad::{GamepadAxis, GamepadButton, GamepadId, DEFAULT_DEAD_ZONES};
use crate::events::{KeyCode, MouseButton};
use crate::input_action::{self, InputActionMap, InputActionState};

/// Tracks the current state of keyboard and mouse input.
///
/// Updated each frame by the engine before layer/application callbacks.
/// Query methods let any code with an `&Input` reference poll the current
/// state without needing to track events manually.
///
/// Supports both "held" queries ([`is_key_pressed`](Self::is_key_pressed))
/// and single-frame "just pressed" queries
/// ([`is_key_just_pressed`](Self::is_key_just_pressed)).
pub struct Input {
    keys_pressed: HashSet<KeyCode>,
    keys_prev: HashSet<KeyCode>,
    mouse_buttons_pressed: HashSet<MouseButton>,
    mouse_buttons_prev: HashSet<MouseButton>,
    mouse_x: f64,
    mouse_y: f64,
    mouse_delta_x: f64,
    mouse_delta_y: f64,
    scroll_delta_x: f64,
    scroll_delta_y: f64,
    // Gamepad state
    gamepad_buttons: HashMap<GamepadId, HashSet<GamepadButton>>,
    gamepad_buttons_prev: HashMap<GamepadId, HashSet<GamepadButton>>,
    gamepad_axes: HashMap<GamepadId, HashMap<GamepadAxis, f32>>,
    connected_gamepads: HashSet<GamepadId>,
    // Global dead zones (applied to raw gamepad axis queries).
    global_dead_zones: [f32; GamepadAxis::COUNT],
    // Input action mapping
    action_map: Option<InputActionMap>,
    action_state: InputActionState,
}

impl Input {
    pub(crate) fn new() -> Self {
        Self {
            keys_pressed: HashSet::new(),
            keys_prev: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_prev: HashSet::new(),
            mouse_x: 0.0,
            mouse_y: 0.0,
            mouse_delta_x: 0.0,
            mouse_delta_y: 0.0,
            scroll_delta_x: 0.0,
            scroll_delta_y: 0.0,
            gamepad_buttons: HashMap::new(),
            gamepad_buttons_prev: HashMap::new(),
            gamepad_axes: HashMap::new(),
            connected_gamepads: HashSet::new(),
            global_dead_zones: DEFAULT_DEAD_ZONES,
            action_map: None,
            action_state: InputActionState::default(),
        }
    }

    // -- Public query API -----------------------------------------------------

    /// Returns `true` while the key is held down (every frame).
    pub fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// Returns `true` only on the first frame the key is pressed.
    pub fn is_key_just_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key) && !self.keys_prev.contains(&key)
    }

    /// Returns `true` while the mouse button is held down (every frame).
    pub fn is_mouse_button_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button)
    }

    /// Returns `true` only on the first frame the key is released.
    pub fn is_key_just_released(&self, key: KeyCode) -> bool {
        !self.keys_pressed.contains(&key) && self.keys_prev.contains(&key)
    }

    /// Returns `true` only on the first frame the mouse button is pressed.
    pub fn is_mouse_button_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button) && !self.mouse_buttons_prev.contains(&button)
    }

    /// Returns `true` only on the first frame the mouse button is released.
    pub fn is_mouse_button_just_released(&self, button: MouseButton) -> bool {
        !self.mouse_buttons_pressed.contains(&button) && self.mouse_buttons_prev.contains(&button)
    }

    pub fn mouse_position(&self) -> (f64, f64) {
        (self.mouse_x, self.mouse_y)
    }

    pub fn mouse_x(&self) -> f64 {
        self.mouse_x
    }

    pub fn mouse_y(&self) -> f64 {
        self.mouse_y
    }

    /// Raw mouse motion delta accumulated this frame (sub-pixel precision).
    /// Reset to (0, 0) each frame.
    pub fn mouse_delta(&self) -> (f64, f64) {
        (self.mouse_delta_x, self.mouse_delta_y)
    }

    /// Scroll wheel delta accumulated this frame.
    /// Positive Y = scroll up, negative Y = scroll down.
    /// Reset to (0, 0) each frame.
    pub fn scroll_delta(&self) -> (f64, f64) {
        (self.scroll_delta_x, self.scroll_delta_y)
    }

    // -- Gamepad query API ----------------------------------------------------

    /// Returns `true` while the gamepad button is held down.
    pub fn is_gamepad_button_pressed(&self, gamepad: GamepadId, button: GamepadButton) -> bool {
        self.gamepad_buttons
            .get(&gamepad)
            .is_some_and(|b| b.contains(&button))
    }

    /// Returns `true` only on the first frame the gamepad button is pressed.
    pub fn is_gamepad_button_just_pressed(
        &self,
        gamepad: GamepadId,
        button: GamepadButton,
    ) -> bool {
        let pressed = self
            .gamepad_buttons
            .get(&gamepad)
            .is_some_and(|b| b.contains(&button));
        let was_pressed = self
            .gamepad_buttons_prev
            .get(&gamepad)
            .is_some_and(|b| b.contains(&button));
        pressed && !was_pressed
    }

    /// Returns `true` only on the first frame the gamepad button is released.
    pub fn is_gamepad_button_just_released(
        &self,
        gamepad: GamepadId,
        button: GamepadButton,
    ) -> bool {
        let pressed = self
            .gamepad_buttons
            .get(&gamepad)
            .is_some_and(|b| b.contains(&button));
        let was_pressed = self
            .gamepad_buttons_prev
            .get(&gamepad)
            .is_some_and(|b| b.contains(&button));
        !pressed && was_pressed
    }

    /// Get the current value of a gamepad axis with global dead zone applied.
    /// Returns 0.0 if not connected or no data.
    pub fn gamepad_axis(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32 {
        let raw = self.gamepad_axis_raw(gamepad, axis);
        input_action::apply_dead_zone(raw, self.global_dead_zones[axis.index()])
    }

    /// Get the raw (unfiltered) axis value. Used by the action system which
    /// applies its own per-binding dead zones.
    pub(crate) fn gamepad_axis_raw(&self, gamepad: GamepadId, axis: GamepadAxis) -> f32 {
        self.gamepad_axes
            .get(&gamepad)
            .and_then(|a| a.get(&axis))
            .copied()
            .unwrap_or(0.0)
    }

    /// Returns `true` if the given gamepad is connected.
    pub fn is_gamepad_connected(&self, gamepad: GamepadId) -> bool {
        self.connected_gamepads.contains(&gamepad)
    }

    /// Returns all connected gamepad IDs.
    pub fn connected_gamepads(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.connected_gamepads.iter().copied()
    }

    // -- Input action query API -----------------------------------------------

    /// Returns `true` while the named action is active (any binding pressed).
    pub fn is_action_pressed(&self, name: &str) -> bool {
        self.action_state.is_action_pressed(name)
    }

    /// Returns `true` only on the first frame the action becomes active.
    pub fn is_action_just_pressed(&self, name: &str) -> bool {
        self.action_state.is_action_just_pressed(name)
    }

    /// Returns `true` only on the first frame the action becomes inactive.
    pub fn is_action_just_released(&self, name: &str) -> bool {
        self.action_state.is_action_just_released(name)
    }

    /// Returns the continuous value of an axis action (-1.0..1.0).
    /// Returns 0.0 for button actions that are not pressed, 1.0 when pressed.
    pub fn action_value(&self, name: &str) -> f32 {
        self.action_state.action_value(name)
    }

    // -- Mutation (engine-internal) -------------------------------------------

    pub(crate) fn press_key(&mut self, key: KeyCode) {
        self.keys_pressed.insert(key);
    }

    pub(crate) fn release_key(&mut self, key: KeyCode) {
        self.keys_pressed.remove(&key);
    }

    pub(crate) fn press_mouse_button(&mut self, button: MouseButton) {
        self.mouse_buttons_pressed.insert(button);
    }

    pub(crate) fn release_mouse_button(&mut self, button: MouseButton) {
        self.mouse_buttons_pressed.remove(&button);
    }

    pub(crate) fn set_mouse_position(&mut self, x: f64, y: f64) {
        self.mouse_x = x;
        self.mouse_y = y;
    }

    pub(crate) fn accumulate_mouse_delta(&mut self, dx: f64, dy: f64) {
        self.mouse_delta_x += dx;
        self.mouse_delta_y += dy;
    }

    pub(crate) fn accumulate_scroll_delta(&mut self, dx: f64, dy: f64) {
        self.scroll_delta_x += dx;
        self.scroll_delta_y += dy;
    }

    #[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
    pub(crate) fn gamepad_connect(&mut self, gamepad: GamepadId) {
        self.connected_gamepads.insert(gamepad);
    }

    #[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
    pub(crate) fn gamepad_disconnect(&mut self, gamepad: GamepadId) {
        self.connected_gamepads.remove(&gamepad);
        self.gamepad_buttons.remove(&gamepad);
        self.gamepad_buttons_prev.remove(&gamepad);
        self.gamepad_axes.remove(&gamepad);
    }

    #[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
    pub(crate) fn press_gamepad_button(&mut self, gamepad: GamepadId, button: GamepadButton) {
        self.gamepad_buttons
            .entry(gamepad)
            .or_default()
            .insert(button);
    }

    #[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
    pub(crate) fn release_gamepad_button(&mut self, gamepad: GamepadId, button: GamepadButton) {
        if let Some(buttons) = self.gamepad_buttons.get_mut(&gamepad) {
            buttons.remove(&button);
        }
    }

    #[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
    pub(crate) fn set_gamepad_axis(&mut self, gamepad: GamepadId, axis: GamepadAxis, value: f32) {
        self.gamepad_axes
            .entry(gamepad)
            .or_default()
            .insert(axis, value);
    }

    // -- Dead zone configuration -----------------------------------------------

    /// Set the global dead zone for a specific axis.
    pub fn set_global_dead_zone(&mut self, axis: GamepadAxis, value: f32) {
        self.global_dead_zones[axis.index()] = value.clamp(0.0, 0.99);
    }

    /// Get the global dead zone for a specific axis.
    pub fn get_global_dead_zone(&self, axis: GamepadAxis) -> f32 {
        self.global_dead_zones[axis.index()]
    }

    /// Set all global dead zones at once.
    pub(crate) fn set_global_dead_zones(&mut self, dead_zones: [f32; GamepadAxis::COUNT]) {
        self.global_dead_zones = dead_zones;
    }

    /// Get a copy of all global dead zones.
    pub fn global_dead_zones(&self) -> [f32; GamepadAxis::COUNT] {
        self.global_dead_zones
    }

    // -- Input action mapping ---------------------------------------------------

    /// Set the input action map. Called once after project load.
    pub(crate) fn set_action_map(&mut self, map: InputActionMap) {
        self.action_map = Some(map);
    }

    /// Returns `true` if an action map has been set.
    pub(crate) fn has_action_map(&self) -> bool {
        self.action_map.is_some()
    }

    /// Returns the number of actions in the current action map, or 0 if none.
    pub(crate) fn action_count(&self) -> usize {
        self.action_map.as_ref().map_or(0, |m| m.actions.len())
    }

    /// Evaluate all action bindings against current raw input.
    /// Called each frame before `on_update`.
    pub(crate) fn update_actions(&mut self) {
        if let Some(ref map) = self.action_map {
            // Temporarily move action_state out to avoid split borrow:
            // update() needs &Input for raw queries while mutating state.
            let map = map.clone();
            let mut action_state = std::mem::take(&mut self.action_state);
            action_state.update(&map, self);
            self.action_state = action_state;
        }
    }

    /// Clear all pressed state (call on window focus loss to avoid stuck keys).
    /// Clears both current and previous frame sets to prevent spurious
    /// "just released" events on the next frame.
    pub(crate) fn clear_all(&mut self) {
        self.keys_pressed.clear();
        self.keys_prev.clear();
        self.mouse_buttons_pressed.clear();
        self.mouse_buttons_prev.clear();
        self.action_state.clear();
    }

    /// Snapshot current state so next frame can detect transitions.
    /// Call at the end of each frame, after all updates and rendering.
    pub(crate) fn end_frame(&mut self) {
        self.keys_prev.clone_from(&self.keys_pressed);
        self.mouse_buttons_prev
            .clone_from(&self.mouse_buttons_pressed);
        self.mouse_delta_x = 0.0;
        self.mouse_delta_y = 0.0;
        self.scroll_delta_x = 0.0;
        self.scroll_delta_y = 0.0;
        // Snapshot gamepad buttons for just-pressed/just-released detection.
        self.gamepad_buttons_prev.clone_from(&self.gamepad_buttons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_press_and_release() {
        let mut input = Input::new();
        assert!(!input.is_key_pressed(KeyCode::A));

        input.press_key(KeyCode::A);
        assert!(input.is_key_pressed(KeyCode::A));

        input.release_key(KeyCode::A);
        assert!(!input.is_key_pressed(KeyCode::A));
    }

    #[test]
    fn mouse_button_press_and_release() {
        let mut input = Input::new();
        assert!(!input.is_mouse_button_pressed(MouseButton::Left));

        input.press_mouse_button(MouseButton::Left);
        assert!(input.is_mouse_button_pressed(MouseButton::Left));

        input.release_mouse_button(MouseButton::Left);
        assert!(!input.is_mouse_button_pressed(MouseButton::Left));
    }

    #[test]
    fn mouse_position_tracking() {
        let mut input = Input::new();
        assert_eq!(input.mouse_position(), (0.0, 0.0));

        input.set_mouse_position(100.5, 200.3);
        assert_eq!(input.mouse_x(), 100.5);
        assert_eq!(input.mouse_y(), 200.3);
        assert_eq!(input.mouse_position(), (100.5, 200.3));
    }

    #[test]
    fn key_just_pressed_fires_once() {
        let mut input = Input::new();

        // Frame 1: press A.
        input.press_key(KeyCode::A);
        assert!(input.is_key_just_pressed(KeyCode::A)); // first frame → true
        input.end_frame();

        // Frame 2: A still held.
        assert!(input.is_key_pressed(KeyCode::A)); // held → true
        assert!(!input.is_key_just_pressed(KeyCode::A)); // not first frame → false
        input.end_frame();

        // Frame 3: release A, then press again.
        input.release_key(KeyCode::A);
        input.end_frame();
        input.press_key(KeyCode::A);
        assert!(input.is_key_just_pressed(KeyCode::A)); // re-press → true
    }

    #[test]
    fn mouse_button_just_pressed_fires_once() {
        let mut input = Input::new();

        input.press_mouse_button(MouseButton::Left);
        assert!(input.is_mouse_button_just_pressed(MouseButton::Left));
        input.end_frame();

        assert!(!input.is_mouse_button_just_pressed(MouseButton::Left));
        assert!(input.is_mouse_button_pressed(MouseButton::Left));
    }

    #[test]
    fn clear_all_prevents_spurious_just_released() {
        let mut input = Input::new();

        // Frame 1: press A.
        input.press_key(KeyCode::A);
        input.press_mouse_button(MouseButton::Left);
        input.end_frame();

        // Simulate focus loss — should clear both current AND previous.
        input.clear_all();

        // Next frame: A should NOT be "just released" (both sets empty).
        assert!(!input.is_key_just_released(KeyCode::A));
        assert!(!input.is_key_pressed(KeyCode::A));
        assert!(!input.is_mouse_button_just_pressed(MouseButton::Left));
        assert!(!input.is_mouse_button_pressed(MouseButton::Left));
    }

    #[test]
    fn gamepad_axis_dead_zone_applied() {
        let mut input = Input::new();
        input.gamepad_connect(0);

        // Default dead zone for left stick is 0.15.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.1);
        assert!(input.gamepad_axis(0, GamepadAxis::LeftStickX).abs() < 0.001);

        // Raw should return unfiltered value.
        assert!((input.gamepad_axis_raw(0, GamepadAxis::LeftStickX) - 0.1).abs() < 0.001);

        // Above dead zone should return a remapped value.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.5);
        let filtered = input.gamepad_axis(0, GamepadAxis::LeftStickX);
        assert!(filtered > 0.0 && filtered < 0.5);

        // Full deflection → ~1.0.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 1.0);
        assert!((input.gamepad_axis(0, GamepadAxis::LeftStickX) - 1.0).abs() < 0.001);

        // Triggers default to 0.0 dead zone — pass through unchanged.
        input.set_gamepad_axis(0, GamepadAxis::LeftTrigger, 0.05);
        assert!((input.gamepad_axis(0, GamepadAxis::LeftTrigger) - 0.05).abs() < 0.001);
    }

    #[test]
    fn set_global_dead_zone() {
        let mut input = Input::new();
        input.gamepad_connect(0);

        // Increase dead zone.
        input.set_global_dead_zone(GamepadAxis::LeftStickX, 0.3);
        assert!((input.get_global_dead_zone(GamepadAxis::LeftStickX) - 0.3).abs() < 0.001);

        // Value within new dead zone should be 0.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.25);
        assert!(input.gamepad_axis(0, GamepadAxis::LeftStickX).abs() < 0.001);

        // Above new dead zone should be non-zero.
        input.set_gamepad_axis(0, GamepadAxis::LeftStickX, 0.5);
        assert!(input.gamepad_axis(0, GamepadAxis::LeftStickX) > 0.0);
    }

    #[test]
    fn multiple_keys_tracked_independently() {
        let mut input = Input::new();
        input.press_key(KeyCode::A);
        input.press_key(KeyCode::LeftShift);

        assert!(input.is_key_pressed(KeyCode::A));
        assert!(input.is_key_pressed(KeyCode::LeftShift));
        assert!(!input.is_key_pressed(KeyCode::B));

        input.release_key(KeyCode::A);
        assert!(!input.is_key_pressed(KeyCode::A));
        assert!(input.is_key_pressed(KeyCode::LeftShift));
    }
}
