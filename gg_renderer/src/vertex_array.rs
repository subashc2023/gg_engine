use ash::vk;

use super::buffer::{IndexBuffer, VertexBuffer};

// ---------------------------------------------------------------------------
// VertexArray
// ---------------------------------------------------------------------------

/// Groups vertex buffers and an index buffer into a single bindable unit.
///
/// Created via [`Renderer::create_vertex_array`](super::Renderer::create_vertex_array).
/// In OpenGL this maps to a Vertex Array Object (VAO).
/// In Vulkan there is no native equivalent — this is a CPU-side abstraction
/// that owns the buffers, validates their layouts, and records bind commands.
pub struct VertexArray {
    vertex_buffers: Vec<VertexBuffer>,
    index_buffer: Option<IndexBuffer>,
    device: ash::Device,
}

impl VertexArray {
    pub fn new(device: &ash::Device) -> Self {
        Self {
            vertex_buffers: Vec::new(),
            index_buffer: None,
            device: device.clone(),
        }
    }

    /// Add a vertex buffer to this array.
    ///
    /// # Panics
    /// Panics if the vertex buffer has no layout set — a layout is required
    /// so that the pipeline knows how to interpret the buffer's data.
    pub fn add_vertex_buffer(&mut self, vb: VertexBuffer) {
        assert!(
            vb.layout().is_some_and(|l| !l.elements().is_empty()),
            "Vertex buffer has no layout"
        );
        self.vertex_buffers.push(vb);
    }

    pub fn set_index_buffer(&mut self, ib: IndexBuffer) {
        self.index_buffer = Some(ib);
    }

    pub fn vertex_buffers(&self) -> &[VertexBuffer] {
        &self.vertex_buffers
    }

    pub fn index_buffer(&self) -> Option<&IndexBuffer> {
        self.index_buffer.as_ref()
    }

    // -- Vulkan helpers for pipeline creation --------------------------------

    /// Generate all Vulkan binding descriptions from contained vertex buffers.
    /// Each vertex buffer gets its own binding index (0, 1, 2, ...).
    pub fn vk_binding_descriptions(&self) -> Vec<vk::VertexInputBindingDescription> {
        self.vertex_buffers
            .iter()
            .enumerate()
            .filter_map(|(i, vb)| vb.layout().map(|l| l.vk_binding_description(i as u32)))
            .collect()
    }

    /// Generate all Vulkan attribute descriptions from contained vertex buffers.
    /// Locations are assigned sequentially across all buffers.
    pub fn vk_attribute_descriptions(&self) -> Vec<vk::VertexInputAttributeDescription> {
        let mut attrs = Vec::new();
        let mut location = 0u32;
        for (binding, vb) in self.vertex_buffers.iter().enumerate() {
            if let Some(layout) = vb.layout() {
                for elem in layout.elements() {
                    attrs.push(vk::VertexInputAttributeDescription {
                        location,
                        binding: binding as u32,
                        format: elem.data_type.to_vk_format(),
                        offset: elem.offset,
                    });
                    location += 1;
                }
            }
        }
        attrs
    }

    // -- Command recording ---------------------------------------------------

    /// Bind all vertex buffers and the index buffer into a command buffer.
    pub fn bind(&self, cmd_buf: vk::CommandBuffer) {
        let buffers: Vec<vk::Buffer> = self.vertex_buffers.iter().map(|vb| vb.handle()).collect();
        let offsets: Vec<vk::DeviceSize> = vec![0; buffers.len()];

        if !buffers.is_empty() {
            unsafe {
                self.device
                    .cmd_bind_vertex_buffers(cmd_buf, 0, &buffers, &offsets);
            }
        }

        if let Some(ib) = &self.index_buffer {
            ib.bind(&self.device, cmd_buf);
        }
    }
}
