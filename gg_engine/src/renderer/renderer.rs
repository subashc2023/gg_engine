use std::path::Path;
use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::buffer::{IndexBuffer, VertexBuffer};
use super::camera_system::CameraSystem;
use super::draw_context::DrawContext;
use super::font::{Font, FontCpuData};
use super::framebuffer::{Framebuffer, FramebufferSpec};
use super::gpu_allocation::GpuAllocator;
use super::gpu_particle_system::GpuParticleSystem;
use super::lighting::{LightEnvironment, LightingSystem};
use super::material::{MaterialHandle, MaterialLibrary};
use super::pipeline::{self, Pipeline};
use super::render_command::RenderCommand;
use super::renderer_2d::{
    BatchCircleVertex, BatchLineVertex, BatchQuadVertex, Renderer2DData, Renderer2DStats,
    SpriteInstanceData,
};
use super::renderer_api::{RendererAPI, VulkanRendererAPI};
use super::shader::Shader;
use super::gpu_profiling::GpuProfiler;
use super::postprocess::PostProcessPipeline;
use super::shadow_map::{self, ShadowMapSystem};
use super::sub_texture::SubTexture2D;
use super::texture::TextureCpuData;
use super::texture::TextureSpecification;
use super::texture::{Texture2D, TransferBatch};
use super::vertex_array::VertexArray;
use super::VulkanContext;

use crate::profiling::ProfileTimer;
use crate::scene::{CircleRendererComponent, SpriteRendererComponent, TextComponent};

// ---------------------------------------------------------------------------
// WireframeMode — editor wireframe visualization
// ---------------------------------------------------------------------------

