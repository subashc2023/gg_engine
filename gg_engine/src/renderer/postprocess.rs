use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::pipeline::Pipeline;
use super::shader::Shader;

use crate::error::{EngineError, EngineResult};
use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// TonemappingMode
// ---------------------------------------------------------------------------

/// Tone mapping operator for HDR-to-LDR conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TonemappingMode {
    /// No tone mapping (pass-through).
    #[default]
    None,
    /// ACES filmic tone curve (industry standard).
    ACES,
    /// Reinhard tone mapping (simple, preserves color ratios).
    Reinhard,
}

impl TonemappingMode {
    pub const ALL: &[Self] = &[Self::None, Self::ACES, Self::Reinhard];

    fn to_int(self) -> i32 {
        match self {
            Self::None => 0,
            Self::ACES => 1,
            Self::Reinhard => 2,
        }
    }
}

impl std::fmt::Display for TonemappingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::ACES => write!(f, "ACES"),
            Self::Reinhard => write!(f, "Reinhard"),
        }
    }
}

// ---------------------------------------------------------------------------
// Push constant structures (must match GLSL layout exactly)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct DownsamplePushConstants {
    texel_size: [f32; 2],
    threshold: f32,
    first_pass: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UpsamplePushConstants {
    texel_size: [f32; 2],
    filter_radius: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CompositePushConstants {
    bloom_intensity: f32,
    exposure: f32,
    contrast: f32,
    saturation: f32,
    tonemapping_mode: i32,
    apply_shadow: i32,
    _pad0: f32,
    _pad1: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BilateralBlurPushConstants {
    texel_size: [f32; 2],
    direction: [f32; 2],
    near_plane: f32,
    far_plane: f32,
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ContactShadowPushConstants {
    inv_view_projection: [f32; 16], // 64 bytes
    view_projection: [f32; 16],     // 64 bytes
    light_direction: [f32; 4],      // 16 bytes: xyz = dir toward light, w = 0
    max_distance: f32,              // 4 bytes
    thickness: f32,                 // 4 bytes (world-space units)
    intensity: f32,                 // 4 bytes
    step_count: i32,                // 4 bytes
    near_plane: f32,                // 4 bytes
    far_plane: f32,                 // 4 bytes
    debug_mode: i32,                // 4 bytes: 0=normal, 1=depth, 2=raw, 3=precision
    _pad1: f32,                     // 4 bytes padding to 16-byte alignment
}
// Total: 176 bytes (within 256-byte push constant limit on desktop GPUs)

// ---------------------------------------------------------------------------
// Internal image for intermediate render targets
// ---------------------------------------------------------------------------

struct PostProcessImage {
    image: vk::Image,
    _allocation: GpuAllocation,
    image_view: vk::ImageView,
    framebuffer: vk::Framebuffer,
    descriptor_set: vk::DescriptorSet,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// PostProcessPipeline
// ---------------------------------------------------------------------------

/// Number of bloom mip levels (each half the previous resolution).
const BLOOM_MIP_LEVELS: usize = 4;

/// Internal image format for all post-processing render targets.
/// 16-bit float preserves HDR range through bloom and tone mapping,
/// eliminating 8-bit color banding on smooth gradients.
const PP_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// Post-processing pipeline: bloom, tone mapping, and color grading.
///
/// Created lazily when enabled. Operates on the scene's offscreen framebuffer
/// color output, writing to an internal output image that can be registered
/// with egui for viewport display.
pub struct PostProcessPipeline {
    device: ash::Device,

    // Render passes
    store_pass: vk::RenderPass, // LOAD_OP_DONT_CARE (downsample + composite)
    blend_pass: vk::RenderPass, // LOAD_OP_LOAD (upsample, additive)

    // Pipelines
    downsample_pipeline: Pipeline,
    upsample_pipeline: Pipeline,
    composite_pipeline: Pipeline,

    // Contact shadows pipeline + intermediate images
    contact_shadow_pipeline: Option<Pipeline>,
    contact_shadowed: Option<PostProcessImage>, // shadow factor output (ping-pong A)
    shadow_temp: Option<PostProcessImage>,      // bilateral blur intermediate (ping-pong B)
    bilateral_blur_pipeline: Option<Pipeline>,
    /// Descriptor set for the 1x depth (either direct or resolved from MSAA).
    depth_ds: Option<vk::DescriptorSet>,
    depth_sampler: Option<vk::Sampler>,
    /// Descriptor set for the G-buffer normal attachment.
    normal_ds: Option<vk::DescriptorSet>,

    // MSAA depth resolve resources (only when MSAA enabled).
    depth_resolve_pipeline: Option<Pipeline>,
    msaa_depth_ds: Option<vk::DescriptorSet>,
    resolved_depth: Option<PostProcessImage>,

    // Per-frame contact shadow data (set by caller before execute).
    cs_inv_vp: [f32; 16],
    cs_vp: [f32; 16],
    cs_light_dir: [f32; 3],
    cs_near: f32,
    cs_far: f32,
    cs_has_light: bool,

    // Sampler layout (1 combined image sampler at binding 0)
    sampler_ds_layout: vk::DescriptorSetLayout,
    ds_pool: vk::DescriptorPool,

    // Linear sampler used by all passes
    linear_sampler: vk::Sampler,

    // Pipeline cache for shader hot-reload
    pipeline_cache: vk::PipelineCache,

    // Intermediate images
    bloom_mips: Vec<PostProcessImage>,
    output: PostProcessImage,

    // Descriptor set for the scene color input (created when scene DS is provided)
    scene_ds: vk::DescriptorSet,

    // Public settings
    pub enabled: bool,
    pub bloom_enabled: bool,
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,
    pub bloom_filter_radius: f32,
    pub tonemapping: TonemappingMode,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,

    // Contact shadow settings
    pub contact_shadows_enabled: bool,
    pub contact_shadows_max_distance: f32,
    pub contact_shadows_thickness: f32,
    pub contact_shadows_intensity: f32,
    pub contact_shadows_step_count: i32,
    pub contact_shadows_debug: i32,

    width: u32,
    height: u32,
}

impl PostProcessPipeline {
    /// Create the post-processing pipeline.
    ///
    /// `scene_color_view` — offscreen framebuffer's color attachment image view.
    /// `scene_depth_view` — 1x depth image view (non-MSAA), or `None` if MSAA.
    /// `msaa_depth_view` — MSAA depth image view (when MSAA enabled), or `None`.
    /// `scene_normal_view` — G-buffer normal attachment view, or `None`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &ash::Device,
        allocator: &Arc<Mutex<GpuAllocator>>,
        descriptor_pool: vk::DescriptorPool,
        texture_ds_layout: vk::DescriptorSetLayout,
        scene_color_view: vk::ImageView,
        scene_depth_view: Option<vk::ImageView>,
        msaa_depth_view: Option<vk::ImageView>,
        scene_normal_view: Option<vk::ImageView>,
        pipeline_cache: vk::PipelineCache,
        width: u32,
        height: u32,
    ) -> EngineResult<Self> {
        let _timer = ProfileTimer::new("PostProcessPipeline::new");

        // Use 16-bit float for all internal images — preserves HDR range through
        // bloom and tone mapping, eliminating 8-bit color banding on smooth gradients.
        let pp_format = PP_FORMAT;

        // --- Render passes ---
        let store_pass = create_render_pass(device, pp_format, vk::AttachmentLoadOp::DONT_CARE)?;
        let blend_pass = create_render_pass(device, pp_format, vk::AttachmentLoadOp::LOAD)?;

        // --- Descriptor set layout (1 combined image sampler at binding 0) ---
        let sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);

        let sampler_ds_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default()
                    .bindings(std::slice::from_ref(&sampler_binding)),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP descriptor set layout: {e}")))?;

        // --- Descriptor pool ---
        // bloom mips + output + scene + contact_shadowed + shadow_temp + depth + msaa_depth + resolved_depth + normal
        let max_sets = (BLOOM_MIP_LEVELS + 8) as u32;
        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: max_sets,
        };
        let ds_pool = unsafe {
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .pool_sizes(std::slice::from_ref(&pool_size))
                    .max_sets(max_sets)
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP descriptor pool: {e}")))?;

        // --- Linear sampler ---
        let linear_sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .mipmap_mode(vk::SamplerMipmapMode::LINEAR),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP linear sampler: {e}")))?;

        // --- Create intermediate images ---
        let mut bloom_mips = Vec::with_capacity(BLOOM_MIP_LEVELS);
        let mut mip_w = width / 2;
        let mut mip_h = height / 2;
        for _ in 0..BLOOM_MIP_LEVELS {
            mip_w = mip_w.max(1);
            mip_h = mip_h.max(1);
            bloom_mips.push(create_pp_image(
                device,
                allocator,
                ds_pool,
                sampler_ds_layout,
                linear_sampler,
                store_pass,
                pp_format,
                mip_w,
                mip_h,
            )?);
            mip_w /= 2;
            mip_h /= 2;
        }

        // Output image (full resolution, for egui display).
        // Uses the TEXTURE descriptor set layout so it's compatible with egui-ash-renderer.
        let output = create_pp_image(
            device,
            allocator,
            descriptor_pool, // Use the main pool for egui compatibility
            texture_ds_layout,
            linear_sampler,
            store_pass,
            pp_format,
            width,
            height,
        )?;

        // --- Scene color descriptor set ---
        let scene_ds = allocate_and_write_ds(
            device,
            ds_pool,
            sampler_ds_layout,
            linear_sampler,
            scene_color_view,
        )?;

        // --- Shaders ---
        let downsample_shader = Shader::new(
            device,
            "bloom_downsample",
            super::shaders::BLOOM_DOWNSAMPLE_VERT_SPV,
            super::shaders::BLOOM_DOWNSAMPLE_FRAG_SPV,
        )?;
        let upsample_shader = Shader::new(
            device,
            "bloom_upsample",
            super::shaders::BLOOM_UPSAMPLE_VERT_SPV,
            super::shaders::BLOOM_UPSAMPLE_FRAG_SPV,
        )?;
        let composite_shader = Shader::new(
            device,
            "postprocess_composite",
            super::shaders::POSTPROCESS_COMPOSITE_VERT_SPV,
            super::shaders::POSTPROCESS_COMPOSITE_FRAG_SPV,
        )?;

        // --- Pipelines ---
        let downsample_pipeline = create_fullscreen_pipeline(
            device,
            &downsample_shader,
            store_pass,
            &[sampler_ds_layout],
            std::mem::size_of::<DownsamplePushConstants>() as u32,
            false, // no additive blending
            pipeline_cache,
        )?;

        let upsample_pipeline = create_fullscreen_pipeline(
            device,
            &upsample_shader,
            blend_pass,
            &[sampler_ds_layout],
            std::mem::size_of::<UpsamplePushConstants>() as u32,
            true, // additive blending
            pipeline_cache,
        )?;

        let composite_pipeline = create_fullscreen_pipeline(
            device,
            &composite_shader,
            store_pass,
            &[sampler_ds_layout, sampler_ds_layout, sampler_ds_layout], // scene + bloom + shadow
            std::mem::size_of::<CompositePushConstants>() as u32,
            false,
            pipeline_cache,
        )?;

        // --- Bilateral blur pipeline ---
        let bilateral_blur_shader = Shader::new(
            device,
            "bilateral_blur",
            super::shaders::BILATERAL_BLUR_VERT_SPV,
            super::shaders::BILATERAL_BLUR_FRAG_SPV,
        )?;
        let bilateral_blur_pipeline = create_fullscreen_pipeline(
            device,
            &bilateral_blur_shader,
            store_pass,
            &[sampler_ds_layout, sampler_ds_layout], // shadow + depth
            std::mem::size_of::<BilateralBlurPushConstants>() as u32,
            false,
            pipeline_cache,
        )?;

        // --- Contact shadows (requires either 1x depth or MSAA depth) ---
        let has_any_depth = scene_depth_view.is_some() || msaa_depth_view.is_some();

        // Nearest sampler for depth (exact values, no interpolation).
        let depth_sampler = if has_any_depth {
            Some(
                unsafe {
                    device.create_sampler(
                        &vk::SamplerCreateInfo::default()
                            .mag_filter(vk::Filter::NEAREST)
                            .min_filter(vk::Filter::NEAREST)
                            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                        None,
                    )
                }
                .map_err(|e| EngineError::Gpu(format!("Failed to create depth sampler: {e}")))?,
            )
        } else {
            None
        };

        // Contact shadow pipeline + intermediate image (shared by MSAA and non-MSAA paths).
        let (contact_shadow_pipeline, contact_shadowed, shadow_temp) = if has_any_depth {
            let cs_shader = Shader::new(
                device,
                "contact_shadows",
                super::shaders::CONTACT_SHADOWS_VERT_SPV,
                super::shaders::CONTACT_SHADOWS_FRAG_SPV,
            )?;
            let cs_pipeline = create_fullscreen_pipeline(
                device,
                &cs_shader,
                store_pass,
                &[sampler_ds_layout, sampler_ds_layout], // depth + normal
                std::mem::size_of::<ContactShadowPushConstants>() as u32,
                false,
                pipeline_cache,
            )?;
            let cs_image = create_pp_image(
                device,
                allocator,
                ds_pool,
                sampler_ds_layout,
                linear_sampler,
                store_pass,
                pp_format,
                width,
                height,
            )?;
            let cs_temp = create_pp_image(
                device,
                allocator,
                ds_pool,
                sampler_ds_layout,
                linear_sampler,
                store_pass,
                pp_format,
                width,
                height,
            )?;
            (Some(cs_pipeline), Some(cs_image), Some(cs_temp))
        } else {
            (None, None, None)
        };

        // MSAA depth resolve resources (resolve MSAA depth → 1x R16F image).
        let (depth_resolve_pipeline, msaa_depth_ds, resolved_depth, depth_ds) =
            if let Some(msaa_dv) = msaa_depth_view {
                let resolve_shader = Shader::new(
                    device,
                    "depth_resolve",
                    super::shaders::DEPTH_RESOLVE_VERT_SPV,
                    super::shaders::DEPTH_RESOLVE_FRAG_SPV,
                )?;
                let resolve_pipeline = create_fullscreen_pipeline(
                    device,
                    &resolve_shader,
                    store_pass,
                    &[sampler_ds_layout], // MSAA depth input
                    0,                    // no push constants
                    false,
                    pipeline_cache,
                )?;
                let resolved = create_pp_image(
                    device,
                    allocator,
                    ds_pool,
                    sampler_ds_layout,
                    depth_sampler.unwrap(),
                    store_pass,
                    pp_format,
                    width,
                    height,
                )?;
                let msaa_ds = allocate_and_write_ds_with_layout(
                    device,
                    ds_pool,
                    sampler_ds_layout,
                    depth_sampler.unwrap(),
                    msaa_dv,
                    vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
                )?;
                // Contact shadows sample the resolved (1x) depth image.
                let d_ds = resolved.descriptor_set;
                (
                    Some(resolve_pipeline),
                    Some(msaa_ds),
                    Some(resolved),
                    Some(d_ds),
                )
            } else if let Some(depth_view) = scene_depth_view {
                // Non-MSAA: sample depth directly.
                let d_ds = allocate_and_write_ds_with_layout(
                    device,
                    ds_pool,
                    sampler_ds_layout,
                    depth_sampler.unwrap(),
                    depth_view,
                    vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
                )?;
                (None, None, None, Some(d_ds))
            } else {
                (None, None, None, None)
            };

        // --- Normal G-buffer descriptor set ---
        let normal_ds = if let Some(nv) = scene_normal_view {
            Some(allocate_and_write_ds(
                device,
                ds_pool,
                sampler_ds_layout,
                linear_sampler,
                nv,
            )?)
        } else {
            None
        };

        Ok(Self {
            device: device.clone(),
            store_pass,
            blend_pass,
            downsample_pipeline,
            upsample_pipeline,
            composite_pipeline,
            contact_shadow_pipeline,
            contact_shadowed,
            shadow_temp,
            bilateral_blur_pipeline: Some(bilateral_blur_pipeline),
            depth_ds,
            depth_sampler,
            normal_ds,
            depth_resolve_pipeline,
            msaa_depth_ds,
            resolved_depth,
            cs_inv_vp: [0.0; 16],
            cs_vp: [0.0; 16],
            cs_light_dir: [0.0; 3],
            cs_near: 0.1,
            cs_far: 1000.0,
            cs_has_light: false,
            sampler_ds_layout,
            ds_pool,
            linear_sampler,
            pipeline_cache,
            bloom_mips,
            output,
            scene_ds,
            enabled: true,
            bloom_enabled: true,
            bloom_threshold: 0.8,
            bloom_intensity: 0.3,
            bloom_filter_radius: 1.0,
            tonemapping: TonemappingMode::ACES,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            contact_shadows_enabled: false,
            contact_shadows_max_distance: 0.5,
            contact_shadows_thickness: 0.15,
            contact_shadows_intensity: 0.6,
            contact_shadows_step_count: 24,
            contact_shadows_debug: 0,
            width,
            height,
        })
    }

    /// Resize internal images to match a new viewport size.
    /// Call when the offscreen framebuffer is resized.
    #[allow(clippy::too_many_arguments)]
    pub fn resize(
        &mut self,
        allocator: &Arc<Mutex<GpuAllocator>>,
        scene_color_view: vk::ImageView,
        scene_depth_view: Option<vk::ImageView>,
        msaa_depth_view: Option<vk::ImageView>,
        scene_normal_view: Option<vk::ImageView>,
        width: u32,
        height: u32,
    ) -> EngineResult<()> {
        if width == self.width && height == self.height {
            // Size unchanged but input views may have changed (e.g. MSAA
            // framebuffer recreation at the same dimensions). Update only
            // the descriptor sets that reference external images.
            return self.rebind_input_views(
                allocator,
                scene_color_view,
                scene_depth_view,
                msaa_depth_view,
                scene_normal_view,
            );
        }

        // Wait for GPU to finish using old resources.
        unsafe {
            let _ = self.device.device_wait_idle();
        }

        let pp_format = PP_FORMAT;

        // --- Destroy old resources (free DS back to pools, destroy Vulkan objects) ---

        // Free old bloom mip descriptor sets from the internal pool and destroy Vulkan objects.
        // drain(..) moves ownership so GpuAllocation Drop frees the backing memory.
        for mip in self.bloom_mips.drain(..) {
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[mip.descriptor_set]);
                self.device.destroy_framebuffer(mip.framebuffer, None);
                self.device.destroy_image_view(mip.image_view, None);
                self.device.destroy_image(mip.image, None);
            }
        }

        // Free old scene descriptor set from the internal pool.
        unsafe {
            let _ = self
                .device
                .free_descriptor_sets(self.ds_pool, &[self.scene_ds]);
        }

        // Destroy old output Vulkan objects. Keep the descriptor set alive so
        // that egui primitives tessellated this frame still reference a valid
        // handle — we update it in-place to point to the new image below.
        let old_output_ds = self.output.descriptor_set;
        unsafe {
            self.device
                .destroy_framebuffer(self.output.framebuffer, None);
            self.device.destroy_image_view(self.output.image_view, None);
            self.device.destroy_image(self.output.image, None);
        }

        // --- Recreate all resources ---

        // Recreate bloom mips.
        let mut mip_w = width / 2;
        let mut mip_h = height / 2;
        for _ in 0..BLOOM_MIP_LEVELS {
            mip_w = mip_w.max(1);
            mip_h = mip_h.max(1);
            self.bloom_mips.push(create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                self.store_pass,
                pp_format,
                mip_w,
                mip_h,
            )?);
            mip_w /= 2;
            mip_h /= 2;
        }

        // Recreate output image/view/framebuffer, reusing the existing
        // descriptor set so the raw handle stays stable for egui.
        self.output = create_pp_image_reuse_ds(
            &self.device,
            allocator,
            old_output_ds,
            self.linear_sampler,
            self.store_pass,
            pp_format,
            width,
            height,
        )?;

        // Recreate scene descriptor set.
        self.scene_ds = allocate_and_write_ds(
            &self.device,
            self.ds_pool,
            self.sampler_ds_layout,
            self.linear_sampler,
            scene_color_view,
        )?;

        // --- Destroy old normal DS ---
        if let Some(n_ds) = self.normal_ds.take() {
            unsafe {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[n_ds]);
            }
        }

        // --- Destroy old contact shadow / resolve resources ---
        if let Some(cs_img) = self.contact_shadowed.take() {
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[cs_img.descriptor_set]);
                self.device.destroy_framebuffer(cs_img.framebuffer, None);
                self.device.destroy_image_view(cs_img.image_view, None);
                self.device.destroy_image(cs_img.image, None);
            }
        }
        if let Some(st_img) = self.shadow_temp.take() {
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[st_img.descriptor_set]);
                self.device.destroy_framebuffer(st_img.framebuffer, None);
                self.device.destroy_image_view(st_img.image_view, None);
                self.device.destroy_image(st_img.image, None);
            }
        }
        // Only free depth_ds if it's NOT the resolved_depth's DS (avoid double-free).
        let resolved_ds = self.resolved_depth.as_ref().map(|r| r.descriptor_set);
        if let Some(d_ds) = self.depth_ds.take() {
            if resolved_ds != Some(d_ds) {
                unsafe {
                    let _ = self.device.free_descriptor_sets(self.ds_pool, &[d_ds]);
                }
            }
        }
        if let Some(rd) = self.resolved_depth.take() {
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[rd.descriptor_set]);
                self.device.destroy_framebuffer(rd.framebuffer, None);
                self.device.destroy_image_view(rd.image_view, None);
                self.device.destroy_image(rd.image, None);
            }
        }
        if let Some(m_ds) = self.msaa_depth_ds.take() {
            unsafe {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[m_ds]);
            }
        }

        // --- Recreate contact shadow + resolve resources ---
        let has_any_depth = scene_depth_view.is_some() || msaa_depth_view.is_some();
        let depth_samp = self.depth_sampler.unwrap_or(self.linear_sampler);

        if has_any_depth && self.contact_shadow_pipeline.is_some() {
            self.contact_shadowed = Some(create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                self.store_pass,
                pp_format,
                width,
                height,
            )?);
            self.shadow_temp = Some(create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                self.store_pass,
                pp_format,
                width,
                height,
            )?);
        }

        if let Some(msaa_dv) = msaa_depth_view {
            // Create depth resolve pipeline if switching to MSAA at runtime.
            if self.depth_resolve_pipeline.is_none() {
                let resolve_shader = Shader::new(
                    &self.device,
                    "depth_resolve",
                    super::shaders::DEPTH_RESOLVE_VERT_SPV,
                    super::shaders::DEPTH_RESOLVE_FRAG_SPV,
                )?;
                self.depth_resolve_pipeline = Some(create_fullscreen_pipeline(
                    &self.device,
                    &resolve_shader,
                    self.store_pass,
                    &[self.sampler_ds_layout],
                    0,
                    false,
                    self.pipeline_cache,
                )?);
            }

            // MSAA: resolve depth → 1x intermediate, contact shadows sample that.
            let resolved = create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                self.store_pass,
                pp_format,
                width,
                height,
            )?;
            self.msaa_depth_ds = Some(allocate_and_write_ds_with_layout(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                msaa_dv,
                vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
            )?);
            self.depth_ds = Some(resolved.descriptor_set);
            self.resolved_depth = Some(resolved);
        } else if let Some(depth_view) = scene_depth_view {
            // Non-MSAA: sample depth directly.
            self.depth_ds = Some(allocate_and_write_ds_with_layout(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                depth_view,
                vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
            )?);
        }

        // --- Recreate normal DS ---
        if let Some(nv) = scene_normal_view {
            self.normal_ds = Some(allocate_and_write_ds(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                nv,
            )?);
        }

        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Update only the descriptor sets that reference external framebuffer
    /// images. Called when the framebuffer is recreated at the same size
    /// (e.g. MSAA sample count change) so the intermediate bloom/output images
    /// can stay as-is but the input bindings must point to the new views.
    fn rebind_input_views(
        &mut self,
        allocator: &Arc<Mutex<GpuAllocator>>,
        scene_color_view: vk::ImageView,
        scene_depth_view: Option<vk::ImageView>,
        msaa_depth_view: Option<vk::ImageView>,
        scene_normal_view: Option<vk::ImageView>,
    ) -> EngineResult<()> {
        unsafe {
            let _ = self.device.device_wait_idle();
        }

        // Update scene color DS in-place.
        update_ds(
            &self.device,
            self.scene_ds,
            self.linear_sampler,
            scene_color_view,
        );

        let has_any_depth = scene_depth_view.is_some() || msaa_depth_view.is_some();

        // --- Rebuild depth / MSAA-depth descriptor sets ---
        // Free old depth-related DSes.
        let resolved_ds = self.resolved_depth.as_ref().map(|r| r.descriptor_set);
        if let Some(d_ds) = self.depth_ds.take() {
            if resolved_ds != Some(d_ds) {
                unsafe {
                    let _ = self.device.free_descriptor_sets(self.ds_pool, &[d_ds]);
                }
            }
        }
        if let Some(rd) = self.resolved_depth.take() {
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[rd.descriptor_set]);
                self.device.destroy_framebuffer(rd.framebuffer, None);
                self.device.destroy_image_view(rd.image_view, None);
                self.device.destroy_image(rd.image, None);
            }
        }
        if let Some(m_ds) = self.msaa_depth_ds.take() {
            unsafe {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[m_ds]);
            }
        }

        // Create depth sampler if we now need one but didn't before.
        if has_any_depth && self.depth_sampler.is_none() {
            self.depth_sampler = Some(
                unsafe {
                    self.device.create_sampler(
                        &vk::SamplerCreateInfo::default()
                            .mag_filter(vk::Filter::NEAREST)
                            .min_filter(vk::Filter::NEAREST)
                            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                        None,
                    )
                }
                .map_err(|e| EngineError::Gpu(format!("Failed to create depth sampler: {e}")))?,
            );
        }
        let depth_samp = self.depth_sampler.unwrap_or(self.linear_sampler);

        // Recreate MSAA depth resolve or direct depth DS.
        if let Some(msaa_dv) = msaa_depth_view {
            // Create depth resolve pipeline if not yet available (switching
            // from non-MSAA to MSAA at runtime).
            if self.depth_resolve_pipeline.is_none() {
                let resolve_shader = Shader::new(
                    &self.device,
                    "depth_resolve",
                    super::shaders::DEPTH_RESOLVE_VERT_SPV,
                    super::shaders::DEPTH_RESOLVE_FRAG_SPV,
                )?;
                self.depth_resolve_pipeline = Some(create_fullscreen_pipeline(
                    &self.device,
                    &resolve_shader,
                    self.store_pass,
                    &[self.sampler_ds_layout],
                    0,
                    false,
                    self.pipeline_cache,
                )?);
            }

            let resolved = create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                self.store_pass,
                PP_FORMAT,
                self.width,
                self.height,
            )?;
            self.msaa_depth_ds = Some(allocate_and_write_ds_with_layout(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                msaa_dv,
                vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
            )?);
            self.depth_ds = Some(resolved.descriptor_set);
            self.resolved_depth = Some(resolved);
        } else if let Some(dv) = scene_depth_view {
            self.depth_ds = Some(allocate_and_write_ds_with_layout(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                depth_samp,
                dv,
                vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
            )?);
        }

        // --- Rebuild normal DS ---
        if let Some(n_ds) = self.normal_ds.take() {
            unsafe {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[n_ds]);
            }
        }
        if let Some(nv) = scene_normal_view {
            self.normal_ds = Some(allocate_and_write_ds(
                &self.device,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                nv,
            )?);
        }

        // --- Rebuild contact shadow intermediate images if depth availability changed ---
        let need_cs = has_any_depth && self.contact_shadow_pipeline.is_some();
        let has_cs = self.contact_shadowed.is_some();
        if need_cs && !has_cs {
            // Depth became available — create contact shadow images.
            self.contact_shadowed = Some(create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                self.store_pass,
                PP_FORMAT,
                self.width,
                self.height,
            )?);
            self.shadow_temp = Some(create_pp_image(
                &self.device,
                allocator,
                self.ds_pool,
                self.sampler_ds_layout,
                self.linear_sampler,
                self.store_pass,
                PP_FORMAT,
                self.width,
                self.height,
            )?);
        } else if !need_cs && has_cs {
            // Depth gone — tear down contact shadow images.
            if let Some(cs) = self.contact_shadowed.take() {
                unsafe {
                    let _ = self
                        .device
                        .free_descriptor_sets(self.ds_pool, &[cs.descriptor_set]);
                    self.device.destroy_framebuffer(cs.framebuffer, None);
                    self.device.destroy_image_view(cs.image_view, None);
                    self.device.destroy_image(cs.image, None);
                }
            }
            if let Some(st) = self.shadow_temp.take() {
                unsafe {
                    let _ = self
                        .device
                        .free_descriptor_sets(self.ds_pool, &[st.descriptor_set]);
                    self.device.destroy_framebuffer(st.framebuffer, None);
                    self.device.destroy_image_view(st.image_view, None);
                    self.device.destroy_image(st.image, None);
                }
            }
        }

        Ok(())
    }

    /// Set per-frame contact shadow data (camera matrices + light direction).
    /// Must be called before `execute()` each frame when contact shadows are active.
    pub fn set_contact_shadow_data(
        &mut self,
        inv_vp: glam::Mat4,
        vp: glam::Mat4,
        light_dir: glam::Vec3,
        near_plane: f32,
        far_plane: f32,
    ) {
        self.cs_inv_vp = inv_vp.to_cols_array();
        self.cs_vp = vp.to_cols_array();
        self.cs_light_dir = [light_dir.x, light_dir.y, light_dir.z];
        self.cs_near = near_plane;
        self.cs_far = far_plane;
        self.cs_has_light = true;
    }

    /// Clear per-frame contact shadow data (no directional light this frame).
    pub fn clear_contact_shadow_data(&mut self) {
        self.cs_has_light = false;
    }

    /// Execute the post-processing pipeline.
    ///
    /// Records render passes into `cmd_buf`. The scene color must be in
    /// `SHADER_READ_ONLY_OPTIMAL` layout (after the offscreen scene pass barrier).
    /// After execution, the output image is in `SHADER_READ_ONLY_OPTIMAL`.
    pub fn execute(&self, cmd_buf: vk::CommandBuffer) {
        let _timer = ProfileTimer::new("PostProcess::execute");

        // Resolve MSAA depth to 1x if needed (before contact shadows).
        if self.depth_resolve_pipeline.is_some() && self.contact_shadows_active() {
            self.execute_depth_resolve(cmd_buf);
        }

        // Contact shadows: output shadow factor, then bilateral blur.
        let has_shadow = if self.contact_shadows_active() {
            self.execute_contact_shadows(cmd_buf);
            // Bilateral blur: H pass → shadow_temp, V pass → contact_shadowed.
            // Skip blur in debug modes — it only processes .r and would corrupt RGB debug output.
            if self.contact_shadows_debug == 0
                && self.bilateral_blur_pipeline.is_some()
                && self.shadow_temp.is_some()
            {
                self.execute_bilateral_blur(cmd_buf);
            }
            true
        } else {
            false
        };

        // Bloom always reads from raw scene (shadow applied in composite).
        if self.bloom_enabled && self.bloom_mips.len() == BLOOM_MIP_LEVELS {
            self.execute_bloom_downsample(cmd_buf, self.scene_ds);
            self.execute_bloom_upsample(cmd_buf);
        }
        self.execute_composite(cmd_buf, has_shadow);
    }

    /// Returns true when contact shadows should run this frame.
    fn contact_shadows_active(&self) -> bool {
        self.contact_shadows_enabled
            && self.cs_has_light
            && self.contact_shadow_pipeline.is_some()
            && self.contact_shadowed.is_some()
            && self.depth_ds.is_some()
            && self.normal_ds.is_some()
    }

    /// The output image's descriptor set handle for egui texture registration.
    pub fn output_egui_handle(&self) -> u64 {
        self.output.descriptor_set.as_raw()
    }

    /// Current pipeline dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Hot-reload post-processing shaders from compiled SPIR-V.
    ///
    /// Rebuilds all post-processing pipelines (bloom downsample/upsample,
    /// composite, contact shadows, depth resolve) using the provided compiled
    /// shaders. The old pipelines are dropped after replacement.
    pub(crate) fn reload_shaders(
        &mut self,
        compiled: &[(String, super::shader_compiler::CompiledShader)],
    ) -> EngineResult<u32> {
        let mut count = 0u32;

        for (name, cs) in compiled {
            match name.as_str() {
                "bloom_downsample" => {
                    let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                    let pipeline = create_fullscreen_pipeline(
                        &self.device,
                        &shader,
                        self.store_pass,
                        &[self.sampler_ds_layout],
                        std::mem::size_of::<DownsamplePushConstants>() as u32,
                        false,
                        self.pipeline_cache,
                    )?;
                    self.downsample_pipeline = pipeline;
                    count += 1;
                }
                "bloom_upsample" => {
                    let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                    let pipeline = create_fullscreen_pipeline(
                        &self.device,
                        &shader,
                        self.blend_pass,
                        &[self.sampler_ds_layout],
                        std::mem::size_of::<UpsamplePushConstants>() as u32,
                        true,
                        self.pipeline_cache,
                    )?;
                    self.upsample_pipeline = pipeline;
                    count += 1;
                }
                "postprocess_composite" => {
                    let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                    let pipeline = create_fullscreen_pipeline(
                        &self.device,
                        &shader,
                        self.store_pass,
                        &[
                            self.sampler_ds_layout,
                            self.sampler_ds_layout,
                            self.sampler_ds_layout,
                        ],
                        std::mem::size_of::<CompositePushConstants>() as u32,
                        false,
                        self.pipeline_cache,
                    )?;
                    self.composite_pipeline = pipeline;
                    count += 1;
                }
                "contact_shadows" => {
                    if self.contact_shadow_pipeline.is_some() {
                        let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                        let pipeline = create_fullscreen_pipeline(
                            &self.device,
                            &shader,
                            self.store_pass,
                            &[self.sampler_ds_layout, self.sampler_ds_layout],
                            std::mem::size_of::<ContactShadowPushConstants>() as u32,
                            false,
                            self.pipeline_cache,
                        )?;
                        self.contact_shadow_pipeline = Some(pipeline);
                        count += 1;
                    }
                }
                "bilateral_blur" => {
                    if self.bilateral_blur_pipeline.is_some() {
                        let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                        let pipeline = create_fullscreen_pipeline(
                            &self.device,
                            &shader,
                            self.store_pass,
                            &[self.sampler_ds_layout, self.sampler_ds_layout],
                            std::mem::size_of::<BilateralBlurPushConstants>() as u32,
                            false,
                            self.pipeline_cache,
                        )?;
                        self.bilateral_blur_pipeline = Some(pipeline);
                        count += 1;
                    }
                }
                "depth_resolve" => {
                    if self.depth_resolve_pipeline.is_some() {
                        let shader = Shader::new(&self.device, name, &cs.vert_spv, &cs.frag_spv)?;
                        let pipeline = create_fullscreen_pipeline(
                            &self.device,
                            &shader,
                            self.store_pass,
                            &[self.sampler_ds_layout],
                            0,
                            false,
                            self.pipeline_cache,
                        )?;
                        self.depth_resolve_pipeline = Some(pipeline);
                        count += 1;
                    }
                }
                _ => {} // Not a post-process shader.
            }
        }

        Ok(count)
    }

    // -- Internal passes ------------------------------------------------------

    fn execute_bloom_downsample(
        &self,
        cmd_buf: vk::CommandBuffer,
        scene_source: vk::DescriptorSet,
    ) {
        for (i, mip) in self.bloom_mips.iter().enumerate() {
            // Source: scene for first pass, previous mip for subsequent.
            let source_ds = if i == 0 {
                scene_source
            } else {
                self.bloom_mips[i - 1].descriptor_set
            };

            let source_w = if i == 0 {
                self.width
            } else {
                self.bloom_mips[i - 1].width
            };
            let source_h = if i == 0 {
                self.height
            } else {
                self.bloom_mips[i - 1].height
            };

            let extent = vk::Extent2D {
                width: mip.width,
                height: mip.height,
            };

            let clear = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            };

            let rp_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.store_pass)
                .framebuffer(mip.framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                })
                .clear_values(std::slice::from_ref(&clear));

            let pc = DownsamplePushConstants {
                texel_size: [1.0 / source_w as f32, 1.0 / source_h as f32],
                threshold: self.bloom_threshold,
                first_pass: if i == 0 { 1 } else { 0 },
            };

            unsafe {
                self.device
                    .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

                self.device.cmd_set_viewport(
                    cmd_buf,
                    0,
                    &[vk::Viewport {
                        x: 0.0,
                        y: 0.0,
                        width: mip.width as f32,
                        height: mip.height as f32,
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

                self.device.cmd_bind_pipeline(
                    cmd_buf,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.downsample_pipeline.pipeline(),
                );

                self.device.cmd_bind_descriptor_sets(
                    cmd_buf,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.downsample_pipeline.layout(),
                    0,
                    &[source_ds],
                    &[],
                );

                let pc_bytes = std::slice::from_raw_parts(
                    &pc as *const DownsamplePushConstants as *const u8,
                    std::mem::size_of::<DownsamplePushConstants>(),
                );
                self.device.cmd_push_constants(
                    cmd_buf,
                    self.downsample_pipeline.layout(),
                    vk::ShaderStageFlags::FRAGMENT,
                    0,
                    pc_bytes,
                );

                // Fullscreen triangle (3 vertices, no vertex buffer).
                self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);

                self.device.cmd_end_render_pass(cmd_buf);
            }

            // Barrier: ensure downsample write is visible before next pass reads.
            self.barrier_shader_read(cmd_buf, mip.image);
        }
    }

    fn execute_bloom_upsample(&self, cmd_buf: vk::CommandBuffer) {
        // Upsample from bottom of chain to top: mip[3] → mip[2] → mip[1] → mip[0].
        for i in (0..BLOOM_MIP_LEVELS - 1).rev() {
            let target = &self.bloom_mips[i];
            let source = &self.bloom_mips[i + 1];

            let extent = vk::Extent2D {
                width: target.width,
                height: target.height,
            };

            // Transition target from SHADER_READ_ONLY to COLOR_ATTACHMENT for load+write.
            self.barrier_to_color_attachment(cmd_buf, target.image);

            let rp_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.blend_pass)
                .framebuffer(target.framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                });

            let pc = UpsamplePushConstants {
                texel_size: [1.0 / source.width as f32, 1.0 / source.height as f32],
                filter_radius: self.bloom_filter_radius,
                _pad: 0.0,
            };

            unsafe {
                self.device
                    .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

                self.device.cmd_set_viewport(
                    cmd_buf,
                    0,
                    &[vk::Viewport {
                        x: 0.0,
                        y: 0.0,
                        width: target.width as f32,
                        height: target.height as f32,
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

                self.device.cmd_bind_pipeline(
                    cmd_buf,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.upsample_pipeline.pipeline(),
                );

                self.device.cmd_bind_descriptor_sets(
                    cmd_buf,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.upsample_pipeline.layout(),
                    0,
                    &[source.descriptor_set],
                    &[],
                );

                let pc_bytes = std::slice::from_raw_parts(
                    &pc as *const UpsamplePushConstants as *const u8,
                    std::mem::size_of::<UpsamplePushConstants>(),
                );
                self.device.cmd_push_constants(
                    cmd_buf,
                    self.upsample_pipeline.layout(),
                    vk::ShaderStageFlags::FRAGMENT,
                    0,
                    pc_bytes,
                );

                self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);

                self.device.cmd_end_render_pass(cmd_buf);
            }

            // Barrier: ensure upsample write is visible.
            self.barrier_shader_read(cmd_buf, target.image);
        }
    }

    fn execute_composite(&self, cmd_buf: vk::CommandBuffer, has_shadow: bool) {
        let extent = vk::Extent2D {
            width: self.output.width,
            height: self.output.height,
        };

        let clear = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };

        let rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.store_pass)
            .framebuffer(self.output.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(std::slice::from_ref(&clear));

        let bloom_ds = if self.bloom_enabled && !self.bloom_mips.is_empty() {
            self.bloom_mips[0].descriptor_set
        } else {
            // No bloom — bind scene as dummy (bloom_intensity will be 0).
            self.scene_ds
        };

        // Shadow DS: use blurred shadow factor, or scene as dummy when off.
        let shadow_ds = if has_shadow {
            self.contact_shadowed.as_ref().unwrap().descriptor_set
        } else {
            self.scene_ds // dummy — apply_shadow=0 means it won't be sampled
        };

        let pc = CompositePushConstants {
            bloom_intensity: if self.bloom_enabled {
                self.bloom_intensity
            } else {
                0.0
            },
            exposure: self.exposure,
            contrast: self.contrast,
            saturation: self.saturation,
            tonemapping_mode: self.tonemapping.to_int(),
            apply_shadow: if has_shadow {
                if self.contact_shadows_debug > 0 {
                    2
                } else {
                    1
                }
            } else {
                0
            },
            _pad0: 0.0,
            _pad1: 0.0,
        };

        unsafe {
            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.output.width as f32,
                    height: self.output.height as f32,
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

            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_pipeline.pipeline(),
            );

            // Set 0 = scene, Set 1 = bloom, Set 2 = shadow.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_pipeline.layout(),
                0,
                &[self.scene_ds, bloom_ds, shadow_ds],
                &[],
            );

            let pc_bytes = std::slice::from_raw_parts(
                &pc as *const CompositePushConstants as *const u8,
                std::mem::size_of::<CompositePushConstants>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                self.composite_pipeline.layout(),
                vk::ShaderStageFlags::FRAGMENT,
                0,
                pc_bytes,
            );

            self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);

            self.device.cmd_end_render_pass(cmd_buf);
        }

        // Barrier: output is now in SHADER_READ_ONLY for egui sampling.
        self.barrier_shader_read(cmd_buf, self.output.image);
    }

    // -- MSAA depth resolve pass ------------------------------------------------

    fn execute_depth_resolve(&self, cmd_buf: vk::CommandBuffer) {
        let resolve_pipeline = self.depth_resolve_pipeline.as_ref().unwrap();
        let msaa_ds = self.msaa_depth_ds.unwrap();
        let resolved = self.resolved_depth.as_ref().unwrap();

        let extent = vk::Extent2D {
            width: resolved.width,
            height: resolved.height,
        };

        let clear = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [1.0, 0.0, 0.0, 1.0],
            },
        };

        let rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.store_pass)
            .framebuffer(resolved.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(std::slice::from_ref(&clear));

        unsafe {
            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: resolved.width as f32,
                    height: resolved.height as f32,
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

            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                resolve_pipeline.pipeline(),
            );

            // Set 0 = MSAA depth.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                resolve_pipeline.layout(),
                0,
                &[msaa_ds],
                &[],
            );

            self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);

            self.device.cmd_end_render_pass(cmd_buf);
        }

        // Barrier: resolved depth ready for contact shadow sampling.
        self.barrier_shader_read(cmd_buf, resolved.image);
    }

    // -- Contact shadows pass ---------------------------------------------------

    fn execute_contact_shadows(&self, cmd_buf: vk::CommandBuffer) {
        let cs_image = self.contact_shadowed.as_ref().unwrap();
        let cs_pipeline = self.contact_shadow_pipeline.as_ref().unwrap();
        let depth_ds = self.depth_ds.unwrap();
        let normal_ds = self.normal_ds.unwrap();

        let extent = vk::Extent2D {
            width: cs_image.width,
            height: cs_image.height,
        };

        let clear = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };

        let rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.store_pass)
            .framebuffer(cs_image.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(std::slice::from_ref(&clear));

        let pc = ContactShadowPushConstants {
            inv_view_projection: self.cs_inv_vp,
            view_projection: self.cs_vp,
            light_direction: [
                self.cs_light_dir[0],
                self.cs_light_dir[1],
                self.cs_light_dir[2],
                0.0,
            ],
            max_distance: self.contact_shadows_max_distance,
            thickness: self.contact_shadows_thickness,
            intensity: self.contact_shadows_intensity,
            step_count: self.contact_shadows_step_count,
            near_plane: self.cs_near,
            far_plane: self.cs_far,
            debug_mode: self.contact_shadows_debug,
            _pad1: 0.0,
        };

        unsafe {
            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: cs_image.width as f32,
                    height: cs_image.height as f32,
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

            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                cs_pipeline.pipeline(),
            );

            // Set 0 = depth, Set 1 = normal.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                cs_pipeline.layout(),
                0,
                &[depth_ds, normal_ds],
                &[],
            );

            let pc_bytes = std::slice::from_raw_parts(
                &pc as *const ContactShadowPushConstants as *const u8,
                std::mem::size_of::<ContactShadowPushConstants>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                cs_pipeline.layout(),
                vk::ShaderStageFlags::FRAGMENT,
                0,
                pc_bytes,
            );

            self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);

            self.device.cmd_end_render_pass(cmd_buf);
        }

        // Barrier: contact-shadowed image ready for downstream sampling.
        self.barrier_shader_read(cmd_buf, cs_image.image);
    }

    // -- Bilateral blur pass (H then V) ----------------------------------------

    fn execute_bilateral_blur(&self, cmd_buf: vk::CommandBuffer) {
        let blur_pipeline = self.bilateral_blur_pipeline.as_ref().unwrap();
        let shadow_a = self.contact_shadowed.as_ref().unwrap(); // CS output, final blurred result
        let shadow_b = self.shadow_temp.as_ref().unwrap(); // intermediate
        let depth_ds = self.depth_ds.unwrap();

        let extent = vk::Extent2D {
            width: shadow_a.width,
            height: shadow_a.height,
        };

        let clear = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [1.0, 1.0, 1.0, 1.0],
            },
        };

        let texel_size = [1.0 / shadow_a.width as f32, 1.0 / shadow_a.height as f32];

        // Horizontal pass: read shadow_a → write shadow_b.
        let pc_h = BilateralBlurPushConstants {
            texel_size,
            direction: [1.0, 0.0],
            near_plane: self.cs_near,
            far_plane: self.cs_far,
            _pad: [0.0; 2],
        };

        unsafe {
            let rp_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.store_pass)
                .framebuffer(shadow_b.framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                })
                .clear_values(std::slice::from_ref(&clear));

            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: shadow_a.width as f32,
                    height: shadow_a.height as f32,
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

            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                blur_pipeline.pipeline(),
            );

            // Set 0 = shadow input (shadow_a), Set 1 = depth.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                blur_pipeline.layout(),
                0,
                &[shadow_a.descriptor_set, depth_ds],
                &[],
            );

            let pc_bytes = std::slice::from_raw_parts(
                &pc_h as *const BilateralBlurPushConstants as *const u8,
                std::mem::size_of::<BilateralBlurPushConstants>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                blur_pipeline.layout(),
                vk::ShaderStageFlags::FRAGMENT,
                0,
                pc_bytes,
            );

            self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);
            self.device.cmd_end_render_pass(cmd_buf);
        }

        self.barrier_shader_read(cmd_buf, shadow_b.image);

        // Vertical pass: read shadow_b → write shadow_a (overwrite CS output with blurred).
        let pc_v = BilateralBlurPushConstants {
            texel_size,
            direction: [0.0, 1.0],
            near_plane: self.cs_near,
            far_plane: self.cs_far,
            _pad: [0.0; 2],
        };

        unsafe {
            let rp_info = vk::RenderPassBeginInfo::default()
                .render_pass(self.store_pass)
                .framebuffer(shadow_a.framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent,
                })
                .clear_values(std::slice::from_ref(&clear));

            self.device
                .cmd_begin_render_pass(cmd_buf, &rp_info, vk::SubpassContents::INLINE);

            self.device.cmd_set_viewport(
                cmd_buf,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: shadow_a.width as f32,
                    height: shadow_a.height as f32,
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

            self.device.cmd_bind_pipeline(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                blur_pipeline.pipeline(),
            );

            // Set 0 = shadow input (shadow_b), Set 1 = depth.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                blur_pipeline.layout(),
                0,
                &[shadow_b.descriptor_set, depth_ds],
                &[],
            );

            let pc_bytes = std::slice::from_raw_parts(
                &pc_v as *const BilateralBlurPushConstants as *const u8,
                std::mem::size_of::<BilateralBlurPushConstants>(),
            );
            self.device.cmd_push_constants(
                cmd_buf,
                blur_pipeline.layout(),
                vk::ShaderStageFlags::FRAGMENT,
                0,
                pc_bytes,
            );

            self.device.cmd_draw(cmd_buf, 3, 1, 0, 0);
            self.device.cmd_end_render_pass(cmd_buf);
        }

        // Barrier: blurred shadow factor ready for composite sampling.
        self.barrier_shader_read(cmd_buf, shadow_a.image);
    }

    // -- Barrier helpers ------------------------------------------------------

    fn barrier_shader_read(&self, cmd_buf: vk::CommandBuffer, image: vk::Image) {
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    fn barrier_to_color_attachment(&self, cmd_buf: vk::CommandBuffer, image: vk::Image) {
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::COLOR_ATTACHMENT_READ,
            );

        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }
}

