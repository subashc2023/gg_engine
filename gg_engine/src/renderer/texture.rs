use std::path::Path;
use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};

use super::buffer::create_staging_buffer;
use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::RendererResources;

use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// TransferBatch — batches GPU upload commands with fence-based tracking
// ---------------------------------------------------------------------------

/// A submitted batch of transfer commands tracked by a Vulkan fence.
struct PendingTransfer {
    fence: vk::Fence,
    command_buffer: vk::CommandBuffer,
    /// Staging buffers + allocations kept alive until the fence signals.
    staging_resources: Vec<(vk::Buffer, GpuAllocation)>,
}

/// Batches texture/font upload commands into single command buffer submissions,
/// using fences instead of `queue_wait_idle()` to track completion.
///
/// Staging buffers are kept alive until their fence signals, then freed in bulk.
/// Since all submissions go to the same graphics queue, pipeline barriers ensure
/// that uploaded textures are usable for rendering immediately after submission.
pub(crate) struct TransferBatch {
    device: ash::Device,
    command_pool: vk::CommandPool,
    graphics_queue: vk::Queue,

    /// Command buffer being recorded (None if no uploads pending).
    active_cmd_buf: Option<vk::CommandBuffer>,
    /// Staging resources for the active (not yet submitted) batch.
    active_staging: Vec<(vk::Buffer, GpuAllocation)>,

    /// Submitted batches waiting for fence completion.
    pending: Vec<PendingTransfer>,
}

