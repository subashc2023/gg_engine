use ash::khr;
use ash::vk;

use super::{PresentMode, VulkanContext};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SwapchainError {
    SurfaceCapabilities(vk::Result),
    SurfaceFormats(vk::Result),
    SwapchainCreation(vk::Result),
    SwapchainImages(vk::Result),
    ImageViewCreation(vk::Result),
    RenderPassCreation(vk::Result),
    FramebufferCreation(vk::Result),
    CommandPoolCreation(vk::Result),
    CommandBufferAllocation(vk::Result),
    SemaphoreCreation(vk::Result),
    FenceCreation(vk::Result),
}

impl std::fmt::Display for SwapchainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SurfaceCapabilities(e) => write!(f, "Failed to get surface capabilities: {e}"),
            Self::SurfaceFormats(e) => write!(f, "Failed to get surface formats: {e}"),
            Self::SwapchainCreation(e) => write!(f, "Failed to create swapchain: {e}"),
            Self::SwapchainImages(e) => write!(f, "Failed to get swapchain images: {e}"),
            Self::ImageViewCreation(e) => write!(f, "Failed to create image view: {e}"),
            Self::RenderPassCreation(e) => write!(f, "Failed to create render pass: {e}"),
            Self::FramebufferCreation(e) => write!(f, "Failed to create framebuffer: {e}"),
            Self::CommandPoolCreation(e) => write!(f, "Failed to create command pool: {e}"),
            Self::CommandBufferAllocation(e) => {
                write!(f, "Failed to allocate command buffer: {e}")
            }
            Self::SemaphoreCreation(e) => write!(f, "Failed to create semaphore: {e}"),
            Self::FenceCreation(e) => write!(f, "Failed to create fence: {e}"),
        }
    }
}

impl std::error::Error for SwapchainError {}

// ---------------------------------------------------------------------------
// Swapchain
// ---------------------------------------------------------------------------

/// Maximum number of frames that can be in-flight simultaneously.
const MAX_FRAMES_IN_FLIGHT: usize = 2;

pub struct Swapchain {
    swapchain_loader: khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    _images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    framebuffers: Vec<vk::Framebuffer>,
    command_pool: vk::CommandPool,
    command_buffers: Vec<vk::CommandBuffer>,
    // Per-frame-in-flight sync (indexed by current_frame).
    image_available_semaphores: Vec<vk::Semaphore>,
    in_flight_fences: Vec<vk::Fence>,
    // Per-swapchain-image sync (indexed by image_index).
    // Needed because the presentation engine holds onto render_finished until re-acquire.
    render_finished_semaphores: Vec<vk::Semaphore>,
    present_mode: vk::PresentModeKHR,
    current_frame: usize,
    device: ash::Device,
}