impl Drop for PostProcessPipeline {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();

            // Images are cleaned up by their GpuAllocation drop impls.
            // Clean up Vulkan objects manually.
            for mip in &self.bloom_mips {
                self.device.destroy_framebuffer(mip.framebuffer, None);
                self.device.destroy_image_view(mip.image_view, None);
                self.device.destroy_image(mip.image, None);
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[mip.descriptor_set]);
            }

            self.device
                .destroy_framebuffer(self.output.framebuffer, None);
            self.device.destroy_image_view(self.output.image_view, None);
            self.device.destroy_image(self.output.image, None);

            let _ = self
                .device
                .free_descriptor_sets(self.ds_pool, &[self.scene_ds]);

            // Contact shadow resources.
            if let Some(cs_img) = &self.contact_shadowed {
                self.device.destroy_framebuffer(cs_img.framebuffer, None);
                self.device.destroy_image_view(cs_img.image_view, None);
                self.device.destroy_image(cs_img.image, None);
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[cs_img.descriptor_set]);
            }
            if let Some(st_img) = &self.shadow_temp {
                self.device.destroy_framebuffer(st_img.framebuffer, None);
                self.device.destroy_image_view(st_img.image_view, None);
                self.device.destroy_image(st_img.image, None);
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[st_img.descriptor_set]);
            }
            // Only free depth_ds if it's not the resolved_depth's DS (shared handle).
            let resolved_ds = self.resolved_depth.as_ref().map(|r| r.descriptor_set);
            if let Some(d_ds) = self.depth_ds {
                if resolved_ds != Some(d_ds) {
                    let _ = self.device.free_descriptor_sets(self.ds_pool, &[d_ds]);
                }
            }
            // MSAA depth resolve resources.
            if let Some(rd) = &self.resolved_depth {
                self.device.destroy_framebuffer(rd.framebuffer, None);
                self.device.destroy_image_view(rd.image_view, None);
                self.device.destroy_image(rd.image, None);
                let _ = self
                    .device
                    .free_descriptor_sets(self.ds_pool, &[rd.descriptor_set]);
            }
            if let Some(m_ds) = self.msaa_depth_ds {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[m_ds]);
            }
            if let Some(n_ds) = self.normal_ds {
                let _ = self.device.free_descriptor_sets(self.ds_pool, &[n_ds]);
            }
            if let Some(d_sampler) = self.depth_sampler {
                self.device.destroy_sampler(d_sampler, None);
            }

            self.device.destroy_sampler(self.linear_sampler, None);
            self.device
                .destroy_descriptor_set_layout(self.sampler_ds_layout, None);
            self.device.destroy_descriptor_pool(self.ds_pool, None);

            self.device.destroy_render_pass(self.store_pass, None);
            self.device.destroy_render_pass(self.blend_pass, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Create a render pass with a single UNORM color attachment and no depth.
fn create_render_pass(
    device: &ash::Device,
    format: vk::Format,
    load_op: vk::AttachmentLoadOp,
) -> EngineResult<vk::RenderPass> {
    let attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(load_op)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(if load_op == vk::AttachmentLoadOp::LOAD {
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        } else {
            vk::ImageLayout::UNDEFINED
        })
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

    let attachment_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&attachment_ref));

    // Explicit external dependency for correct layout transitions.
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ),
    ];

    let create_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(&dependencies);

    unsafe { device.create_render_pass(&create_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP render pass: {e}")))
}

/// Create a post-processing intermediate image with framebuffer and descriptor set.
#[allow(clippy::too_many_arguments)]
fn create_pp_image(
    device: &ash::Device,
    allocator: &Arc<Mutex<GpuAllocator>>,
    ds_pool: vk::DescriptorPool,
    ds_layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    render_pass: vk::RenderPass,
    format: vk::Format,
    width: u32,
    height: u32,
) -> EngineResult<PostProcessImage> {
    // Create image.
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP image: {e}")))?;

    let allocation = GpuAllocator::allocate_for_image(
        allocator,
        device,
        image,
        "PostProcess",
        MemoryLocation::GpuOnly,
    )?;

    // Image view.
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    let image_view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP image view: {e}")))?;

    // Framebuffer.
    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(std::slice::from_ref(&image_view))
        .width(width)
        .height(height)
        .layers(1);

    let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP framebuffer: {e}")))?;

    // Descriptor set.
    let descriptor_set = allocate_and_write_ds(device, ds_pool, ds_layout, sampler, image_view)?;

    Ok(PostProcessImage {
        image,
        _allocation: allocation,
        image_view,
        framebuffer,
        descriptor_set,
        width,
        height,
    })
}

/// Like [`create_pp_image`] but reuses an existing descriptor set instead of
/// allocating a new one. This keeps the raw `VkDescriptorSet` handle stable
/// so that external references (e.g. egui texture IDs) remain valid across
/// resize operations.
#[allow(clippy::too_many_arguments)]
fn create_pp_image_reuse_ds(
    device: &ash::Device,
    allocator: &Arc<Mutex<GpuAllocator>>,
    existing_ds: vk::DescriptorSet,
    sampler: vk::Sampler,
    render_pass: vk::RenderPass,
    format: vk::Format,
    width: u32,
    height: u32,
) -> EngineResult<PostProcessImage> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP image: {e}")))?;

    let allocation = GpuAllocator::allocate_for_image(
        allocator,
        device,
        image,
        "PostProcess",
        MemoryLocation::GpuOnly,
    )?;

    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    let image_view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP image view: {e}")))?;

    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(std::slice::from_ref(&image_view))
        .width(width)
        .height(height)
        .layers(1);

    let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP framebuffer: {e}")))?;

    // Update the existing descriptor set to point to the new image view.
    update_ds(device, existing_ds, sampler, image_view);

    Ok(PostProcessImage {
        image,
        _allocation: allocation,
        image_view,
        framebuffer,
        descriptor_set: existing_ds,
        width,
        height,
    })
}

