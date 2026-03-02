mod buffer;
mod draw_context;
mod orthographic_camera;
mod pipeline;
mod render_command;
#[allow(clippy::module_inception)]
mod renderer;
mod renderer_api;
mod shader;
mod shader_library;
pub mod shaders;
mod swapchain;
mod texture;
mod vertex_array;
mod vulkan_context;

pub use buffer::{
    as_bytes, BufferElement, BufferLayout, IndexBuffer, ShaderDataType, VertexBuffer,
};
pub(crate) use draw_context::DrawContext;
pub use orthographic_camera::OrthographicCamera;
pub use pipeline::Pipeline;
pub use renderer::Renderer;
pub use shader::Shader;
pub use shader_library::ShaderLibrary;
pub use swapchain::{Swapchain, SwapchainError};
pub use texture::Texture2D;
pub use vertex_array::VertexArray;
pub use vulkan_context::{VulkanContext, VulkanInitError};

// ---------------------------------------------------------------------------
// PresentMode
// ---------------------------------------------------------------------------

/// Desired presentation mode for the swapchain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PresentMode {
    /// VSync — guaranteed available on all drivers.
    #[default]
    Fifo,
    /// Triple-buffered, no vsync. Falls back to Immediate, then Fifo.
    Mailbox,
    /// Immediate (tearing allowed). Falls back to Mailbox, then Fifo.
    Immediate,
}

impl std::fmt::Display for PresentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fifo => write!(f, "Fifo (VSync)"),
            Self::Mailbox => write!(f, "Mailbox (Triple-buffered)"),
            Self::Immediate => write!(f, "Immediate (No VSync)"),
        }
    }
}

// ---------------------------------------------------------------------------
// RendererBackend
// ---------------------------------------------------------------------------

/// Which rendering API the engine is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererBackend {
    None,
    Vulkan,
}

impl RendererBackend {
    pub fn current() -> Self {
        RendererBackend::Vulkan
    }
}

impl std::fmt::Display for RendererBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Vulkan => write!(f, "Vulkan"),
        }
    }
}
