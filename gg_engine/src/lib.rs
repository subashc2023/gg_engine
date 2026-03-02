mod application;
pub mod events;
mod input;
mod layer;
mod logging;
pub mod renderer;
mod timestep;

/// Shared-ownership smart pointer for rendering resources.
/// Wraps `Arc<T>` for thread-safe reference counting.
pub type Ref<T> = std::sync::Arc<T>;

/// Owning smart pointer (heap-allocated, single owner).
pub type Scope<T> = Box<T>;

pub use application::{run, Application, WindowConfig};
pub use egui;
pub use renderer::{
    as_bytes, BufferElement, BufferLayout, IndexBuffer, OrthographicCamera, Pipeline, PresentMode,
    Renderer, RendererBackend, Shader, ShaderDataType, VertexArray, VertexBuffer,
};
pub use input::Input;
pub use layer::{Layer, LayerStack};
pub use timestep::Timestep;
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
    pub use crate::renderer::{
        as_bytes, BufferElement, BufferLayout, IndexBuffer, OrthographicCamera, Pipeline,
        PresentMode, Renderer, RendererBackend, Shader, ShaderDataType, VertexArray, VertexBuffer,
    };
    pub use crate::timestep::Timestep;
    pub use crate::{run, Application, Ref, Scope, WindowConfig};
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
