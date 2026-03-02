use ash::vk;
use glam::Mat4;

use super::draw_context::DrawContext;
use super::vertex_array::VertexArray;

// ---------------------------------------------------------------------------
// VulkanRendererAPI
// ---------------------------------------------------------------------------

pub(crate) struct VulkanRendererAPI {
    device: ash::Device,
    clear_color: [f32; 4],
}

impl VulkanRendererAPI {
    pub fn new(device: &ash::Device) -> Self {
        Self {
            device: device.clone(),
            clear_color: [0.01, 0.01, 0.01, 1.0],
        }
    }

    pub fn set_clear_color(&mut self, color: [f32; 4]) {
        self.clear_color = color;
    }

    pub fn clear_color(&self) -> [f32; 4] {
        self.clear_color
    }

    pub fn set_viewport(&self, ctx: &DrawContext) {
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: ctx.extent.width as f32,
            height: ctx.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: ctx.extent,
        };
        unsafe {
            self.device.cmd_set_viewport(ctx.cmd_buf, 0, &[viewport]);
            self.device.cmd_set_scissor(ctx.cmd_buf, 0, &[scissor]);
        }
    }

    pub fn draw_indexed(
        &self,
        ctx: &DrawContext,
        pipeline: vk::Pipeline,
        pipeline_layout: vk::PipelineLayout,
        vertex_array: &VertexArray,
        vp_matrix: &Mat4,
    ) {
        unsafe {
            self.device.cmd_bind_pipeline(
                ctx.cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline,
            );

            // Push the view-projection matrix as a push constant.
            let matrix_bytes = std::slice::from_raw_parts(
                vp_matrix as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            );
            self.device.cmd_push_constants(
                ctx.cmd_buf,
                pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                matrix_bytes,
            );
        }
        vertex_array.bind(ctx.cmd_buf);
        let index_count = vertex_array
            .index_buffer()
            .expect("VertexArray has no index buffer")
            .count();
        unsafe {
            self.device
                .cmd_draw_indexed(ctx.cmd_buf, index_count, 1, 0, 0, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// RendererAPI (enum dispatch)
// ---------------------------------------------------------------------------

/// Rendering API backend. Enum dispatch — zero-cost for a single variant,
/// trivially extensible when a second backend is added.
pub(crate) enum RendererAPI {
    Vulkan(VulkanRendererAPI),
}

impl RendererAPI {
    pub fn set_clear_color(&mut self, color: [f32; 4]) {
        match self {
            Self::Vulkan(api) => api.set_clear_color(color),
        }
    }

    pub fn clear_color(&self) -> [f32; 4] {
        match self {
            Self::Vulkan(api) => api.clear_color(),
        }
    }

    pub fn set_viewport(&self, ctx: &DrawContext) {
        match self {
            Self::Vulkan(api) => api.set_viewport(ctx),
        }
    }

    pub fn draw_indexed(
        &self,
        ctx: &DrawContext,
        pipeline: vk::Pipeline,
        pipeline_layout: vk::PipelineLayout,
        vertex_array: &VertexArray,
        vp_matrix: &Mat4,
    ) {
        match self {
            Self::Vulkan(api) => {
                api.draw_indexed(ctx, pipeline, pipeline_layout, vertex_array, vp_matrix)
            }
        }
    }
}
