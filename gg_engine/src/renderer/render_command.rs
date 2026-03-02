use ash::vk;
use glam::{Mat4, Vec4};

use super::draw_context::DrawContext;
use super::renderer_api::RendererAPI;
use super::vertex_array::VertexArray;

/// Static dispatch layer — associated functions that forward to `RendererAPI`.
///
/// This exists so that higher-level code (`Renderer`) doesn't match on the
/// enum directly, keeping the abstraction boundary clean.
pub(crate) struct RenderCommand;

#[allow(clippy::too_many_arguments)]
impl RenderCommand {
    pub fn set_clear_color(api: &mut RendererAPI, color: [f32; 4]) {
        api.set_clear_color(color);
    }

    pub fn clear_color(api: &RendererAPI) -> [f32; 4] {
        api.clear_color()
    }

    pub fn set_viewport(api: &RendererAPI, ctx: &DrawContext) {
        api.set_viewport(ctx);
    }

    pub fn draw_indexed(
        api: &RendererAPI,
        ctx: &DrawContext,
        pipeline: vk::Pipeline,
        pipeline_layout: vk::PipelineLayout,
        vertex_array: &VertexArray,
        vp_matrix: &Mat4,
        transform: &Mat4,
        color: Option<&Vec4>,
    ) {
        api.draw_indexed(ctx, pipeline, pipeline_layout, vertex_array, vp_matrix, transform, color);
    }
}
