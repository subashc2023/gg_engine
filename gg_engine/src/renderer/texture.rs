use std::path::Path;
use std::sync::{Arc, Mutex};

use ash::vk::{self, Handle};

use super::buffer::create_staging_buffer;
use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::RendererResources;

use crate::error::{EngineError, EngineResult};
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
    fn ensure_active(&mut self) -> EngineResult<()> {
        if self.active_cmd_buf.is_some() {
            return Ok(());
        }
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_pool(self.command_pool)
            .command_buffer_count(1);

        let cmd_buf =
            unsafe { self.device.allocate_command_buffers(&alloc_info) }.map_err(|e| {
                EngineError::Gpu(format!("Failed to allocate transfer command buffer: {e}"))
            })?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            self.device
                .begin_command_buffer(cmd_buf, &begin_info)
                .map_err(|e| {
                    EngineError::Gpu(format!("Failed to begin transfer command buffer: {e}"))
                })?;
        }

        self.active_cmd_buf = Some(cmd_buf);
        Ok(())
    }

    /// Record a staging-buffer-to-image copy with layout transitions.
    /// The staging buffer and allocation are held until the fence signals.
    ///
    /// When `mip_levels > 1`, generates mipmaps via blit chain after uploading
    /// mip level 0. The image must have been created with `TRANSFER_SRC` usage.
    pub fn record_image_upload(
        &mut self,
        image: vk::Image,
        staging_buffer: vk::Buffer,
        staging_alloc: GpuAllocation,
        width: u32,
        height: u32,
        mip_levels: u32,
    ) -> EngineResult<()> {
        self.ensure_active()?;
        let cmd_buf = self.active_cmd_buf.unwrap();

        transition_image_layout(
            &self.device,
            cmd_buf,
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            0,
            mip_levels,
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

        if mip_levels > 1 {
            generate_mipmaps_cmd(&self.device, cmd_buf, image, width, height, mip_levels);
        } else {
            transition_image_layout(
                &self.device,
                cmd_buf,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                0,
                1,
            );
        }

        self.active_staging.push((staging_buffer, staging_alloc));
        Ok(())
    }

    /// Submit the active command buffer with a fence. No-op if nothing recorded.
    pub fn submit(&mut self) -> EngineResult<()> {
        let cmd_buf = match self.active_cmd_buf.take() {
            Some(cb) => cb,
            None => return Ok(()),
        };

        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { self.device.create_fence(&fence_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create transfer fence: {e}")))?;

        unsafe {
            self.device.end_command_buffer(cmd_buf).map_err(|e| {
                EngineError::Gpu(format!("Failed to end transfer command buffer: {e}"))
            })?;

            let cmd_bufs = [cmd_buf];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            self.device
                .queue_submit(self.graphics_queue, &[submit_info], fence)
                .map_err(|e| EngineError::Gpu(format!("Failed to submit transfer batch: {e}")))?;
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
                self.device
                    .get_fence_status(transfer.fence)
                    .unwrap_or(false)
            };
            if signaled {
                // Free staging buffers (GpuAllocation auto-frees on drop).
                for (buffer, _alloc) in transfer.staging_resources.drain(..) {
                    unsafe {
                        self.device.destroy_buffer(buffer, None);
                    }
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
                unsafe {
                    self.device.destroy_buffer(buffer, None);
                }
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
    // -- Compressed formats (require pre-compressed data, no CPU decoding) --
    /// BC1 / DXT1, sRGB (4×4 block, 8 bytes). RGB, 1-bit alpha.
    Bc1Srgb,
    /// BC3 / DXT5, sRGB (4×4 block, 16 bytes). RGB + full alpha.
    Bc3Srgb,
    /// BC5, unsigned UNORM (4×4 block, 16 bytes). Two-channel (normal maps).
    Bc5Unorm,
    /// BC7, sRGB (4×4 block, 16 bytes). High quality RGBA.
    Bc7Srgb,
    /// ASTC 4×4, sRGB (4×4 block, 16 bytes).
    Astc4x4Srgb,
    /// ASTC 6×6, sRGB (6×6 block, 16 bytes).
    Astc6x6Srgb,
    /// ASTC 8×8, sRGB (8×8 block, 16 bytes).
    Astc8x8Srgb,
    // -- Floating-point formats (HDR / data) --
    /// 16-bit float RGBA (8 bytes/pixel). Used for HDR cubemaps, IBL textures.
    Rgba16Float,
    /// 16-bit float RG (4 bytes/pixel). Used for BRDF integration LUT.
    Rg16Float,
}

impl ImageFormat {
    pub(crate) fn to_vk(self) -> vk::Format {
        match self {
            ImageFormat::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
            ImageFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
            ImageFormat::Bc1Srgb => vk::Format::BC1_RGBA_SRGB_BLOCK,
            ImageFormat::Bc3Srgb => vk::Format::BC3_SRGB_BLOCK,
            ImageFormat::Bc5Unorm => vk::Format::BC5_UNORM_BLOCK,
            ImageFormat::Bc7Srgb => vk::Format::BC7_SRGB_BLOCK,
            ImageFormat::Astc4x4Srgb => vk::Format::ASTC_4X4_SRGB_BLOCK,
            ImageFormat::Astc6x6Srgb => vk::Format::ASTC_6X6_SRGB_BLOCK,
            ImageFormat::Astc8x8Srgb => vk::Format::ASTC_8X8_SRGB_BLOCK,
            ImageFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
            ImageFormat::Rg16Float => vk::Format::R16G16_SFLOAT,
        }
    }

    /// Whether this format uses block compression (BC/ASTC).
    pub fn is_compressed(self) -> bool {
        !matches!(
            self,
            ImageFormat::Rgba8Srgb
                | ImageFormat::Rgba8Unorm
                | ImageFormat::Rgba16Float
                | ImageFormat::Rg16Float
        )
    }

    /// Block dimensions (width, height) for compressed formats. Returns (1, 1) for uncompressed.
    pub fn block_dimensions(self) -> (u32, u32) {
        match self {
            ImageFormat::Rgba8Srgb
            | ImageFormat::Rgba8Unorm
            | ImageFormat::Rgba16Float
            | ImageFormat::Rg16Float => (1, 1),
            ImageFormat::Bc1Srgb
            | ImageFormat::Bc3Srgb
            | ImageFormat::Bc5Unorm
            | ImageFormat::Bc7Srgb
            | ImageFormat::Astc4x4Srgb => (4, 4),
            ImageFormat::Astc6x6Srgb => (6, 6),
            ImageFormat::Astc8x8Srgb => (8, 8),
        }
    }

    /// Bytes per block for compressed formats, or bytes per pixel (4) for uncompressed.
    pub fn block_bytes(self) -> u32 {
        match self {
            ImageFormat::Rgba8Srgb | ImageFormat::Rgba8Unorm => 4,
            ImageFormat::Rg16Float => 4,
            ImageFormat::Rgba16Float => 8,
            ImageFormat::Bc1Srgb => 8,
            _ => 16, // BC3, BC5, BC7, all ASTC variants
        }
    }

    /// Calculate the expected data size in bytes for a texture of the given dimensions.
    pub fn data_size(self, width: u32, height: u32) -> u64 {
        let (bw, bh) = self.block_dimensions();
        let blocks_x = width.div_ceil(bw);
        let blocks_y = height.div_ceil(bh);
        blocks_x as u64 * blocks_y as u64 * self.block_bytes() as u64
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
    /// Generate mipmaps via GPU blit chain. Useful for textures viewed at
    /// varying zoom levels. Not recommended for pixel-art with NEAREST filtering.
    pub generate_mipmaps: bool,
}

impl Default for TextureSpecification {
    fn default() -> Self {
        Self {
            format: ImageFormat::Rgba8Srgb,
            filter: vk::Filter::NEAREST,
            address_mode: vk::SamplerAddressMode::REPEAT,
            anisotropy: true,
            max_anisotropy: 16.0,
            generate_mipmaps: false,
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
            generate_mipmaps: false,
        }
    }

    /// Preset with linear filtering and mipmap generation enabled (trilinear).
    /// Good for photographic or non-pixel-art textures viewed at varying zoom.
    pub fn linear_mipmapped() -> Self {
        Self {
            filter: vk::Filter::LINEAR,
            generate_mipmaps: true,
            ..Self::default()
        }
    }

    /// Preset with nearest filtering and mipmap generation enabled.
    /// Sharp pixels when zoomed in, smooth mip-level blending when zoomed out.
    /// Ideal for pixel art that needs to look crisp up close but avoid
    /// moiré/aliasing artifacts at small scales.
    pub fn nearest_mipmapped() -> Self {
        Self {
            generate_mipmaps: true,
            ..Self::default() // filter defaults to NEAREST
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
    pub(crate) fn load_cpu_data(
        path: &Path,
        spec: TextureSpecification,
    ) -> EngineResult<TextureCpuData> {
        let img = image::open(path).map_err(|e| {
            EngineError::Gpu(format!("Failed to load texture '{}': {e}", path.display()))
        })?;
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
    ) -> EngineResult<Self> {
        Self::from_rgba8_with_spec(
            res,
            allocator,
            width,
            height,
            pixels,
            &TextureSpecification::default(),
        )
    }

    /// Create a texture from raw RGBA8 pixel data with a custom specification.
    pub(crate) fn from_rgba8_with_spec(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        width: u32,
        height: u32,
        pixels: &[u8],
        spec: &TextureSpecification,
    ) -> EngineResult<Self> {
        let _timer = ProfileTimer::new("Texture2D::from_rgba8_with_spec");
        let image_size = (width * height * 4) as vk::DeviceSize;
        assert_eq!(pixels.len() as vk::DeviceSize, image_size);

        let device = res.device;
        let graphics_queue = res.graphics_queue;
        let command_pool = res.command_pool;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;

        let vk_format = spec.format.to_vk();
        let mip_levels = if spec.generate_mipmaps {
            calculate_mip_levels(width, height)
        } else {
            1
        };

        // 1. Create staging buffer with pixel data.
        let (staging_buffer, _staging_alloc) = create_staging_buffer(allocator, device, pixels)?;

        // 2. Create Vulkan image + GPU memory.
        let (image, allocation) =
            create_texture_image(device, allocator, width, height, mip_levels, vk_format)?;

        // 3. One-shot command buffer: transition + copy + mipmap generation.
        execute_one_shot(device, command_pool, graphics_queue, |cmd_buf| {
            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                0,
                mip_levels,
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

            if mip_levels > 1 {
                generate_mipmaps_cmd(device, cmd_buf, image, width, height, mip_levels);
            } else {
                transition_image_layout(
                    device,
                    cmd_buf,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    0,
                    1,
                );
            }
        })?;

        // 4. Staging buffer + allocation auto-freed when _staging_alloc drops.
        unsafe {
            device.destroy_buffer(staging_buffer, None);
        }
        drop(_staging_alloc);

        // 5. Create image view, sampler, and descriptor set.
        let (image_view, sampler, descriptor_set) = create_texture_view_sampler_ds(
            device,
            image,
            vk_format,
            mip_levels,
            spec,
            descriptor_pool,
            descriptor_set_layout,
        )?;

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

    /// Estimated GPU memory usage in bytes (RGBA8 = width * height * 4).
    pub fn gpu_memory_bytes(&self) -> u64 {
        self._width as u64 * self._height as u64 * 4
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
    ) -> EngineResult<Self> {
        Self::from_rgba8_with_spec_batched(
            res,
            allocator,
            data.width,
            data.height,
            &data.pixels,
            &data.spec,
            batch,
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
    ) -> EngineResult<Self> {
        let _timer = ProfileTimer::new("Texture2D::from_rgba8_with_spec_batched");
        let image_size = (width * height * 4) as vk::DeviceSize;
        assert_eq!(pixels.len() as vk::DeviceSize, image_size);

        let device = res.device;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;

        let vk_format = spec.format.to_vk();
        let mip_levels = if spec.generate_mipmaps {
            calculate_mip_levels(width, height)
        } else {
            1
        };

        // 1. Create staging buffer with pixel data.
        let (staging_buffer, staging_alloc) = create_staging_buffer(allocator, device, pixels)?;

        // 2. Create Vulkan image + GPU memory.
        let (image, allocation) =
            create_texture_image(device, allocator, width, height, mip_levels, vk_format)?;

        // 3. Record the staging copy + layout transitions into the batch.
        batch.record_image_upload(
            image,
            staging_buffer,
            staging_alloc,
            width,
            height,
            mip_levels,
        )?;

        // 4. Create image view, sampler, and descriptor set.
        let (image_view, sampler, descriptor_set) = create_texture_view_sampler_ds(
            device,
            image,
            vk_format,
            mip_levels,
            spec,
            descriptor_pool,
            descriptor_set_layout,
        )?;

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

    /// Create a texture from pre-compressed data (BC/ASTC formats).
    ///
    /// The `data` must contain correctly formatted compressed blocks for the
    /// specified `format`. No GPU-side mipmap generation is performed (compressed
    /// formats are not blittable); provide pre-computed mipmaps in the data or
    /// use single-level.
    ///
    /// Returns an error if the format is not compressed or the data size is wrong.
    #[allow(dead_code)] // API reserved for future compressed texture loading
    pub(crate) fn from_compressed(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        width: u32,
        height: u32,
        data: &[u8],
        format: ImageFormat,
        spec: &TextureSpecification,
    ) -> EngineResult<Self> {
        if !format.is_compressed() {
            return Err(EngineError::Gpu(
                "from_compressed requires a compressed ImageFormat".to_string(),
            ));
        }

        let expected_size = format.data_size(width, height);
        if data.len() as u64 != expected_size {
            return Err(EngineError::Gpu(format!(
                "Compressed data size mismatch: expected {} bytes, got {}",
                expected_size,
                data.len()
            )));
        }

        let device = res.device;
        let vk_format = format.to_vk();

        // Staging buffer.
        let (staging_buffer, _staging_alloc) =
            super::buffer::create_staging_buffer(allocator, device, data)?;

        // Image (no TRANSFER_SRC — cannot blit compressed formats).
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

        let image = unsafe { device.create_image(&image_info, None) }.map_err(|e| {
            EngineError::Gpu(format!("Failed to create compressed texture image: {e}"))
        })?;

        let allocation = GpuAllocator::allocate_for_image(
            allocator,
            device,
            image,
            "CompressedTexture2D",
            MemoryLocation::GpuOnly,
        )?;

        // Upload.
        execute_one_shot(device, res.command_pool, res.graphics_queue, |cmd_buf| {
            transition_image_layout(
                device,
                cmd_buf,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                0,
                1,
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
                0,
                1,
            );
        })?;

        unsafe {
            device.destroy_buffer(staging_buffer, None);
        }
        drop(_staging_alloc);

        // Compressed textures don't support mipmaps via GPU blit.
        let actual_spec = TextureSpecification {
            format,
            generate_mipmaps: false,
            ..spec.clone()
        };

        let (image_view, sampler, descriptor_set) = create_texture_view_sampler_ds(
            device,
            image,
            vk_format,
            1,
            &actual_spec,
            res.descriptor_pool,
            res.texture_ds_layout,
        )?;

        Ok(Self {
            image,
            _allocation: allocation,
            image_view,
            sampler,
            descriptor_set,
            descriptor_pool: res.descriptor_pool,
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
// Public-within-renderer wrappers
// ---------------------------------------------------------------------------

/// Execute a one-shot command buffer (visible to sibling modules).
pub(super) fn execute_one_shot_pub(
    device: &ash::Device,
    command_pool: vk::CommandPool,
    queue: vk::Queue,
    record: impl FnOnce(vk::CommandBuffer),
) -> EngineResult<()> {
    execute_one_shot(device, command_pool, queue, record)
}

/// Calculate mip levels (visible to sibling modules).
pub(super) fn calculate_mip_levels_pub(width: u32, height: u32) -> u32 {
    calculate_mip_levels(width, height)
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
) -> EngineResult<()> {
    let alloc_info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);

    let cmd_buf = unsafe { device.allocate_command_buffers(&alloc_info) }.map_err(|e| {
        EngineError::Gpu(format!("Failed to allocate one-shot command buffer: {e}"))
    })?[0];

    let begin_info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

    unsafe {
        device
            .begin_command_buffer(cmd_buf, &begin_info)
            .map_err(|e| {
                EngineError::Gpu(format!("Failed to begin one-shot command buffer: {e}"))
            })?;
    }

    record(cmd_buf);

    unsafe {
        device
            .end_command_buffer(cmd_buf)
            .map_err(|e| EngineError::Gpu(format!("Failed to end one-shot command buffer: {e}")))?;

        let cmd_bufs = [cmd_buf];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
        device
            .queue_submit(queue, &[submit_info], vk::Fence::null())
            .map_err(|e| {
                EngineError::Gpu(format!("Failed to submit one-shot command buffer: {e}"))
            })?;
        device
            .queue_wait_idle(queue)
            .map_err(|e| EngineError::Gpu(format!("Failed to wait for queue idle: {e}")))?;

        device.free_command_buffers(command_pool, &[cmd_buf]);
    }
    Ok(())
}

/// Create a Vulkan image with GPU-only memory for a 2D texture.
fn create_texture_image(
    device: &ash::Device,
    allocator: &Arc<Mutex<GpuAllocator>>,
    width: u32,
    height: u32,
    mip_levels: u32,
    vk_format: vk::Format,
) -> EngineResult<(vk::Image, GpuAllocation)> {
    let mut usage = vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED;
    if mip_levels > 1 {
        usage |= vk::ImageUsageFlags::TRANSFER_SRC;
    }
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(mip_levels)
        .array_layers(1)
        .format(vk_format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create texture image: {e}")))?;

    let allocation = GpuAllocator::allocate_for_image(
        allocator,
        device,
        image,
        "Texture2D",
        MemoryLocation::GpuOnly,
    )?;

    Ok((image, allocation))
}

/// Create an image view, sampler, and descriptor set for a loaded texture image.
fn create_texture_view_sampler_ds(
    device: &ash::Device,
    image: vk::Image,
    vk_format: vk::Format,
    mip_levels: u32,
    spec: &TextureSpecification,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set_layout: vk::DescriptorSetLayout,
) -> EngineResult<(vk::ImageView, vk::Sampler, vk::DescriptorSet)> {
    // Image view.
    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk_format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: mip_levels,
            base_array_layer: 0,
            layer_count: 1,
        });

    let image_view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create texture image view: {e}")))?;

    // Sampler — use LINEAR mipmap interpolation when mipmaps are present
    // to avoid visible popping between mip levels.
    let mipmap_mode = if mip_levels > 1 {
        vk::SamplerMipmapMode::LINEAR
    } else {
        match spec.filter {
            vk::Filter::LINEAR => vk::SamplerMipmapMode::LINEAR,
            _ => vk::SamplerMipmapMode::NEAREST,
        }
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
        .max_lod(if mip_levels > 1 {
            mip_levels as f32
        } else {
            0.0
        });

    let sampler = unsafe { device.create_sampler(&sampler_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create texture sampler: {e}")))?;

    // Descriptor set.
    let layouts = [descriptor_set_layout];
    let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(descriptor_pool)
        .set_layouts(&layouts);

    let ds_vec = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
        .map_err(|e| EngineError::Gpu(format!("Failed to allocate texture descriptor set: {e}")))?;
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

    Ok((image_view, sampler, descriptor_set))
}

/// Calculate the number of mip levels for an image of the given dimensions.
fn calculate_mip_levels(width: u32, height: u32) -> u32 {
    (width.max(height) as f32).log2().floor() as u32 + 1
}

/// Insert a pipeline barrier to transition an image between layouts.
///
/// `base_mip_level` and `level_count` specify which mip levels to transition.
fn transition_image_layout(
    device: &ash::Device,
    cmd_buf: vk::CommandBuffer,
    image: vk::Image,
    old_layout: vk::ImageLayout,
    new_layout: vk::ImageLayout,
    base_mip_level: u32,
    level_count: u32,
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
            base_mip_level,
            level_count,
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

/// Generate mipmaps for an image using `vkCmdBlitImage`.
///
/// Assumes mip level 0 is in `TRANSFER_DST_OPTIMAL` layout with valid data.
/// Leaves **all** mip levels in `SHADER_READ_ONLY_OPTIMAL` layout.
fn generate_mipmaps_cmd(
    device: &ash::Device,
    cmd_buf: vk::CommandBuffer,
    image: vk::Image,
    width: u32,
    height: u32,
    mip_levels: u32,
) {
    let mut mip_width = width as i32;
    let mut mip_height = height as i32;

    for i in 1..mip_levels {
        // Transition mip (i-1): TRANSFER_DST → TRANSFER_SRC (so we can blit from it).
        let barrier_to_src = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: i - 1,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
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

        // Blit from mip (i-1) to mip i.
        let next_width = (mip_width / 2).max(1);
        let next_height = (mip_height / 2).max(1);

        let blit = vk::ImageBlit {
            src_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: i - 1,
                base_array_layer: 0,
                layer_count: 1,
            },
            src_offsets: [
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: mip_width,
                    y: mip_height,
                    z: 1,
                },
            ],
            dst_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: i,
                base_array_layer: 0,
                layer_count: 1,
            },
            dst_offsets: [
                vk::Offset3D { x: 0, y: 0, z: 0 },
                vk::Offset3D {
                    x: next_width,
                    y: next_height,
                    z: 1,
                },
            ],
        };

        unsafe {
            device.cmd_blit_image(
                cmd_buf,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[blit],
                vk::Filter::LINEAR,
            );
        }

        // Transition mip (i-1): TRANSFER_SRC → SHADER_READ_ONLY (done with it).
        let barrier_to_read = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: i - 1,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier_to_read],
            );
        }

        mip_width = next_width;
        mip_height = next_height;
    }

    // Transition last mip level: TRANSFER_DST → SHADER_READ_ONLY.
    let barrier_last = vk::ImageMemoryBarrier::default()
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: mip_levels - 1,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ);

    unsafe {
        device.cmd_pipeline_barrier(
            cmd_buf,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_last],
        );
    }
}
