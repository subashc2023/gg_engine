use ash::vk;

use super::buffer::find_memory_type;

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
}

// ---------------------------------------------------------------------------
// Framebuffer
// ---------------------------------------------------------------------------

/// Offscreen framebuffer with color and depth attachments, suitable for
/// rendering a scene to a texture that can then be displayed in egui.
pub struct Framebuffer {
    // Color attachment.
    color_image: vk::Image,
    color_memory: vk::DeviceMemory,
    color_view: vk::ImageView,
    sampler: vk::Sampler,
    descriptor_set: vk::DescriptorSet,

    // Depth attachment.
    depth_image: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,

    // Render pass and framebuffer.
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,

    // Egui texture handle (set after registration).
    egui_texture_id: Option<egui::TextureId>,

    // Spec / format info.
    spec: FramebufferSpec,
    color_format: vk::Format,
    depth_format: vk::Format,

    // Vulkan handles needed for resize/cleanup.
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
}

impl Framebuffer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        descriptor_pool: vk::DescriptorPool,
        descriptor_set_layout: vk::DescriptorSetLayout,
        color_format: vk::Format,
        depth_format: vk::Format,
        spec: FramebufferSpec,
    ) -> Self {
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

        let render_pass = create_offscreen_render_pass(device, color_format, depth_format);

        let sampler = create_sampler(device);

        let (color_image, color_memory, color_view) =
            create_color_resources(instance, physical_device, device, &spec, color_format);

        let (depth_image, depth_memory, depth_view) =
            create_depth_resources(instance, physical_device, device, &spec, depth_format);

        let framebuffer = create_vk_framebuffer(device, render_pass, color_view, depth_view, &spec);

        let descriptor_set =
            allocate_descriptor_set(device, descriptor_pool, descriptor_set_layout);
        write_descriptor_set(device, descriptor_set, color_view, sampler);

        Self {
            color_image,
            color_memory,
            color_view,
            sampler,
            descriptor_set,
            depth_image,
            depth_memory,
            depth_view,
            render_pass,
            framebuffer,
            egui_texture_id: None,
            spec,
            color_format,
            depth_format,
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

        self.spec = FramebufferSpec { width, height };

        // Destroy old framebuffer and attachment resources (keep render pass, sampler, descriptor set).
        unsafe {
            self.device.destroy_framebuffer(self.framebuffer, None);
            self.device.destroy_image_view(self.color_view, None);
            self.device.destroy_image(self.color_image, None);
            self.device.free_memory(self.color_memory, None);
            self.device.destroy_image_view(self.depth_view, None);
            self.device.destroy_image(self.depth_image, None);
            self.device.free_memory(self.depth_memory, None);
        }

        // Recreate at new size.
        let (color_image, color_memory, color_view) = create_color_resources(
            &self.instance,
            self.physical_device,
            &self.device,
            &self.spec,
            self.color_format,
        );
        self.color_image = color_image;
        self.color_memory = color_memory;
        self.color_view = color_view;

        let (depth_image, depth_memory, depth_view) = create_depth_resources(
            &self.instance,
            self.physical_device,
            &self.device,
            &self.spec,
            self.depth_format,
        );
        self.depth_image = depth_image;
        self.depth_memory = depth_memory;
        self.depth_view = depth_view;

        self.framebuffer = create_vk_framebuffer(
            &self.device,
            self.render_pass,
            self.color_view,
            self.depth_view,
            &self.spec,
        );

        // Update the existing descriptor set in-place with the new image view.
        write_descriptor_set(
            &self.device,
            self.descriptor_set,
            self.color_view,
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

    pub(crate) fn render_pass(&self) -> vk::RenderPass {
        self.render_pass
    }

    pub(crate) fn vk_framebuffer(&self) -> vk::Framebuffer {
        self.framebuffer
    }

    pub(crate) fn descriptor_set(&self) -> vk::DescriptorSet {
        self.descriptor_set
    }

    pub(crate) fn color_image(&self) -> vk::Image {
        self.color_image
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
            self.device.destroy_image_view(self.color_view, None);
            self.device.destroy_image(self.color_image, None);
            self.device.free_memory(self.color_memory, None);
            self.device.destroy_image_view(self.depth_view, None);
            self.device.destroy_image(self.depth_image, None);
            self.device.free_memory(self.depth_memory, None);
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
    color_format: vk::Format,
    depth_format: vk::Format,
) -> vk::RenderPass {
    let color_attachment = vk::AttachmentDescription::default()
        .format(color_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);

    let depth_attachment = vk::AttachmentDescription::default()
        .format(depth_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::DONT_CARE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

    let color_attachment_ref = vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    };

    let depth_attachment_ref = vk::AttachmentReference {
        attachment: 1,
        layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
    };

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_attachment_ref))
        .depth_stencil_attachment(&depth_attachment_ref);

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

    let attachments = [color_attachment, depth_attachment];
    let render_pass_info = vk::RenderPassCreateInfo::default()
        .attachments(&attachments)
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
    color_format: vk::Format,
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
        .format(color_format)
        .tiling(vk::ImageTiling::OPTIMAL)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
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
        .format(color_format)
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
    depth_format: vk::Format,
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
        .format(depth_format)
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
        .format(depth_format)
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
    color_view: vk::ImageView,
    depth_view: vk::ImageView,
    spec: &FramebufferSpec,
) -> vk::Framebuffer {
    let attachments = [color_view, depth_view];
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
