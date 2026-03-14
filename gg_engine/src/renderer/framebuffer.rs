use std::sync::{Arc, Mutex};

use ash::vk;
use serde::{Deserialize, Serialize};

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::RendererResources;
use crate::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// MsaaSamples — public MSAA configuration enum
// ---------------------------------------------------------------------------

/// MSAA sample count for offscreen framebuffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MsaaSamples {
    /// No multisampling (default).
    #[default]
    S1,
    /// 2x MSAA.
    S2,
    /// 4x MSAA.
    S4,
    /// 8x MSAA.
    S8,
}

impl MsaaSamples {
    pub const ALL: &[MsaaSamples] = &[
        MsaaSamples::S1,
        MsaaSamples::S2,
        MsaaSamples::S4,
        MsaaSamples::S8,
    ];

    pub fn to_vk(self) -> vk::SampleCountFlags {
        match self {
            MsaaSamples::S1 => vk::SampleCountFlags::TYPE_1,
            MsaaSamples::S2 => vk::SampleCountFlags::TYPE_2,
            MsaaSamples::S4 => vk::SampleCountFlags::TYPE_4,
            MsaaSamples::S8 => vk::SampleCountFlags::TYPE_8,
        }
    }

    pub fn from_vk(flags: vk::SampleCountFlags) -> Self {
        if flags.contains(vk::SampleCountFlags::TYPE_8) {
            MsaaSamples::S8
        } else if flags.contains(vk::SampleCountFlags::TYPE_4) {
            MsaaSamples::S4
        } else if flags.contains(vk::SampleCountFlags::TYPE_2) {
            MsaaSamples::S2
        } else {
            MsaaSamples::S1
        }
    }

    /// Return all sample counts up to and including the device maximum.
    /// `max` can be either a single flag (e.g. TYPE_8 from `max_msaa_samples()`)
    /// or a full bitmask — both are handled correctly.
    pub fn available_up_to(max: vk::SampleCountFlags) -> Vec<MsaaSamples> {
        let max_raw = max.as_raw();
        let mut result = vec![MsaaSamples::S1];
        if max_raw >= vk::SampleCountFlags::TYPE_2.as_raw() {
            result.push(MsaaSamples::S2);
        }
        if max_raw >= vk::SampleCountFlags::TYPE_4.as_raw() {
            result.push(MsaaSamples::S4);
        }
        if max_raw >= vk::SampleCountFlags::TYPE_8.as_raw() {
            result.push(MsaaSamples::S8);
        }
        result
    }

    /// Clamp to the highest supported level given the device maximum.
    pub fn clamp_to_device(self, max: vk::SampleCountFlags) -> Self {
        if max.contains(self.to_vk()) {
            self
        } else {
            // Fall back to the highest supported.
            let available = Self::available_up_to(max);
            *available.last().unwrap_or(&MsaaSamples::S1)
        }
    }
}

impl std::fmt::Display for MsaaSamples {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsaaSamples::S1 => write!(f, "Off"),
            MsaaSamples::S2 => write!(f, "2x"),
            MsaaSamples::S4 => write!(f, "4x"),
            MsaaSamples::S8 => write!(f, "8x"),
        }
    }
}

// ---------------------------------------------------------------------------
// Attachment format types
// ---------------------------------------------------------------------------

/// Logical attachment format for framebuffer specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramebufferTextureFormat {
    /// Standard RGBA color — maps to swapchain color format (B8G8R8A8_SRGB).
    RGBA8,
    /// 16-bit float RGBA — for HDR scene rendering (R16G16B16A16_SFLOAT).
    RGBA16F,
    /// Signed 32-bit integer — for entity ID / picking buffer.
    RedInteger,
    /// 16-bit float RGBA for world-space normals (R16G16B16A16_SFLOAT).
    NormalMap,
    /// Depth-only — maps to engine depth format (D32_SFLOAT).
    Depth,
}

/// Per-attachment specification.
#[derive(Debug, Clone, Copy)]
pub struct FramebufferTextureSpec {
    pub format: FramebufferTextureFormat,
}

impl From<FramebufferTextureFormat> for FramebufferTextureSpec {
    fn from(format: FramebufferTextureFormat) -> Self {
        Self { format }
    }
}

fn is_depth_format(format: FramebufferTextureFormat) -> bool {
    matches!(format, FramebufferTextureFormat::Depth)
}

