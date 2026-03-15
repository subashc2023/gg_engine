pub mod gamepad;
mod key;
mod mouse;
mod window;

pub use gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId};
pub use key::{KeyCode, KeyEvent};
pub use mouse::{MouseButton, MouseEvent};
pub use window::WindowEvent;

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Window(WindowEvent),
    Key(KeyEvent),
    Mouse(MouseEvent),
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Window(e) => write!(f, "{e}"),
            Event::Key(e) => write!(f, "{e}"),
            Event::Mouse(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_window_close() {
        let event = Event::Window(WindowEvent::Close);
        assert_eq!(event.to_string(), "WindowClose");
    }

    #[test]
    fn display_window_resize() {
        let event = Event::Window(WindowEvent::Resize {
            width: 1920,
            height: 1080,
        });
        assert_eq!(event.to_string(), "WindowResize(1920, 1080)");
    }

    #[test]
    fn display_key_pressed() {
        let event = Event::Key(KeyEvent::Pressed {
            key_code: KeyCode::Space,
            repeat: false,
        });
        assert_eq!(event.to_string(), "KeyPressed(Space, repeat=false)");
    }

    #[test]
    fn display_key_released() {
        let event = Event::Key(KeyEvent::Released {
            key_code: KeyCode::Escape,
        });
        assert_eq!(event.to_string(), "KeyReleased(Escape)");
    }

    #[test]
    fn display_mouse_moved() {
        let event = Event::Mouse(MouseEvent::Moved { x: 100.0, y: 200.0 });
        assert_eq!(event.to_string(), "MouseMoved(100, 200)");
    }

    #[test]
    fn display_mouse_scrolled() {
        let event = Event::Mouse(MouseEvent::Scrolled {
            x_offset: 0.0,
            y_offset: 1.5,
        });
        assert_eq!(event.to_string(), "MouseScrolled(0, 1.5)");
    }

    #[test]
    fn display_mouse_button_pressed() {
        let event = Event::Mouse(MouseEvent::ButtonPressed(MouseButton::Left));
        assert_eq!(event.to_string(), "MouseButtonPressed(Left)");
    }

    #[test]
    fn display_mouse_button_released() {
        let event = Event::Mouse(MouseEvent::ButtonReleased(MouseButton::Right));
        assert_eq!(event.to_string(), "MouseButtonReleased(Right)");
    }

    #[test]
    fn display_key_typed() {
        let event = Event::Key(KeyEvent::Typed('a'));
        assert_eq!(event.to_string(), "KeyTyped(a)");
    }

    #[test]
    fn event_is_copy() {
        let event = Event::Window(WindowEvent::Close);
        let copy = event;
        assert_eq!(event, copy);
    }
}
