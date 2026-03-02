use ash::vk;
use glam::{Mat4, Vec4};

use super::buffer::{IndexBuffer, VertexBuffer};
use super::draw_context::DrawContext;
use super::orthographic_camera::OrthographicCamera;
use super::pipeline::{self, Pipeline};
use super::render_command::RenderCommand;
use super::renderer_api::{RendererAPI, VulkanRendererAPI};
use super::shader::Shader;
use super::vertex_array::VertexArray;
use super::VulkanContext;

/// High-level renderer. Owns the `RendererAPI` and the current frame's
/// `DrawContext`. Provides `begin_scene` / `end_scene` / `submit` for
/// structured draw call recording, and factory methods for creating
/// rendering resources.
pub struct Renderer {
    api: RendererAPI,
    draw_context: Option<DrawContext>,
    view_projection: Mat4,

    // Handles needed for resource creation.
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    render_pass: vk::RenderPass,
}

impl Renderer {
    pub(crate) fn new(vk_ctx: &VulkanContext, render_pass: vk::RenderPass) -> Self {
        let api = RendererAPI::Vulkan(VulkanRendererAPI::new(vk_ctx.device()));
        Self {
            api,
            draw_context: None,
            view_projection: Mat4::IDENTITY,
            instance: vk_ctx.instance().clone(),
            physical_device: vk_ctx.physical_device(),
            device: vk_ctx.device().clone(),
            render_pass,
        }
    }

    // -- Public resource creation API -----------------------------------------

    /// Create a shader from pre-compiled SPIR-V bytecode.
    pub fn create_shader(&self, name: &str, vert_spv: &[u8], frag_spv: &[u8]) -> Shader {
        Shader::new(&self.device, name, vert_spv, frag_spv)
    }

    /// Create a GPU vertex buffer from raw byte data.
    ///
    /// Use [`as_bytes`](super::as_bytes) to convert typed vertex slices.
    pub fn create_vertex_buffer(&self, data: &[u8]) -> VertexBuffer {
        VertexBuffer::new(&self.instance, self.physical_device, &self.device, data)
    }

    /// Create a GPU index buffer from u32 indices.
    pub fn create_index_buffer(&self, indices: &[u32]) -> IndexBuffer {
        IndexBuffer::new(&self.instance, self.physical_device, &self.device, indices)
    }

    /// Create an empty vertex array.
    pub fn create_vertex_array(&self) -> VertexArray {
        VertexArray::new(&self.device)
    }

    /// Create a graphics pipeline from a shader and vertex array.
    ///
    /// When `has_material_color` is true, the pipeline layout includes a
    /// fragment-stage push constant range for a `vec4` color at offset 128.
    pub fn create_pipeline(
        &self,
        shader: &Shader,
        va: &VertexArray,
        has_material_color: bool,
    ) -> Pipeline {
        pipeline::create_pipeline(&self.device, shader, va, self.render_pass, has_material_color)
    }

    // -- Clear color ----------------------------------------------------------

    /// Set the clear color used at the start of each render pass.
    pub fn set_clear_color(&mut self, color: [f32; 4]) {
        RenderCommand::set_clear_color(&mut self.api, color);
    }

    /// Get the current clear color.
    pub fn clear_color(&self) -> [f32; 4] {
        RenderCommand::clear_color(&self.api)
    }

    // -- Scene management (engine-internal) -----------------------------------

    /// Begin a new scene — stores the camera's view-projection matrix,
    /// saves the draw context, and sets viewport/scissor.
    pub(crate) fn begin_scene(&mut self, camera: &OrthographicCamera, ctx: DrawContext) {
        self.view_projection = *camera.view_projection_matrix();
        self.draw_context = Some(ctx);
        RenderCommand::set_viewport(&self.api, &ctx);
    }

    /// End the current scene — clears the draw context.
    pub(crate) fn end_scene(&mut self) {
        self.draw_context = None;
    }

    /// Submit a draw call: bind pipeline, push VP + transform matrices,
    /// optionally push material color, bind vertex array, draw indexed.
    pub fn submit(
        &self,
        pipeline: &Pipeline,
        vertex_array: &VertexArray,
        transform: &Mat4,
        color: Option<Vec4>,
    ) {
        let ctx = self
            .draw_context
            .expect("Renderer::submit called outside begin_scene/end_scene");
        RenderCommand::draw_indexed(
            &self.api,
            &ctx,
            pipeline.pipeline(),
            pipeline.layout(),
            vertex_array,
            &self.view_projection,
            transform,
            color.as_ref(),
        );
    }
}