impl TransferBatch {
    pub fn new(
        device: &ash::Device,
        command_pool: vk::CommandPool,
        graphics_queue: vk::Queue,
    ) -> Self {
        Self {
            device: device.clone(),
            command_pool,
            graphics_queue,
            active_cmd_buf: None,
            active_staging: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Ensure a command buffer is being recorded. Allocates one lazily.
    fn ensure_active(&mut self) -> Result<(), String> {
        if self.active_cmd_buf.is_some() {
            return Ok(());
        }
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_pool(self.command_pool)
            .command_buffer_count(1);

        let cmd_buf = unsafe { self.device.allocate_command_buffers(&alloc_info) }
            .map_err(|e| format!("Failed to allocate transfer command buffer: {e}"))?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            self.device
                .begin_command_buffer(cmd_buf, &begin_info)
                .map_err(|e| format!("Failed to begin transfer command buffer: {e}"))?;
        }

        self.active_cmd_buf = Some(cmd_buf);
        Ok(())
    }

    /// Record a staging-buffer-to-image copy with layout transitions.
    /// The staging buffer and allocation are held until the fence signals.
    pub fn record_image_upload(
        &mut self,
        image: vk::Image,
        staging_buffer: vk::Buffer,
        staging_alloc: GpuAllocation,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.ensure_active()?;
        let cmd_buf = self.active_cmd_buf.unwrap();

        transition_image_layout(
            &self.device,
            cmd_buf,
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
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
            image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
            image_extent: vk::Extent3D {
                width,
                height,
                depth: 1,
            },
        };

        unsafe {
            self.device.cmd_copy_buffer_to_image(
                cmd_buf,
                staging_buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }

        transition_image_layout(
            &self.device,
            cmd_buf,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );

        self.active_staging.push((staging_buffer, staging_alloc));
        Ok(())
    }

    /// Submit the active command buffer with a fence. No-op if nothing recorded.
    pub fn submit(&mut self) -> Result<(), String> {
        let cmd_buf = match self.active_cmd_buf.take() {
            Some(cb) => cb,
            None => return Ok(()),
        };

        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { self.device.create_fence(&fence_info, None) }
            .map_err(|e| format!("Failed to create transfer fence: {e}"))?;

        unsafe {
            self.device
                .end_command_buffer(cmd_buf)
                .map_err(|e| format!("Failed to end transfer command buffer: {e}"))?;

            let cmd_bufs = [cmd_buf];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| format!("Failed to submit transfer batch: {e}"))?;
        }

        let staging = std::mem::take(&mut self.active_staging);
        self.pending.push(PendingTransfer {
            fence,
            command_buffer: cmd_buf,
            staging_resources: staging,
        });
        Ok(())
    }

    /// Poll all pending fences. Free staging resources for completed batches.
    pub fn poll(&mut self) {
        self.pending.retain_mut(|transfer| {
            let signaled = unsafe {
                self.device.get_fence_status(transfer.fence).unwrap_or(false)
            };
            if signaled {
                // Free staging buffers (GpuAllocation auto-frees on drop).
                for (buffer, _alloc) in transfer.staging_resources.drain(..) {
                    unsafe { self.device.destroy_buffer(buffer, None); }
                }
                unsafe {
                    self.device.destroy_fence(transfer.fence, None);
                    self.device
                        .free_command_buffers(self.command_pool, &[transfer.command_buffer]);
                }
                false // remove from pending
            } else {
                true // keep waiting
            }
        });
    }

    /// Wait for all pending transfers to complete (used at shutdown).
    pub fn wait_all(&mut self) {
        // Submit any active batch first.
        if let Err(e) = self.submit() {
            log::error!(target: "gg_engine", "Failed to submit final transfer batch: {e}");
        }

        if self.pending.is_empty() {
            return;
        }

        let fences: Vec<vk::Fence> = self.pending.iter().map(|t| t.fence).collect();
        unsafe {
            let _ = self.device.wait_for_fences(&fences, true, u64::MAX);
        }

        // Clean up all pending transfers.
        for transfer in self.pending.drain(..) {
            for (buffer, _alloc) in transfer.staging_resources {
                unsafe { self.device.destroy_buffer(buffer, None); }
            }
            unsafe {
                self.device.destroy_fence(transfer.fence, None);
                self.device
                    .free_command_buffers(self.command_pool, &[transfer.command_buffer]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ImageFormat — pixel format enum
// ---------------------------------------------------------------------------

/// Pixel format for textures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageFormat {
    /// 8-bit RGBA, sRGB color space (standard for color textures).
    #[default]
    Rgba8Srgb,
    /// 8-bit RGBA, linear/UNORM (used for SDF font atlases, data textures).
    Rgba8Unorm,
}

impl ImageFormat {
    fn to_vk(self) -> vk::Format {
        match self {
            ImageFormat::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
            ImageFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        }
    }
}

// ---------------------------------------------------------------------------
// TextureSpecification — creation parameters
// ---------------------------------------------------------------------------

/// Specification for creating a [`Texture2D`].
///
/// Use [`TextureSpecification::default()`] for standard color textures
/// (sRGB, nearest filtering, repeat wrap). Override fields as needed.
#[derive(Clone, Debug)]
pub struct TextureSpecification {
    /// Pixel format.
    pub format: ImageFormat,
    /// Magnification and minification filter.
    pub filter: vk::Filter,
    /// Texture address / wrap mode.
    pub address_mode: vk::SamplerAddressMode,
    /// Enable anisotropic filtering.
    pub anisotropy: bool,
    /// Maximum anisotropy level (ignored if `anisotropy` is false).
    pub max_anisotropy: f32,
}

impl Default for TextureSpecification {
    fn default() -> Self {
        Self {
            format: ImageFormat::Rgba8Srgb,
            filter: vk::Filter::NEAREST,
            address_mode: vk::SamplerAddressMode::REPEAT,
            anisotropy: true,
            max_anisotropy: 16.0,
        }
    }
}

impl TextureSpecification {
    /// Preset for SDF font atlases: linear filtering, UNORM format, clamp-to-edge.
    pub fn font_atlas() -> Self {
        Self {
            format: ImageFormat::Rgba8Unorm,
            filter: vk::Filter::LINEAR,
            address_mode: vk::SamplerAddressMode::CLAMP_TO_EDGE,
            anisotropy: false,
            max_anisotropy: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// TextureCpuData — CPU-side pixel data (Send-safe, no Vulkan types)
// ---------------------------------------------------------------------------

/// CPU-side texture data ready for GPU upload. Produced by background
/// threads (image decode), consumed on the main thread for Vulkan upload.
pub struct TextureCpuData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub spec: TextureSpecification,
}

// ---------------------------------------------------------------------------
// Texture2D
// ---------------------------------------------------------------------------

/// A 2D texture backed by a Vulkan image, image view, sampler, and descriptor
/// set. Created via [`Renderer::create_texture_from_file`] or
/// [`Renderer::create_texture_from_rgba8`].
pub struct Texture2D {
    image: vk::Image,
    _allocation: GpuAllocation,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    descriptor_set: vk::DescriptorSet,
    descriptor_pool: vk::DescriptorPool,
    bindless_index: u32,
    _width: u32,
    _height: u32,
    device: ash::Device,
}

impl Texture2D {
    /// Load an image file and return CPU-side pixel data (no GPU work).
    /// Suitable for calling on a background thread.
    pub(crate) fn load_cpu_data(path: &Path, spec: TextureSpecification) -> Result<TextureCpuData, String> {
        let img = image::open(path)
            .map_err(|e| format!("Failed to load texture '{}': {e}", path.display()))?;
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(TextureCpuData {
            width,
            height,
            pixels: rgba.into_raw(),
            spec,
        })
    }

    /// Load a texture from an image file (PNG, JPEG, etc).
    ///
    /// Returns `None` if the file cannot be loaded or decoded.
    pub(crate) fn from_file(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        path: &Path,
    ) -> Option<Self> {
        let _timer = ProfileTimer::new("Texture2D::from_file");
        let img = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                log::error!("Failed to load texture '{}': {e}", path.display());
                return None;
            }
        };
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let pixels = rgba.into_raw();

        match Self::from_rgba8(res, allocator, width, height, &pixels) {
            Ok(tex) => Some(tex),
            Err(e) => {
                log::error!("Failed to create texture GPU resources: {e}");
                None
            }
        }
    }

    /// Create a texture from raw RGBA8 pixel data with default spec (sRGB, nearest, repeat).
    pub(crate) fn from_rgba8(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<Self, String> {
        Self::from_rgba8_with_spec(res, allocator, width, height, pixels, &TextureSpecification::default())
    }

    /// Create a texture from raw RGBA8 pixel data with a custom specification.
    pub(crate) fn from_rgba8_with_spec(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        width: u32,
        height: u32,
        pixels: &[u8],
        spec: &TextureSpecification,
    ) -> Result<Self, String> {
        let _timer = ProfileTimer::new("Texture2D::from_rgba8_with_spec");
        let image_size = (width * height * 4) as vk::DeviceSize;
        assert_eq!(pixels.len() as vk::DeviceSize, image_size);

        let device = res.device;
        let graphics_queue = res.graphics_queue;
        let command_pool = res.command_pool;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;

        let vk_format = spec.format.to_vk();

        // 1. Create staging buffer with pixel data.
        let (staging_buffer, _staging_alloc) =
            create_staging_buffer(allocator, device, pixels)?;

        // 2. Create Vulkan image.
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(vk_format)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1);

        let image =
            unsafe { device.create_image(&image_info, None) }
                .map_err(|e| format!("Failed to create texture image: {e}"))?;

        // 3. Allocate and bind DEVICE_LOCAL memory via sub-allocator.
        let allocation =
            GpuAllocator::allocate_for_image(allocator, device, image, "Texture2D", MemoryLocation::GpuOnly)?;

        // 4. One-shot command buffer: transition + copy + transition.
        execute_one_shot(device, command_pool, graphics_queue, |cmd_buf| {
            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
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
                image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };

            unsafe {
                device.cmd_copy_buffer_to_image(
                    cmd_buf,
                    staging_buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }

            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        })?;

        // 5. Staging buffer + allocation auto-freed when _staging_alloc drops.
        unsafe {
            device.destroy_buffer(staging_buffer, None);
        }
        drop(_staging_alloc);

        // 6. Create image view.
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk_format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });

        let image_view = unsafe { device.create_image_view(&view_info, None) }
            .map_err(|e| format!("Failed to create texture image view: {e}"))?;

        // 7. Create sampler.
        let mipmap_mode = match spec.filter {
            vk::Filter::LINEAR => vk::SamplerMipmapMode::LINEAR,
            _ => vk::SamplerMipmapMode::NEAREST,
        };

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(spec.filter)
            .min_filter(spec.filter)
            .address_mode_u(spec.address_mode)
            .address_mode_v(spec.address_mode)
            .address_mode_w(spec.address_mode)
            .anisotropy_enable(spec.anisotropy)
            .max_anisotropy(spec.max_anisotropy)
            .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .mipmap_mode(mipmap_mode)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(0.0);

        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .map_err(|e| format!("Failed to create texture sampler: {e}"))?;

        // 8. Allocate descriptor set and write combined image sampler.
        let layouts = [descriptor_set_layout];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);

        let ds_vec = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .map_err(|e| format!("Failed to allocate texture descriptor set: {e}"))?;
        let descriptor_set = ds_vec[0];

        let image_info_ds = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(image_view)
            .sampler(sampler);

        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(std::slice::from_ref(&image_info_ds));

        unsafe {
            device.update_descriptor_sets(&[write], &[]);
        }

        Ok(Self {
            image,
            _allocation: allocation,
            image_view,
            sampler,
            descriptor_set,
            descriptor_pool,
            bindless_index: 0,
            _width: width,
            _height: height,
            device: device.clone(),
        })
    }

    /// The descriptor set for binding this texture in a draw call.
    pub fn descriptor_set(&self) -> vk::DescriptorSet {
        self.descriptor_set
    }

    /// The Vulkan image view backing this texture.
    pub(crate) fn image_view(&self) -> vk::ImageView {
        self.image_view
    }

    /// The Vulkan sampler for this texture.
    pub(crate) fn sampler(&self) -> vk::Sampler {
        self.sampler
    }

    /// The width of the texture in pixels.
    pub fn width(&self) -> u32 {
        self._width
    }

    /// The height of the texture in pixels.
    pub fn height(&self) -> u32 {
        self._height
    }

    /// Opaque handle for registering this texture with egui via
    /// [`Application::egui_user_textures`]. Returns the raw Vulkan
    /// descriptor set as a `u64`.
    pub fn egui_handle(&self) -> u64 {
        self.descriptor_set.as_raw()
    }

    /// The global bindless descriptor array index for this texture.
    pub fn bindless_index(&self) -> u32 {
        self.bindless_index
    }

    /// Set the bindless descriptor array index (called by Renderer after registration).
    pub(crate) fn set_bindless_index(&mut self, index: u32) {
        self.bindless_index = index;
    }

    /// Create a texture from pre-loaded CPU data, recording the upload into a
    /// [`TransferBatch`] instead of blocking on `queue_wait_idle`.
    pub(crate) fn from_cpu_data_batched(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        data: &TextureCpuData,
        batch: &mut TransferBatch,
    ) -> Result<Self, String> {
        Self::from_rgba8_with_spec_batched(
            res, allocator, data.width, data.height, &data.pixels, &data.spec, batch,
        )
    }

    /// Create a texture from raw RGBA8 pixel data, recording the staging copy
    /// into a [`TransferBatch`] for deferred, fence-tracked submission.
    ///
    /// The returned texture is usable for rendering after [`TransferBatch::submit`]
    /// because subsequent draw commands on the same queue are serialized behind
    /// the pipeline barriers recorded here.
    pub(crate) fn from_rgba8_with_spec_batched(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        width: u32,
        height: u32,
        pixels: &[u8],
        spec: &TextureSpecification,
        batch: &mut TransferBatch,
    ) -> Result<Self, String> {
        let _timer = ProfileTimer::new("Texture2D::from_rgba8_with_spec_batched");
        let image_size = (width * height * 4) as vk::DeviceSize;
        assert_eq!(pixels.len() as vk::DeviceSize, image_size);

        let device = res.device;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;

        let vk_format = spec.format.to_vk();

        // 1. Create staging buffer with pixel data.
        let (staging_buffer, staging_alloc) =
            create_staging_buffer(allocator, device, pixels)?;

        // 2. Create Vulkan image.
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(vk_format)
            .tiling(vk::ImageTiling::OPTIMAL)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1);

        let image =
            unsafe { device.create_image(&image_info, None) }
                .map_err(|e| format!("Failed to create texture image: {e}"))?;

        // 3. Allocate and bind DEVICE_LOCAL memory via sub-allocator.
        let allocation =
            GpuAllocator::allocate_for_image(allocator, device, image, "Texture2D", MemoryLocation::GpuOnly)?;

        // 4. Record the staging copy + layout transitions into the batch.
        batch.record_image_upload(image, staging_buffer, staging_alloc, width, height)?;

        // 5. Create image view.
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk_format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });

