use std::sync::{Arc, Mutex};

use ash::khr;
use ash::vk;

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
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
    DepthImageCreation(vk::Result),
    DepthMemoryAllocation(String),
    NoSupportedDepthFormat,
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
            Self::DepthImageCreation(e) => write!(f, "Failed to create depth image: {e}"),
            Self::DepthMemoryAllocation(e) => {
                write!(f, "Failed to allocate depth image memory: {e}")
            }
            Self::NoSupportedDepthFormat => {
                write!(f, "No supported depth format found (tried D32_SFLOAT, D32_SFLOAT_S8_UINT, D24_UNORM_S8_UINT)")
            }
        }
    }
}

impl std::error::Error for SwapchainError {}

// ---------------------------------------------------------------------------
// Swapchain
// ---------------------------------------------------------------------------

use super::MAX_FRAMES_IN_FLIGHT;

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
    // Depth buffer resources.
    depth_format: vk::Format,
    depth_image: vk::Image,
    depth_allocation: Option<GpuAllocation>,
    depth_image_view: vk::ImageView,
    // GPU allocator for depth buffer recreation on resize.
    allocator: Arc<Mutex<GpuAllocator>>,
    device: ash::Device,
}

impl Swapchain {
    pub fn new(
        vk_ctx: &VulkanContext,
        width: u32,
        height: u32,
        desired_present_mode: PresentMode,
        allocator: &Arc<Mutex<GpuAllocator>>,
    ) -> Result<Self, SwapchainError> {
        let device = vk_ctx.device().clone();
        let swapchain_loader = khr::swapchain::Device::new(vk_ctx.instance(), vk_ctx.device());

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

        // Find a supported depth format.
        let depth_format = find_depth_format(vk_ctx.instance(), vk_ctx.physical_device())?;

        // Create render pass (color + depth attachments).
        let render_pass = create_render_pass(&device, format.format, depth_format)?;

        // Create depth buffer resources.
        let (depth_image, depth_allocation, depth_image_view) = create_depth_resources(
            allocator,
            &device,
            extent,
            depth_format,
        )?;

        // Create framebuffers (color + depth).
        let framebuffers =
            create_framebuffers(&device, render_pass, &image_views, depth_image_view, extent)?;

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
            depth_format,
            depth_image,
            depth_allocation: Some(depth_allocation),
            depth_image_view,
            allocator: allocator.clone(),
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
    ) -> Result<(), SwapchainError> {
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(SwapchainError::SurfaceCapabilities)?;
        }

        // Destroy old framebuffers, image views, and depth resources.
        for &fb in &self.framebuffers {
            unsafe { self.device.destroy_framebuffer(fb, None) };
        }
        for &view in &self.image_views {
            unsafe { self.device.destroy_image_view(view, None) };
        }
        // Destroy old depth resources.
        unsafe {
            self.device.destroy_image_view(self.depth_image_view, None);
            self.device.destroy_image(self.depth_image, None);
        }
        self.depth_allocation.take(); // frees memory via GpuAllocation drop

        // Destroy old per-swapchain-image render_finished semaphores
        // (image count may change after recreation).
        for &sem in &self.render_finished_semaphores {
            unsafe { self.device.destroy_semaphore(sem, None) };
        }

        // Render pass reused — it depends on formats (which don't change), not dimensions.

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
        .map_err(SwapchainError::SurfaceCapabilities)?;

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

        self.swapchain = unsafe {
            self.swapchain_loader
                .create_swapchain(&swapchain_info, None)
        }
        .map_err(SwapchainError::SwapchainCreation)?;

        unsafe {
            self.swapchain_loader.destroy_swapchain(old_swapchain, None);
        }

        let images = unsafe { self.swapchain_loader.get_swapchain_images(self.swapchain) }
            .map_err(SwapchainError::SwapchainImages)?;

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
                    .map_err(SwapchainError::ImageViewCreation)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let (depth_image, depth_allocation, depth_image_view) = create_depth_resources(
            &self.allocator,
            &self.device,
            extent,
            self.depth_format,
        )?;
        self.depth_image = depth_image;
        self.depth_allocation = Some(depth_allocation);
        self.depth_image_view = depth_image_view;

        self.framebuffers = create_framebuffers(
            &self.device,
            self.render_pass,
            &self.image_views,
            self.depth_image_view,
            extent,
        )?;