/// Controls how wireframe rendering is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WireframeMode {
    /// Normal filled rendering (default).
    #[default]
    Off,
    /// Full wireframe: all geometry rendered as lines only.
    WireOnly,
    /// Wireframe overlay: geometry rendered filled first, then wireframe on top.
    Overlay,
}

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

    // Per-frame per-viewport camera UBO (VP matrix + time).
    camera: CameraSystem,

    // Per-frame per-viewport material UBO (PBR surface properties).
    material_library: MaterialLibrary,

    // Per-frame per-viewport lighting UBO (directional + point lights).
    lighting: LightingSystem,

    // Camera eye position for specular lighting (set each frame by the caller).
    camera_position: Vec3,

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

    // Maximum MSAA sample count supported by the GPU.
    max_msaa_samples: vk::SampleCountFlags,

    // Lazily initialized default 3D mesh pipelines (used by Scene::render_scene).
    mesh3d_pipeline: Option<Arc<Pipeline>>,
    mesh3d_offscreen_pipeline: Option<Arc<Pipeline>>,
    mesh3d_use_offscreen: bool,

    // Wireframe 3D mesh pipeline variants (PolygonMode::LINE).
    mesh3d_wireframe_pipeline: Option<Arc<Pipeline>>,
    mesh3d_wireframe_offscreen_pipeline: Option<Arc<Pipeline>>,

    // Current wireframe rendering mode.
    wireframe_mode: WireframeMode,
    // Tracks whether wireframe is active for the current draw pass
    // (set by set_wireframe_active, used by mesh3d_pipeline).
    wireframe_active: bool,

    // Offscreen render pass info (stored when offscreen pipelines are created).
    offscreen_render_pass: Option<vk::RenderPass>,
    offscreen_color_attachment_count: u32,
    offscreen_sample_count: vk::SampleCountFlags,

    // Shadow mapping system (lazily initialized).
    shadow_map: Option<ShadowMapSystem>,
    shadow_pipeline: Option<Arc<Pipeline>>,

    // GPU timestamp profiler.
    gpu_profiler: Option<GpuProfiler>,

    // Post-processing pipeline (lazily initialized when scene framebuffer is available).
    postprocess: Option<PostProcessPipeline>,
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

        // Create descriptor pool for texture samplers + camera/material/lighting/shadow UBO sets.
        // Camera + material + lighting + shadow-camera UBOs each need one
        // descriptor set per (frame, viewport) slot. Shadow also needs sampler sets.
        let ubo_slot_count = (super::MAX_FRAMES_IN_FLIGHT * super::MAX_VIEWPORTS) as u32;
        let total_ubo_sets = ubo_slot_count * 4; // camera + material + lighting + shadow camera
        let total_sampler_descriptors = 100 + ubo_slot_count; // textures + shadow map samplers
        let total_sets = total_sampler_descriptors + total_ubo_sets;
        let pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: total_sampler_descriptors,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER,
                descriptor_count: total_ubo_sets,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(total_sets)
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

        // Camera UBO: descriptor set layout, per-slot buffers, descriptor sets.
        let camera = CameraSystem::new(allocator, device, descriptor_pool)?;

        // Material UBO: descriptor set layout, per-slot buffers, descriptor sets.
        let material_library = MaterialLibrary::new(allocator, device, descriptor_pool)?;

        // Lighting UBO: descriptor set layout, per-slot buffers, descriptor sets.
        let lighting = LightingSystem::new(allocator, device, descriptor_pool)?;

        // Shadow map system: depth image, render pass, UBO, descriptor sets.
        let shadow_map = ShadowMapSystem::new(
            allocator,
            device,
            descriptor_pool,
            depth_format,
            shadow_map::DEFAULT_SHADOW_MAP_SIZE,
            shadow_map::DEFAULT_SHADOW_MAP_SIZE,
            command_pool,
            vk_ctx.graphics_queue(),
        )?;

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
            device: device.clone(),
            render_pass,
            graphics_queue: vk_ctx.graphics_queue(),
            command_pool,
            allocator: allocator.clone(),
            descriptor_pool,
            texture_descriptor_set_layout,
            camera,
            material_library,
            lighting,
            camera_position: Vec3::ZERO,
            color_format,
            depth_format,
            pipeline_cache,
            renderer_2d: None,
            line_width: 4.0,
            last_stats_2d: Renderer2DStats::default(),
            transfer_batch,
            gpu_particles: None,
            max_msaa_samples: vk_ctx.max_msaa_samples(),
            mesh3d_pipeline: None,
            mesh3d_offscreen_pipeline: None,
            mesh3d_use_offscreen: false,
            mesh3d_wireframe_pipeline: None,
            mesh3d_wireframe_offscreen_pipeline: None,
            wireframe_mode: WireframeMode::Off,
            wireframe_active: false,
            offscreen_render_pass: None,
            offscreen_color_attachment_count: 0,
            offscreen_sample_count: vk::SampleCountFlags::TYPE_1,
            shadow_map: Some(shadow_map),
            shadow_pipeline: None,
            gpu_profiler: None,
            postprocess: None,
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
            self.camera.ds_layout(),
            &[],
            blend_enable,
            self.pipeline_cache,
            vk::SampleCountFlags::TYPE_1,
            false,
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
            self.camera.ds_layout(),
            &[self.texture_descriptor_set_layout],
            true,
            self.pipeline_cache,
            vk::SampleCountFlags::TYPE_1,
            false,
        )?))
    }

    /// Create a graphics pipeline for 3D mesh rendering.
    ///
    /// Configurable face culling, depth testing, and blend mode. Pipeline
    /// layout includes camera UBO (set 0), bindless textures (set 1), and
    /// material UBO (set 2). Uses single-color-attachment by default;
    /// pass `color_attachment_count > 1` for offscreen framebuffers with
    /// entity ID attachments.
    #[allow(clippy::too_many_arguments)]
    pub fn create_3d_pipeline(
        &self,
        shader: &Shader,
        vertex_layout: &super::BufferLayout,
        cull_mode: super::CullMode,
        depth_config: super::DepthConfig,
        blend_mode: super::BlendMode,
        color_attachment_count: u32,
        msaa: super::MsaaSamples,
    ) -> Result<Arc<Pipeline>, String> {
        let shadow_ds_layout = self
            .shadow_map
            .as_ref()
            .expect("Shadow map system not initialized")
            .ds_layout();
        Ok(Arc::new(pipeline::create_3d_pipeline(
            &self.device,
            shader,
            vertex_layout,
            self.render_pass,
            self.camera.ds_layout(),
            &[
                self.texture_descriptor_set_layout,
                self.material_library.ds_layout(),
                self.lighting.ds_layout(),
                shadow_ds_layout,
            ],
            cull_mode,
            depth_config,
            blend_mode,
            color_attachment_count,
            self.pipeline_cache,
            msaa.to_vk(),
            false,
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

    /// Create a texture from raw RGBA8 pixel data with custom specification.
    pub fn create_texture_from_rgba8_with_spec(
        &self,
        width: u32,
        height: u32,
        pixels: &[u8],
        spec: TextureSpecification,
    ) -> Result<Texture2D, String> {
        let mut texture = Texture2D::from_rgba8_with_spec(
            &self.resources(),
            &self.allocator,
            width,
            height,
            pixels,
            &spec,
        )?;
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
        samples: vk::SampleCountFlags,
    ) -> Result<(), String> {
        // Store offscreen render pass info for lazy 3D pipeline creation.
        self.offscreen_render_pass = Some(render_pass);
        self.offscreen_color_attachment_count = color_attachment_count;
        self.offscreen_sample_count = samples;
        // Invalidate cached offscreen mesh3d pipelines (render pass may have changed).
        self.mesh3d_offscreen_pipeline = None;
        self.mesh3d_wireframe_offscreen_pipeline = None;

        if let Some(data) = &mut self.renderer_2d {
            data.create_offscreen_pipeline(
                &self.device,
                render_pass,
                self.camera.ds_layout(),
                color_attachment_count,
                self.pipeline_cache,
                samples,
            )?;
        }
        Ok(())
    }

    /// Return the maximum MSAA sample count supported by the GPU.
    pub fn max_msaa_samples(&self) -> vk::SampleCountFlags {
        self.max_msaa_samples
    }

    /// Access the material library (immutable).
    pub fn material_library(&self) -> &MaterialLibrary {
        &self.material_library
    }

    /// Access the material library (mutable).
    pub fn material_library_mut(&mut self) -> &mut MaterialLibrary {
        &mut self.material_library
    }

    /// Write a material's GPU data to the UBO for the current frame/viewport.
    pub fn write_material(
        &self,
        handle: &MaterialHandle,
        current_frame: usize,
        viewport_index: usize,
    ) {
        self.material_library
            .write_material_ubo(handle, current_frame, viewport_index);
    }

    /// Upload lighting environment data to the GPU for the current frame/viewport.
    ///
    /// Call this once before rendering 3D meshes each frame. If not called,
    /// the previous frame's light data remains in the UBO.
    pub fn upload_lights(&self, env: &LightEnvironment) {
        if let Some(ctx) = self.draw_context {
            let gpu_data = env.to_gpu_data();
            self.lighting
                .write_ubo(&gpu_data, ctx.current_frame, ctx.viewport_index);
        }
    }

    // -- Shadow Mapping ------------------------------------------------------

    /// Lazily create the shadow depth-only pipeline. The shadow map system
    /// itself is created eagerly in `Renderer::new` (needed for descriptor
    /// set layout at pipeline creation time).
    pub fn init_shadow_pipeline(&mut self) -> Result<(), String> {
        if self.shadow_pipeline.is_some() {
            return Ok(());
        }
        let sm = self
            .shadow_map
            .as_ref()
            .expect("Shadow map system not initialized");

        let shader = self.create_shader(
            "shadow",
            super::shaders::SHADOW_VERT_SPV,
            super::shaders::SHADOW_FRAG_SPV,
        )?;
        let pipeline = Arc::new(shadow_map::create_shadow_pipeline(
            &self.device,
            &shader,
            sm.render_pass(),
            sm.camera_ds_layout(),
            self.pipeline_cache,
        )?);

        self.shadow_pipeline = Some(pipeline);
        log::info!(target: "gg_engine", "Shadow pipeline created ({}x{})",
            sm.width(), sm.height());
        Ok(())
    }

    /// Returns `true` if the shadow pipeline is ready for rendering.
    pub fn has_shadow_pipeline(&self) -> bool {
        self.shadow_pipeline.is_some()
    }

    /// Begin the shadow depth-only render pass for a specific cascade.
    /// Must be called OUTSIDE the main render pass (before `begin_scene`).
    pub fn begin_shadow_pass(
        &self,
        light_vp: &Mat4,
        cascade: usize,
        cmd_buf: vk::CommandBuffer,
        _current_frame: usize,
        _viewport_index: usize,
    ) {
        let sm = self
            .shadow_map
            .as_ref()
            .expect("Shadow map not initialized");
        let pipeline = self
            .shadow_pipeline
            .as_ref()
            .expect("Shadow pipeline not initialized");

        let extent = vk::Extent2D {
            width: sm.width(),
            height: sm.height(),
        };

        let clear_value = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };

        let rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(sm.render_pass())
            .framebuffer(sm.framebuffer(cascade))
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(std::slice::from_ref(&clear_value));

        unsafe {
            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            // Set viewport and scissor to shadow map dimensions.
            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: sm.width() as f32,
                    height: sm.height() as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            self.device.cmd_set_scissor(
                cmd_buf,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                }],
            );

            // Bind shadow pipeline.
            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.pipeline(),
            );

            // Push light VP matrix (bytes [0..64]) via push constants.
            // This is recorded into the command buffer, so each cascade gets
            // its own VP even though they share the same command buffer.
            let vp_bytes = std::slice::from_raw_parts(
                light_vp as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                pipeline.layout(),
                vk::ShaderStageFlags::VERTEX,
                0,
                vp_bytes,
            );
        }
    }

    /// Submit a mesh to the shadow pass. Push the model matrix and draw.
    /// Must be called between `begin_shadow_pass` / `end_shadow_pass`.
    pub fn submit_shadow(
        &self,
        vertex_array: &VertexArray,
        transform: &Mat4,
        cmd_buf: vk::CommandBuffer,
    ) {
        let pipeline = self
            .shadow_pipeline
            .as_ref()
            .expect("Shadow pipeline not initialized");

        unsafe {
            // Push model matrix at offset 64 (after the light VP at offset 0).
            let transform_bytes = std::slice::from_raw_parts(
                transform as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                pipeline.layout(),
                vk::ShaderStageFlags::VERTEX,
                64, // offset: light VP is [0..64], model is [64..128]
                transform_bytes,
            );
        }

        vertex_array.bind(cmd_buf);
        let index_count = vertex_array
            .index_buffer()
            .expect("VertexArray has no index buffer")
            .count();
        unsafe {
            self.device
                .cmd_draw_indexed(cmd_buf, index_count, 1, 0, 0, 0);
        }
    }

    /// End the shadow depth-only render pass.
    pub fn end_shadow_pass(&self, cmd_buf: vk::CommandBuffer) {
        unsafe {
            self.device.cmd_end_render_pass(cmd_buf);
        }
    }

    /// Tell the batch renderer to use the offscreen pipeline (or switch back).
    pub(crate) fn use_offscreen_pipeline(&mut self, use_offscreen: bool) {
        if let Some(data) = &mut self.renderer_2d {
            data.set_use_offscreen(use_offscreen);
        }
        self.mesh3d_use_offscreen = use_offscreen;
    }

    /// Set the wireframe rendering mode.
    pub fn set_wireframe_mode(&mut self, mode: WireframeMode) {
        self.wireframe_mode = mode;
        let wireframe = mode == WireframeMode::WireOnly;
        self.wireframe_active = wireframe;
        if let Some(data) = &mut self.renderer_2d {
            data.set_wireframe(wireframe);
        }
    }

    /// Get the current wireframe rendering mode.
    pub fn wireframe_mode(&self) -> WireframeMode {
        self.wireframe_mode
    }

    /// Temporarily enable/disable wireframe for both 2D and 3D renderers.
    /// Used for the overlay pass (render filled, then wireframe on top).
    pub fn set_wireframe_active(&mut self, wireframe: bool) {
        self.wireframe_active = wireframe;
        if let Some(data) = &mut self.renderer_2d {
            data.set_wireframe(wireframe);
        }
    }

    // -- GPU Profiling -----------------------------------------------------------

    /// Initialize the GPU timestamp profiler.
    pub fn init_gpu_profiler(&mut self, timestamp_period_ns: f32) {
        match GpuProfiler::new(&self.device, timestamp_period_ns) {
            Ok(profiler) => {
                self.gpu_profiler = Some(profiler);
                log::info!(target: "gg_engine", "GPU profiler initialized");
            }
            Err(e) => log::warn!(target: "gg_engine", "Failed to create GPU profiler: {e}"),
        }
    }

    /// Access the GPU profiler (immutable).
    pub fn gpu_profiler(&self) -> Option<&GpuProfiler> {
        self.gpu_profiler.as_ref()
    }

    /// Access the GPU profiler (mutable).
    pub fn gpu_profiler_mut(&mut self) -> Option<&mut GpuProfiler> {
        self.gpu_profiler.as_mut()
    }

    // -- Post-Processing --------------------------------------------------------

    /// Create or recreate the post-processing pipeline for a scene framebuffer.
    pub fn init_postprocess(
        &mut self,
        scene_color_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.postprocess = Some(PostProcessPipeline::new(
            &self.device,
            &self.allocator,
            self.descriptor_pool,
            self.texture_descriptor_set_layout,
            scene_color_view,
            self.pipeline_cache,
            width,
            height,
        )?);
        log::info!(target: "gg_engine", "Post-processing pipeline created ({width}x{height})");
        Ok(())
    }

    /// Resize the post-processing pipeline to match a new viewport size.
    pub fn resize_postprocess(
        &mut self,
        scene_color_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if let Some(pp) = &mut self.postprocess {
            pp.resize(
                &self.allocator,
                scene_color_view,
                width,
                height,
            )
        } else {
            self.init_postprocess(scene_color_view, width, height)
        }
    }

    /// Access the post-processing pipeline (immutable).
    pub fn postprocess(&self) -> Option<&PostProcessPipeline> {
        self.postprocess.as_ref()
    }

    /// Access the post-processing pipeline (mutable).
    pub fn postprocess_mut(&mut self) -> Option<&mut PostProcessPipeline> {
        self.postprocess.as_mut()
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
            self.camera.ds_layout(),
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
            ctx.current_frame,
        );
    }

    /// Flush all pending batches (quads, circles, lines, text, instances).
    ///
    /// Used by [`Scene::render_scene`](crate::scene::Scene) to ensure correct
    /// cross-type draw ordering when the renderable type changes during sorted
    /// iteration. Empty batches are no-ops.
    pub fn flush_all_batches(&self) {
        if let Some(data) = &self.renderer_2d {
            if let Some(ctx) = self.draw_context {
                self.flush_pending(data, &ctx);
            }
        }
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
            anim_start_time: 0.0,
            anim_fps: 0.0,
            anim_start_frame: 0.0,
            anim_frame_count: 0.0,
            anim_columns: 0.0,
            anim_looping: 0.0,
            anim_cell_size: [0.0, 0.0],
            anim_tex_size: [0.0, 0.0],
        };

        if !data.push_instance(instance) {
            // Batch full — flush and retry.
            self.flush_instance_batch();
            data.push_instance(instance);
        }
    }

    /// Push a GPU-animated sprite instance.
    ///
    /// The vertex shader computes UV coordinates from the animation parameters
    /// and `u_time`, eliminating per-entity CPU UV computation.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_gpu_animated_sprite(
        &self,
        transform: &Mat4,
        color: Vec4,
        tex_index: f32,
        entity_id: i32,
        anim_start_time: f32,
        anim_fps: f32,
        anim_start_frame: f32,
        anim_frame_count: f32,
        anim_columns: f32,
        anim_looping: f32,
        anim_cell_size: [f32; 2],
        anim_tex_size: [f32; 2],
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
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tex_index,
            tiling_factor: 1.0,
            entity_id,
            anim_start_time,
            anim_fps,
            anim_start_frame,
            anim_frame_count,
            anim_columns,
            anim_looping,
            anim_cell_size,
            anim_tex_size,
        };

        if !data.push_instance(instance) {
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
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

    // -- Sub-textured / transformed quads ------------------------------------

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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
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

    /// Draw a wireframe box (12 edges) using a transform matrix and local-space bounds.
    /// For a unit cube use `min = (-0.5, -0.5, -0.5)`, `max = (0.5, 0.5, 0.5)`.
    /// Degenerate axes (min == max) produce a flat outline (e.g. a plane).
    pub fn draw_box_outline(
        &self,
        transform: &Mat4,
        bounds_min: Vec3,
        bounds_max: Vec3,
        color: Vec4,
        entity_id: i32,
    ) {
        let mn = bounds_min;
        let mx = bounds_max;
        let c = [
            *transform * Vec4::new(mn.x, mn.y, mn.z, 1.0),
            *transform * Vec4::new(mx.x, mn.y, mn.z, 1.0),
            *transform * Vec4::new(mx.x, mx.y, mn.z, 1.0),
            *transform * Vec4::new(mn.x, mx.y, mn.z, 1.0),
            *transform * Vec4::new(mn.x, mn.y, mx.z, 1.0),
            *transform * Vec4::new(mx.x, mn.y, mx.z, 1.0),
            *transform * Vec4::new(mx.x, mx.y, mx.z, 1.0),
            *transform * Vec4::new(mn.x, mx.y, mx.z, 1.0),
        ];
        let v = |i: usize| Vec3::new(c[i].x, c[i].y, c[i].z);

        // 4 bottom edges.
        self.draw_line(v(0), v(1), color, entity_id);
        self.draw_line(v(1), v(2), color, entity_id);
        self.draw_line(v(2), v(3), color, entity_id);
        self.draw_line(v(3), v(0), color, entity_id);
        // 4 top edges.
        self.draw_line(v(4), v(5), color, entity_id);
        self.draw_line(v(5), v(6), color, entity_id);
        self.draw_line(v(6), v(7), color, entity_id);
        self.draw_line(v(7), v(4), color, entity_id);
        // 4 vertical edges.
        self.draw_line(v(0), v(4), color, entity_id);
        self.draw_line(v(1), v(5), color, entity_id);
        self.draw_line(v(2), v(6), color, entity_id);
        self.draw_line(v(3), v(7), color, entity_id);
    }

    /// Get the current line width used for line rendering.
    pub fn line_width(&self) -> f32 {
        self.line_width
    }

    /// Set the line width used for line rendering.
    /// Requires `wideLines` device feature for values other than 1.0.
    /// On macOS (MoltenVK), wide lines are not supported — width is clamped to 1.0.
    /// Flushes any pending lines so they render at the previous width.
    pub fn set_line_width(&mut self, width: f32) {
        // macOS / MoltenVK does not support wide lines; clamp to 1.0.
        #[cfg(target_os = "macos")]
        let width = 1.0_f32;

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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
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
        self.draw_context = Some(ctx);
        RenderCommand::set_viewport(&self.api, &ctx);

        // Write VP matrix + time to the camera UBO for this (frame, viewport) slot.
        self.camera
            .set_view_projection(*camera_vp, ctx.current_frame, ctx.viewport_index);

        // Reset batch state for this frame.
        if let Some(data) = &self.renderer_2d {
            data.reset_batch();
        }
    }

    /// Returns the current view-projection matrix.
    pub fn view_projection(&self) -> Mat4 {
        self.camera.view_projection()
    }

    /// Override the view-projection matrix for the current scene.
    ///
    /// Call this between `begin_scene` / `end_scene` to change the camera
    /// used for subsequent draw calls. Used by [`Scene`](crate::scene::Scene)
    /// to render through the primary ECS camera entity.
    pub fn set_view_projection(&mut self, vp: Mat4) {
        if let Some(ctx) = self.draw_context {
            self.camera
                .set_view_projection(vp, ctx.current_frame, ctx.viewport_index);
        }
    }

    /// Set the camera eye position (used for specular lighting calculations).
    ///
    /// Call this after [`set_view_projection`] each frame. The position is
    /// automatically included in the lighting UBO when [`upload_lights`] is called.
    pub fn set_camera_position(&mut self, pos: Vec3) {
        self.camera_position = pos;
    }

    /// Get the current camera eye position.
    pub fn camera_position(&self) -> Vec3 {
        self.camera_position
    }

    /// Set the scene time used for GPU-computed animation.
    ///
    /// Call this before [`set_view_projection`] so the UBO includes the
    /// correct time value for the current frame.
    pub fn set_scene_time(&mut self, t: f32) {
        self.camera.set_scene_time(t);
    }

    /// End the current scene — flushes any pending batches (quads + circles + lines),
    /// snapshots stats, and clears the draw context.
    pub(crate) fn end_scene(&mut self) {
        let _timer = ProfileTimer::new("Renderer::end_scene");
        if let Some(data) = &self.renderer_2d {
            if let Some(ctx) = self.draw_context {
                self.flush_pending(data, &ctx);
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

    /// Flush all pending batch types (quads, circles, lines, text, instances).
    /// Empty batches are no-ops.
    fn flush_pending(&self, data: &Renderer2DData, ctx: &DrawContext) {
        let ds = self
            .camera
            .descriptor_set(ctx.current_frame, ctx.viewport_index);
        if data.has_pending_quads() {
            data.flush_quads(ctx.cmd_buf, ds, ctx.current_frame);
        }
        if data.has_pending_circles() {
            data.flush_circles(ctx.cmd_buf, ds, ctx.current_frame);
        }
        if data.has_pending_lines() {
            data.flush_lines(ctx.cmd_buf, ds, ctx.current_frame, self.line_width);
        }
        if data.has_pending_text() {
            data.flush_text(ctx.cmd_buf, ds, ctx.current_frame);
        }
        if data.has_pending_instances() {
            data.flush_instances(ctx.cmd_buf, ds, ctx.current_frame);
        }
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
            transform,
            None,
            Some(texture.descriptor_set()),
        );
    }

    /// Submit a 3D draw call: binds camera UBO (set 0), pushes model
    /// transform, optionally binds material UBO (set 2), draws indexed.
    pub fn submit_3d(
        &self,
        pipeline: &Pipeline,
        vertex_array: &VertexArray,
        transform: &Mat4,
        material_handle: Option<&super::MaterialHandle>,
        entity_id: i32,
    ) {
        let ctx = self
            .draw_context
            .expect("Renderer::submit_3d called outside begin_scene/end_scene");

        let cmd = ctx.cmd_buf;
        let device = &self.device;

        unsafe {
            // Bind the pipeline.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline.pipeline());

            // Set 0: camera UBO.
            let camera_ds = self
                .camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout(),
                0,
                &[camera_ds],
                &[],
            );

            // Set 1: bindless texture array (shared with 2D renderer).
            if let Some(ref r2d) = self.renderer_2d {
                let bindless_ds = r2d.bindless_descriptor_set(ctx.current_frame);
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline.layout(),
                    1,
                    &[bindless_ds],
                    &[],
                );
            }

            // Push model transform (offset 0, 64 bytes) + entity_id (offset 64, 4 bytes).
            let mut push_data = [0u8; 68];
            let transform_bytes = std::slice::from_raw_parts(
                transform as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            );
            push_data[..64].copy_from_slice(transform_bytes);
            push_data[64..68].copy_from_slice(&entity_id.to_ne_bytes());
            device.cmd_push_constants(
                cmd,
                pipeline.layout(),
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                &push_data,
            );

            // Material: push properties at offset 68 (48 bytes).
            // This ensures each draw call gets its own material data embedded in the
            // command stream, unlike the UBO which is shared across all draws.
            let mat_handle = material_handle
                .cloned()
                .unwrap_or_else(|| self.material_library.default_handle());
            if let Some(mat) = self.material_library.get(&mat_handle) {
                let albedo_tex_index: i32 = mat
                    .albedo_texture
                    .as_ref()
                    .map(|t| t.bindless_index() as i32)
                    .unwrap_or(-1);
                // 11 floats (44 bytes) of material data + 1 int (4 bytes) texture index = 48 bytes.
                let mut frag_data = [0u32; 12];
                frag_data[0] = mat.metallic.to_bits();
                frag_data[1] = mat.roughness.to_bits();
                frag_data[2] = mat.emissive_strength.to_bits();
                frag_data[3] = mat.albedo_color.x.to_bits();
                frag_data[4] = mat.albedo_color.y.to_bits();
                frag_data[5] = mat.albedo_color.z.to_bits();
                frag_data[6] = mat.albedo_color.w.to_bits();
                frag_data[7] = mat.emissive_color.x.to_bits();
                frag_data[8] = mat.emissive_color.y.to_bits();
                frag_data[9] = mat.emissive_color.z.to_bits();
                frag_data[10] = 0; // padding (.w of emissive_color vec4)
                frag_data[11] = albedo_tex_index as u32;
                let frag_bytes = std::slice::from_raw_parts(frag_data.as_ptr() as *const u8, 48);
                device.cmd_push_constants(
                    cmd,
                    pipeline.layout(),
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    68,
                    frag_bytes,
                );
            }

            // Set 2: material descriptor set (still bound for pipeline layout compatibility).
            let material_ds = self
                .material_library
                .descriptor_set(ctx.current_frame, ctx.viewport_index);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout(),
                2,
                &[material_ds],
                &[],
            );

            // Set 3: lighting UBO.
            let lighting_ds = self
                .lighting
                .descriptor_set(ctx.current_frame, ctx.viewport_index);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout(),
                3,
                &[lighting_ds],
                &[],
            );

            // Set 4: shadow map (if initialized).
            if let Some(ref sm) = self.shadow_map {
                let shadow_ds = sm.descriptor_set(ctx.current_frame, ctx.viewport_index);
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline.layout(),
                    4,
                    &[shadow_ds],
                    &[],
                );
            }
        }

        // Bind vertex/index buffers and draw.
        vertex_array.bind(cmd);
        let index_count = vertex_array
            .index_buffer()
            .expect("VertexArray has no index buffer")
            .count();
        unsafe {
            device.cmd_draw_indexed(cmd, index_count, 1, 0, 0, 0);
        }
    }

    // -- Default 3D Mesh Pipeline -------------------------------------------

    /// Get or lazily create the default mesh3d pipeline for scene rendering.
    ///
    /// Uses the built-in `mesh3d` shader with backface culling, standard
    /// depth testing, and opaque blending. Automatically selects the
    /// offscreen or swapchain variant based on `use_offscreen_pipeline()`.
    pub fn mesh3d_pipeline(&mut self) -> Result<Arc<Pipeline>, String> {
        self.mesh3d_pipeline_inner(self.wireframe_active)
    }

    /// Get the wireframe variant of the mesh3d pipeline (for overlay pass).
    pub fn mesh3d_wireframe_pipeline(&mut self) -> Result<Arc<Pipeline>, String> {
        self.mesh3d_pipeline_inner(true)
    }

    fn mesh3d_pipeline_inner(&mut self, wireframe: bool) -> Result<Arc<Pipeline>, String> {
        // Use the bindless texture descriptor set layout (from Renderer2DData)
        // so that 3D meshes can sample from the shared bindless texture array.
        let bindless_layout = self
            .renderer_2d
            .as_ref()
            .map(|r2d| r2d.bindless_ds_layout())
            .unwrap_or(self.texture_descriptor_set_layout);

        if self.mesh3d_use_offscreen {
            // Select the appropriate cached pipeline.
            let cached = if wireframe {
                &self.mesh3d_wireframe_offscreen_pipeline
            } else {
                &self.mesh3d_offscreen_pipeline
            };
            if let Some(ref pipeline) = cached {
                return Ok(Arc::clone(pipeline));
            }
            let offscreen_rp = self
                .offscreen_render_pass
                .ok_or("No offscreen render pass set for mesh3d pipeline")?;
            let shader = self.create_shader(
                "mesh3d",
                super::shaders::MESH3D_VERT_SPV,
                super::shaders::MESH3D_FRAG_SPV,
            )?;
            let vertex_layout = super::mesh::Mesh::vertex_layout();
            let shadow_ds_layout = self
                .shadow_map
                .as_ref()
                .expect("Shadow map system not initialized")
                .ds_layout();
            let pipeline = Arc::new(pipeline::create_3d_pipeline(
                &self.device,
                &shader,
                &vertex_layout,
                offscreen_rp,
                self.camera.ds_layout(),
                &[
                    bindless_layout,
                    self.material_library.ds_layout(),
                    self.lighting.ds_layout(),
                    shadow_ds_layout,
                ],
                super::CullMode::Back,
                super::DepthConfig::STANDARD_3D,
                super::BlendMode::Opaque,
                self.offscreen_color_attachment_count,
                self.pipeline_cache,
                self.offscreen_sample_count,
                wireframe,
            )?);
            if wireframe {
                self.mesh3d_wireframe_offscreen_pipeline = Some(Arc::clone(&pipeline));
            } else {
                self.mesh3d_offscreen_pipeline = Some(Arc::clone(&pipeline));
            }
            Ok(pipeline)
        } else {
            let cached = if wireframe {
                &self.mesh3d_wireframe_pipeline
            } else {
                &self.mesh3d_pipeline
            };
            if let Some(ref pipeline) = cached {
                return Ok(Arc::clone(pipeline));
            }
            let shader = self.create_shader(
                "mesh3d_swapchain",
                super::shaders::MESH3D_SWAPCHAIN_VERT_SPV,
                super::shaders::MESH3D_SWAPCHAIN_FRAG_SPV,
            )?;
            let vertex_layout = super::mesh::Mesh::vertex_layout();
            let shadow_ds_layout = self
                .shadow_map
                .as_ref()
                .expect("Shadow map system not initialized")
                .ds_layout();
            let pipeline = Arc::new(pipeline::create_3d_pipeline(
                &self.device,
                &shader,
                &vertex_layout,
                self.render_pass,
                self.camera.ds_layout(),
                &[
                    bindless_layout,
                    self.material_library.ds_layout(),
                    self.lighting.ds_layout(),
                    shadow_ds_layout,
                ],
                super::CullMode::Back,
                super::DepthConfig::STANDARD_3D,
                super::BlendMode::Opaque,
                1,
                self.pipeline_cache,
                vk::SampleCountFlags::TYPE_1,
                wireframe,
            )?);
            if wireframe {
                self.mesh3d_wireframe_pipeline = Some(Arc::clone(&pipeline));
            } else {
                self.mesh3d_pipeline = Some(Arc::clone(&pipeline));
            }
            Ok(pipeline)
        }
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
            self.camera
                .descriptor_set(ctx.current_frame, ctx.viewport_index),
            data,
        );
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
        // Drop GPU particle system (owns its own descriptor pool/layout).
        drop(self.gpu_particles.take());
        // Drop post-processing pipeline (owns its own descriptor pool/images).
        drop(self.postprocess.take());
        // Drop GPU profiler (owns query pools).
        drop(self.gpu_profiler.take());
        // Drop shadow pipeline before shadow map (pipeline references render pass).
        drop(self.shadow_pipeline.take());
        // Drop shadow map system (owns descriptor set layouts, render pass, image).
        drop(self.shadow_map.take());
        unsafe {
            self.device
                .destroy_pipeline_cache(self.pipeline_cache, None);
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.texture_descriptor_set_layout, None);
            // CameraSystem::Drop handles camera_ubo_ds_layout + UBO buffer cleanup.
            // Camera descriptor sets are freed by the pool destruction above.
        }
    }
}