impl Swapchain {
    pub fn new(
        vk_ctx: &VulkanContext,
        width: u32,
        height: u32,
        desired_present_mode: PresentMode,
    ) -> Result<Self, SwapchainError> {
        let device = vk_ctx.device().clone();
        let swapchain_loader =
            khr::swapchain::Device::new(vk_ctx.instance(), vk_ctx.device());

        // Query surface capabilities
        let capabilities = unsafe {
            vk_ctx
                .surface_loader()
                .get_physical_device_surface_capabilities(
                    vk_ctx.physical_device(),
                    vk_ctx.surface(),
                )
        }
        .map_err(SwapchainError::SurfaceCapabilities)?;

        // Pick surface format
        let formats = unsafe {
            vk_ctx
                .surface_loader()
                .get_physical_device_surface_formats(vk_ctx.physical_device(), vk_ctx.surface())
        }
        .map_err(SwapchainError::SurfaceFormats)?;

        let format = formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .copied()
            .unwrap_or(formats[0]);

        // Clamp extent
        let extent = vk::Extent2D {
            width: width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        };

        // Image count (min + 1, clamped to max)
        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        // Resolve present mode
        let available_modes = query_present_modes(
            vk_ctx.surface_loader(),
            vk_ctx.physical_device(),
            vk_ctx.surface(),
        );
        let present_mode = resolve_present_mode(desired_present_mode, &available_modes);

        // Create swapchain
        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(vk_ctx.surface())
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_info, None) }
            .map_err(SwapchainError::SwapchainCreation)?;

        // Get images and create views
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain) }
            .map_err(SwapchainError::SwapchainImages)?;

        let image_views = images
            .iter()
            .map(|&image| {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format.format)
                    .components(vk::ComponentMapping::default())
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                unsafe { device.create_image_view(&view_info, None) }
                    .map_err(SwapchainError::ImageViewCreation)
            })
            .collect::<Result<Vec<_>, _>>()?;

        log::info!(
            target: "gg_engine",
            "Swapchain created: {}x{}, {} images, format {:?}, present mode {:?}",
            extent.width, extent.height, images.len(), format.format, present_mode
        );

        // Create render pass
        let color_attachment = vk::AttachmentDescription::default()
            .format(format.format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

        let color_attachment_ref = vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        };

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref));

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&color_attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None) }
            .map_err(SwapchainError::RenderPassCreation)?;

        // Create framebuffers
        let framebuffers = image_views
            .iter()
            .map(|view| {
                let fb_info = vk::FramebufferCreateInfo::default()
                    .render_pass(render_pass)
                    .attachments(std::slice::from_ref(view))
                    .width(extent.width)
                    .height(extent.height)
                    .layers(1);
                unsafe { device.create_framebuffer(&fb_info, None) }
                    .map_err(SwapchainError::FramebufferCreation)
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Create command pool
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(vk_ctx.graphics_queue_family_index())
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        let command_pool = unsafe { device.create_command_pool(&pool_info, None) }
            .map_err(SwapchainError::CommandPoolCreation)?;

        // Allocate command buffers (one per frame-in-flight)
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(MAX_FRAMES_IN_FLIGHT as u32);

        let command_buffers = unsafe { device.allocate_command_buffers(&alloc_info) }
            .map_err(SwapchainError::CommandBufferAllocation)?;

        // Create per-frame-in-flight sync primitives
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

        let mut image_available_semaphores = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut in_flight_fences = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            image_available_semaphores.push(
                unsafe { device.create_semaphore(&semaphore_info, None) }
                    .map_err(SwapchainError::SemaphoreCreation)?,
            );
            in_flight_fences.push(
                unsafe { device.create_fence(&fence_info, None) }
                    .map_err(SwapchainError::FenceCreation)?,
            );
        }

        // Create per-swapchain-image render_finished semaphores.
        // The presentation engine holds these until the image is re-acquired,
        // so we need one per swapchain image to avoid reuse conflicts.
        let mut render_finished_semaphores = Vec::with_capacity(images.len());
        for _ in 0..images.len() {
            render_finished_semaphores.push(
                unsafe { device.create_semaphore(&semaphore_info, None) }
                    .map_err(SwapchainError::SemaphoreCreation)?,
            );
        }

        Ok(Self {
            swapchain_loader,
            swapchain,
            _images: images,
            image_views,
            format,
            extent,
            render_pass,
            framebuffers,
            command_pool,
            command_buffers,
            image_available_semaphores,
            render_finished_semaphores,
            in_flight_fences,
            present_mode,
            current_frame: 0,
            device,
        })
    }

    /// Recreate swapchain after window resize or present mode change.
    ///
    /// If `new_present_mode` is `Some`, the present mode is re-resolved.
    pub fn recreate(
        &mut self,
        vk_ctx: &VulkanContext,
        width: u32,
        height: u32,
        new_present_mode: Option<PresentMode>,
    ) {
        unsafe {
            let _ = self.device.device_wait_idle();
        }

        // Destroy old framebuffers and image views
        for &fb in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(fb, None) };
        }
        for &view in &self.image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }

        // Destroy old per-swapchain-image render_finished semaphores
        // (image count may change after recreation).
        for &sem in &self.render_finished_semaphores {
            unsafe { self.device.destroy_semaphore(sem, None) };
        }

        let old_swapchain = self.swapchain;

        // Re-resolve present mode if requested.
        if let Some(desired) = new_present_mode {
            let available = query_present_modes(
                vk_ctx.surface_loader(),
                vk_ctx.physical_device(),
                vk_ctx.surface(),
            );
            self.present_mode = resolve_present_mode(desired, &available);
        }

        // Query capabilities again
        let capabilities = unsafe {
            vk_ctx
                .surface_loader()
                .get_physical_device_surface_capabilities(
                    vk_ctx.physical_device(),
                    vk_ctx.surface(),
                )
        }
        .expect("Failed to get surface capabilities during resize");

        let extent = vk::Extent2D {
            width: width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        };

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let swapchain_info = vk::SwapchainCreateInfoKHR::default()
            .surface(vk_ctx.surface())
            .min_image_count(image_count)
            .image_format(self.format.format)
            .image_color_space(self.format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(self.present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        self.swapchain =
            unsafe { self.swapchain_loader.create_swapchain(&swapchain_info, None) }
                .expect("Failed to recreate swapchain");

        unsafe {
            self.swapchain_loader
                .destroy_swapchain(old_swapchain, None);
        }

        let images = unsafe { self.swapchain_loader.get_swapchain_images(self.swapchain) }
            .expect("Failed to get swapchain images during resize");

        self.image_views = images
            .iter()
            .map(|&image| {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.format.format)
                    .components(vk::ComponentMapping::default())
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                unsafe { self.device.create_image_view(&view_info, None) }
                    .expect("Failed to create image view during resize")
            })
            .collect();

        self.framebuffers = self
            .image_views
            .iter()
            .map(|view| {
                let fb_info = vk::FramebufferCreateInfo::default()
                    .render_pass(self.render_pass)
                    .attachments(std::slice::from_ref(view))
                    .width(extent.width)
                    .height(extent.height)
                    .layers(1);
                unsafe { self.device.create_framebuffer(&fb_info, None) }
                    .expect("Failed to create framebuffer during resize")
            })
            .collect();

        // Create new render_finished semaphores matching new image count.
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        self.render_finished_semaphores = images
            .iter()
            .map(|_| {
                unsafe { self.device.create_semaphore(&semaphore_info, None) }
                    .expect("Failed to create render_finished semaphore during resize")
            })
            .collect();

        self._images = images;
        self.extent = extent;
        self.current_frame = 0;

        log::info!(
            target: "gg_engine",
            "Swapchain recreated: {}x{}, present mode {:?}",
            extent.width, extent.height, self.present_mode
        );
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    pub fn swapchain_loader(&self) -> &khr::swapchain::Device {
        &self.swapchain_loader
    }

    pub fn swapchain(&self) -> vk::SwapchainKHR {
        self.swapchain
    }

    pub fn render_pass(&self) -> vk::RenderPass {
        self.render_pass
    }

    pub fn format(&self) -> vk::SurfaceFormatKHR {
        self.format
    }

    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    pub fn command_pool(&self) -> vk::CommandPool {
        self.command_pool
    }

    pub fn command_buffer(&self, index: usize) -> vk::CommandBuffer {
        self.command_buffers[index]
    }

    pub fn framebuffer(&self, index: usize) -> vk::Framebuffer {
        self.framebuffers[index]
    }

    pub fn current_frame(&self) -> usize {
        self.current_frame
    }

    pub fn image_available_semaphore(&self) -> vk::Semaphore {
        self.image_available_semaphores[self.current_frame]
    }

    pub fn render_finished_semaphore(&self, image_index: u32) -> vk::Semaphore {
        self.render_finished_semaphores[image_index as usize]
    }

    pub fn in_flight_fence(&self) -> vk::Fence {
        self.in_flight_fences[self.current_frame]
    }

    pub fn present_mode(&self) -> vk::PresentModeKHR {
        self.present_mode
    }

    pub fn advance_frame(&mut self) {
        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
    }
}