        // Create new render_finished semaphores matching new image count.
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        self.render_finished_semaphores = images
            .iter()
            .map(|_| {
                unsafe { self.device.create_semaphore(&semaphore_info, None) }
                    .map_err(SwapchainError::SemaphoreCreation)
            })
            .collect::<Result<Vec<_>, _>>()?;

        self._images = images;
        self.extent = extent;
        self.current_frame = 0;

        log::info!(
            target: "gg_engine",
            "Swapchain recreated: {}x{}, present mode {:?}",
            extent.width, extent.height, self.present_mode
        );

        Ok(())
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

    pub fn depth_format(&self) -> vk::Format {
        self.depth_format
    }

    pub fn present_mode(&self) -> vk::PresentModeKHR {
        self.present_mode
    }

    pub fn advance_frame(&mut self) {
        self.current_frame = (self.current_frame + 1) % MAX_FRAMES_IN_FLIGHT;
    }
}

// ---------------------------------------------------------------------------
// Depth buffer helpers
// ---------------------------------------------------------------------------

/// Find a supported depth format. Prefers D32_SFLOAT, falls back to
/// D32_SFLOAT_S8_UINT, then D24_UNORM_S8_UINT.
fn find_depth_format(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::Format, SwapchainError> {
    let candidates = [
        vk::Format::D32_SFLOAT,
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D24_UNORM_S8_UINT,
    ];
    for &format in &candidates {
        let props =
            unsafe { instance.get_physical_device_format_properties(physical_device, format) };
        if props
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT)
        {
            return Ok(format);
        }
    }
    Err(SwapchainError::NoSupportedDepthFormat)
}

/// Create the depth image, allocate memory via sub-allocator, create the image view.
fn create_depth_resources(
    allocator: &Arc<Mutex<GpuAllocator>>,
    device: &ash::Device,
    extent: vk::Extent2D,
    depth_format: vk::Format,
) -> Result<(vk::Image, GpuAllocation, vk::ImageView), SwapchainError> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
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

    let depth_image = unsafe { device.create_image(&image_info, None) }
        .map_err(SwapchainError::DepthImageCreation)?;

    let allocation = GpuAllocator::allocate_for_image(
        allocator,
        device,
        depth_image,
        "SwapchainDepth",
        MemoryLocation::GpuOnly,
    )
    .map_err(SwapchainError::DepthMemoryAllocation)?;

    let view_info = vk::ImageViewCreateInfo::default()
        .image(depth_image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(depth_format)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    let depth_image_view = unsafe { device.create_image_view(&view_info, None) }
        .map_err(SwapchainError::ImageViewCreation)?;

    Ok((depth_image, allocation, depth_image_view))
}


// ---------------------------------------------------------------------------
// Render pass helper
// ---------------------------------------------------------------------------

fn create_render_pass(
    device: &ash::Device,
    color_format: vk::Format,
    depth_format: vk::Format,
) -> Result<vk::RenderPass, SwapchainError> {
    let color_attachment = vk::AttachmentDescription::default()
        .format(color_format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

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
        .map_err(SwapchainError::RenderPassCreation)
}

// ---------------------------------------------------------------------------
// Framebuffer helper
// ---------------------------------------------------------------------------

fn create_framebuffers(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    color_views: &[vk::ImageView],
    depth_view: vk::ImageView,
    extent: vk::Extent2D,
) -> Result<Vec<vk::Framebuffer>, SwapchainError> {
    color_views
        .iter()
        .map(|&color_view| {
            let attachments = [color_view, depth_view];
            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            unsafe { device.create_framebuffer(&fb_info, None) }
                .map_err(SwapchainError::FramebufferCreation)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Present mode helpers
// ---------------------------------------------------------------------------

fn query_present_modes(
    surface_loader: &khr::surface::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Vec<vk::PresentModeKHR> {
    unsafe { surface_loader.get_physical_device_surface_present_modes(physical_device, surface) }
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
        // NOTE: Caller (EngineRunner::Drop) must call device_wait_idle before
        // dropping the swapchain.
        unsafe {
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
            // Destroy depth Vulkan objects.
            self.device.destroy_image_view(self.depth_image_view, None);
            self.device.destroy_image(self.depth_image, None);
        }
        // Free depth allocation via GpuAllocation drop.
        self.depth_allocation.take();
        unsafe {
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
