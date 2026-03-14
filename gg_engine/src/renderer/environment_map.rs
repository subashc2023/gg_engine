use std::sync::{Arc, Mutex};

use ash::vk;

use super::buffer::create_staging_buffer;
use super::compute::{create_compute_pipeline, ComputePipeline, ComputeShader};
use super::cubemap::Cubemap;
use super::gpu_allocation::{GpuAllocator, MemoryLocation};
use super::lighting::LightingSystem;
use super::pipeline::Pipeline;
use super::texture::{ImageFormat, Texture2D, TextureSpecification};
use super::vertex_array::VertexArray;
use super::{BufferElement, BufferLayout, RendererResources, ShaderDataType};
use crate::error::{EngineError, EngineResult};
use crate::shaders;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Environment cubemap face resolution (per face, base mip).
const ENV_CUBEMAP_SIZE: u32 = 1024;

/// Irradiance map face resolution (low-res for diffuse).
const IRRADIANCE_SIZE: u32 = 64;

/// Pre-filtered specular map base resolution (matches source cubemap for
/// artifact-free reflections at roughness 0).
const PREFILTER_SIZE: u32 = 1024;

/// Number of pre-filtered specular mip levels.  More levels = smoother
/// roughness gradation (each mip is linearly spaced in perceptual roughness).
const PREFILTER_MIP_LEVELS: u32 = 9; // mips 0-8: roughness 0.0-1.0

/// BRDF integration LUT resolution.
const BRDF_LUT_SIZE: u32 = 256;