/// Map a logical format to a Vulkan format.
fn resolve_vk_format(
    format: FramebufferTextureFormat,
    color_format: vk::Format,
    depth_format: vk::Format,
) -> vk::Format {
    match format {
        FramebufferTextureFormat::RGBA8 => color_format,
        FramebufferTextureFormat::RGBA16F => vk::Format::R16G16B16A16_SFLOAT,
        FramebufferTextureFormat::RedInteger => vk::Format::R32_SINT,
        FramebufferTextureFormat::NormalMap => vk::Format::R16G16B16A16_SFLOAT,
        FramebufferTextureFormat::Depth => depth_format,
    }
}

// ---------------------------------------------------------------------------
// FramebufferSpec
// ---------------------------------------------------------------------------

/// Maximum allowed framebuffer dimension. Should eventually come from GPU
/// capabilities, but this is a safe upper bound for now (~8K).
const MAX_FRAMEBUFFER_SIZE: u32 = 8192;

/// Maximum number of clear values (color attachments + depth).
/// Avoids heap allocation in [`Framebuffer::clear_values`].
const MAX_CLEAR_VALUES: usize = 8;

/// Stack-allocated clear value array returned by [`Framebuffer::clear_values`].
/// Dereferences to `&[vk::ClearValue]` for seamless use with Vulkan APIs.
pub(crate) struct ClearValues {
    values: [vk::ClearValue; MAX_CLEAR_VALUES],
    len: usize,
}

impl ClearValues {
    fn new() -> Self {
        Self {
            values: [vk::ClearValue::default(); MAX_CLEAR_VALUES],
            len: 0,
        }
    }

    fn push(&mut self, value: vk::ClearValue) {
        debug_assert!(self.len < MAX_CLEAR_VALUES, "ClearValues overflow");
        if self.len < MAX_CLEAR_VALUES {
            self.values[self.len] = value;
            self.len += 1;
        }
    }
}

impl Default for ClearValues {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for ClearValues {
    type Target = [vk::ClearValue];
    fn deref(&self) -> &[vk::ClearValue] {
        &self.values[..self.len]
    }
}

/// Configuration for creating an offscreen framebuffer.
pub struct FramebufferSpec {
    pub width: u32,
    pub height: u32,
    pub attachments: Vec<FramebufferTextureSpec>,
    /// MSAA sample count. Default is TYPE_1 (no MSAA).
    pub samples: vk::SampleCountFlags,
}

// ---------------------------------------------------------------------------
// Internal attachment structs
// ---------------------------------------------------------------------------

struct ColorAttachment {
    image: vk::Image,
    _allocation: GpuAllocation,
    view: vk::ImageView,
    _format: FramebufferTextureFormat,
}

struct DepthAttachment {
    image: vk::Image,
    _allocation: GpuAllocation,
    view: vk::ImageView,
}

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

/// Offscreen framebuffer with configurable color and depth attachments,
/// suitable for rendering a scene to a texture that can then be displayed
/// in egui. Supports optional MSAA with automatic resolve.
pub struct Framebuffer {
    // Color attachments (1x resolve targets when MSAA; render targets when no MSAA).
    color_attachments: Vec<ColorAttachment>,
    color_attachment_specs: Vec<FramebufferTextureSpec>,

    // Optional depth attachment (1x, only used when no MSAA).
    depth_attachment: Option<DepthAttachment>,
    depth_attachment_spec: Option<FramebufferTextureSpec>,

    // MSAA attachments (empty/None when sample_count == TYPE_1).
    msaa_color_attachments: Vec<ColorAttachment>,
    msaa_depth_attachment: Option<DepthAttachment>,

    // MSAA sample count (TYPE_1 = no MSAA).
    sample_count: vk::SampleCountFlags,

    // Sampler + descriptor for egui display (always points to color_attachments[0]).
    sampler: vk::Sampler,
    descriptor_set: vk::DescriptorSet,

    // Render pass and framebuffer.
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,

    // Egui texture handle (set after registration).
    egui_texture_id: Option<egui::TextureId>,

    // Spec / format info.
    spec: FramebufferSpec,
    color_format: vk::Format,
    depth_format: vk::Format,

    // Pixel readback (per-frame-in-flight staging buffer, persistently mapped).
    readback_buffer: vk::Buffer,
    readback_allocation: GpuAllocation,

    // Pending readback request for current frame.
    pending_readback: Option<(usize, i32, i32)>, // (attachment_index, x, y)

