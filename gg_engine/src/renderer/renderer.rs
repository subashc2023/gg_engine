use std::path::Path;
use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::buffer::{IndexBuffer, VertexBuffer};
use super::draw_context::DrawContext;
use super::font::{Font, FontCpuData};
use super::framebuffer::{Framebuffer, FramebufferSpec};
use super::gpu_allocation::GpuAllocator;
use super::gpu_particle_system::GpuParticleSystem;
use super::pipeline::{self, Pipeline};
use super::render_command::RenderCommand;
use super::renderer_2d::{
    BatchCircleVertex, BatchLineVertex, BatchQuadVertex, Renderer2DData, Renderer2DStats,
    SpriteInstanceData,
};
use super::renderer_api::{RendererAPI, VulkanRendererAPI};
use super::shader::Shader;
use super::sub_texture::SubTexture2D;
use super::texture::TextureCpuData;
use super::texture::{Texture2D, TransferBatch};
use super::uniform_buffer::{CameraData, UniformBuffer};
use super::vertex_array::VertexArray;
use super::VulkanContext;

use crate::profiling::ProfileTimer;
use crate::scene::{CircleRendererComponent, SpriteRendererComponent, TextComponent};

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
    device: ash::Device,
    render_pass: vk::RenderPass,
    graphics_queue: vk::Queue,
    command_pool: vk::CommandPool,

    // GPU sub-allocator for buffer/image memory.
    allocator: Arc<Mutex<GpuAllocator>>,

    // Texture descriptor infrastructure.
    descriptor_pool: vk::DescriptorPool,
    texture_descriptor_set_layout: vk::DescriptorSetLayout,

    // Camera UBO (per-frame VP matrix).
    camera_ubo: UniformBuffer,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    camera_ubo_ds: [vk::DescriptorSet; 2],

    // Format info for framebuffer creation.
    color_format: vk::Format,
    depth_format: vk::Format,

    // Pipeline cache for faster startup on subsequent runs.
    pipeline_cache: vk::PipelineCache,

    // Built-in 2D renderer resources.
    renderer_2d: Option<Renderer2DData>,

    // Line rendering.
    line_width: f32,

    // Stats from the previous frame (snapshotted at end_scene).
    last_stats_2d: Renderer2DStats,

    // Batched async texture/font upload system (fence-tracked, no queue_wait_idle).
    transfer_batch: TransferBatch,

    // GPU-driven particle system (compute shader simulation + instanced rendering).
    gpu_particles: Option<GpuParticleSystem>,
}

impl Renderer {
    pub(crate) fn new(
        vk_ctx: &VulkanContext,
        allocator: &Arc<Mutex<GpuAllocator>>,
        render_pass: vk::RenderPass,
        command_pool: vk::CommandPool,
        color_format: vk::Format,
        depth_format: vk::Format,
    ) -> Result<Self, String> {
        let device = vk_ctx.device();
        let api = RendererAPI::Vulkan(VulkanRendererAPI::new(device));

        // Create descriptor pool for texture samplers + camera UBO sets.
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 100,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: 2,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(102)
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }
            .map_err(|e| format!("Failed to create descriptor pool: {e}"))?;

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
                .map_err(|e| format!("Failed to create descriptor set layout: {e}"))?;