        let image_view = unsafe { device.create_image_view(&view_info, None) }
            .map_err(|e| format!("Failed to create texture image view: {e}"))?;

        // 6. Create sampler.
        let mipmap_mode = match spec.filter {
            vk::Filter::LINEAR => vk::SamplerMipmapMode::LINEAR,
            _ => vk::SamplerMipmapMode::NEAREST,
        };

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(spec.filter)
            .min_filter(spec.filter)
            .address_mode_u(spec.address_mode)
            .address_mode_v(spec.address_mode)
            .address_mode_w(spec.address_mode)
            .anisotropy_enable(spec.anisotropy)
            .max_anisotropy(spec.max_anisotropy)
            .border_color(vk::BorderColor::FLOAT_TRANSPARENT_BLACK)
            .unnormalized_coordinates(false)
            .compare_enable(false)
            .mipmap_mode(mipmap_mode)
            .mip_lod_bias(0.0)
            .min_lod(0.0)
            .max_lod(0.0);

        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .map_err(|e| format!("Failed to create texture sampler: {e}"))?;

        // 7. Allocate descriptor set and write combined image sampler.
        let layouts = [descriptor_set_layout];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);

        let ds_vec = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .map_err(|e| format!("Failed to allocate texture descriptor set: {e}"))?;
        let descriptor_set = ds_vec[0];

        let image_info_ds = vk::DescriptorImageInfo::default()
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image_view(image_view)
            .sampler(sampler);

        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(std::slice::from_ref(&image_info_ds));

        unsafe {
            device.update_descriptor_sets(&[write], &[]);
        }

        Ok(Self {
            image,
            _allocation: allocation,
            image_view,
            sampler,
            descriptor_set,
            descriptor_pool,
            bindless_index: 0,
            _width: width,
            _height: height,
            device: device.clone(),
        })
    }
}

