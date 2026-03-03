use ash::vk;

use super::buffer::find_memory_type;
use super::RendererResources;

// ---------------------------------------------------------------------------
// Attachment format types
// ---------------------------------------------------------------------------

/// Logical attachment format for framebuffer specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramebufferTextureFormat {
    /// Standard RGBA color — maps to swapchain color format (B8G8R8A8_SRGB).
    RGBA8,
    /// Signed 32-bit integer — for entity ID / picking buffer.
    RedInteger,
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
        FramebufferTextureFormat::RedInteger => vk::Format::R32_SINT,
        FramebufferTextureFormat::Depth => depth_format,
    }
}

// ---------------------------------------------------------------------------
// FramebufferSpec
// ---------------------------------------------------------------------------

/// Maximum allowed framebuffer dimension. Should eventually come from GPU
/// capabilities, but this is a safe upper bound for now (~8K).
const MAX_FRAMEBUFFER_SIZE: u32 = 8192;

/// Configuration for creating an offscreen framebuffer.
pub struct FramebufferSpec {
    pub width: u32,
    pub height: u32,
    pub attachments: Vec<FramebufferTextureSpec>,
}

// ---------------------------------------------------------------------------
// Internal attachment structs
// ---------------------------------------------------------------------------

struct ColorAttachment {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    format: FramebufferTextureFormat,
}

struct DepthAttachment {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

/// Offscreen framebuffer with configurable color and depth attachments,
/// suitable for rendering a scene to a texture that can then be displayed
/// in egui.
pub struct Framebuffer {
    // Color attachments (one per color spec in order).
    color_attachments: Vec<ColorAttachment>,
    color_attachment_specs: Vec<FramebufferTextureSpec>,

    // Optional depth attachment (at most one).
    depth_attachment: Option<DepthAttachment>,
    depth_attachment_spec: Option<FramebufferTextureSpec>,

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
    readback_memory: vk::DeviceMemory,
    readback_mapping: *mut i32, // persistent map, 2 × i32 (one per frame slot)

    // Pending readback request for current frame.
    pending_readback: Option<(usize, i32, i32)>, // (attachment_index, x, y)

    // Last successfully read pixel value.
    last_readback: i32,

    // Vulkan handles needed for resize/cleanup.
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
}

impl Framebuffer {
    pub(crate) fn new(res: &RendererResources<'_>, spec: FramebufferSpec) -> Self {
        let instance = res.instance;
        let physical_device = res.physical_device;
        let device = res.device;
        let descriptor_pool = res.descriptor_pool;
        let descriptor_set_layout = res.texture_ds_layout;
        let color_format = res.color_format;
        let depth_format = res.depth_format;
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
        );

        let sampler = create_sampler(device);

        let color_attachments: Vec<ColorAttachment> = color_specs
            .iter()
            .map(|cs| {
                let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
                let (image, memory, view) =
                    create_color_resources(instance, physical_device, device, &spec, vk_fmt);
                ColorAttachment {
                    image,
                    memory,
                    view,
                    format: cs.format,
                }
            })
            .collect();

        let depth_attachment = depth_spec.map(|ds| {
            let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
            let (image, memory, view) =
                create_depth_resources(instance, physical_device, device, &spec, vk_fmt);
            DepthAttachment {
                image,
                memory,
                view,
            }
        });

        let color_views: Vec<vk::ImageView> = color_attachments.iter().map(|a| a.view).collect();
        let depth_view = depth_attachment.as_ref().map(|a| a.view);
        let framebuffer =
            create_vk_framebuffer(device, render_pass, &color_views, depth_view, &spec);

        let descriptor_set =
            allocate_descriptor_set(device, descriptor_pool, descriptor_set_layout);
        write_descriptor_set(device, descriptor_set, color_attachments[0].view, sampler);

        let (readback_buffer, readback_memory, readback_mapping) =
            create_readback_staging_buffer(instance, physical_device, device);

        Self {
            color_attachments,
            color_attachment_specs: color_specs,
            depth_attachment,
            depth_attachment_spec: depth_spec,
            sampler,
            descriptor_set,
            render_pass,
            framebuffer,
            egui_texture_id: None,
            spec,
            color_format,
            depth_format,
            readback_buffer,
            readback_memory,
            readback_mapping,
            pending_readback: None,
            last_readback: -1,
            instance: instance.clone(),
            physical_device,
            device: device.clone(),
        }
    }