        // -- Camera UBO descriptor set layout: binding 0, UNIFORM_BUFFER, vertex stage --
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let ubo_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&ubo_binding));
        let camera_ubo_ds_layout =
            unsafe { device.create_descriptor_set_layout(&ubo_layout_info, None) }
                .map_err(|e| format!("Failed to create camera UBO descriptor set layout: {e}"))?;

        // -- Camera UBO buffer (64 bytes, double-buffered) --
        let camera_ubo = UniformBuffer::new(allocator, device, CameraData::SIZE)?;

        // -- Allocate 2 descriptor sets for the camera UBO --
        let ubo_layouts = [camera_ubo_ds_layout; 2];
        let ubo_ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&ubo_layouts);
        let ubo_ds_vec = unsafe { device.allocate_descriptor_sets(&ubo_ds_alloc_info) }
            .map_err(|e| format!("Failed to allocate camera UBO descriptor sets: {e}"))?;
        let camera_ubo_ds = [ubo_ds_vec[0], ubo_ds_vec[1]];

        // -- Write each descriptor set pointing to the UBO buffer --
        for (i, &ds) in camera_ubo_ds.iter().enumerate() {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(camera_ubo.buffer(i))
                .offset(0)
                .range(CameraData::SIZE as u64);
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(std::slice::from_ref(&buffer_info));
            unsafe {
                device.update_descriptor_sets(&[write], &[]);
            }
        }

        // -- Pipeline cache (load from disk if available) --
        let cache_data = Self::load_pipeline_cache_data();
        let cache_create_info = if cache_data.is_empty() {
            vk::PipelineCacheCreateInfo::default()
        } else {
            vk::PipelineCacheCreateInfo::default().initial_data(&cache_data)
        };
        let pipeline_cache = unsafe { device.create_pipeline_cache(&cache_create_info, None) }
            .map_err(|e| format!("Failed to create pipeline cache: {e}"))?;

        let transfer_batch = TransferBatch::new(device, command_pool, vk_ctx.graphics_queue());

        Ok(Self {
            api,
            draw_context: None,
            view_projection: Mat4::IDENTITY,
            device: device.clone(),
            render_pass,
            graphics_queue: vk_ctx.graphics_queue(),
            command_pool,
            allocator: allocator.clone(),
            descriptor_pool,
            texture_descriptor_set_layout,
            camera_ubo,
            camera_ubo_ds_layout,
            camera_ubo_ds,
            color_format,
            depth_format,
            pipeline_cache,
            renderer_2d: None,
            line_width: 4.0,
            last_stats_2d: Renderer2DStats::default(),
            transfer_batch,
            gpu_particles: None,
        })
    }

    // -- Pipeline cache persistence -------------------------------------------

    fn pipeline_cache_path() -> Option<std::path::PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("pipeline_cache.bin")))
    }

    fn load_pipeline_cache_data() -> Vec<u8> {
        Self::pipeline_cache_path()
            .and_then(|p| std::fs::read(&p).ok())
            .unwrap_or_default()
    }

    fn save_pipeline_cache(&self) {
        let data = unsafe { self.device.get_pipeline_cache_data(self.pipeline_cache) };
        match data {
            Ok(bytes) => {
                if let Some(path) = Self::pipeline_cache_path() {
                    if let Err(e) = std::fs::write(&path, &bytes) {
                        log::warn!("Failed to save pipeline cache: {}", e);
                    } else {
                        log::info!("Pipeline cache saved ({} bytes)", bytes.len());
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to read pipeline cache data: {:?}", e);
            }
        }
    }

    // -- Public resource creation API -----------------------------------------

    /// Create a shader from pre-compiled SPIR-V bytecode.
    pub fn create_shader(
        &self,
        name: &str,
        vert_spv: &[u8],
        frag_spv: &[u8],
    ) -> Result<Arc<Shader>, String> {
        Ok(Arc::new(Shader::new(
            &self.device,
            name,
            vert_spv,
            frag_spv,
        )?))
    }

    /// Create a GPU vertex buffer from raw byte data.
    ///
    /// Use [`as_bytes`](super::as_bytes) to convert typed vertex slices.
    pub fn create_vertex_buffer(&self, data: &[u8]) -> Result<VertexBuffer, String> {
        VertexBuffer::new(&self.allocator, &self.device, data)
    }

    /// Create a GPU index buffer from u32 indices.
    pub fn create_index_buffer(&self, indices: &[u32]) -> Result<IndexBuffer, String> {
        IndexBuffer::new(&self.allocator, &self.device, indices)
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
    ) -> Result<Arc<Pipeline>, String> {
        Ok(Arc::new(pipeline::create_pipeline(
            &self.device,
            shader,
            va,
            self.render_pass,
            has_material_color,
            self.camera_ubo_ds_layout,
            &[],
            blend_enable,
            self.pipeline_cache,
        )?))
    }

    /// Create a graphics pipeline for textured rendering.
    ///
    /// Includes the texture descriptor set layout and enables alpha blending.
    pub fn create_texture_pipeline(
        &self,
        shader: &Shader,
        va: &VertexArray,
    ) -> Result<Arc<Pipeline>, String> {
        Ok(Arc::new(pipeline::create_pipeline(
            &self.device,
            shader,
            va,
            self.render_pass,
            false,
            self.camera_ubo_ds_layout,
            &[self.texture_descriptor_set_layout],
            true,
            self.pipeline_cache,
        )?))
    }

    /// Load a texture from an image file.
    ///
    /// Returns `None` if the file cannot be loaded or decoded.
    pub fn create_texture_from_file(&self, path: &Path) -> Option<Texture2D> {
        let mut texture = Texture2D::from_file(&self.resources(), &self.allocator, path)?;
        if let Some(data) = &self.renderer_2d {
            let index = data.register_texture(&texture);
            texture.set_bindless_index(index);
        }
        Some(texture)
    }

    /// Create a texture from raw RGBA8 pixel data.
    pub fn create_texture_from_rgba8(
        &self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<Texture2D, String> {
        let mut texture =
            Texture2D::from_rgba8(&self.resources(), &self.allocator, width, height, pixels)?;
        if let Some(data) = &self.renderer_2d {
            let index = data.register_texture(&texture);
            texture.set_bindless_index(index);
        }
        Ok(texture)
    }

    /// Load a font from a TTF file and generate an MSDF atlas.
    /// The atlas texture is registered in the bindless descriptor array.
    ///
    /// Returns `None` if the font file cannot be loaded or parsed.
    pub fn create_font(&self, path: &Path) -> Option<Font> {
        let mut font = Font::load(&self.resources(), &self.allocator, path)?;
        if let Some(data) = &self.renderer_2d {
            let index = data.register_texture(&font.atlas_texture);
            font.atlas_texture.set_bindless_index(index);
        }
        Some(font)
    }

    /// Upload a texture from pre-loaded CPU data (async path).
    /// Records the staging copy into the internal [`TransferBatch`] — call
    /// [`flush_transfers`] before rendering to submit the batch.
    pub fn upload_texture(&mut self, data: &TextureCpuData) -> Result<Texture2D, String> {
        let res = super::RendererResources {
            device: &self.device,
            graphics_queue: self.graphics_queue,
            command_pool: self.command_pool,
            descriptor_pool: self.descriptor_pool,
            texture_ds_layout: self.texture_descriptor_set_layout,
            color_format: self.color_format,
            depth_format: self.depth_format,
        };
        let mut texture = Texture2D::from_cpu_data_batched(
            &res,
            &self.allocator,
            data,
            &mut self.transfer_batch,
        )?;
        if let Some(r2d) = &self.renderer_2d {
            let index = r2d.register_texture(&texture);
            texture.set_bindless_index(index);
        }
        Ok(texture)
    }

    /// Upload a font from pre-generated CPU data (async path).
    /// Records the atlas upload into the internal [`TransferBatch`].
    pub fn upload_font(&mut self, data: FontCpuData) -> Result<Font, String> {
        let res = super::RendererResources {
            device: &self.device,
            graphics_queue: self.graphics_queue,
            command_pool: self.command_pool,
            descriptor_pool: self.descriptor_pool,
            texture_ds_layout: self.texture_descriptor_set_layout,
            color_format: self.color_format,
            depth_format: self.depth_format,
        };
        let mut font =
            Font::from_cpu_data_batched(&res, &self.allocator, data, &mut self.transfer_batch)?;
        if let Some(r2d) = &self.renderer_2d {
            let index = r2d.register_texture(&font.atlas_texture);
            font.atlas_texture.set_bindless_index(index);
        }
        Ok(font)
    }

    /// Submit any pending texture/font uploads as a single command buffer with
    /// a fence. Call this before rendering to ensure uploaded textures are
    /// available. No-op if nothing is pending.
    pub fn flush_transfers(&mut self) {
        if let Err(e) = self.transfer_batch.submit() {
            log::error!("Failed to submit transfer batch: {e}");
        }
    }

    /// Poll completed transfer fences and free their staging buffers.
    /// Call once per frame (e.g., at the start of the update loop).
    pub fn poll_transfers(&mut self) {
        self.transfer_batch.poll();
    }

    /// Return a texture's bindless slot to the free-list for reuse.
    ///
    /// Call this before dropping a texture to avoid exhausting the 4096 slot limit.
    /// The slot will be recycled by the next `create_texture_*` call.
    pub fn unregister_texture(&self, texture: &Texture2D) {
        if let Some(data) = &self.renderer_2d {
            data.unregister_texture(texture.bindless_index());
        }
    }

    /// The descriptor set layout used for texture pipelines.
    pub fn texture_descriptor_set_layout(&self) -> vk::DescriptorSetLayout {
        self.texture_descriptor_set_layout
    }

    /// Create an offscreen framebuffer for rendering to a texture.
    pub fn create_framebuffer(&self, spec: FramebufferSpec) -> Result<Framebuffer, String> {
        Framebuffer::new(&self.resources(), &self.allocator, spec)
    }

    /// Bundle Renderer-owned Vulkan state into a lightweight view for internal
    /// factory functions, avoiding 7-8 individual parameter lists.
    fn resources(&self) -> super::RendererResources<'_> {
        super::RendererResources {
            device: &self.device,
            graphics_queue: self.graphics_queue,
            command_pool: self.command_pool,
            descriptor_pool: self.descriptor_pool,
            texture_ds_layout: self.texture_descriptor_set_layout,
            color_format: self.color_format,
            depth_format: self.depth_format,
        }
    }

    /// Update the stored render pass handle (e.g. after swapchain recreation).
    pub(crate) fn update_render_pass(&mut self, render_pass: vk::RenderPass) {
        self.render_pass = render_pass;
    }

    /// Create an offscreen batch pipeline compatible with the given render pass
    /// (e.g. a framebuffer with multiple color attachments for picking).
    pub fn create_offscreen_batch_pipeline(
        &mut self,
        render_pass: vk::RenderPass,
        color_attachment_count: u32,
    ) -> Result<(), String> {
        if let Some(data) = &mut self.renderer_2d {
            data.create_offscreen_pipeline(
                &self.device,
                render_pass,
                self.camera_ubo_ds_layout,
                color_attachment_count,
                self.pipeline_cache,
            )?;
        }
        Ok(())
    }

    /// Tell the batch renderer to use the offscreen pipeline (or switch back).
    pub(crate) fn use_offscreen_pipeline(&mut self, use_offscreen: bool) {
        if let Some(data) = &mut self.renderer_2d {
            data.set_use_offscreen(use_offscreen);
        }
    }

    /// Hot-reload all shaders from the given source directory.
    ///
    /// Compiles `.glsl` files with `glslc` at runtime, creates new shader
    /// modules, and rebuilds all pipelines. Waits for GPU idle before
    /// swapping. On failure, returns an error string and keeps old pipelines.
    pub fn reload_shaders(&mut self, shader_dir: &std::path::Path) -> Result<u32, String> {
        if let Some(data) = &mut self.renderer_2d {
            unsafe {
                self.device
                    .device_wait_idle()
                    .map_err(|e| format!("device_wait_idle failed: {e}"))?;
            }
            data.reload_shaders(shader_dir)
        } else {
            Err("2D renderer not initialized".to_string())
        }
    }

    // -- Built-in 2D renderer -------------------------------------------------

    /// Initialize built-in 2D rendering resources (batch pipeline,
    /// dynamic VBs, static IB, bindless descriptor sets, 1×1 white
    /// default texture). Called once by the engine after Vulkan is ready.
    pub(crate) fn init_2d(&mut self) -> Result<(), String> {
        let _timer = ProfileTimer::new("Renderer::init_2d");
        let white_texture = self.create_texture_from_rgba8(1, 1, &[255, 255, 255, 255])?;
        let data = Renderer2DData::new(
            &self.allocator,
            &self.device,
            self.render_pass,
            self.camera_ubo_ds_layout,
            white_texture,
            self.pipeline_cache,
        )?;
        // White texture gets bindless index 0.
        data.register_texture(&data.white_texture);
        self.renderer_2d = Some(data);
        Ok(())
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
        entity_id: i32,
    ) {
        self.push_quad_to_batch_uv(
            transform,
            color,
            tex_index,
            &QUAD_TEX_COORDS,
            tiling_factor,
            entity_id,
        );
    }

    fn push_quad_to_batch_uv(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        tex_coords: &[[f32; 2]; 4],
        tiling_factor: f32,
        entity_id: i32,
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
            entity_id,
        }; 4];

        for (i, v) in vertices.iter_mut().enumerate() {
            let world_pos = *transform * QUAD_POSITIONS[i];
            v.position = [world_pos.x, world_pos.y, world_pos.z];
            v.tex_coord = [
                tex_coords[i][0] * tiling_factor,
                tex_coords[i][1] * tiling_factor,
            ];
        }

        if !data.push_quad(vertices) {
            // Batch full — flush and retry.
            self.flush_quad_batch();
            data.push_quad(vertices);
        }
    }

    /// Push a particle quad directly — bypasses Mat4 construction.
    /// Uses one sin/cos + direct vertex math instead of a full matrix transform.
    pub fn draw_particle(&self, position: &Vec3, size: f32, rotation: f32, color: Vec4) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        let half = size * 0.5;
        let (sin_r, cos_r) = rotation.sin_cos();
        let cx = cos_r * half;
        let cy = sin_r * half;

        // Four corners of a rotated quad centered at `position`.
        //   TL = (-cos - (-sin), -sin - cos)  = (-cx + cy, -cy - cx)
        //   TR = ( cos - (-sin),  sin - cos)   = ( cx + cy,  cy - cx)
        //   BR = ( cos - sin,     sin - (-cos)) = ( cx - cy,  cy + cx)
        //   BL = (-cos - sin,    -sin - (-cos)) = (-cx - cy, -cy + cx)
        let px = position.x;
        let py = position.y;
        let pz = position.z;
        let col = [color.x, color.y, color.z, color.w];

        let vertices = [
            BatchQuadVertex {
                position: [px - cx + cy, py - cy - cx, pz],
                color: col,
                tex_coord: [0.0, 0.0],
                tex_index: 0.0,
                entity_id: -1,
            },
            BatchQuadVertex {
                position: [px + cx + cy, py + cy - cx, pz],
                color: col,
                tex_coord: [1.0, 0.0],
                tex_index: 0.0,
                entity_id: -1,
            },
            BatchQuadVertex {
                position: [px + cx - cy, py + cy + cx, pz],
                color: col,
                tex_coord: [1.0, 1.0],
                tex_index: 0.0,
                entity_id: -1,
            },
            BatchQuadVertex {
                position: [px - cx - cy, py - cy + cx, pz],
                color: col,
                tex_coord: [0.0, 1.0],
                tex_index: 0.0,
                entity_id: -1,
            },
        ];

        if !data.push_quad(vertices) {
            self.flush_quad_batch();
            data.push_quad(vertices);
        }
    }

    /// Flush the current quad batch (if any quads are pending).
    fn flush_quad_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_quad_batch called outside begin_scene/end_scene");

        data.flush_quads(
            ctx.cmd_buf,
            self.camera_ubo_ds[ctx.current_frame],
            ctx.current_frame,
        );
    }

    /// Flush the current circle batch (if any circles are pending).
    fn flush_circle_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_circle_batch called outside begin_scene/end_scene");

        data.flush_circles(
            ctx.cmd_buf,
            self.camera_ubo_ds[ctx.current_frame],
            ctx.current_frame,
        );
    }

    // -- Internal: push a sprite instance into the instanced batch -----------

    fn push_sprite_instance(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        tiling_factor: f32,
        entity_id: i32,
    ) {
        self.push_sprite_instance_uv(
            transform,
            color,
            tex_index,
            tiling_factor,
            [0.0, 0.0],
            [1.0, 1.0],
            entity_id,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push_sprite_instance_uv(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        tiling_factor: f32,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        entity_id: i32,
    ) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        let cols = transform.to_cols_array_2d();
        let instance = SpriteInstanceData {
            transform_col0: cols[0],
            transform_col1: cols[1],
            transform_col2: cols[2],
            transform_col3: cols[3],
            color: [color.x, color.y, color.z, color.w],
            uv_min,
            uv_max,
            tex_index,
            tiling_factor,
            entity_id,
            _pad: 0,
        };

        if !data.push_instance(instance) {
            // Batch full — flush and retry.
            self.flush_instance_batch();
            data.push_instance(instance);
        }
    }

    /// Flush the current instance batch (if any instances are pending).
    fn flush_instance_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_instance_batch called outside begin_scene/end_scene");

        data.flush_instances(
            ctx.cmd_buf,
            self.camera_ubo_ds[ctx.current_frame],
            ctx.current_frame,
        );
    }

    // -- Transform-based quads (raw Mat4) ------------------------------------

    /// Draw a flat-colored quad with a pre-built transform matrix.
    /// `entity_id` is written to the entity ID attachment for mouse picking
    /// (`-1` means no entity).
    pub fn draw_quad_transform(&self, transform: &Mat4, color: Vec4, entity_id: i32) {
        self.push_quad_to_batch(transform, color, 0.0, 1.0, entity_id);
    }

    /// Draw a textured quad with a pre-built transform matrix.
    pub fn draw_textured_quad_transform(
        &self,
        transform: &Mat4,
        texture: &Texture2D,
        tiling_factor: f32,
        tint_color: Vec4,
    ) {
        self.push_quad_to_batch(
            transform,
            tint_color,
            texture.bindless_index() as f32,
            tiling_factor,
            -1,
        );
    }

    /// Draw a sprite (entity with a [`SpriteRendererComponent`]) using a
    /// pre-built transform matrix. Writes the entity ID to the picking
    /// attachment so it can be read back for mouse picking.
    ///
    /// If the sprite has a texture, it is sampled and multiplied by the
    /// sprite's color (acting as a tint). The `tiling_factor` controls
    /// texture coordinate scaling. If no texture is set, the white default
    /// texture is used (flat-colored quad).
    pub fn draw_sprite(&self, transform: &Mat4, sprite: &SpriteRendererComponent, entity_id: i32) {
        let tex_index = sprite
            .texture
            .as_ref()
            .map(|t| t.bindless_index() as f32)
            .unwrap_or(0.0); // 0 = white texture
        self.push_sprite_instance(
            transform,
            sprite.color,
            tex_index,
            sprite.tiling_factor,
            entity_id,
        );
    }

    // -- Axis-aligned quads (no rotation) ------------------------------------

    /// Draw a flat-colored quad at a 3D position with the given size and color.
    pub fn draw_quad(&self, position: &Vec3, size: &Vec2, color: Vec4) {
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::IDENTITY,
            *position,
        );
        // tex_index 0 = white texture
        self.push_quad_to_batch(&transform, color, 0.0, 1.0, -1);
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
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::IDENTITY,
            *position,
        );
        self.push_quad_to_batch(
            &transform,
            tint_color,
            texture.bindless_index() as f32,
            tiling_factor,
            -1,
        );
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
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::from_rotation_z(rotation),
            *position,
        );
        self.push_quad_to_batch(&transform, color, 0.0, 1.0, -1);
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
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::from_rotation_z(rotation),
            *position,
        );
        self.push_quad_to_batch(
            &transform,
            tint_color,
            texture.bindless_index() as f32,
            tiling_factor,
            -1,
        );
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

    // -- Sub-textured quads (sprite sheet regions) ----------------------------

    /// Draw a sub-textured quad at a 3D position.
    ///
    /// Uses the pre-computed texture coordinates from the [`SubTexture2D`] to
    /// render a specific region of a sprite sheet / texture atlas.
    /// `tint_color` is multiplied with the sampled texel — pass `Vec4::ONE`
    /// for no tint.
    pub fn draw_sub_textured_quad(
        &self,
        position: &Vec3,
        size: &Vec2,
        sub_texture: &SubTexture2D,
        tint_color: Vec4,
    ) {
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::IDENTITY,
            *position,
        );
        self.push_quad_to_batch_uv(
            &transform,
            tint_color,
            sub_texture.bindless_index() as f32,
            sub_texture.tex_coords(),
            1.0,
            -1,
        );
    }

    /// Draw a sub-textured quad at a 2D position (z = 0).
    pub fn draw_sub_textured_quad_2d(
        &self,
        position: &Vec2,
        size: &Vec2,
        sub_texture: &SubTexture2D,
        tint_color: Vec4,
    ) {
        self.draw_sub_textured_quad(
            &Vec3::new(position.x, position.y, 0.0),
            size,
            sub_texture,
            tint_color,
        );
    }

    /// Draw a sub-textured quad using a pre-built transform matrix.
    ///
    /// Used by the animation system to render the current frame of a
    /// sprite sheet at the entity's world transform.
    pub fn draw_sub_textured_quad_transformed(
        &self,
        transform: &Mat4,
        sub_texture: &SubTexture2D,
        tint_color: Vec4,
        entity_id: i32,
    ) {
        let tc = sub_texture.tex_coords();
        // tc[0] = (min_u, min_v), tc[2] = (max_u, max_v)
        self.push_sprite_instance_uv(
            transform,
            tint_color,
            sub_texture.bindless_index() as f32,
            1.0,
            tc[0],
            tc[2],
            entity_id,
        );
    }

    /// Draw a textured quad with explicit UV coordinates and a pre-built
    /// transform matrix.  Skips [`SubTexture2D`] construction — useful for
    /// tight inner loops such as tilemap rendering.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_textured_quad_transformed_uv(
        &self,
        transform: &Mat4,
        tex_index: f32,
        uv_min: [f32; 2],
        uv_max: [f32; 2],
        tint_color: Vec4,
        entity_id: i32,
    ) {
        self.push_sprite_instance_uv(
            transform, tint_color, tex_index, 1.0, uv_min, uv_max, entity_id,
        );
    }

    /// Draw a rotated sub-textured quad. `rotation` is in radians (Z-axis).
    pub fn draw_rotated_sub_textured_quad(
        &self,
        position: &Vec3,
        size: &Vec2,
        rotation: f32,
        sub_texture: &SubTexture2D,
        tint_color: Vec4,
    ) {
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(size.x, size.y, 1.0),
            Quat::from_rotation_z(rotation),
            *position,
        );
        self.push_quad_to_batch_uv(
            &transform,
            tint_color,
            sub_texture.bindless_index() as f32,
            sub_texture.tex_coords(),
            1.0,
            -1,
        );
    }

    /// Draw a rotated sub-textured quad at a 2D position (z = 0).
    /// `rotation` is in radians (Z-axis).
    pub fn draw_rotated_sub_textured_quad_2d(
        &self,
        position: &Vec2,
        size: &Vec2,
        rotation: f32,
        sub_texture: &SubTexture2D,
        tint_color: Vec4,
    ) {
        self.draw_rotated_sub_textured_quad(
            &Vec3::new(position.x, position.y, 0.0),
            size,
            rotation,
            sub_texture,
            tint_color,
        );
    }

    // -- Circle drawing -------------------------------------------------------

    /// Internal: push a circle (quad) into the circle batch.
    fn push_circle_to_batch(
        &self,
        transform: &Mat4,
        color: Vec4,
        thickness: f32,
        fade: f32,
        entity_id: i32,
    ) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        let col = [color.x, color.y, color.z, color.w];

        let mut vertices = [BatchCircleVertex {
            world_position: [0.0; 3],
            local_position: [0.0; 3],
            color: col,
            thickness,
            fade,
            entity_id,
        }; 4];

        for (i, v) in vertices.iter_mut().enumerate() {
            let world_pos = *transform * QUAD_POSITIONS[i];
            v.world_position = [world_pos.x, world_pos.y, world_pos.z];
            // Local position: quad corners * 2 → range [-1, 1].
            v.local_position = [QUAD_POSITIONS[i].x * 2.0, QUAD_POSITIONS[i].y * 2.0, 0.0];
        }

        if !data.push_circle(vertices) {
            self.flush_circle_batch();
            data.push_circle(vertices);
        }
    }

    /// Draw a circle using a pre-built transform matrix.
    /// `entity_id` is written to the entity ID attachment for mouse picking
    /// (`-1` means no entity).
    pub fn draw_circle(
        &self,
        transform: &Mat4,
        color: Vec4,
        thickness: f32,
        fade: f32,
        entity_id: i32,
    ) {
        self.push_circle_to_batch(transform, color, thickness, fade, entity_id);
    }

    /// Draw a [`CircleRendererComponent`] using a pre-built transform matrix.
    /// Writes the entity ID to the picking attachment.
    pub fn draw_circle_component(
        &self,
        transform: &Mat4,
        circle: &CircleRendererComponent,
        entity_id: i32,
    ) {
        self.push_circle_to_batch(
            transform,
            circle.color,
            circle.thickness,
            circle.fade,
            entity_id,
        );
    }

    // -- Line drawing ----------------------------------------------------------

    /// Internal: push a line (2 vertices) into the line batch.
    fn push_line_to_batch(&self, p0: Vec3, p1: Vec3, color: Vec4, entity_id: i32) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        let col = [color.x, color.y, color.z, color.w];

        let vertices = [
            BatchLineVertex {
                position: [p0.x, p0.y, p0.z],
                color: col,
                entity_id,
            },
            BatchLineVertex {
                position: [p1.x, p1.y, p1.z],
                color: col,
                entity_id,
            },
        ];

        if !data.push_line(vertices) {
            self.flush_line_batch();
            data.push_line(vertices);
        }
    }

    /// Flush the current line batch (if any lines are pending).
    fn flush_line_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_line_batch called outside begin_scene/end_scene");

        data.flush_lines(
            ctx.cmd_buf,
            self.camera_ubo_ds[ctx.current_frame],
            ctx.current_frame,
            self.line_width,
        );
    }

    /// Draw a line from `p0` to `p1` in world space.
    pub fn draw_line(&self, p0: Vec3, p1: Vec3, color: Vec4, entity_id: i32) {
        self.push_line_to_batch(p0, p1, color, entity_id);
    }

    /// Draw a wireframe rectangle at a 3D position with the given size and color.
    /// The rectangle lies in the XY plane at the given Z coordinate.
    pub fn draw_rect(&self, position: &Vec3, size: &Vec2, color: Vec4, entity_id: i32) {
        let hx = size.x * 0.5;
        let hy = size.y * 0.5;
        let z = position.z;

        let p0 = Vec3::new(position.x - hx, position.y - hy, z); // bottom-left
        let p1 = Vec3::new(position.x + hx, position.y - hy, z); // bottom-right
        let p2 = Vec3::new(position.x + hx, position.y + hy, z); // top-right
        let p3 = Vec3::new(position.x - hx, position.y + hy, z); // top-left

        self.draw_line(p0, p1, color, entity_id);
        self.draw_line(p1, p2, color, entity_id);
        self.draw_line(p2, p3, color, entity_id);
        self.draw_line(p3, p0, color, entity_id);
    }

    /// Draw a wireframe rectangle using a pre-built transform matrix.
    /// Transforms the unit quad corners by the matrix and draws 4 lines.
    pub fn draw_rect_transform(&self, transform: &Mat4, color: Vec4, entity_id: i32) {
        // Transform the unit quad corners.
        let mut corners = [Vec3::ZERO; 4];
        for (i, corner) in corners.iter_mut().enumerate() {
            let world_pos = *transform * QUAD_POSITIONS[i];
            *corner = Vec3::new(world_pos.x, world_pos.y, world_pos.z);
        }

        // Draw 4 lines connecting the corners.
        self.draw_line(corners[0], corners[1], color, entity_id);
        self.draw_line(corners[1], corners[2], color, entity_id);
        self.draw_line(corners[2], corners[3], color, entity_id);
        self.draw_line(corners[3], corners[0], color, entity_id);
    }

    /// Get the current line width used for line rendering.
    pub fn line_width(&self) -> f32 {
        self.line_width
    }

    /// Set the line width used for line rendering.
    /// Requires `wideLines` device feature for values other than 1.0.
    /// Flushes any pending lines so they render at the previous width.
    pub fn set_line_width(&mut self, width: f32) {
        if (self.line_width - width).abs() > f32::EPSILON {
            if self.draw_context.is_some() {
                self.flush_line_batch();
            }
            self.line_width = width;
        }
    }

    // -- Text drawing ----------------------------------------------------------

    /// Internal: push a text glyph quad into the text batch.
    fn push_text_quad_to_batch(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        tex_coords: &[[f32; 2]; 4],
        entity_id: i32,
    ) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");

        let mut vertices = [BatchQuadVertex {
            position: [0.0; 3],
            color: [color.x, color.y, color.z, color.w],
            tex_coord: [0.0; 2],
            tex_index,
            entity_id,
        }; 4];

        for (i, v) in vertices.iter_mut().enumerate() {
            let world_pos = *transform * QUAD_POSITIONS[i];
            v.position = [world_pos.x, world_pos.y, world_pos.z];
            v.tex_coord = tex_coords[i];
        }

        if !data.push_text_quad(vertices) {
            self.flush_text_batch();
            data.push_text_quad(vertices);
        }
    }

    /// Flush the current text batch (if any text quads are pending).
    fn flush_text_batch(&self) {
        let data = self
            .renderer_2d
            .as_ref()
            .expect("Renderer2D not initialized — call init_2d first");
        let ctx = self
            .draw_context
            .expect("flush_text_batch called outside begin_scene/end_scene");

        data.flush_text(
            ctx.cmd_buf,
            self.camera_ubo_ds[ctx.current_frame],
            ctx.current_frame,
        );
    }

    /// Draw a text string using an SDF font.
    ///
    /// Each character is rendered as a separate quad using the font's atlas.
    /// The `transform` positions the text origin (top-left of first character).
    /// `font_size` controls the scaling of glyphs relative to the transform.
    /// `kerning` adds extra horizontal spacing between characters (in font units).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_string(
        &self,
        text: &str,
        transform: &Mat4,
        font: &Font,
        font_size: f32,
        color: Vec4,
        line_spacing: f32,
        kerning: f32,
        entity_id: i32,
    ) {
        let tex_index = font.bindless_index() as f32;
        let scale = font_size;

        let mut cursor_x: f32 = 0.0;
        let mut cursor_y: f32 = 0.0;

        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\n' {
                cursor_x = 0.0;
                cursor_y -= font.line_height * scale * line_spacing;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            if ch == '\t' {
                // Treat tab as 4 spaces.
                if let Some(space_glyph) = font.glyph(' ') {
                    cursor_x += (space_glyph.advance_x + kerning) * scale * 4.0;
                }
                continue;
            }

            let glyph = match font.glyph(ch).or_else(|| font.glyph('?')) {
                Some(g) => g,
                None => continue,
            };

            // Skip rendering for whitespace (no width/height), but advance cursor.
            if glyph.width > 0.0 && glyph.height > 0.0 {
                // Position the glyph quad relative to the cursor.
                let x = cursor_x + glyph.bearing_x * scale;
                let y = cursor_y + (glyph.bearing_y - glyph.height) * scale;
                let w = glyph.width * scale;
                let h = glyph.height * scale;

                // Build a transform for this glyph: translate + scale relative to parent transform.
                let glyph_transform = *transform
                    * Mat4::from_scale_rotation_translation(
                        Vec3::new(w, h, 1.0),
                        glam::Quat::IDENTITY,
                        Vec3::new(x + w * 0.5, y + h * 0.5, 0.0),
                    );

                self.push_text_quad_to_batch(
                    &glyph_transform,
                    color,
                    tex_index,
                    &glyph.tex_coords,
                    entity_id,
                );
            }

            // Advance cursor: glyph advance + font kerning pair + user kerning offset.
            let mut advance = glyph.advance_x;
            if let Some(&next_ch) = chars.peek() {
                advance += font.kerning(ch, next_ch);
            }
            cursor_x += (advance + kerning) * scale;
        }
    }

    /// Draw a [`TextComponent`] using a pre-built transform matrix.
    pub fn draw_text_component(&self, transform: &Mat4, text: &TextComponent, entity_id: i32) {
        if let Some(font) = &text.font {
            self.draw_text_string(
                &text.text,
                transform,
                font,
                text.font_size,
                text.color,
                text.line_spacing,
                text.kerning,
                entity_id,
            );
        }
    }

    // -- GPU synchronization ---------------------------------------------------

    /// Wait for the GPU to finish all in-flight work.
    ///
    /// Call this before destroying resources that may still be referenced by
    /// pending command buffers (e.g. textures owned by a scene being replaced).
    pub fn wait_gpu_idle(&self) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }
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

    /// Begin a new scene — stores the view-projection matrix,
    /// saves the draw context, sets viewport/scissor, and resets the batch.
    pub(crate) fn begin_scene(&mut self, camera_vp: &Mat4, ctx: DrawContext) {
        let _timer = ProfileTimer::new("Renderer::begin_scene");
        self.view_projection = *camera_vp;
        self.draw_context = Some(ctx);
        RenderCommand::set_viewport(&self.api, &ctx);

        // Write VP matrix to the camera UBO for this frame.
        let camera_data = CameraData {
            view_projection: *camera_vp,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &camera_data as *const CameraData as *const u8,
                CameraData::SIZE,
            )
        };
        self.camera_ubo.update(ctx.current_frame, bytes);

        // Reset batch state for this frame.
        if let Some(data) = &self.renderer_2d {
            data.reset_batch();
        }
    }

    /// Returns the current view-projection matrix.
    pub fn view_projection(&self) -> Mat4 {
        self.view_projection
    }

    /// Override the view-projection matrix for the current scene.
    ///
    /// Call this between `begin_scene` / `end_scene` to change the camera
    /// used for subsequent draw calls. Used by [`Scene`](crate::scene::Scene)
    /// to render through the primary ECS camera entity.
    pub fn set_view_projection(&mut self, vp: Mat4) {
        self.view_projection = vp;

        // Update the camera UBO if we have an active draw context.
        if let Some(ctx) = self.draw_context {
            let camera_data = CameraData {
                view_projection: vp,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &camera_data as *const CameraData as *const u8,
                    CameraData::SIZE,
                )
            };
            self.camera_ubo.update(ctx.current_frame, bytes);
        }
    }

    /// End the current scene — flushes any pending batches (quads + circles + lines),
    /// snapshots stats, and clears the draw context.
    pub(crate) fn end_scene(&mut self) {
        let _timer = ProfileTimer::new("Renderer::end_scene");
        if let Some(data) = &self.renderer_2d {
            if let Some(ctx) = self.draw_context {
                // Flush any remaining quads.
                if data.has_pending_quads() {
                    data.flush_quads(
                        ctx.cmd_buf,
                        self.camera_ubo_ds[ctx.current_frame],
                        ctx.current_frame,
                    );
                }
                // Flush any remaining circles.
                if data.has_pending_circles() {
                    data.flush_circles(
                        ctx.cmd_buf,
                        self.camera_ubo_ds[ctx.current_frame],
                        ctx.current_frame,
                    );
                }
                // Flush any remaining lines.
                if data.has_pending_lines() {
                    data.flush_lines(
                        ctx.cmd_buf,
                        self.camera_ubo_ds[ctx.current_frame],
                        ctx.current_frame,
                        self.line_width,
                    );
                }
                // Flush any remaining text.
                if data.has_pending_text() {
                    data.flush_text(
                        ctx.cmd_buf,
                        self.camera_ubo_ds[ctx.current_frame],
                        ctx.current_frame,
                    );
                }
                // Flush any remaining sprite instances.
                if data.has_pending_instances() {
                    data.flush_instances(
                        ctx.cmd_buf,
                        self.camera_ubo_ds[ctx.current_frame],
                        ctx.current_frame,
                    );
                }
            }
            // Snapshot stats for this frame (available via stats_2d() until next end_scene).
            let quad_stats = data.quad_stats();
            let circle_stats = data.circle_stats();
            let line_stats = data.line_stats();
            let text_stats = data.text_stats();
            let instance_stats = data.instance_stats();
            self.last_stats_2d = Renderer2DStats {
                draw_calls: quad_stats.draw_calls
                    + circle_stats.draw_calls
                    + line_stats.draw_calls
                    + text_stats.draw_calls
                    + instance_stats.draw_calls,
                quad_count: quad_stats.quad_count
                    + circle_stats.quad_count
                    + line_stats.quad_count
                    + text_stats.quad_count
                    + instance_stats.quad_count,
            };
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
            self.camera_ubo_ds[ctx.current_frame],
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
            self.camera_ubo_ds[ctx.current_frame],
            transform,
            None,
            Some(texture.descriptor_set()),
        );
    }

    // -- GPU Particle System ------------------------------------------------

    /// Returns `true` if a GPU particle system has been created.
    pub fn has_gpu_particle_system(&self) -> bool {
        self.gpu_particles.is_some()
    }

    /// Create a GPU-driven particle system with the given maximum particle count.
    /// Uses a compute shader for simulation and instanced rendering for drawing.
    pub fn create_gpu_particle_system(&mut self, max_particles: u32) -> Result<(), String> {
        let system = GpuParticleSystem::new(
            &self.allocator,
            &self.device,
            max_particles,
            self.pipeline_cache,
        )?;
        self.gpu_particles = Some(system);
        Ok(())
    }

    /// Queue a particle emission for the GPU particle system.
    /// Emissions are processed during the next compute dispatch (1-frame latency).
    pub fn emit_particles(&mut self, props: &crate::particle_system::ParticleProps) {
        if let Some(ps) = &mut self.gpu_particles {
            ps.emit(props);
        }
    }

    /// Record compute dispatch commands for the GPU particle system.
    /// Must be called OUTSIDE a render pass (before `begin_scene`).
    pub(crate) fn dispatch_particle_compute(
        &mut self,
        cmd_buf: vk::CommandBuffer,
        current_frame: usize,
        dt: f32,
    ) {
        if let Some(ps) = &mut self.gpu_particles {
            ps.dispatch(cmd_buf, current_frame, dt);
        }
    }

    /// Render GPU particles using the instanced sprite pipeline.
    /// Must be called INSIDE a render pass (between `begin_scene`/`end_scene`).
    pub fn render_gpu_particles(&self) {
        let (Some(ps), Some(data)) = (&self.gpu_particles, &self.renderer_2d) else {
            return;
        };
        let ctx = self
            .draw_context
            .expect("render_gpu_particles called outside begin_scene/end_scene");
        ps.render(
            ctx.cmd_buf,
            ctx.current_frame,
            self.camera_ubo_ds[ctx.current_frame],
            data,
        );
    }

    /// Returns true if a GPU particle system has been created.
    pub fn has_gpu_particles(&self) -> bool {
        self.gpu_particles.is_some()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Wait for any pending async texture uploads before tearing down.
        self.transfer_batch.wait_all();
        self.save_pipeline_cache();
        // Drop Renderer2DData (owns white_texture) before destroying the
        // descriptor pool, so Texture2D::Drop can still free its descriptor set.
        drop(self.renderer_2d.take());
        unsafe {
            self.device
                .destroy_pipeline_cache(self.pipeline_cache, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.texture_descriptor_set_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.camera_ubo_ds_layout, None);
            // camera_ubo buffers/memory cleaned up by UniformBuffer::Drop.
        }
    }
}
