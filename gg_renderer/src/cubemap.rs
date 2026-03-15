use std::sync::{Arc, Mutex};

use ash::vk;

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use gg_core::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// Cubemap — 6-face cube texture (HDR environment maps, IBL)
// ---------------------------------------------------------------------------

/// A cubemap texture backed by a Vulkan image with 6 array layers and a
/// `VK_IMAGE_VIEW_TYPE_CUBE` view. Used for environment maps and IBL.
pub struct Cubemap {
    image: vk::Image,
    _allocation: GpuAllocation,
    /// CUBE view over all 6 faces and all mip levels (for sampling in shaders).
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    /// Per-face per-mip views for compute shader `imageStore` writes.
    /// Indexed as `face_mip_views[face * mip_levels + mip]`.
    face_mip_views: Vec<vk::ImageView>,
    width: u32,
    mip_levels: u32,
    _format: vk::Format,
    device: ash::Device,
}

impl Cubemap {
    /// Number of faces in a cubemap.
    pub const NUM_FACES: u32 = 6;

    /// Create a cubemap with the given resolution, format, and mip level count.
    ///
    /// The image is created in `UNDEFINED` layout. Use
    /// [`Cubemap::transition_all_layers`] to prepare it for use.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        width: u32,
        mip_levels: u32,
        format: vk::Format,
        filter: vk::Filter,
    ) -> EngineResult<Self> {
        // Create image: TYPE_2D with CUBE_COMPATIBLE flag, 6 array layers.
        let usage = vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::STORAGE
            | vk::ImageUsageFlags::TRANSFER_DST
            | vk::ImageUsageFlags::TRANSFER_SRC;

        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height: width, // cubemap faces are square
                depth: 1,
            })
            .mip_levels(mip_levels)
            .array_layers(Self::NUM_FACES)
            .format(format)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1)
            .flags(vk::ImageCreateFlags::CUBE_COMPATIBLE);

        let image = unsafe { device.create_image(&image_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create cubemap image: {e}")))?;

        let allocation = GpuAllocator::allocate_for_image(
            allocator,
            device,
            image,
            "Cubemap",
            MemoryLocation::GpuOnly,
        )?;

        // Cube view: all 6 faces, all mip levels.
        let cube_view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::CUBE)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count: Self::NUM_FACES,
            });

        let image_view = unsafe { device.create_image_view(&cube_view_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create cubemap CUBE view: {e}")))?;

        // Per-face per-mip 2D views for compute shader writes (imageStore).
        let mut face_mip_views = Vec::with_capacity((Self::NUM_FACES * mip_levels) as usize);
        for face in 0..Self::NUM_FACES {
            for mip in 0..mip_levels {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: mip,
                        level_count: 1,
                        base_array_layer: face,
                        layer_count: 1,
                    });

                let view = unsafe { device.create_image_view(&view_info, None) }.map_err(|e| {
                    EngineError::Gpu(format!(
                        "Failed to create cubemap face view (face={face}, mip={mip}): {e}"
                    ))
                })?;
                face_mip_views.push(view);
            }
        }

        // Sampler: linear, clamp-to-edge, with trilinear mip interpolation.
        let mipmap_mode = if mip_levels > 1 {
            vk::SamplerMipmapMode::LINEAR
        } else {
            match filter {
                vk::Filter::LINEAR => vk::SamplerMipmapMode::LINEAR,
                _ => vk::SamplerMipmapMode::NEAREST,
            }
        };

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(filter)
            .min_filter(filter)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .anisotropy_enable(false)
            .max_anisotropy(1.0)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .mipmap_mode(mipmap_mode)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(if mip_levels > 1 {
                mip_levels as f32
            } else {
                0.0
            });

        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create cubemap sampler: {e}")))?;

        Ok(Self {
            image,
            _allocation: allocation,
            image_view,
            sampler,
            face_mip_views,
            width,
            mip_levels,
            _format: format,
            device: device.clone(),
        })
    }

    /// The Vulkan image handle.
    pub fn image(&self) -> vk::Image {
        self.image
    }

    /// The CUBE image view (for sampling in shaders as `samplerCube`).
    pub fn image_view(&self) -> vk::ImageView {
        self.image_view
    }

    /// The sampler for this cubemap.
    pub fn sampler(&self) -> vk::Sampler {
        self.sampler
    }

    /// Get the 2D view for a specific face and mip level (for compute `imageStore`).
    pub fn face_mip_view(&self, face: u32, mip: u32) -> vk::ImageView {
        self.face_mip_views[(face * self.mip_levels + mip) as usize]
    }

    /// Face resolution at the base mip level.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Number of mip levels.
    pub fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    /// Resolution at a given mip level.
    pub fn mip_width(&self, mip: u32) -> u32 {
        (self.width >> mip).max(1)
    }

    /// Insert a pipeline barrier transitioning all 6 faces (all mips) between layouts.
    pub fn transition_all_layers(
        device: &ash::Device,
        cmd_buf: vk::CommandBuffer,
        image: vk::Image,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
        mip_levels: u32,
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
                layer_count: Self::NUM_FACES,
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

    /// Insert a pipeline barrier transitioning a single face+mip.
    pub fn transition_face_mip(
        device: &ash::Device,
        cmd_buf: vk::CommandBuffer,
        image: vk::Image,
        face: u32,
        mip: u32,
        old_layout: vk::ImageLayout,
        new_layout: vk::ImageLayout,
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
                base_mip_level: mip,
                level_count: 1,
                base_array_layer: face,
                layer_count: 1,
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
}

impl Drop for Cubemap {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_sampler(self.sampler, None);
            for &view in &self.face_mip_views {
                self.device.destroy_image_view(view, None);
            }
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
            // _allocation drops automatically via GpuAllocation::drop
        }
    }
}