/// Update an existing descriptor set's binding 0 to a new combined image sampler.
fn update_ds(
    device: &ash::Device,
    ds: vk::DescriptorSet,
    sampler: vk::Sampler,
    image_view: vk::ImageView,
) {
    let image_info = vk::DescriptorImageInfo::default()
        .sampler(sampler)
        .image_view(image_view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

    let write = vk::WriteDescriptorSet::default()
        .dst_set(ds)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));

    unsafe {
        device.update_descriptor_sets(&[write], &[]);
    }
}

/// Allocate a descriptor set and write a combined image sampler to binding 0.
fn allocate_and_write_ds(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    image_view: vk::ImageView,
) -> EngineResult<vk::DescriptorSet> {
    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let ds = unsafe { device.allocate_descriptor_sets(&alloc_info) }
        .map_err(|e| EngineError::Gpu(format!("Failed to allocate PP descriptor set: {e}")))?[0];

    let image_info = vk::DescriptorImageInfo::default()
        .sampler(sampler)
        .image_view(image_view)
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

    let write = vk::WriteDescriptorSet::default()
        .dst_set(ds)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));

    unsafe {
        device.update_descriptor_sets(&[write], &[]);
    }

    Ok(ds)
}

/// Like [`allocate_and_write_ds`] but with an explicit image layout.
fn allocate_and_write_ds_with_layout(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
    sampler: vk::Sampler,
    image_view: vk::ImageView,
    image_layout: vk::ImageLayout,
) -> EngineResult<vk::DescriptorSet> {
    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let ds = unsafe { device.allocate_descriptor_sets(&alloc_info) }
        .map_err(|e| EngineError::Gpu(format!("Failed to allocate PP descriptor set: {e}")))?[0];

    let image_info = vk::DescriptorImageInfo::default()
        .sampler(sampler)
        .image_view(image_view)
        .image_layout(image_layout);

    let write = vk::WriteDescriptorSet::default()
        .dst_set(ds)
        .dst_binding(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));

    unsafe {
        device.update_descriptor_sets(&[write], &[]);
    }

    Ok(ds)
}

