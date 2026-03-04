use std::collections::HashSet;

use crate::events::{KeyCode, MouseButton};

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

    /// Returns `true` only on the first frame the mouse button is pressed.
    pub fn is_mouse_button_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button) && !self.mouse_buttons_prev.contains(&button)
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

    /// Snapshot current state so next frame can detect transitions.
    /// Call at the end of each frame, after all updates and rendering.
    pub(crate) fn end_frame(&mut self) {
        self.keys_prev.clone_from(&self.keys_pressed);
        self.mouse_buttons_prev.clone_from(&self.mouse_buttons_pressed);
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
        assert!(input.is_key_pressed(KeyCode::A));       // held → true
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
