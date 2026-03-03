use ash::vk;
use glam::{Mat4, Vec4};

use super::draw_context::DrawContext;
use super::vertex_array::VertexArray;

const MAT4_SIZE: u32 = std::mem::size_of::<Mat4>() as u32;

// ---------------------------------------------------------------------------
// VulkanRendererAPI
// ---------------------------------------------------------------------------

pub(crate) struct VulkanRendererAPI {
    device: ash::Device,
    clear_color: [f32; 4],
}

#[allow(clippy::too_many_arguments)]
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
        camera_ubo_ds: vk::DescriptorSet,
        transform: &Mat4,
        color: Option<&Vec4>,
        descriptor_set: Option<vk::DescriptorSet>,
    ) {
        unsafe {
            self.device
                .cmd_bind_pipeline(ctx.cmd_buf, vk::PipelineBindPoint::GRAPHICS, pipeline);

            // Bind camera UBO descriptor set at set 0.
            self.device.cmd_bind_descriptor_sets(
                ctx.cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline_layout,
                0,
                &[camera_ubo_ds],
                &[],
            );

            // Push the model/transform matrix (offset 0, 64 bytes).
            let transform_bytes = std::slice::from_raw_parts(
                transform as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            );
            self.device.cmd_push_constants(
                ctx.cmd_buf,
                pipeline_layout,
                vk::ShaderStageFlags::VERTEX,
                0,
                transform_bytes,
            );

            // Push material color (offset 64, 16 bytes, fragment stage).
            if let Some(c) = color {
                let color_bytes = std::slice::from_raw_parts(
                    c as *const Vec4 as *const u8,
                    std::mem::size_of::<Vec4>(),
                );
                self.device.cmd_push_constants(
                    ctx.cmd_buf,
                    pipeline_layout,
                    vk::ShaderStageFlags::FRAGMENT,
                    MAT4_SIZE,
                    color_bytes,
                );
            }

            // Bind texture descriptor set at set 1 (after camera UBO at set 0).
            if let Some(ds) = descriptor_set {
                self.device.cmd_bind_descriptor_sets(
                    ctx.cmd_buf,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline_layout,
                    1,
                    &[ds],
                    &[],
                );
            }
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

#[allow(clippy::too_many_arguments)]
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
        camera_ubo_ds: vk::DescriptorSet,
        transform: &Mat4,
        color: Option<&Vec4>,
        descriptor_set: Option<vk::DescriptorSet>,
    ) {
        match self {
            Self::Vulkan(api) => api.draw_indexed(
                ctx,
                pipeline,
                pipeline_layout,
                vertex_array,
                camera_ubo_ds,
                transform,
                color,
                descriptor_set,
            ),
        }
    }
}
