use ash::vk;

/// Per-frame draw context passed through the renderer abstraction layers.
///
/// Holds the active command buffer and the current render area extent.
/// `Copy` because these are lightweight Vulkan handles / value types.
#[derive(Clone, Copy)]
pub(crate) struct DrawContext {
    pub cmd_buf: vk::CommandBuffer,
    pub extent: vk::Extent2D,
}