    // Last successfully read pixel value.
    last_readback: i32,

    // GPU allocator for resize.
    allocator: Arc<Mutex<GpuAllocator>>,
    device: ash::Device,
}

impl Framebuffer {
    pub(crate) fn new(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        spec: FramebufferSpec,
    ) -> EngineResult<Self> {
        let device = res.device;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;
        let color_format = res.color_format;
        let depth_format = res.depth_format;
        let sample_count = spec.samples;
        let msaa = sample_count != vk::SampleCountFlags::TYPE_1;

        debug_assert!(
            spec.width > 0
                && spec.height > 0
                && spec.width <= MAX_FRAMEBUFFER_SIZE
                && spec.height <= MAX_FRAMEBUFFER_SIZE,
            "Invalid framebuffer size: {}x{} (max {})",
            spec.width,
            spec.height,
            MAX_FRAMEBUFFER_SIZE,
        );

        // Split specs into color vs depth buckets.
        let mut color_specs = Vec::new();
        let mut depth_spec: Option<FramebufferTextureSpec> = None;
        for &att in &spec.attachments {
            if is_depth_format(att.format) {
                debug_assert!(
                    depth_spec.is_none(),
                    "Framebuffer spec contains more than one depth attachment"
                );
                depth_spec = Some(att);
            } else {
                color_specs.push(att);
            }
        }

        let render_pass = create_offscreen_render_pass(
            device,
            &color_specs,
            depth_spec.as_ref(),
            color_format,
            depth_format,
            sample_count,
        )?;

        let sampler = create_sampler(device)?;

        // 1x color images (resolve targets when MSAA, render targets otherwise).
        let color_attachments: Vec<ColorAttachment> = color_specs
            .iter()
            .map(|cs| {
                let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
                create_color_attachment(
                    allocator,
                    device,
                    &spec,
                    vk_fmt,
                    cs.format,
                    vk::SampleCountFlags::TYPE_1,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        // MSAA color + depth images (only when MSAA enabled).
        let (msaa_color_attachments, msaa_depth_attachment) = if msaa {
            let msaa_colors: Vec<ColorAttachment> = color_specs
                .iter()
                .map(|cs| {
                    let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
                    create_color_attachment(
                        allocator,
                        device,
                        &spec,
                        vk_fmt,
                        cs.format,
                        sample_count,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;

            let msaa_depth = depth_spec
                .map(|ds| {
                    let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
                    create_depth_attachment(allocator, device, &spec, vk_fmt, sample_count)
                })
                .transpose()?;

            (msaa_colors, msaa_depth)
        } else {
            (Vec::new(), None)
        };

        // 1x depth (only when no MSAA — with MSAA, depth is in msaa_depth_attachment).
        let depth_attachment = if !msaa {
            depth_spec
                .map(|ds| {
                    let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
                    create_depth_attachment(
                        allocator,
                        device,
                        &spec,
                        vk_fmt,
                        vk::SampleCountFlags::TYPE_1,
                    )
                })
                .transpose()?
        } else {
            None
        };

        let framebuffer = create_vk_framebuffer_msaa(
            device,
            render_pass,
            &color_attachments,
            depth_attachment.as_ref(),
            &msaa_color_attachments,
            msaa_depth_attachment.as_ref(),
            &spec,
        )?;

        let descriptor_set =
            allocate_descriptor_set(device, descriptor_pool, descriptor_set_layout)?;
        write_descriptor_set(device, descriptor_set, color_attachments[0].view, sampler);

        let (readback_buffer, readback_allocation) =
            create_readback_staging_buffer(allocator, device)?;

        if msaa {
            log::info!(
                target: "gg_engine",
                "Framebuffer created with {}x MSAA ({}x{})",
                sample_count.as_raw().trailing_zeros().max(1),
                spec.width,
                spec.height,
            );
        }

        Ok(Self {
            color_attachments,
            color_attachment_specs: color_specs,
            depth_attachment,
            depth_attachment_spec: depth_spec,
            msaa_color_attachments,
            msaa_depth_attachment,
            sample_count,
            sampler,
            descriptor_set,
            render_pass,
            framebuffer,
            egui_texture_id: None,
            spec,
            color_format,
            depth_format,
            readback_buffer,
            readback_allocation,
            pending_readback: None,
            last_readback: -1,
            allocator: allocator.clone(),
            device: device.clone(),
        })
    }

    /// Resize the framebuffer. Skips if the size hasn't changed.
    /// The descriptor set handle is reused (updated in-place), so the
    /// egui TextureId remains valid.
    pub fn resize(&mut self, width: u32, height: u32) -> EngineResult<()> {
        if self.spec.width == width && self.spec.height == height {
            return Ok(());
        }

        if width == 0
            || height == 0
            || width > MAX_FRAMEBUFFER_SIZE
            || height > MAX_FRAMEBUFFER_SIZE
        {
            log::warn!(target: "gg_engine",
                "Attempted to resize framebuffer to {}x{} (max {}) — ignoring",
                width, height, MAX_FRAMEBUFFER_SIZE,
            );
            return Ok(());
        }

        self.spec = FramebufferSpec {
            width,
            height,
            attachments: Vec::new(), // attachments list not needed after initial parse
            samples: self.sample_count,
        };

        // NOTE: Caller must ensure GPU is idle before calling resize()
        // (e.g. via device_wait_idle in application.rs).

        let msaa = self.sample_count != vk::SampleCountFlags::TYPE_1;

        // Destroy old framebuffer (keep render pass, sampler, descriptor set).
        unsafe {
            self.device.destroy_framebuffer(self.framebuffer, None);
        }

        // Destroy old color attachment Vulkan objects (allocations auto-free on drop).
        for ca in &self.color_attachments {
            unsafe {
                self.device.destroy_image_view(ca.view, None);
                self.device.destroy_image(ca.image, None);
            }
        }
        self.color_attachments.clear();

        // Destroy old MSAA color attachments.
        for ca in &self.msaa_color_attachments {
            unsafe {
                self.device.destroy_image_view(ca.view, None);
                self.device.destroy_image(ca.image, None);
            }
        }
        self.msaa_color_attachments.clear();

        // Destroy old depth attachment.
        if let Some(da) = self.depth_attachment.take() {
            unsafe {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
            }
        }

        // Destroy old MSAA depth attachment.
        if let Some(da) = self.msaa_depth_attachment.take() {
            unsafe {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
            }
        }

        // Destroy old readback buffer.
        unsafe {
            self.device.destroy_buffer(self.readback_buffer, None);
        }

        // Recreate 1x color attachments (resolve targets when MSAA).
        self.color_attachments = self
            .color_attachment_specs
            .iter()
            .map(|cs| {
                let vk_fmt = resolve_vk_format(cs.format, self.color_format, self.depth_format);
                create_color_attachment(
                    &self.allocator,
                    &self.device,
                    &self.spec,
                    vk_fmt,
                    cs.format,
                    vk::SampleCountFlags::TYPE_1,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Recreate MSAA attachments if enabled.
        if msaa {
            self.msaa_color_attachments = self
                .color_attachment_specs
                .iter()
                .map(|cs| {
                    let vk_fmt = resolve_vk_format(cs.format, self.color_format, self.depth_format);
                    create_color_attachment(
                        &self.allocator,
                        &self.device,
                        &self.spec,
                        vk_fmt,
                        cs.format,
                        self.sample_count,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;

            self.msaa_depth_attachment = self
                .depth_attachment_spec
                .map(|ds| {
                    let vk_fmt = resolve_vk_format(ds.format, self.color_format, self.depth_format);
                    create_depth_attachment(
                        &self.allocator,
                        &self.device,
                        &self.spec,
                        vk_fmt,
                        self.sample_count,
                    )
                })
                .transpose()?;
        }

        // Recreate 1x depth (only when no MSAA).
        if !msaa {
            self.depth_attachment = self
                .depth_attachment_spec
                .map(|ds| {
                    let vk_fmt = resolve_vk_format(ds.format, self.color_format, self.depth_format);
                    create_depth_attachment(
                        &self.allocator,
                        &self.device,
                        &self.spec,
                        vk_fmt,
                        vk::SampleCountFlags::TYPE_1,
                    )
                })
                .transpose()?;
        }

        self.framebuffer = create_vk_framebuffer_msaa(
            &self.device,
            self.render_pass,
            &self.color_attachments,
            self.depth_attachment.as_ref(),
            &self.msaa_color_attachments,
            self.msaa_depth_attachment.as_ref(),
            &self.spec,
        )?;

        // Recreate readback staging buffer.
        let (rb_buf, rb_alloc) = create_readback_staging_buffer(&self.allocator, &self.device)?;
        let old_readback_alloc = std::mem::replace(&mut self.readback_allocation, rb_alloc);
        drop(old_readback_alloc);
        self.readback_buffer = rb_buf;
        self.pending_readback = None;
        self.last_readback = -1;

        // Update the existing descriptor set in-place with the new image view.
        write_descriptor_set(
            &self.device,
            self.descriptor_set,
            self.color_attachments[0].view,
            self.sampler,
        );

        Ok(())
    }

    // -- Accessors ------------------------------------------------------------

    pub fn spec(&self) -> &FramebufferSpec {
        &self.spec
    }

    pub fn width(&self) -> u32 {
        self.spec.width
    }

    pub fn height(&self) -> u32 {
        self.spec.height
    }

    pub fn egui_texture_id(&self) -> Option<egui::TextureId> {
        self.egui_texture_id
    }

    /// Number of color attachments in this framebuffer.
    pub fn color_attachment_count(&self) -> usize {
        self.color_attachments.len()
    }

    /// MSAA sample count (TYPE_1 = no MSAA).
    pub fn sample_count(&self) -> vk::SampleCountFlags {
        self.sample_count
    }

    pub(crate) fn render_pass(&self) -> vk::RenderPass {
        self.render_pass
    }

    pub(crate) fn vk_framebuffer(&self) -> vk::Framebuffer {
        self.framebuffer
    }

    pub(crate) fn descriptor_set(&self) -> vk::DescriptorSet {
        self.descriptor_set
    }

    /// Returns the first color attachment image (used for pipeline barriers).
    pub(crate) fn color_image(&self) -> vk::Image {
        self.color_attachments[0].image
    }

    /// Returns the first color attachment image view (for post-processing input).
    pub fn color_image_view(&self) -> vk::ImageView {
        self.color_attachments[0].view
    }

    /// Returns the 1x depth attachment image view (for post-processing depth sampling).
    /// Returns `None` if no depth attachment exists or if MSAA is enabled.
    pub fn depth_image_view(&self) -> Option<vk::ImageView> {
        self.depth_attachment.as_ref().map(|da| da.view)
    }

    /// Returns the MSAA depth attachment image view (for post-processing depth resolve).
    /// Returns `None` if MSAA is not enabled or no depth attachment exists.
    pub fn msaa_depth_image_view(&self) -> Option<vk::ImageView> {
        self.msaa_depth_attachment.as_ref().map(|da| da.view)
    }

    /// Returns the normal map image view (color attachment at index 2).
    /// Returns `None` if the framebuffer has fewer than 3 color attachments.
    pub fn normal_image_view(&self) -> Option<vk::ImageView> {
        self.color_attachments.get(2).map(|ca| ca.view)
    }

    /// Returns the raw depth `vk::Image` handle (either 1x or MSAA, whichever exists).
    /// Used for pipeline barriers that need to flush depth writes before shader reads.
    pub fn depth_image(&self) -> Option<vk::Image> {
        self.msaa_depth_attachment
            .as_ref()
            .or(self.depth_attachment.as_ref())
            .map(|da| da.image)
    }

    /// Returns the MSAA sample count as a u32 (1 = no MSAA).
    pub fn sample_count_u32(&self) -> u32 {
        match self.sample_count {
            vk::SampleCountFlags::TYPE_1 => 1,
            vk::SampleCountFlags::TYPE_2 => 2,
            vk::SampleCountFlags::TYPE_4 => 4,
            vk::SampleCountFlags::TYPE_8 => 8,
            _ => 1,
        }
    }

    /// Build the correct clear value array for this framebuffer's attachments.
    /// Color attachments use the supplied clear color; RedInteger clears to -1;
    /// depth clears to 1.0/0.
    ///
    /// Order must match render pass attachment order:
    /// - MSAA: [msaa_colors..., msaa_depth, resolve_colors...]
    /// - No MSAA: [colors..., depth]
    ///
    /// Returns a stack-allocated [`ClearValues`] (no heap allocation).
    pub(crate) fn clear_values(&self, clear_color: [f32; 4]) -> ClearValues {
        let mut values = ClearValues::new();
        let msaa = self.sample_count != vk::SampleCountFlags::TYPE_1;

        let push_color_clears = |values: &mut ClearValues, specs: &[FramebufferTextureSpec]| {
            for cs in specs {
                match cs.format {
                    FramebufferTextureFormat::RedInteger => {
                        values.push(vk::ClearValue {
                            color: vk::ClearColorValue {
                                int32: [-1, 0, 0, 0],
                            },
                        });
                    }
                    FramebufferTextureFormat::NormalMap => {
                        values.push(vk::ClearValue {
                            color: vk::ClearColorValue {
                                float32: [0.0, 0.0, 0.0, 0.0],
                            },
                        });
                    }
                    _ => {
                        values.push(vk::ClearValue {
                            color: vk::ClearColorValue {
                                float32: clear_color,
                            },
                        });
                    }
                }
            }
        };

        if msaa {
            // MSAA color attachments (rendered to, then auto-resolved).
            push_color_clears(&mut values, &self.color_attachment_specs);
            // MSAA depth (reverse-Z: clear to 0.0 = far plane).
            if self.msaa_depth_attachment.is_some() {
                values.push(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 0.0,
                        stencil: 0,
                    },
                });
            }
            // Resolve color attachments (load=DONT_CARE, but clear values still needed).
            push_color_clears(&mut values, &self.color_attachment_specs);
        } else {
            // Non-MSAA: colors then depth.
            push_color_clears(&mut values, &self.color_attachment_specs);
            // Reverse-Z: clear to 0.0 = far plane.
            if self.depth_attachment.is_some() {
                values.push(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 0.0,
                        stencil: 0,
                    },
                });
            }
        }

        values
    }

    /// Request a pixel readback from the given color attachment at (x, y).
    /// Coordinates are in framebuffer pixel space. Out-of-bounds coordinates
    /// are silently ignored to avoid Vulkan validation errors.
    pub fn schedule_pixel_readback(&mut self, attachment_index: usize, x: i32, y: i32) {
        if x >= 0 && y >= 0 && (x as u32) < self.width() && (y as u32) < self.height() {
            self.pending_readback = Some((attachment_index, x, y));
        }
    }

    /// Read the staging buffer for the given frame slot (data from 2 frames ago).
    /// Called after waiting on the frame's fence.
    pub(crate) fn read_pixel_result(&mut self, current_frame: usize) {
        let ptr = self
            .readback_allocation
            .mapped_ptr()
            .expect("Readback buffer must be mapped") as *const i32;
        unsafe {
            self.last_readback = *ptr.add(current_frame);
        }
    }

    /// Get the last readback value (-1 = no entity / background).
    pub fn hovered_entity(&self) -> i32 {
        self.last_readback
    }

    /// Get the image handle for a specific color attachment.
    pub(crate) fn color_attachment_image(&self, index: usize) -> vk::Image {
        self.color_attachments[index].image
    }

    /// Take the pending readback request (resets it to None).
    pub(crate) fn take_pending_readback(&mut self) -> Option<(usize, i32, i32)> {
        self.pending_readback.take()
    }

    /// Get the readback staging buffer handle.
    pub(crate) fn readback_buffer(&self) -> vk::Buffer {
        self.readback_buffer
    }

    pub(crate) fn set_egui_texture_id(&mut self, id: egui::TextureId) {
        self.egui_texture_id = Some(id);
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_framebuffer(self.framebuffer, None);
            self.device.destroy_sampler(self.sampler, None);

            for ca in &self.color_attachments {
                self.device.destroy_image_view(ca.view, None);
                self.device.destroy_image(ca.image, None);
            }

            for ca in &self.msaa_color_attachments {
                self.device.destroy_image_view(ca.view, None);
                self.device.destroy_image(ca.image, None);
            }

            if let Some(da) = &self.depth_attachment {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
            }

            if let Some(da) = &self.msaa_depth_attachment {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
            }

            self.device.destroy_buffer(self.readback_buffer, None);
            self.device.destroy_render_pass(self.render_pass, None);
            // Descriptor set is freed when the pool is destroyed.
        }
        // GpuAllocations in color_attachments, depth_attachment,
        // msaa_color_attachments, msaa_depth_attachment, readback_allocation
        // auto-free on drop.
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_offscreen_render_pass(
    device: &ash::Device,
    color_specs: &[FramebufferTextureSpec],
    depth_spec: Option<&FramebufferTextureSpec>,
    color_format: vk::Format,
    depth_format: vk::Format,
    sample_count: vk::SampleCountFlags,
) -> EngineResult<vk::RenderPass> {
    let msaa = sample_count != vk::SampleCountFlags::TYPE_1;

    let mut attachment_descriptions = Vec::new();
    let mut color_attachment_refs = Vec::new();
    let mut resolve_attachment_refs = Vec::new();

    if msaa {
        // --- MSAA path ---
        // Attachment layout:
        //   [0..N-1]    = MSAA color (rendered to, store=DONT_CARE after resolve)
        //   [N]         = MSAA depth (if present)
        //   [N+1..2N]   = 1x resolve color (resolve targets, final=SHADER_READ_ONLY)

        // MSAA color attachments.
        for (i, cs) in color_specs.iter().enumerate() {
            let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
            attachment_descriptions.push(
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(sample_count)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
            );
            color_attachment_refs.push(vk::AttachmentReference {
                attachment: i as u32,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            });
        }

        // MSAA depth attachment (if present).
        // STORE depth so post-processing (contact shadows) can resolve and sample it.
        let depth_attachment_ref = depth_spec.map(|ds| {
            let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
            attachment_descriptions.push(
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(sample_count)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL),
            );
            vk::AttachmentReference {
                attachment: color_specs.len() as u32,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            }
        });

        // 1x resolve color attachments.
        let resolve_base = attachment_descriptions.len() as u32;
        for (i, cs) in color_specs.iter().enumerate() {
            let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
            attachment_descriptions.push(
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            );
            resolve_attachment_refs.push(vk::AttachmentReference {
                attachment: resolve_base + i as u32,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            });
        }

        let mut subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs)
            .resolve_attachments(&resolve_attachment_refs);

        if let Some(ref depth_ref) = depth_attachment_ref {
            subpass = subpass.depth_stencil_attachment(depth_ref);
        }

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );

        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachment_descriptions)
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        unsafe { device.create_render_pass(&render_pass_info, None) }.map_err(|e| {
            EngineError::Gpu(format!("Failed to create MSAA offscreen render pass: {e}"))
        })
    } else {
        // --- Non-MSAA path (unchanged) ---
        for (i, cs) in color_specs.iter().enumerate() {
            let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
            attachment_descriptions.push(
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL),
            );
            color_attachment_refs.push(vk::AttachmentReference {
                attachment: i as u32,
                layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            });
        }

        let depth_attachment_ref = depth_spec.map(|ds| {
            let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
            attachment_descriptions.push(
                vk::AttachmentDescription::default()
                    .format(vk_fmt)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    // STORE depth so post-processing (contact shadows) can sample it.
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                    .initial_layout(vk::ImageLayout::UNDEFINED)
                    // Transition to read-only for post-processing depth sampling.
                    .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL),
            );
            vk::AttachmentReference {
                attachment: color_specs.len() as u32,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            }
        });

        let mut subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_refs);

        if let Some(ref depth_ref) = depth_attachment_ref {
            subpass = subpass.depth_stencil_attachment(depth_ref);
        }

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            );

        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachment_descriptions)
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        unsafe { device.create_render_pass(&render_pass_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create offscreen render pass: {e}")))
    }
}

fn create_color_attachment(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
    spec: &FramebufferSpec,
    vk_format: vk::Format,
    fb_format: FramebufferTextureFormat,
    samples: vk::SampleCountFlags,
) -> EngineResult<ColorAttachment> {
    let is_msaa = samples != vk::SampleCountFlags::TYPE_1;

    // MSAA images only need COLOR_ATTACHMENT (transient, never sampled/read back).
    // 1x images need SAMPLED (egui) + TRANSFER_SRC (pixel readback).
    let usage = if is_msaa {
        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSIENT_ATTACHMENT
    } else {
        vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC
    };

    let label = if is_msaa { "FB_MSAA_Color" } else { "FB_Color" };

    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width: spec.width,
            height: spec.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(vk_format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(samples);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create FB color image: {e}")))?;

    let allocation =
        GpuAllocator::allocate_for_image(allocator, device, image, label, MemoryLocation::GpuOnly)?;

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

    let view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create FB color image view: {e}")))?;

    Ok(ColorAttachment {
        image,
        _allocation: allocation,
        view,
        _format: fb_format,
    })
}

fn create_depth_attachment(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
    spec: &FramebufferSpec,
    vk_format: vk::Format,
    samples: vk::SampleCountFlags,
) -> EngineResult<DepthAttachment> {
    let is_msaa = samples != vk::SampleCountFlags::TYPE_1;
    let label = if is_msaa { "FB_MSAA_Depth" } else { "FB_Depth" };

    // SAMPLED allows post-processing passes (e.g. contact shadows) to read the depth buffer.
    let usage = vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED;

    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width: spec.width,
            height: spec.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(vk_format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(samples);

    let image = unsafe { device.create_image(&image_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create FB depth image: {e}")))?;

    let allocation =
        GpuAllocator::allocate_for_image(allocator, device, image, label, MemoryLocation::GpuOnly)?;

    let view_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk_format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    let view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create FB depth image view: {e}")))?;

    Ok(DepthAttachment {
        image,
        _allocation: allocation,
        view,
    })
}

fn create_sampler(device: &ash::Device) -> EngineResult<vk::Sampler> {
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .anisotropy_enable(false)
        .border_color(vk::BorderColor::FLOAT_OPAQUE_BLACK)
        .unnormalized_coordinates(false)
        .compare_enable(false)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .mip_lod_bias(0.0)
        .min_lod(0.0)
        .max_lod(0.0);

    unsafe { device.create_sampler(&sampler_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create FB sampler: {e}")))
}

/// Build the vk::Framebuffer with attachment views in render pass order.
///
/// MSAA order: [msaa_color_views..., msaa_depth_view, resolve_color_views...]
/// Non-MSAA:   [color_views..., depth_view]
fn create_vk_framebuffer_msaa(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    color_attachments: &[ColorAttachment],
    depth_attachment: Option<&DepthAttachment>,
    msaa_color_attachments: &[ColorAttachment],
    msaa_depth_attachment: Option<&DepthAttachment>,
    spec: &FramebufferSpec,
) -> EngineResult<vk::Framebuffer> {
    let mut views: Vec<vk::ImageView> = Vec::new();

    if !msaa_color_attachments.is_empty() {
        // MSAA path: msaa colors, msaa depth, resolve colors.
        for ca in msaa_color_attachments {
            views.push(ca.view);
        }
        if let Some(da) = msaa_depth_attachment {
            views.push(da.view);
        }
        for ca in color_attachments {
            views.push(ca.view);
        }
    } else {
        // Non-MSAA: colors, depth.
        for ca in color_attachments {
            views.push(ca.view);
        }
        if let Some(da) = depth_attachment {
            views.push(da.view);
        }
    }

    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(&views)
        .width(spec.width)
        .height(spec.height)
        .layers(1);

    unsafe { device.create_framebuffer(&fb_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create offscreen framebuffer: {e}")))
}

fn allocate_descriptor_set(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
) -> EngineResult<vk::DescriptorSet> {
    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let ds_vec = unsafe { device.allocate_descriptor_sets(&alloc_info) }
        .map_err(|e| EngineError::Gpu(format!("Failed to allocate FB descriptor set: {e}")))?;
    Ok(ds_vec[0])
}

/// Create a small HOST_VISIBLE staging buffer for pixel readback (2 × i32,
/// one per frame-in-flight slot). Returns (buffer, allocation).
fn create_readback_staging_buffer(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
) -> EngineResult<(vk::Buffer, GpuAllocation)> {
    let size = (2 * std::mem::size_of::<i32>()) as u64;

    let buf_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer = unsafe { device.create_buffer(&buf_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create readback buffer: {e}")))?;

    let allocation = GpuAllocator::allocate_for_buffer(
        allocator,
        device,
        buffer,
        "ReadbackBuffer",
        MemoryLocation::GpuToCpu,
    )?;

    // Initialize both slots to -1 (no entity).
    // SAFETY: GpuToCpu allocation guarantees HOST_VISIBLE mapped memory.
    // Buffer is sized for exactly 2 × i32, so both writes are in-bounds.
    let mapping = allocation
        .mapped_ptr()
        .expect("Readback buffer (GpuToCpu) must be host-mapped") as *mut i32;
    unsafe {
        std::ptr::write(mapping, -1);
        std::ptr::write(mapping.add(1), -1);
    }

    Ok((buffer, allocation))
}

fn write_descriptor_set(
    device: &ash::Device,
    set: vk::DescriptorSet,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
) {
    let image_info = vk::DescriptorImageInfo::default()
        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image_view(image_view)
        .sampler(sampler);

    let write = vk::WriteDescriptorSet::default()
        .dst_set(set)
        .dst_binding(0)
        .dst_array_element(0)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .image_info(std::slice::from_ref(&image_info));

    unsafe {
        device.update_descriptor_sets(&[write], &[]);
    }
}