// ---------------------------------------------------------------------------
// Present mode helpers
// ---------------------------------------------------------------------------

fn query_present_modes(
    surface_loader: &khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Vec<vk::PresentModeKHR> {
    unsafe {
        surface_loader
            .get_physical_device_surface_present_modes(physical_device, surface)
    }
    .unwrap_or_else(|_| vec![vk::PresentModeKHR::FIFO])
}

fn resolve_present_mode(
    desired: PresentMode,
    available: &[vk::PresentModeKHR],
) -> vk::PresentModeKHR {
    match desired {
        PresentMode::Fifo => vk::PresentModeKHR::FIFO,
        PresentMode::Mailbox => {
            if available.contains(&vk::PresentModeKHR::MAILBOX) {
                vk::PresentModeKHR::MAILBOX
            } else if available.contains(&vk::PresentModeKHR::IMMEDIATE) {
                vk::PresentModeKHR::IMMEDIATE
            } else {
                vk::PresentModeKHR::FIFO
            }
        }
        PresentMode::Immediate => {
            if available.contains(&vk::PresentModeKHR::IMMEDIATE) {
                vk::PresentModeKHR::IMMEDIATE
            } else if available.contains(&vk::PresentModeKHR::MAILBOX) {
                vk::PresentModeKHR::MAILBOX
            } else {
                vk::PresentModeKHR::FIFO
            }
        }
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            for &fence in &self.in_flight_fences {
                self.device.destroy_fence(fence, None);
            }
            for &sem in &self.render_finished_semaphores {
                self.device.destroy_semaphore(sem, None);
            }
            for &sem in &self.image_available_semaphores {
                self.device.destroy_semaphore(sem, None);
            }
            self.device.destroy_command_pool(self.command_pool, None);
            for &fb in &self.framebuffers {
                self.device.destroy_framebuffer(fb, None);
            }
            self.device.destroy_render_pass(self.render_pass, None);
            for &view in &self.image_views {
                self.device.destroy_image_view(view, None);
            }
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
        }
        log::info!(target: "gg_engine", "Swapchain destroyed");
    }
}
