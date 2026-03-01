use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    // Alphabetic
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,

    // Digits
    Num0, Num1, Num2, Num3, Num4,
    Num5, Num6, Num7, Num8, Num9,

    // Function keys
    F1, F2, F3, F4, F5, F6,
    F7, F8, F9, F10, F11, F12,

    // Modifiers
    LeftShift, RightShift,
    LeftCtrl, RightCtrl,
    LeftAlt, RightAlt,

    // Navigation
    Up, Down, Left, Right,
    Home, End, PageUp, PageDown,

    // Common
    Space, Enter, Escape, Tab,
    Backspace, Delete, Insert,

    Unknown,
}

impl fmt::Display for KeyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Pressed { key_code: KeyCode, repeat: bool },
    Released { key_code: KeyCode },
    Typed(char),
}

impl fmt::Display for KeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyEvent::Pressed { key_code, repeat } => {
                write!(f, "KeyPressed({key_code}, repeat={repeat})")
            }
            KeyEvent::Released { key_code } => {
                write!(f, "KeyReleased({key_code})")
            }
            KeyEvent::Typed(c) => {
                write!(f, "KeyTyped({c})")
            }
        }
    }
}
