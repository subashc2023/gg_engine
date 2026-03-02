mod application;
pub mod events;
mod input;
mod layer;
mod logging;
mod orthographic_camera_controller;
pub mod profiling;
pub mod renderer;
mod timestep;

/// Shared-ownership smart pointer for rendering resources.
/// Wraps `Arc<T>` for thread-safe reference counting.
pub type Ref<T> = std::sync::Arc<T>;

/// Owning smart pointer (heap-allocated, single owner).
pub type Scope<T> = Box<T>;

pub use application::{run, Application, WindowConfig};
pub use egui;
pub use glam;
pub use input::Input;
pub use layer::{Layer, LayerStack};
pub use log;
pub use logging::init as log_init;
pub use orthographic_camera_controller::OrthographicCameraController;
pub use renderer::shaders;
pub use renderer::{
    as_bytes, BufferElement, BufferLayout, IndexBuffer, OrthographicCamera, Pipeline, PresentMode,
    Renderer, RendererBackend, Shader, ShaderDataType, ShaderLibrary, Texture2D, VertexArray,
    VertexBuffer,
};
pub use timestep::Timestep;

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
        PresentMode, Renderer, RendererBackend, Shader, ShaderDataType, ShaderLibrary, Texture2D,
        VertexArray, VertexBuffer,
    };
    pub use crate::timestep::Timestep;
    pub use crate::orthographic_camera_controller::OrthographicCameraController;
    pub use crate::profiling::{
        begin_session, drain_profile_results, end_session, ProfileResult, ProfileTimer,
    };
    pub use crate::{profile_scope, run, Application, Ref, Scope, WindowConfig};
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
