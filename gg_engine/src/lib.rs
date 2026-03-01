mod application;
pub mod events;
mod layer;
mod logging;
pub mod renderer;

pub use application::{run, Application, WindowConfig};
pub use layer::{Layer, LayerStack};
pub use log;
pub use logging::init as log_init;

pub fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Convenience re-exports for client applications.
pub mod prelude {
    pub use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
    pub use crate::layer::{Layer, LayerStack};
    pub use crate::{run, Application, WindowConfig};
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
