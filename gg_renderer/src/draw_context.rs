use ash::vk;

/// Per-frame draw context passed through the renderer abstraction layers.
///
/// Holds the active command buffer and the current render area extent.
/// `Copy` because these are lightweight Vulkan handles / value types.
#[derive(Clone, Copy)]
pub struct DrawContext {
    pub cmd_buf: vk::CommandBuffer,
    pub extent: vk::Extent2D,
    pub current_frame: usize,
    /// Which viewport is being rendered (0..MAX_VIEWPORTS).
    /// Used to select the correct camera UBO slot so multiple viewports
    /// don't overwrite each other's VP data within the same frame.
    pub viewport_index: usize,
}