// ---------------------------------------------------------------------------
// Push constant structures (must match GLSL exactly)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct EquirectToCubePush {
    face: i32,
    face_size: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IrradiancePush {
    face: i32,
    face_size: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PrefilterPush {
    face: i32,
    face_size: i32,
    roughness: f32,
    sample_count: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BrdfLutPush {
    size: i32,
}

/// Push constants for the skybox shader.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SkyboxPushConstants {
    pub vp_rotation: [f32; 16], // mat4
    pub exposure: f32,
    pub rotation_y: f32,
}

// ---------------------------------------------------------------------------
// EnvironmentMapSystem
// ---------------------------------------------------------------------------

/// Manages IBL preprocessing (equirect→cubemap, irradiance, prefilter, BRDF LUT)
/// and skybox rendering for a scene's environment map.
///
/// Follows the lazy-init pattern: created via `Renderer::ensure_environment_map()`
/// only when a 3D scene actually uses an environment map.
pub(crate) struct EnvironmentMapSystem {
    // Source HDR environment cubemap.
    env_cubemap: Cubemap,
    // Preprocessed IBL textures.
    irradiance_map: Cubemap,
    prefiltered_map: Cubemap,
    brdf_lut: Texture2D,

    // Fallback 1x1 black cubemap (bound when no environment is loaded).
    fallback_cubemap: Cubemap,

    // Compute pipelines for IBL preprocessing.
    equirect_to_cube_pipeline: ComputePipeline,
    irradiance_pipeline: ComputePipeline,
    prefilter_pipeline: ComputePipeline,
    brdf_pipeline: ComputePipeline,

    // Compute descriptor set resources.
    compute_ds_pool: vk::DescriptorPool,
    compute_sampler2d_ds_layout: vk::DescriptorSetLayout,
    compute_sampler_cube_ds_layout: vk::DescriptorSetLayout,
    compute_brdf_ds_layout: vk::DescriptorSetLayout,

    // Skybox rendering.
    skybox_vertex_array: VertexArray,

    device: ash::Device,
    allocator: Arc<Mutex<GpuAllocator>>,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,
    pipeline_cache: vk::PipelineCache,

    /// Whether a real environment map has been loaded (vs fallback).
    has_environment: bool,

    /// Whether the BRDF LUT has been generated (only done once, ever).
    brdf_generated: bool,
}

impl EnvironmentMapSystem {
    /// Create the environment map system with fallback textures and compute pipelines.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        command_pool: vk::CommandPool,
        graphics_queue: vk::Queue,
        pipeline_cache: vk::PipelineCache,
        lighting: &LightingSystem,
        res: &RendererResources<'_>,
    ) -> EngineResult<Self> {
        // --- Fallback cubemap: 1x1 black ---
        let fallback_cubemap = Cubemap::new(
            allocator,
            device,
            1,
            1,
            vk::Format::R16G16B16A16_SFLOAT,
            vk::Filter::LINEAR,
        )?;
        // Transition to SHADER_READ_ONLY and clear to black.
        Self::clear_cubemap_to_black(device, command_pool, graphics_queue, &fallback_cubemap)?;

        // --- Allocate target cubemaps (will be overwritten on load) ---
        let env_cubemap = Cubemap::new(
            allocator,
            device,
            ENV_CUBEMAP_SIZE,
            super::texture::calculate_mip_levels_pub(ENV_CUBEMAP_SIZE, ENV_CUBEMAP_SIZE),
            vk::Format::R16G16B16A16_SFLOAT,
            vk::Filter::LINEAR,
        )?;
        let irradiance_map = Cubemap::new(
            allocator,
            device,
            IRRADIANCE_SIZE,
            1,
            vk::Format::R16G16B16A16_SFLOAT,
            vk::Filter::LINEAR,
        )?;
        let prefiltered_map = Cubemap::new(
            allocator,
            device,
            PREFILTER_SIZE,
            PREFILTER_MIP_LEVELS,
            vk::Format::R16G16B16A16_SFLOAT,
            vk::Filter::LINEAR,
        )?;

        // --- BRDF LUT (CPU-generated, RG16F for precision) ---
        let brdf_data = Self::generate_brdf_lut_rg16f(BRDF_LUT_SIZE);
        let brdf_lut = Texture2D::from_rgba8_with_spec(
            res,
            allocator,
            BRDF_LUT_SIZE,
            BRDF_LUT_SIZE,
            &brdf_data,
            &TextureSpecification {
                format: ImageFormat::Rg16Float,
                filter: vk::Filter::LINEAR,
                address_mode: vk::SamplerAddressMode::CLAMP_TO_EDGE,
                anisotropy: false,
                max_anisotropy: 1.0,
                generate_mipmaps: false,
            },
        )?;

        // --- Compute descriptor set pool ---
        // Sized for: equirect-to-cube (6) + irradiance (6) + prefilter (MIPS×6) + BRDF (1).
        let compute_pool_sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 128,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: 128,
            },
        ];
        let compute_pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&compute_pool_sizes)
            .max_sets(128)
            .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
        let compute_ds_pool = unsafe { device.create_descriptor_pool(&compute_pool_info, None) }
            .map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to create environment compute descriptor pool: {e}"
                ))
            })?;

        // --- Compute descriptor set layouts ---
        // Layout for equirect_to_cube: binding 0 = sampler2D, binding 1 = image2D
        let sampler2d_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let compute_sampler2d_ds_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&sampler2d_bindings),
                None,
            )
        }
        .map_err(|e| {
            EngineError::Gpu(format!("Failed to create equirect compute DS layout: {e}"))
        })?;

        // Layout for irradiance/prefilter: binding 0 = samplerCube, binding 1 = image2D
        let sampler_cube_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::COMPUTE),
        ];
        let compute_sampler_cube_ds_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&sampler_cube_bindings),
                None,
            )
        }
        .map_err(|e| {
            EngineError::Gpu(format!("Failed to create cubemap compute DS layout: {e}"))
        })?;

        // Layout for BRDF LUT: binding 0 = image2D (write-only)
        let brdf_bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)];
        let compute_brdf_ds_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&brdf_bindings),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create BRDF compute DS layout: {e}")))?;

        // --- Compute pipelines ---
        let equirect_shader = ComputeShader::new(
            device,
            "equirect_to_cube",
            shaders::EQUIRECT_TO_CUBE_COMP_SPV,
        )?;
        let equirect_to_cube_pipeline = create_compute_pipeline(
            device,
            &equirect_shader,
            &[compute_sampler2d_ds_layout],
            std::mem::size_of::<EquirectToCubePush>() as u32,
            pipeline_cache,
        )?;

        let irradiance_shader = ComputeShader::new(
            device,
            "irradiance_convolve",
            shaders::IRRADIANCE_CONVOLVE_COMP_SPV,
        )?;
        let irradiance_pipeline = create_compute_pipeline(
            device,
            &irradiance_shader,
            &[compute_sampler_cube_ds_layout],
            std::mem::size_of::<IrradiancePush>() as u32,
            pipeline_cache,
        )?;

        let prefilter_shader = ComputeShader::new(
            device,
            "prefilter_specular",
            shaders::PREFILTER_SPECULAR_COMP_SPV,
        )?;
        let prefilter_pipeline = create_compute_pipeline(
            device,
            &prefilter_shader,
            &[compute_sampler_cube_ds_layout],
            std::mem::size_of::<PrefilterPush>() as u32,
            pipeline_cache,
        )?;

        let brdf_shader = ComputeShader::new(device, "brdf_lut", shaders::BRDF_LUT_COMP_SPV)?;
        let brdf_pipeline = create_compute_pipeline(
            device,
            &brdf_shader,
            &[compute_brdf_ds_layout],
            std::mem::size_of::<BrdfLutPush>() as u32,
            pipeline_cache,
        )?;

        // --- Skybox unit cube vertex array ---
        let skybox_vertex_array = Self::create_skybox_cube(allocator, device)?;

        // --- Write fallback IBL descriptors to lighting system ---
        lighting.write_ibl_descriptors(
            fallback_cubemap.image_view(),
            fallback_cubemap.sampler(),
            fallback_cubemap.image_view(),
            fallback_cubemap.sampler(),
            brdf_lut.image_view(),
            brdf_lut.sampler(),
            fallback_cubemap.image_view(),
            fallback_cubemap.sampler(),
        );

        Ok(Self {
            env_cubemap,
            irradiance_map,
            prefiltered_map,
            brdf_lut,
            fallback_cubemap,
            equirect_to_cube_pipeline,
            irradiance_pipeline,
            prefilter_pipeline,
            brdf_pipeline,
            compute_ds_pool,
            compute_sampler2d_ds_layout,
            compute_sampler_cube_ds_layout: compute_sampler_cube_ds_layout,
            compute_brdf_ds_layout,
            skybox_vertex_array,
            device: device.clone(),
            allocator: allocator.clone(),
            command_pool,
            graphics_queue,
            pipeline_cache,
            has_environment: false,
            brdf_generated: false,
        })
    }

    /// Whether a real environment map is loaded.
    pub fn has_environment(&self) -> bool {
        self.has_environment
    }

    /// Max prefilter mip level (for roughness LOD in shader).
    pub fn max_prefilter_mip(&self) -> i32 {
        (PREFILTER_MIP_LEVELS - 1) as i32
    }

    /// The skybox vertex array (unit cube).
    pub fn skybox_vertex_array(&self) -> &VertexArray {
        &self.skybox_vertex_array
    }

    /// Get the active environment cubemap (source or fallback).
    pub fn active_env_cubemap(&self) -> &Cubemap {
        if self.has_environment {
            &self.env_cubemap
        } else {
            &self.fallback_cubemap
        }
    }

    /// Get the active irradiance map (processed or fallback).
    pub fn active_irradiance(&self) -> &Cubemap {
        if self.has_environment {
            &self.irradiance_map
        } else {
            &self.fallback_cubemap
        }
    }

    /// Get the active prefiltered specular map (processed or fallback).
    pub fn active_prefiltered(&self) -> &Cubemap {
        if self.has_environment {
            &self.prefiltered_map
        } else {
            &self.fallback_cubemap
        }
    }

    /// Get the BRDF integration LUT.
    pub fn brdf_lut(&self) -> &Texture2D {
        &self.brdf_lut
    }

    /// Update the lighting system's IBL descriptors with the current active textures.
    pub fn update_lighting_descriptors(&self, lighting: &LightingSystem) {
        let irr = self.active_irradiance();
        let pref = self.active_prefiltered();
        let env = self.active_env_cubemap();
        lighting.write_ibl_descriptors(
            irr.image_view(),
            irr.sampler(),
            pref.image_view(),
            pref.sampler(),
            self.brdf_lut.image_view(),
            self.brdf_lut.sampler(),
            env.image_view(),
            env.sampler(),
        );
    }

    /// Load an HDR equirectangular image and run the full IBL preprocessing chain.
    ///
    /// `pixels_rgba_f16` is RGBA half-float data (8 bytes/pixel, R16G16B16A16_SFLOAT).
    #[allow(clippy::too_many_arguments)]
    pub fn load_hdr(
        &mut self,
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        command_pool: vk::CommandPool,
        queue: vk::Queue,
        descriptor_pool: vk::DescriptorPool,
        texture_ds_layout: vk::DescriptorSetLayout,
        pixels_rgba_f16: &[u8],
        width: u32,
        height: u32,
    ) -> EngineResult<()> {
        log::info!(target: "gg_engine", "Loading HDR environment map ({width}x{height})...");

        // 1. Upload equirectangular HDR as a temporary 2D texture (R16G16B16A16_SFLOAT).
        let (staging_buf, staging_alloc) =
            create_staging_buffer(allocator, device, pixels_rgba_f16)?;
        let equirect_image = Self::create_hdr_image(allocator, device, width, height)?;

        super::texture::execute_one_shot_pub(device, command_pool, queue, |cmd_buf| {
            // Transition equirect image for upload.
            Self::transition_image(
                device,
                cmd_buf,
                equirect_image.0,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                1,
                1,
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            );

            let region = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D::default(),
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };
            unsafe {
                device.cmd_copy_buffer_to_image(
                    cmd_buf,
                    staging_buf,
                    equirect_image.0,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            Self::transition_image(
                device,
                cmd_buf,
                equirect_image.0,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                1,
                1,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::COMPUTE_SHADER,
            );
        })?;

        // Create equirect image view + sampler for compute read.
        let equirect_view = unsafe {
            device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(equirect_image.0)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::R16G16B16A16_SFLOAT)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    }),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create equirect view: {e}")))?;

        let equirect_sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::REPEAT)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }
        .map_err(|e| EngineError::Gpu(format!("Failed to create equirect sampler: {e}")))?;

        // 2. Run compute preprocessing chain in one command buffer.
        super::texture::execute_one_shot_pub(device, command_pool, queue, |cmd_buf| {
            log::info!(target: "gg_engine", "Running IBL compute preprocessing chain...");
            // -- Equirect → Cubemap --
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.env_cubemap.image(),
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::GENERAL,
                self.env_cubemap.mip_levels(),
                vk::AccessFlags::empty(),
                vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
            );

            for face in 0..6u32 {
                let ds = Self::alloc_and_write_sampler2d_storage_ds(
                    device,
                    self.compute_ds_pool,
                    self.compute_sampler2d_ds_layout,
                    equirect_view,
                    equirect_sampler,
                    self.env_cubemap.face_mip_view(face, 0),
                );
                unsafe {
                    device.cmd_bind_pipeline(
                        cmd_buf,
                        vk::PipelineBindPoint::COMPUTE,
                        self.equirect_to_cube_pipeline.pipeline(),
                    );
                    device.cmd_bind_descriptor_sets(
                        cmd_buf,
                        vk::PipelineBindPoint::COMPUTE,
                        self.equirect_to_cube_pipeline.layout(),
                        0,
                        &[ds],
                        &[],
                    );
                }
                let push = EquirectToCubePush {
                    face: face as i32,
                    face_size: ENV_CUBEMAP_SIZE as i32,
                };
                let push_bytes = unsafe {
                    std::slice::from_raw_parts(
                        &push as *const _ as *const u8,
                        std::mem::size_of_val(&push),
                    )
                };
                unsafe {
                    device.cmd_push_constants(
                        cmd_buf,
                        self.equirect_to_cube_pipeline.layout(),
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        push_bytes,
                    );
                }
                let groups = (ENV_CUBEMAP_SIZE + 15) / 16;
                unsafe {
                    device.cmd_dispatch(cmd_buf, groups, groups, 1);
                }
            }

            // Barrier: env_cubemap GENERAL → SHADER_READ_ONLY for sampling.
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.env_cubemap.image(),
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                self.env_cubemap.mip_levels(),
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::TRANSFER,
            );

            // Generate mipmaps for the environment cubemap (blit chain per face).
            Self::generate_cubemap_mipmaps(device, cmd_buf, &self.env_cubemap);

            // env_cubemap is now in SHADER_READ_ONLY (mipmaps generated).

            // -- Irradiance Convolution --
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.irradiance_map.image(),
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::GENERAL,
                1,
                vk::AccessFlags::empty(),
                vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
            );

            for face in 0..6u32 {
                let ds = Self::alloc_and_write_sampler2d_storage_ds(
                    device,
                    self.compute_ds_pool,
                    self.compute_sampler_cube_ds_layout,
                    self.env_cubemap.image_view(),
                    self.env_cubemap.sampler(),
                    self.irradiance_map.face_mip_view(face, 0),
                );
                unsafe {
                    device.cmd_bind_pipeline(
                        cmd_buf,
                        vk::PipelineBindPoint::COMPUTE,
                        self.irradiance_pipeline.pipeline(),
                    );
                    device.cmd_bind_descriptor_sets(
                        cmd_buf,
                        vk::PipelineBindPoint::COMPUTE,
                        self.irradiance_pipeline.layout(),
                        0,
                        &[ds],
                        &[],
                    );
                }
                let push = IrradiancePush {
                    face: face as i32,
                    face_size: IRRADIANCE_SIZE as i32,
                };
                let push_bytes = unsafe {
                    std::slice::from_raw_parts(
                        &push as *const _ as *const u8,
                        std::mem::size_of_val(&push),
                    )
                };
                unsafe {
                    device.cmd_push_constants(
                        cmd_buf,
                        self.irradiance_pipeline.layout(),
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        push_bytes,
                    );
                    device.cmd_dispatch(
                        cmd_buf,
                        (IRRADIANCE_SIZE + 7) / 8,
                        (IRRADIANCE_SIZE + 7) / 8,
                        1,
                    );
                }
            }

            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.irradiance_map.image(),
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                1,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            );

            // -- Pre-filtered Specular --
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.prefiltered_map.image(),
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::GENERAL,
                PREFILTER_MIP_LEVELS,
                vk::AccessFlags::empty(),
                vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
            );

            for mip in 0..PREFILTER_MIP_LEVELS {
                let roughness = mip as f32 / (PREFILTER_MIP_LEVELS - 1).max(1) as f32;
                let mip_size = (PREFILTER_SIZE >> mip).max(1);
                let sample_count = 1024; // All mips need enough samples for convergence

                for face in 0..6u32 {
                    let ds = Self::alloc_and_write_sampler2d_storage_ds(
                        device,
                        self.compute_ds_pool,
                        self.compute_sampler_cube_ds_layout,
                        self.env_cubemap.image_view(),
                        self.env_cubemap.sampler(),
                        self.prefiltered_map.face_mip_view(face, mip),
                    );
                    unsafe {
                        device.cmd_bind_pipeline(
                            cmd_buf,
                            vk::PipelineBindPoint::COMPUTE,
                            self.prefilter_pipeline.pipeline(),
                        );
                        device.cmd_bind_descriptor_sets(
                            cmd_buf,
                            vk::PipelineBindPoint::COMPUTE,
                            self.prefilter_pipeline.layout(),
                            0,
                            &[ds],
                            &[],
                        );
                    }
                    let push = PrefilterPush {
                        face: face as i32,
                        face_size: mip_size as i32,
                        roughness,
                        sample_count,
                    };
                    let push_bytes = unsafe {
                        std::slice::from_raw_parts(
                            &push as *const _ as *const u8,
                            std::mem::size_of_val(&push),
                        )
                    };
                    unsafe {
                        device.cmd_push_constants(
                            cmd_buf,
                            self.prefilter_pipeline.layout(),
                            vk::ShaderStageFlags::COMPUTE,
                            0,
                            push_bytes,
                        );
                        let groups = (mip_size + 15) / 16;
                        device.cmd_dispatch(cmd_buf, groups, groups, 1);
                    }
                }
            }

            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                self.prefiltered_map.image(),
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                PREFILTER_MIP_LEVELS,
                vk::AccessFlags::SHADER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            );

            // BRDF LUT is precomputed on CPU and uploaded as a regular texture.
        })?;

        // Clean up temporary resources.
        unsafe {
            device.destroy_image_view(equirect_view, None);
            device.destroy_sampler(equirect_sampler, None);
            device.destroy_image(equirect_image.0, None);
            device.destroy_buffer(staging_buf, None);
        }
        drop(equirect_image.1); // Free GPU allocation
        drop(staging_alloc);

        self.has_environment = true;
        log::info!(target: "gg_engine", "Environment map loaded and preprocessed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Clear a cubemap to black and transition to SHADER_READ_ONLY_OPTIMAL.
    fn clear_cubemap_to_black(
        device: &ash::Device,
        command_pool: vk::CommandPool,
        queue: vk::Queue,
        cubemap: &Cubemap,
    ) -> EngineResult<()> {
        super::texture::execute_one_shot_pub(device, command_pool, queue, |cmd_buf| {
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                cubemap.image(),
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                cubemap.mip_levels(),
                vk::AccessFlags::empty(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
            );

            let clear_value = vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            };
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: cubemap.mip_levels(),
                base_array_layer: 0,
                layer_count: Cubemap::NUM_FACES,
            };
            unsafe {
                device.cmd_clear_color_image(
                    cmd_buf,
                    cubemap.image(),
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &clear_value,
                    &[range],
                );
            }

            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                cubemap.image(),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                cubemap.mip_levels(),
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            );
        })
    }

    /// Create a unit cube vertex array for skybox rendering (36 verts, no index buffer).
    fn create_skybox_cube(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
    ) -> EngineResult<VertexArray> {
        #[rustfmt::skip]
        let vertices: &[f32] = &[
            // Back face (-Z)
            -1.0, -1.0, -1.0,   1.0, -1.0, -1.0,   1.0,  1.0, -1.0,
             1.0,  1.0, -1.0,  -1.0,  1.0, -1.0,  -1.0, -1.0, -1.0,
            // Front face (+Z)
            -1.0, -1.0,  1.0,   1.0,  1.0,  1.0,   1.0, -1.0,  1.0,
             1.0,  1.0,  1.0,  -1.0, -1.0,  1.0,  -1.0,  1.0,  1.0,
            // Left face (-X)
            -1.0, -1.0, -1.0,  -1.0,  1.0,  1.0,  -1.0, -1.0,  1.0,
            -1.0,  1.0,  1.0,  -1.0, -1.0, -1.0,  -1.0,  1.0, -1.0,
            // Right face (+X)
             1.0, -1.0, -1.0,   1.0, -1.0,  1.0,   1.0,  1.0,  1.0,
             1.0,  1.0,  1.0,   1.0,  1.0, -1.0,   1.0, -1.0, -1.0,
            // Bottom face (-Y)
            -1.0, -1.0, -1.0,  -1.0, -1.0,  1.0,   1.0, -1.0,  1.0,
             1.0, -1.0,  1.0,   1.0, -1.0, -1.0,  -1.0, -1.0, -1.0,
            // Top face (+Y)
            -1.0,  1.0, -1.0,   1.0,  1.0,  1.0,  -1.0,  1.0,  1.0,
             1.0,  1.0,  1.0,  -1.0,  1.0, -1.0,   1.0,  1.0, -1.0,
        ];

        let layout = BufferLayout::new(&[BufferElement::new(ShaderDataType::Float3, "a_position")]);

        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(vertices.as_ptr() as *const u8, vertices.len() * 4)
        };

        let mut vb = super::buffer::VertexBuffer::new(allocator, device, bytes)?;
        vb.set_layout(layout);

        let mut va = VertexArray::new(device);
        va.add_vertex_buffer(vb);
        Ok(va)
    }

    /// Create a temporary HDR image for the equirectangular source.
    fn create_hdr_image(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        width: u32,
        height: u32,
    ) -> EngineResult<(vk::Image, super::gpu_allocation::GpuAllocation)> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(vk::Format::R16G16B16A16_SFLOAT)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1);

        let image = unsafe { device.create_image(&image_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create HDR equirect image: {e}")))?;

        let allocation = super::gpu_allocation::GpuAllocator::allocate_for_image(
            allocator,
            device,
            image,
            "HDR_Equirect",
            MemoryLocation::GpuOnly,
        )?;

        Ok((image, allocation))
    }

    /// Generate a BRDF integration LUT on the CPU (split-sum approximation).
    ///
    /// Returns RG16F data (4 bytes/pixel): R = F0 scale, G = F0 bias.
    /// X axis = NdotV, Y axis = roughness, both [0, 1].
    /// Half-float precision (65536 levels vs 256 for RGBA8) eliminates visible
    /// quantization banding on metallic surfaces.
    fn generate_brdf_lut_rg16f(size: u32) -> Vec<u8> {
        const SAMPLE_COUNT: u32 = 1024;
        const PI: f32 = std::f32::consts::PI;

        fn radical_inverse_vdc(mut bits: u32) -> f32 {
            bits = (bits << 16) | (bits >> 16);
            bits = ((bits & 0x55555555) << 1) | ((bits & 0xAAAAAAAA) >> 1);
            bits = ((bits & 0x33333333) << 2) | ((bits & 0xCCCCCCCC) >> 2);
            bits = ((bits & 0x0F0F0F0F) << 4) | ((bits & 0xF0F0F0F0) >> 4);
            bits = ((bits & 0x00FF00FF) << 8) | ((bits & 0xFF00FF00) >> 8);
            bits as f32 * 2.328_306_4e-10
        }

        fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
            let k = (roughness * roughness) / 2.0;
            n_dot_v / (n_dot_v * (1.0 - k) + k)
        }

        // RG16F = 4 bytes per pixel (2 × f16).
        let mut data = vec![0u8; (size * size * 4) as usize];

        for y in 0..size {
            for x in 0..size {
                let n_dot_v = ((x as f32 + 0.5) / size as f32).max(0.001);
                let roughness = ((y as f32 + 0.5) / size as f32).max(0.001);

                // View vector in tangent space (N = (0,0,1)).
                let v = [
                    (1.0 - n_dot_v * n_dot_v).sqrt(), // sin(theta)
                    0.0f32,
                    n_dot_v, // cos(theta)
                ];

                let mut a = 0.0f32; // F0 scale
                let mut b = 0.0f32; // F0 bias

                let alpha = roughness * roughness;

                for i in 0..SAMPLE_COUNT {
                    // Hammersley sequence.
                    let xi_x = i as f32 / SAMPLE_COUNT as f32;
                    let xi_y = radical_inverse_vdc(i);

                    // Importance sample GGX (N = (0,0,1), so tangent frame is identity).
                    let phi = 2.0 * PI * xi_x;
                    let cos_theta = ((1.0 - xi_y) / (1.0 + (alpha * alpha - 1.0) * xi_y)).sqrt();
                    let sin_theta = (1.0 - cos_theta * cos_theta).sqrt();
                    let h = [phi.cos() * sin_theta, phi.sin() * sin_theta, cos_theta];

                    // Reflect V around H to get L.
                    let v_dot_h = (v[0] * h[0] + v[1] * h[1] + v[2] * h[2]).max(0.0);
                    let l = [
                        2.0 * v_dot_h * h[0] - v[0],
                        2.0 * v_dot_h * h[1] - v[1],
                        2.0 * v_dot_h * h[2] - v[2],
                    ];

                    let n_dot_l = l[2].max(0.0);
                    let n_dot_h = h[2].max(0.0);

                    if n_dot_l > 0.0 {
                        // Smith G term.
                        let g = geometry_schlick_ggx(n_dot_v, roughness)
                            * geometry_schlick_ggx(n_dot_l, roughness);
                        let g_vis = (g * v_dot_h) / (n_dot_h * n_dot_v).max(0.001);
                        let fc = (1.0 - v_dot_h).powi(5);

                        a += (1.0 - fc) * g_vis;
                        b += fc * g_vis;
                    }
                }

                a /= SAMPLE_COUNT as f32;
                b /= SAMPLE_COUNT as f32;

                // Store as RG16F (half-float pair, 4 bytes total).
                let r_f16 = half::f16::from_f32(a.clamp(0.0, 1.0));
                let g_f16 = half::f16::from_f32(b.clamp(0.0, 1.0));
                let idx = ((y * size + x) * 4) as usize;
                data[idx..idx + 2].copy_from_slice(&r_f16.to_le_bytes());
                data[idx + 2..idx + 4].copy_from_slice(&g_f16.to_le_bytes());
            }
        }

        data
    }

    /// Generic image layout transition (for non-cubemap 2D images).
    fn transition_image(
        device: &ash::Device,
        cmd_buf: vk::CommandBuffer,
        image: vk::Image,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
        mip_levels: u32,
        layer_count: u32,
        src_access: vk::AccessFlags,
        dst_access: vk::AccessFlags,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
    ) {
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(old_layout)
            .new_layout(new_layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count,
            })
            .src_access_mask(src_access)
            .dst_access_mask(dst_access);

        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buf,
                src_stage,
                dst_stage,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    /// Allocate a descriptor set with binding 0 = sampler, binding 1 = storage image.
    fn alloc_and_write_sampler2d_storage_ds(
        device: &ash::Device,
        pool: vk::DescriptorPool,
        layout: vk::DescriptorSetLayout,
        sampler_view: vk::ImageView,
        sampler: vk::Sampler,
        storage_view: vk::ImageView,
    ) -> vk::DescriptorSet {
        let layouts = [layout];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let ds = unsafe { device.allocate_descriptor_sets(&alloc_info) }
            .expect("Failed to allocate compute descriptor set")[0];

        let sampler_info = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(sampler_view)
            .sampler(sampler);
        let storage_info = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::GENERAL)
            .image_view(storage_view);

        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&sampler_info)),
            vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .image_info(std::slice::from_ref(&storage_info)),
        ];
        unsafe {
            device.update_descriptor_sets(&writes, &[]);
        }
        ds
    }

    /// Generate mipmaps for a cubemap using blit chain (all 6 faces).
    fn generate_cubemap_mipmaps(
        device: &ash::Device,
        cmd_buf: vk::CommandBuffer,
        cubemap: &Cubemap,
    ) {
        let mip_levels = cubemap.mip_levels();
        if mip_levels <= 1 {
            // No mipmaps to generate, just transition to SHADER_READ_ONLY.
            Cubemap::transition_all_layers(
                device,
                cmd_buf,
                cubemap.image(),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                1,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            );
            return;
        }

        let mut mip_width = cubemap.width() as i32;

        for i in 1..mip_levels {
            // Transition mip (i-1): TRANSFER_DST → TRANSFER_SRC.
            let barrier_to_src = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(cubemap.image())
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: i - 1,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: Cubemap::NUM_FACES,
                })
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ);

            unsafe {
                device.cmd_pipeline_barrier(
                    cmd_buf,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_src],
                );
            }

            let next_width = (mip_width / 2).max(1);

            for face in 0..Cubemap::NUM_FACES {
                let blit = vk::ImageBlit {
                    src_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: i - 1,
                        base_array_layer: face,
                        layer_count: 1,
                    },
                    src_offsets: [
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: mip_width,
                            y: mip_width,
                            z: 1,
                        },
                    ],
                    dst_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: i,
                        base_array_layer: face,
                        layer_count: 1,
                    },
                    dst_offsets: [
                        vk::Offset3D { x: 0, y: 0, z: 0 },
                        vk::Offset3D {
                            x: next_width,
                            y: next_width,
                            z: 1,
                        },
                    ],
                };
                unsafe {
                    device.cmd_blit_image(
                        cmd_buf,
                        cubemap.image(),
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        cubemap.image(),
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[blit],
                        vk::Filter::LINEAR,
                    );
                }
            }

            // Transition mip (i-1): TRANSFER_SRC → SHADER_READ_ONLY.
            // dst_stage includes COMPUTE_SHADER because IBL preprocessing reads
            // this cubemap from compute dispatches (irradiance, prefilter).
            let barrier_to_read = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(cubemap.image())
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: i - 1,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: Cubemap::NUM_FACES,
                })
                .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                .dst_access_mask(vk::AccessFlags::SHADER_READ);

            unsafe {
                device.cmd_pipeline_barrier(
                    cmd_buf,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER
                        | vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier_to_read],
                );
            }

            mip_width = next_width;
        }

        // Transition last mip: TRANSFER_DST → SHADER_READ_ONLY.
        let barrier_last = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(cubemap.image())
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: mip_levels - 1,
                level_count: 1,
                base_array_layer: 0,
                layer_count: Cubemap::NUM_FACES,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER | vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_last],
            );
        }
    }
}

impl Drop for EnvironmentMapSystem {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_descriptor_set_layout(self.compute_sampler2d_ds_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.compute_sampler_cube_ds_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.compute_brdf_ds_layout, None);
            self.device
                .destroy_descriptor_pool(self.compute_ds_pool, None);
        }
    }
}