/// Create a fullscreen-triangle pipeline (no vertex input) with push constants.
fn create_fullscreen_pipeline(
    device: &ash::Device,
    shader: &Shader,
    render_pass: vk::RenderPass,
    ds_layouts: &[vk::DescriptorSetLayout],
    push_constant_size: u32,
    additive_blend: bool,
    pipeline_cache: vk::PipelineCache,
) -> EngineResult<Pipeline> {
    let entry_point = c"main";

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(shader.vert_module())
            .name(entry_point),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(shader.frag_module())
            .name(entry_point),
    ];

    // No vertex input — fullscreen triangle generated in vertex shader.
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(false)
        .depth_write_enable(false);

    let blend_attachment = if additive_blend {
        vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
    } else {
        vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)
    };

    let blend_attachments = [blend_attachment];
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    // Push constants (fragment stage) — only if the shader uses them.
    let push_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: push_constant_size,
    };
    let push_ranges: &[vk::PushConstantRange] = if push_constant_size > 0 {
        std::slice::from_ref(&push_range)
    } else {
        &[]
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(ds_layouts)
        .push_constant_ranges(push_ranges);

    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create PP pipeline layout: {e}")))?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    let pipeline =
        unsafe { device.create_graphics_pipelines(pipeline_cache, &[pipeline_info], None) }
            .map_err(|(_, e)| {
                unsafe {
                    device.destroy_pipeline_layout(pipeline_layout, None);
                }
                EngineError::Gpu(format!("Failed to create PP pipeline: {e}"))
            })?[0];

    Ok(Pipeline::from_raw(
        pipeline,
        pipeline_layout,
        device.clone(),
    ))
}