impl Drop for Texture2D {
    fn drop(&mut self) {
        unsafe {
            // Free the per-texture descriptor set back to the pool.
            let _ = self
                .device
                .free_descriptor_sets(self.descriptor_pool, &[self.descriptor_set]);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
        // GpuAllocation auto-frees memory on drop.
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Record, submit, and wait for a one-shot command buffer.
fn execute_one_shot(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    record: impl FnOnce(vk::CommandBuffer),
) -> Result<(), String> {
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);

    let cmd_buf = unsafe { device.allocate_command_buffers(&alloc_info) }
        .map_err(|e| format!("Failed to allocate one-shot command buffer: {e}"))?[0];

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

    unsafe {
        device
            .begin_command_buffer(cmd_buf, &begin_info)
            .map_err(|e| format!("Failed to begin one-shot command buffer: {e}"))?;
    }

    record(cmd_buf);

    unsafe {
        device
            .end_command_buffer(cmd_buf)
            .map_err(|e| format!("Failed to end one-shot command buffer: {e}"))?;

        let cmd_bufs = [cmd_buf];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
        device
            .queue_submit(queue, &[submit_info], vk::Fence::null())
            .map_err(|e| format!("Failed to submit one-shot command buffer: {e}"))?;
        device
            .queue_wait_idle(queue)
            .map_err(|e| format!("Failed to wait for queue idle: {e}"))?;

        device.free_command_buffers(command_pool, &[cmd_buf]);
    }
    Ok(())
}

/// Insert a pipeline barrier to transition an image between layouts.
fn transition_image_layout(
    device: &ash::Device,
    cmd_buf: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
) {
    let (src_access, dst_access, src_stage, dst_stage) = match (old_layout, new_layout) {
        (vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL) => (
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        (vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL) => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        _ => {
            log::error!(
                "Unsupported image layout transition: {:?} -> {:?} — skipping barrier",
                old_layout,
                new_layout
            );
            return;
        }
    };

    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old_layout)
        .new_layout(new_layout)
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
