use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    Close,
    Resize { width: u32, height: u32 },
    /// The window gained or lost focus. `true` = gained, `false` = lost.
    Focused(bool),
}

impl fmt::Display for WindowEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowEvent::Close => write!(f, "WindowClose"),
            WindowEvent::Resize { width, height } => {
                write!(f, "WindowResize({width}, {height})")
            }
            WindowEvent::Focused(focused) => write!(f, "WindowFocused({focused})"),
        }
    }
}
