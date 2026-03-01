mod buffer;
mod shader;
mod swapchain;
mod triangle;
mod vulkan_context;

pub(crate) use buffer::{IndexBuffer, VertexBuffer};
pub(crate) use shader::Shader;
pub use swapchain::{Swapchain, SwapchainError};
pub(crate) use triangle::TriangleRenderer;
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