    /// Resize the framebuffer. Skips if the size hasn't changed.
    /// The descriptor set handle is reused (updated in-place), so the
    /// egui TextureId remains valid.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.spec.width == width && self.spec.height == height {
            return;
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
            return;
        }

        self.spec = FramebufferSpec {
            width,
            height,
            attachments: Vec::new(), // attachments list not needed after initial parse
        };

        // Destroy old framebuffer, attachment resources, and readback buffer
        // (keep render pass, sampler, descriptor set).
        unsafe {
            self.device.destroy_framebuffer(self.framebuffer, None);

            for ca in &self.color_attachments {
                self.device.destroy_image_view(ca.view, None);
                self.device.destroy_image(ca.image, None);
                self.device.free_memory(ca.memory, None);
            }

            if let Some(da) = &self.depth_attachment {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
                self.device.free_memory(da.memory, None);
            }

            self.device.unmap_memory(self.readback_memory);
            self.device.destroy_buffer(self.readback_buffer, None);
            self.device.free_memory(self.readback_memory, None);
        }

        // Recreate color attachments at new size.
        self.color_attachments = self
            .color_attachment_specs
            .iter()
            .map(|cs| {
                let vk_fmt = resolve_vk_format(cs.format, self.color_format, self.depth_format);
                let (image, memory, view) = create_color_resources(
                    &self.instance,
                    self.physical_device,
                    &self.device,
                    &self.spec,
                    vk_fmt,
                );
                ColorAttachment {
                    image,
                    memory,
                    view,
                    format: cs.format,
                }
            })
            .collect();

        // Recreate depth attachment at new size if present.
        self.depth_attachment = self.depth_attachment_spec.map(|ds| {
            let vk_fmt = resolve_vk_format(ds.format, self.color_format, self.depth_format);
            let (image, memory, view) = create_depth_resources(
                &self.instance,
                self.physical_device,
                &self.device,
                &self.spec,
                vk_fmt,
            );
            DepthAttachment {
                image,
                memory,
                view,
            }
        });

        let color_views: Vec<vk::ImageView> =
            self.color_attachments.iter().map(|a| a.view).collect();
        let depth_view = self.depth_attachment.as_ref().map(|a| a.view);
        self.framebuffer = create_vk_framebuffer(
            &self.device,
            self.render_pass,
            &color_views,
            depth_view,
            &self.spec,
        );

        // Recreate readback staging buffer.
        let (rb_buf, rb_mem, rb_map) =
            create_readback_staging_buffer(&self.instance, self.physical_device, &self.device);
        self.readback_buffer = rb_buf;
        self.readback_memory = rb_mem;
        self.readback_mapping = rb_map;
        self.pending_readback = None;
        self.last_readback = -1;

        // Update the existing descriptor set in-place with the new image view.
        write_descriptor_set(
            &self.device,
            self.descriptor_set,
            self.color_attachments[0].view,
            self.sampler,
        );
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

    /// Build the correct clear value array for this framebuffer's attachments.
    /// Color attachments use the supplied clear color; RedInteger clears to -1;
    /// depth clears to 1.0/0.
    pub(crate) fn clear_values(&self, clear_color: [f32; 4]) -> Vec<vk::ClearValue> {
        let mut values = Vec::with_capacity(self.color_attachments.len() + 1);

        for ca in &self.color_attachments {
            match ca.format {
                FramebufferTextureFormat::RedInteger => {
                    values.push(vk::ClearValue {
                        color: vk::ClearColorValue {
                            int32: [-1, 0, 0, 0],
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

        if self.depth_attachment.is_some() {
            values.push(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
                    stencil: 0,
                },
            });
        }

        values
    }

    /// Request a pixel readback from the given color attachment at (x, y).
    /// Coordinates are in framebuffer pixel space.
    pub fn schedule_pixel_readback(&mut self, attachment_index: usize, x: i32, y: i32) {
        self.pending_readback = Some((attachment_index, x, y));
    }

    /// Read the staging buffer for the given frame slot (data from 2 frames ago).
    /// Called after waiting on the frame's fence.
    pub(crate) fn read_pixel_result(&mut self, current_frame: usize) {
        unsafe {
            self.last_readback = *self.readback_mapping.add(current_frame);
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
                self.device.free_memory(ca.memory, None);
            }

            if let Some(da) = &self.depth_attachment {
                self.device.destroy_image_view(da.view, None);
                self.device.destroy_image(da.image, None);
                self.device.free_memory(da.memory, None);
            }

            self.device.unmap_memory(self.readback_memory);
            self.device.destroy_buffer(self.readback_buffer, None);
            self.device.free_memory(self.readback_memory, None);

            self.device.destroy_render_pass(self.render_pass, None);
            // Descriptor set is freed when the pool is destroyed.
        }
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
) -> vk::RenderPass {
    let mut attachment_descriptions = Vec::new();
    let mut color_attachment_refs = Vec::new();

    // Color attachments get sequential indices starting at 0.
    for (i, cs) in color_specs.iter().enumerate() {
        let vk_fmt = resolve_vk_format(cs.format, color_format, depth_format);
        let desc = vk::AttachmentDescription::default()
            .format(vk_fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        attachment_descriptions.push(desc);

        color_attachment_refs.push(vk::AttachmentReference {
            attachment: i as u32,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        });
    }

    // Depth attachment appended last.
    let depth_attachment_ref = depth_spec.map(|ds| {
        let vk_fmt = resolve_vk_format(ds.format, color_format, depth_format);
        let desc = vk::AttachmentDescription::default()
            .format(vk_fmt)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);
        attachment_descriptions.push(desc);

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

    // Single dependency matching the swapchain render pass structure
    // (1 dependency: EXTERNAL→0) for pipeline compatibility.
    // The exit sync (color write → shader read) is handled by an explicit
    // pipeline barrier between the offscreen and swapchain render passes
    // in the command buffer recording (see application.rs).
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
        .expect("Failed to create offscreen render pass")
}

fn create_color_resources(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    spec: &FramebufferSpec,
    vk_format: vk::Format,
) -> (vk::Image, vk::DeviceMemory, vk::ImageView) {
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
        .usage(
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image =
        unsafe { device.create_image(&image_info, None) }.expect("Failed to create color image");

    let mem_req = unsafe { device.get_image_memory_requirements(image) };
    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        mem_req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    );

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type_index);

    let memory = unsafe { device.allocate_memory(&alloc_info, None) }
        .expect("Failed to allocate color image memory");
    unsafe { device.bind_image_memory(image, memory, 0) }
        .expect("Failed to bind color image memory");

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
        .expect("Failed to create color image view");

    (image, memory, view)
}

fn create_depth_resources(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    spec: &FramebufferSpec,
    vk_format: vk::Format,
) -> (vk::Image, vk::DeviceMemory, vk::ImageView) {
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
        .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1);

    let image =
        unsafe { device.create_image(&image_info, None) }.expect("Failed to create depth image");

    let mem_req = unsafe { device.get_image_memory_requirements(image) };
    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        mem_req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    );

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type_index);

    let memory = unsafe { device.allocate_memory(&alloc_info, None) }
        .expect("Failed to allocate depth image memory");
    unsafe { device.bind_image_memory(image, memory, 0) }
        .expect("Failed to bind depth image memory");

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
        .expect("Failed to create depth image view");

    (image, memory, view)
}

fn create_sampler(device: &ash::Device) -> vk::Sampler {
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

    unsafe { device.create_sampler(&sampler_info, None) }.expect("Failed to create FB sampler")
}

fn create_vk_framebuffer(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    color_views: &[vk::ImageView],
    depth_view: Option<vk::ImageView>,
    spec: &FramebufferSpec,
) -> vk::Framebuffer {
    let mut attachments: Vec<vk::ImageView> = color_views.to_vec();
    if let Some(dv) = depth_view {
        attachments.push(dv);
    }

    let fb_info = vk::FramebufferCreateInfo::default()
        .render_pass(render_pass)
        .attachments(&attachments)
        .width(spec.width)
        .height(spec.height)
        .layers(1);

    unsafe { device.create_framebuffer(&fb_info, None) }
        .expect("Failed to create offscreen framebuffer")
}

fn allocate_descriptor_set(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
) -> vk::DescriptorSet {
    let layouts = [layout];
    let alloc_info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    unsafe { device.allocate_descriptor_sets(&alloc_info) }
        .expect("Failed to allocate FB descriptor set")[0]
}

/// Create a small HOST_VISIBLE staging buffer for pixel readback (2 × i32,
/// one per frame-in-flight slot). Returns (buffer, memory, persistent_map).
fn create_readback_staging_buffer(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
) -> (vk::Buffer, vk::DeviceMemory, *mut i32) {
    let size = (2 * std::mem::size_of::<i32>()) as u64;

    let buf_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer =
        unsafe { device.create_buffer(&buf_info, None) }.expect("Failed to create readback buffer");

    let mem_req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem_type = find_memory_type(
        instance,
        physical_device,
        mem_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type);

    let memory = unsafe { device.allocate_memory(&alloc_info, None) }
        .expect("Failed to allocate readback memory");
    unsafe { device.bind_buffer_memory(buffer, memory, 0) }
        .expect("Failed to bind readback buffer memory");

    let mapping = unsafe { device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty()) }
        .expect("Failed to map readback buffer") as *mut i32;

    // Initialize both slots to -1.
    unsafe {
        *mapping = -1;
        *mapping.add(1) = -1;
    }

    (buffer, memory, mapping)
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
