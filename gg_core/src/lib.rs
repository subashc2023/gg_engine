pub mod error;
pub mod events;
pub mod input;
pub mod input_action;
pub mod logging;
pub mod platform_utils;
pub mod profiling;
pub mod timestep;
pub mod uuid;

/// Shared-ownership smart pointer for rendering resources.
/// Wraps `Arc<T>` for thread-safe reference counting.
pub type Ref<T> = std::sync::Arc<T>;

/// Owning smart pointer (heap-allocated, single owner).
pub type Scope<T> = Box<T>;

pub use error::{EngineError, EngineResult};
pub use events::gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId};
pub use events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
pub use input::Input;
pub use input_action::{ActionType, InputAction, InputActionMap, InputBinding};
pub use logging::{clear_log_buffer, init as log_init, with_log_buffer, LogEntry};
pub use platform_utils::{error_dialog, FileDialogs};
pub use profiling::{
    begin_session, drain_profile_results, end_session, is_session_active, ProfileResult,
    ProfileTimer,
};
pub use timestep::Timestep;
pub use uuid::Uuid;
