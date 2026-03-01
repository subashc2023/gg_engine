use std::collections::HashSet;

use crate::events::{KeyCode, MouseButton};

/// Tracks the current state of keyboard and mouse input.
///
/// Updated each frame by the engine before layer/application callbacks.
/// Query methods let any code with an `&Input` reference poll the current
/// state without needing to track events manually.
pub struct Input {
    keys_pressed: HashSet<KeyCode>,
    mouse_buttons_pressed: HashSet<MouseButton>,
    mouse_x: f64,
    mouse_y: f64,
}

impl Input {
    pub(crate) fn new() -> Self {
        Self {
            keys_pressed: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_x: 0.0,
            mouse_y: 0.0,
        }
    }

    // -- Public query API -----------------------------------------------------

    pub fn is_key_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    pub fn is_mouse_button_pressed(&self, button: MouseButton) -> bool {
        self.mouse_buttons_pressed.contains(&button)
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
