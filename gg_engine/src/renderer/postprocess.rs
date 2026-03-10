use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::pipeline::Pipeline;
use super::shader::Shader;

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
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

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
    store_pass: vk::RenderPass,  // LOAD_OP_DONT_CARE (downsample + composite)
    blend_pass: vk::RenderPass,  // LOAD_OP_LOAD (upsample, additive)

    // Pipelines
    downsample_pipeline: Pipeline,
    upsample_pipeline: Pipeline,
    composite_pipeline: Pipeline,

    // Sampler layout (1 combined image sampler at binding 0)
    sampler_ds_layout: vk::DescriptorSetLayout,
    ds_pool: vk::DescriptorPool,

    // Linear sampler used by all passes
    linear_sampler: vk::Sampler,

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

    width: u32,
    height: u32,
}

impl PostProcessPipeline {
    /// Create the post-processing pipeline.
    ///
    /// `scene_color_view` is the offscreen framebuffer's color attachment
    /// image view (the input to post-processing).
    /// `scene_color_format` is the format of that attachment (typically B8G8R8A8_SRGB).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &ash::Device,
        allocator: &Arc<Mutex<GpuAllocator>>,
        descriptor_pool: vk::DescriptorPool,
        texture_ds_layout: vk::DescriptorSetLayout,
        scene_color_view: vk::ImageView,
        pipeline_cache: vk::PipelineCache,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
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
        .map_err(|e| format!("Failed to create PP descriptor set layout: {e}"))?;

        // --- Descriptor pool (bloom mips + output + scene = BLOOM_MIP_LEVELS + 2) ---
        let max_sets = (BLOOM_MIP_LEVELS + 2) as u32;
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
        .map_err(|e| format!("Failed to create PP descriptor pool: {e}"))?;

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
        .map_err(|e| format!("Failed to create PP linear sampler: {e}"))?;

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
            &[sampler_ds_layout, sampler_ds_layout], // scene + bloom
            std::mem::size_of::<CompositePushConstants>() as u32,
            false,
            pipeline_cache,
        )?;

        Ok(Self {
            device: device.clone(),
            store_pass,
            blend_pass,
            downsample_pipeline,
            upsample_pipeline,
            composite_pipeline,
            sampler_ds_layout,
            ds_pool,
            linear_sampler,
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
            width,
            height,
        })
    }

    /// Resize internal images to match a new viewport size.
    /// Call when the offscreen framebuffer is resized.
    pub fn resize(
        &mut self,
        allocator: &Arc<Mutex<GpuAllocator>>,
        scene_color_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if width == self.width && height == self.height {
            return Ok(());
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
            self.device
                .destroy_image_view(self.output.image_view, None);
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

        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Execute the post-processing pipeline.
    ///
    /// Records render passes into `cmd_buf`. The scene color must be in
    /// `SHADER_READ_ONLY_OPTIMAL` layout (after the offscreen scene pass barrier).
    /// After execution, the output image is in `SHADER_READ_ONLY_OPTIMAL`.
    pub fn execute(&self, cmd_buf: vk::CommandBuffer) {
        let _timer = ProfileTimer::new("PostProcess::execute");

        // Guard: only run bloom if all mip levels were successfully created.
        if self.bloom_enabled && self.bloom_mips.len() == BLOOM_MIP_LEVELS {
            self.execute_bloom_downsample(cmd_buf);
            self.execute_bloom_upsample(cmd_buf);
        }
        self.execute_composite(cmd_buf);
    }

    /// The output image's descriptor set handle for egui texture registration.
    pub fn output_egui_handle(&self) -> u64 {
        self.output.descriptor_set.as_raw()
    }

    /// Current pipeline dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    // -- Internal passes ------------------------------------------------------

    fn execute_bloom_downsample(&self, cmd_buf: vk::CommandBuffer) {
        for (i, mip) in self.bloom_mips.iter().enumerate() {
            // Source: scene for first pass, previous mip for subsequent.
            let source_ds = if i == 0 {
                self.scene_ds
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

    fn execute_composite(&self, cmd_buf: vk::CommandBuffer) {
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
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
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

            // Set 0 = scene, Set 1 = bloom.
            self.device.cmd_bind_descriptor_sets(
                cmd_buf,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_pipeline.layout(),
                0,
                &[self.scene_ds, bloom_ds],
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
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                | vk::AccessFlags::COLOR_ATTACHMENT_READ);

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

            self.device.destroy_framebuffer(self.output.framebuffer, None);
            self.device.destroy_image_view(self.output.image_view, None);
            self.device.destroy_image(self.output.image, None);

            let _ = self
                .device
                .free_descriptor_sets(self.ds_pool, &[self.scene_ds]);

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
) -> Result<vk::RenderPass, String> {
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
        .map_err(|e| format!("Failed to create PP render pass: {e}"))
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
) -> Result<PostProcessImage, String> {
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
        .usage(
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| format!("Failed to create PP image: {e}"))?;

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
        .map_err(|e| format!("Failed to create PP image view: {e}"))?;

    // Framebuffer.
    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(std::slice::from_ref(&image_view))
        .width(width)
        .height(height)
        .layers(1);

    let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }
        .map_err(|e| format!("Failed to create PP framebuffer: {e}"))?;

    // Descriptor set.
    let descriptor_set =
        allocate_and_write_ds(device, ds_pool, ds_layout, sampler, image_view)?;

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
fn create_pp_image_reuse_ds(
    device: &ash::Device,
    allocator: &Arc<Mutex<GpuAllocator>>,
    existing_ds: vk::DescriptorSet,
    sampler: vk::Sampler,
    render_pass: vk::RenderPass,
    format: vk::Format,
    width: u32,
    height: u32,
) -> Result<PostProcessImage, String> {
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
        .map_err(|e| format!("Failed to create PP image: {e}"))?;

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
        .map_err(|e| format!("Failed to create PP image view: {e}"))?;

    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(std::slice::from_ref(&image_view))
        .width(width)
        .height(height)
        .layers(1);

    let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }
        .map_err(|e| format!("Failed to create PP framebuffer: {e}"))?;

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
) -> Result<vk::DescriptorSet, String> {
    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let ds = unsafe { device.allocate_descriptor_sets(&alloc_info) }
        .map_err(|e| format!("Failed to allocate PP descriptor set: {e}"))?[0];

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

/// Create a fullscreen-triangle pipeline (no vertex input) with push constants.
fn create_fullscreen_pipeline(
    device: &ash::Device,
    shader: &Shader,
    render_pass: vk::RenderPass,
    ds_layouts: &[vk::DescriptorSetLayout],
    push_constant_size: u32,
    additive_blend: bool,
    pipeline_cache: vk::PipelineCache,
) -> Result<Pipeline, String> {
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

    // Push constants (fragment stage).
    let push_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: push_constant_size,
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(ds_layouts)
        .push_constant_ranges(std::slice::from_ref(&push_range));

    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| format!("Failed to create PP pipeline layout: {e}"))?;

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
                format!("Failed to create PP pipeline: {e}")
            })?[0];

    Ok(Pipeline::from_raw(pipeline, pipeline_layout, device.clone()))
}
