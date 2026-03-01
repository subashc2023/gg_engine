mod swapchain;
mod triangle;
mod vulkan_context;

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
