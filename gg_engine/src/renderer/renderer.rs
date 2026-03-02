use std::path::Path;
use std::sync::Arc;

use ash::vk;
use glam::{Mat4, Vec4};

use super::buffer::{IndexBuffer, VertexBuffer};
use super::draw_context::DrawContext;
use super::orthographic_camera::OrthographicCamera;
use super::pipeline::{self, Pipeline};
use super::render_command::RenderCommand;
use super::renderer_api::{RendererAPI, VulkanRendererAPI};
use super::shader::Shader;
use super::texture::Texture2D;
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
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,

    // Texture descriptor infrastructure.
    descriptor_pool: vk::DescriptorPool,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,
}

impl Renderer {
    pub(crate) fn new(
        vk_ctx: &VulkanContext,
        render_pass: vk::RenderPass,
        command_pool: vk::CommandPool,
    ) -> Self {
        let device = vk_ctx.device();
        let api = RendererAPI::Vulkan(VulkanRendererAPI::new(device));

        // Create descriptor pool for texture samplers.
        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 100,
        };
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(std::slice::from_ref(&pool_size))
            .max_sets(100);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }
            .expect("Failed to create descriptor pool");

        // Create descriptor set layout: binding 0 = combined image sampler, fragment stage.
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(std::slice::from_ref(&binding));
        let texture_descriptor_set_layout =
            unsafe { device.create_descriptor_set_layout(&layout_info, None) }
                .expect("Failed to create descriptor set layout");

        Self {
            api,
            draw_context: None,
            view_projection: Mat4::IDENTITY,
            instance: vk_ctx.instance().clone(),
            physical_device: vk_ctx.physical_device(),
            device: device.clone(),
            render_pass,
            graphics_queue: vk_ctx.graphics_queue(),
            command_pool,
            descriptor_pool,
            texture_descriptor_set_layout,
        }
    }

    // -- Public resource creation API -----------------------------------------

    /// Create a shader from pre-compiled SPIR-V bytecode.
    pub fn create_shader(&self, name: &str, vert_spv: &[u8], frag_spv: &[u8]) -> Arc<Shader> {
        Arc::new(Shader::new(&self.device, name, vert_spv, frag_spv))
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
    ) -> Arc<Pipeline> {
        Arc::new(pipeline::create_pipeline(
            &self.device,
            shader,
            va,
            self.render_pass,
            has_material_color,
            &[],
            false,
        ))
    }

    /// Create a graphics pipeline for textured rendering.
    ///
    /// Includes the texture descriptor set layout and enables alpha blending.
    pub fn create_texture_pipeline(&self, shader: &Shader, va: &VertexArray) -> Arc<Pipeline> {
        Arc::new(pipeline::create_pipeline(
            &self.device,
            shader,
            va,
            self.render_pass,
            false,
            &[self.texture_descriptor_set_layout],
            true,
        ))
    }

    /// Load a texture from an image file.
    pub fn create_texture_from_file(&self, path: &Path) -> Texture2D {
        Texture2D::from_file(
            &self.instance,
            self.physical_device,
            &self.device,
            self.graphics_queue,
            self.command_pool,
            self.descriptor_pool,
            self.texture_descriptor_set_layout,
            path,
        )
    }

    /// Create a texture from raw RGBA8 pixel data.
    pub fn create_texture_from_rgba8(&self, width: u32, height: u32, pixels: &[u8]) -> Texture2D {
        Texture2D::from_rgba8(
            &self.instance,
            self.physical_device,
            &self.device,
            self.graphics_queue,
            self.command_pool,
            self.descriptor_pool,
            self.texture_descriptor_set_layout,
            width,
            height,
            pixels,
        )
    }

    /// The descriptor set layout used for texture pipelines.
    pub fn texture_descriptor_set_layout(&self) -> vk::DescriptorSetLayout {
        self.texture_descriptor_set_layout
    }

    /// Update the stored render pass handle (e.g. after swapchain recreation).
    pub(crate) fn update_render_pass(&mut self, render_pass: vk::RenderPass) {
        self.render_pass = render_pass;
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
            None,
        );
    }

    /// Submit a textured draw call: like `submit` but binds the texture's
    /// descriptor set.
    pub fn submit_textured(
        &self,
        pipeline: &Pipeline,
        vertex_array: &VertexArray,
        transform: &Mat4,
        texture: &Texture2D,
    ) {
        let ctx = self
            .draw_context
            .expect("Renderer::submit_textured called outside begin_scene/end_scene");
        RenderCommand::draw_indexed(
            &self.api,
            &ctx,
            pipeline.pipeline(),
            pipeline.layout(),
            vertex_array,
            &self.view_projection,
            transform,
            None,
            Some(texture.descriptor_set()),
        );
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.texture_descriptor_set_layout, None);
        }
    }
}
