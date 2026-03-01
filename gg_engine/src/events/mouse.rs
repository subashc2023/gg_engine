use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl fmt::Display for MouseButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseEvent {
    Moved { x: f64, y: f64 },
    Scrolled { x_offset: f64, y_offset: f64 },
    ButtonPressed(MouseButton),
    ButtonReleased(MouseButton),
}

impl fmt::Display for MouseEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MouseEvent::Moved { x, y } => write!(f, "MouseMoved({x}, {y})"),
            MouseEvent::Scrolled { x_offset, y_offset } => {
                write!(f, "MouseScrolled({x_offset}, {y_offset})")
            }
            MouseEvent::ButtonPressed(button) => write!(f, "MouseButtonPressed({button})"),
            MouseEvent::ButtonReleased(button) => write!(f, "MouseButtonReleased({button})"),
        }
    }
}
