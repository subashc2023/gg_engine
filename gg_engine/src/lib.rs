mod application;
pub mod events;
mod input;
mod layer;
mod logging;
pub mod renderer;

pub use application::{run, Application, WindowConfig};
pub use egui;
pub use renderer::PresentMode;
pub use input::Input;
pub use layer::{Layer, LayerStack};
pub use glam;
pub use log;
pub use logging::init as log_init;

pub fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Convenience re-exports for client applications.
pub mod prelude {
    pub use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
    pub use crate::input::Input;
    pub use crate::layer::{Layer, LayerStack};
    pub use crate::renderer::PresentMode;
    pub use crate::{run, Application, WindowConfig};
    pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
    pub use log::{debug, error, info, trace, warn};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_exists() {
        assert!(!engine_version().is_empty());
    }
}
