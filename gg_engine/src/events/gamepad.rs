use std::fmt;

/// Identifies a connected gamepad by index.
pub type GamepadId = usize;

/// Standard gamepad button mapping (follows Xbox/PlayStation conventions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadButton {
    /// A / Cross
    South,
    /// B / Circle
    East,
    /// X / Square
    West,
    /// Y / Triangle
    North,
    /// Left bumper / L1
    LeftBumper,
    /// Right bumper / R1
    RightBumper,
    /// Left trigger (digital threshold) / L2
    LeftTrigger,
    /// Right trigger (digital threshold) / R2
    RightTrigger,
    /// Back / Select / Share
    Select,
    /// Start / Options
    Start,
    /// Guide / Home / PS
    Guide,
    /// Left stick press / L3
    LeftStick,
    /// Right stick press / R3
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// Standard gamepad axis (analog input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GamepadAxis {
    /// Left stick horizontal (-1.0 = left, 1.0 = right)
    LeftStickX,
    /// Left stick vertical (-1.0 = down, 1.0 = up)
    LeftStickY,
    /// Right stick horizontal (-1.0 = left, 1.0 = right)
    RightStickX,
    /// Right stick vertical (-1.0 = down, 1.0 = up)
    RightStickY,
    /// Left trigger analog (0.0 = released, 1.0 = fully pressed)
    LeftTrigger,
    /// Right trigger analog (0.0 = released, 1.0 = fully pressed)
    RightTrigger,
}

/// Gamepad input events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GamepadEvent {
    /// A gamepad was connected.
    Connected(GamepadId),
    /// A gamepad was disconnected.
    Disconnected(GamepadId),
    /// A button was pressed.
    ButtonPressed {
        gamepad: GamepadId,
        button: GamepadButton,
    },
    /// A button was released.
    ButtonReleased {
        gamepad: GamepadId,
        button: GamepadButton,
    },
    /// An axis value changed.
    AxisChanged {
        gamepad: GamepadId,
        axis: GamepadAxis,
        value: f32,
    },
}

impl fmt::Display for GamepadButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for GamepadAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for GamepadEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GamepadEvent::Connected(id) => write!(f, "GamepadConnected({id})"),
            GamepadEvent::Disconnected(id) => write!(f, "GamepadDisconnected({id})"),
            GamepadEvent::ButtonPressed { gamepad, button } => {
                write!(f, "GamepadButtonPressed({gamepad}, {button})")
            }
            GamepadEvent::ButtonReleased { gamepad, button } => {
                write!(f, "GamepadButtonReleased({gamepad}, {button})")
            }
            GamepadEvent::AxisChanged {
                gamepad,
                axis,
                value,
            } => write!(f, "GamepadAxisChanged({gamepad}, {axis}, {value:.2})"),
        }
    }
}
