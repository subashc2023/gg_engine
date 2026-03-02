use std::path::Path;
use std::sync::Arc;

use ash::vk;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::buffer::{IndexBuffer, VertexBuffer};
use super::draw_context::DrawContext;
use super::orthographic_camera::OrthographicCamera;
use super::pipeline::{self, Pipeline};
use super::render_command::RenderCommand;
use super::renderer_2d::{BatchQuadVertex, Renderer2DData, Renderer2DStats};
use super::renderer_api::{RendererAPI, VulkanRendererAPI};
use super::shader::Shader;
use super::texture::Texture2D;
use super::vertex_array::VertexArray;
use super::VulkanContext;

use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// Unit quad positions and tex coords (used for CPU pre-transformation)
// ---------------------------------------------------------------------------

const QUAD_POSITIONS: [Vec4; 4] = [
    Vec4::new(-0.5, 0.5, 0.0, 1.0),  // top-left
    Vec4::new(0.5, 0.5, 0.0, 1.0),   // top-right
    Vec4::new(0.5, -0.5, 0.0, 1.0),  // bottom-right
    Vec4::new(-0.5, -0.5, 0.0, 1.0), // bottom-left
];

const QUAD_TEX_COORDS: [[f32; 2]; 4] = [
    [0.0, 0.0], // top-left
    [1.0, 0.0], // top-right
    [1.0, 1.0], // bottom-right
    [0.0, 1.0], // bottom-left
];

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

    // Built-in 2D renderer resources.
    renderer_2d: Option<Renderer2DData>,

    // Stats from the previous frame (snapshotted at end_scene).
    last_stats_2d: Renderer2DStats,
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
            renderer_2d: None,
            last_stats_2d: Renderer2DStats::default(),
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
    /// When `blend_enable` is true, standard alpha blending is enabled
    /// (src_alpha / one_minus_src_alpha).
    pub fn create_pipeline(
        &self,
        shader: &Shader,
        va: &VertexArray,
        has_material_color: bool,
        blend_enable: bool,
    ) -> Arc<Pipeline> {
        Arc::new(pipeline::create_pipeline(
            &self.device,
            shader,
            va,
            self.render_pass,
            has_material_color,
            &[],
            blend_enable,
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
        let mut texture = Texture2D::from_file(
            &self.instance,
            self.physical_device,
            &self.device,
            self.graphics_queue,
            self.command_pool,
            self.descriptor_pool,
            self.texture_descriptor_set_layout,
            path,
        );
        if let Some(data) = &self.renderer_2d {
            let index = data.register_texture(&texture);
            texture.set_bindless_index(index);
        }
        texture
    }

    /// Create a texture from raw RGBA8 pixel data.
    pub fn create_texture_from_rgba8(&self, width: u32, height: u32, pixels: &[u8]) -> Texture2D {
        let mut texture = Texture2D::from_rgba8(
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
        );
        if let Some(data) = &self.renderer_2d {
            let index = data.register_texture(&texture);
            texture.set_bindless_index(index);
        }
        texture
    }

    /// The descriptor set layout used for texture pipelines.
    pub fn texture_descriptor_set_layout(&self) -> vk::DescriptorSetLayout {
        self.texture_descriptor_set_layout
    }

    /// Update the stored render pass handle (e.g. after swapchain recreation).
    pub(crate) fn update_render_pass(&mut self, render_pass: vk::RenderPass) {
        self.render_pass = render_pass;
    }

    // -- Built-in 2D renderer -------------------------------------------------

    /// Initialize built-in 2D rendering resources (batch pipeline,
    /// dynamic VBs, static IB, bindless descriptor sets, 1×1 white
    /// default texture). Called once by the engine after Vulkan is ready.
    pub(crate) fn init_2d(&mut self) {
        let _timer = ProfileTimer::new("Renderer::init_2d");
        let white_texture = self.create_texture_from_rgba8(1, 1, &[255, 255, 255, 255]);
        let data = Renderer2DData::new(
            &self.instance,
            self.physical_device,
            &self.device,
            self.render_pass,
            white_texture,
        );
        // White texture gets bindless index 0.
        data.register_texture(&data.white_texture);
        self.renderer_2d = Some(data);
    }

    /// Get the 2D renderer batch statistics from the last completed frame.
    pub fn stats_2d(&self) -> Renderer2DStats {
        self.last_stats_2d
    }

    // -- Internal: push a quad into the batch ---------------------------------

    fn push_quad_to_batch(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        tiling_factor: f32,
    ) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        // Pre-transform quad vertices on CPU.
        let mut vertices = [BatchQuadVertex {
            position: [0.0; 3],
            color: [color.x, color.y, color.z, color.w],
            tex_coord: [0.0; 2],
            tex_index,
        }; 4];

        for (i, v) in vertices.iter_mut().enumerate() {
            let world_pos = *transform * QUAD_POSITIONS[i];
            v.position = [world_pos.x, world_pos.y, world_pos.z];
            v.tex_coord = [
                QUAD_TEX_COORDS[i][0] * tiling_factor,
                QUAD_TEX_COORDS[i][1] * tiling_factor,
            ];
        }

        if !data.push_quad(vertices) {
            // Batch full — flush and retry.
            self.flush_batch();
            data.push_quad(vertices);
        }
    }

    /// Flush the current 2D batch (if any quads are pending).
    fn flush_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_batch called outside begin_scene/end_scene");

        data.flush(ctx.cmd_buf, &self.view_projection, ctx.current_frame);
    }


    // -- Axis-aligned quads (no rotation) ------------------------------------

    /// Draw a flat-colored quad at a 3D position with the given size and color.
    pub fn draw_quad(&self, position: &Vec3, size: &Vec2, color: Vec4) {
        let _timer = ProfileTimer::new("Renderer::draw_quad");
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::IDENTITY,
            *position,
        );
        // tex_index 0 = white texture
        self.push_quad_to_batch(&transform, color, 0.0, 1.0);
    }

    /// Draw a flat-colored quad at a 2D position (z = 0).
    pub fn draw_quad_2d(&self, position: &Vec2, size: &Vec2, color: Vec4) {
        self.draw_quad(&Vec3::new(position.x, position.y, 0.0), size, color);
    }

    /// Draw a textured quad at a 3D position with the given size.
    ///
    /// `tiling_factor` scales the texture coordinates (e.g. 10.0 tiles the
    /// texture 10x in each direction). `tint_color` is multiplied with the
    /// sampled texel — pass `Vec4::ONE` for no tint.
    pub fn draw_textured_quad(
        &self,
        position: &Vec3,
        size: &Vec2,
        texture: &Texture2D,
        tiling_factor: f32,
        tint_color: Vec4,
    ) {
        let _timer = ProfileTimer::new("Renderer::draw_textured_quad");
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::IDENTITY,
            *position,
        );
        self.push_quad_to_batch(&transform, tint_color, texture.bindless_index() as f32, tiling_factor);
    }

    /// Draw a textured quad at a 2D position (z = 0).
    pub fn draw_textured_quad_2d(
        &self,
        position: &Vec2,
        size: &Vec2,
        texture: &Texture2D,
        tiling_factor: f32,
        tint_color: Vec4,
    ) {
        self.draw_textured_quad(
            &Vec3::new(position.x, position.y, 0.0),
            size,
            texture,
            tiling_factor,
            tint_color,
        );
    }

    // -- Rotated quads --------------------------------------------------------

    /// Draw a rotated flat-colored quad. `rotation` is in radians (Z-axis).
    pub fn draw_rotated_quad(&self, position: &Vec3, size: &Vec2, rotation: f32, color: Vec4) {
        let _timer = ProfileTimer::new("Renderer::draw_rotated_quad");
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::from_rotation_z(rotation),
            *position,
        );
        self.push_quad_to_batch(&transform, color, 0.0, 1.0);
    }

    /// Draw a rotated flat-colored quad at a 2D position (z = 0).
    /// `rotation` is in radians (Z-axis).
    pub fn draw_rotated_quad_2d(&self, position: &Vec2, size: &Vec2, rotation: f32, color: Vec4) {
        self.draw_rotated_quad(
            &Vec3::new(position.x, position.y, 0.0),
            size,
            rotation,
            color,
        );
    }

    /// Draw a rotated textured quad. `rotation` is in radians (Z-axis).
    pub fn draw_rotated_textured_quad(
        &self,
        position: &Vec3,
        size: &Vec2,
        rotation: f32,
        texture: &Texture2D,
        tiling_factor: f32,
        tint_color: Vec4,
    ) {
        let _timer = ProfileTimer::new("Renderer::draw_rotated_textured_quad");
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::from_rotation_z(rotation),
            *position,
        );
        self.push_quad_to_batch(&transform, tint_color, texture.bindless_index() as f32, tiling_factor);
    }

    /// Draw a rotated textured quad at a 2D position (z = 0).
    /// `rotation` is in radians (Z-axis).
    pub fn draw_rotated_textured_quad_2d(
        &self,
        position: &Vec2,
        size: &Vec2,
        rotation: f32,
        texture: &Texture2D,
        tiling_factor: f32,
        tint_color: Vec4,
    ) {
        self.draw_rotated_textured_quad(
            &Vec3::new(position.x, position.y, 0.0),
            size,
            rotation,
            texture,
            tiling_factor,
            tint_color,
        );
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
    /// saves the draw context, sets viewport/scissor, and resets the batch.
    pub(crate) fn begin_scene(&mut self, camera: &OrthographicCamera, ctx: DrawContext) {
        let _timer = ProfileTimer::new("Renderer::begin_scene");
        self.view_projection = *camera.view_projection_matrix();
        self.draw_context = Some(ctx);
        RenderCommand::set_viewport(&self.api, &ctx);

        // Reset batch state for this frame.
        if let Some(data) = &self.renderer_2d {
            data.reset_batch();
        }
    }

    /// End the current scene — flushes any pending batch, snapshots stats,
    /// and clears the draw context.
    pub(crate) fn end_scene(&mut self) {
        // Flush any remaining quads in the batch.
        if let Some(data) = &self.renderer_2d {
            if data.has_pending() {
                if let Some(ctx) = self.draw_context {
                    data.flush(ctx.cmd_buf, &self.view_projection, ctx.current_frame);
                }
            }
            // Snapshot stats for this frame (available via stats_2d() until next end_scene).
            self.last_stats_2d = data.stats();
        }
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
