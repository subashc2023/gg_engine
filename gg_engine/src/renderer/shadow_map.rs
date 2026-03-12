use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::lighting::NUM_SHADOW_CASCADES;
use super::uniform_buffer::UniformBuffer;
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default shadow map resolution (width and height in texels per cascade).
/// 4 cascades × 4096² × 4 bytes = 256 MB of GPU depth memory
/// (less than the old 2 × 6144² = 288 MB).
pub const DEFAULT_SHADOW_MAP_SIZE: u32 = 4096;

// ---------------------------------------------------------------------------
// ShadowMapSystem — depth-only shadow pass infrastructure
// ---------------------------------------------------------------------------

/// Manages the depth-only shadow map for directional light shadows.
///
/// Owns a depth image (D32_SFLOAT), a comparison sampler, a dedicated
/// render pass + framebuffer, a light-VP UBO, and descriptor sets for
/// binding the shadow map in the main 3D fragment shader (set 4).
///
/// Follows the same slot pattern as LightingSystem / CameraSystem.
pub(crate) struct ShadowMapSystem {
    // Depth image (2-layer array for cascades) + views
    depth_image: vk::Image,
    #[allow(dead_code)] // Owned for memory lifetime; freed on drop.
    depth_allocation: GpuAllocation,
    /// Per-layer views (TYPE_2D) for framebuffer attachments.
    depth_layer_views: [vk::ImageView; NUM_SHADOW_CASCADES],
    /// Full-array view (TYPE_2D_ARRAY) for sampling in the main pass.
    depth_array_view: vk::ImageView,

    // Comparison sampler for hardware PCF (binding 0)
    sampler: vk::Sampler,
    // Non-comparison sampler for PCSS blocker search (binding 1)
    raw_sampler: vk::Sampler,

    // Depth-only render pass + per-cascade framebuffers
    render_pass: vk::RenderPass,
    framebuffers: [vk::Framebuffer; NUM_SHADOW_CASCADES],

    // Resolution
    width: u32,
    height: u32,

    // Light VP UBO — retained for descriptor pool accounting but no longer
    // written per-frame (push constants replaced it for cascade correctness).
    #[allow(dead_code)]
    light_vp_ubo: UniformBuffer,

    // Descriptor set layout for the main pass (set 4):
    //   binding 0 = combined image sampler (shadow map depth texture)
    shadow_ds_layout: vk::DescriptorSetLayout,
    shadow_descriptor_sets: Vec<vk::DescriptorSet>,

    // Descriptor set layout for the shadow pass itself (set 0):
    //   binding 0 = UBO (light VP matrix) — retained for descriptor pool sizing.
    shadow_camera_ds_layout: vk::DescriptorSetLayout,
    #[allow(dead_code)]
    shadow_camera_descriptor_sets: Vec<vk::DescriptorSet>,

    device: ash::Device,
    #[allow(dead_code)] // Kept for potential resize operations.
    allocator: Arc<Mutex<GpuAllocator>>,
}

impl ShadowMapSystem {
    /// Create the shadow map system with the given depth format and resolution.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        descriptor_pool: vk::DescriptorPool,
        depth_format: vk::Format,
        width: u32,
        height: u32,
        command_pool: vk::CommandPool,
        graphics_queue: vk::Queue,
    ) -> Result<Self, String> {
        // --- Depth image (2-layer array for cascades) ---
        let (depth_image, depth_allocation, depth_layer_views, depth_array_view) =
            Self::create_depth_resources(allocator, device, depth_format, width, height)?;

        // Transition all layers from UNDEFINED to DEPTH_STENCIL_READ_ONLY_OPTIMAL
        // so it's valid for sampling in the main pass even before any shadow pass runs.
        Self::transition_depth_initial(device, command_pool, graphics_queue, depth_image)?;

        // --- Comparison sampler (for hardware PCF via sampler2DArrayShadow) ---
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
            .compare_enable(true)
            .compare_op(vk::CompareOp::LESS_OR_EQUAL)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .min_lod(0.0)
            .max_lod(1.0);
        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .map_err(|e| format!("Failed to create shadow sampler: {e}"))?;

        // --- Non-comparison sampler (for PCSS blocker search via sampler2DArray) ---
        let raw_sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_BORDER)
            .border_color(vk::BorderColor::FLOAT_OPAQUE_WHITE)
            .compare_enable(false)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .min_lod(0.0)
            .max_lod(1.0);
        let raw_sampler = unsafe { device.create_sampler(&raw_sampler_info, None) }
            .map_err(|e| format!("Failed to create shadow raw sampler: {e}"))?;

        // --- Depth-only render pass ---
        let render_pass = Self::create_render_pass(device, depth_format)?;

        // --- Per-cascade framebuffers ---
        let mut framebuffers = [vk::Framebuffer::null(); NUM_SHADOW_CASCADES];
        for i in 0..NUM_SHADOW_CASCADES {
            framebuffers[i] =
                Self::create_framebuffer(device, render_pass, depth_layer_views[i], width, height)?;
        }

        // --- Light VP UBO (64 bytes = mat4) ---
        let light_vp_ubo = UniformBuffer::new(allocator, device, std::mem::size_of::<[f32; 16]>())?;

        // --- Shadow camera descriptor set layout (for shadow pass, set 0) ---
        //     binding 0: UBO (light VP matrix), vertex stage
        let shadow_camera_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let shadow_camera_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&shadow_camera_binding));
        let shadow_camera_ds_layout =
            unsafe { device.create_descriptor_set_layout(&shadow_camera_layout_info, None) }
                .map_err(|e| {
                    format!("Failed to create shadow camera descriptor set layout: {e}")
                })?;

        // --- Shadow map descriptor set layout (for main pass, set 4) ---
        //     binding 0: comparison sampler (sampler2DArrayShadow) for PCF
        //     binding 1: non-comparison sampler (sampler2DArray) for PCSS blocker search
        let shadow_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let shadow_ds_layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(&shadow_bindings);
        let shadow_ds_layout =
            unsafe { device.create_descriptor_set_layout(&shadow_ds_layout_info, None) }
                .map_err(|e| format!("Failed to create shadow map descriptor set layout: {e}"))?;

        // --- Allocate descriptor sets ---
        let total_slots = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;

        // Shadow camera sets (UBO)
        let camera_layouts = vec![shadow_camera_ds_layout; total_slots];
        let camera_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&camera_layouts);
        let shadow_camera_descriptor_sets =
            unsafe { device.allocate_descriptor_sets(&camera_alloc_info) }
                .map_err(|e| format!("Failed to allocate shadow camera descriptor sets: {e}"))?;

        // Write UBO to each shadow camera descriptor set
        for (i, &ds) in shadow_camera_descriptor_sets.iter().enumerate() {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(light_vp_ubo.buffer(i))
                .offset(0)
                .range(std::mem::size_of::<[f32; 16]>() as u64);
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(std::slice::from_ref(&buffer_info));
            unsafe {
                device.update_descriptor_sets(&[write], &[]);
            }
        }

        // Shadow map sets (sampler) — all point to the same shadow depth image
        let sampler_layouts = vec![shadow_ds_layout; total_slots];
        let sampler_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&sampler_layouts);
        let shadow_descriptor_sets =
            unsafe { device.allocate_descriptor_sets(&sampler_alloc_info) }
                .map_err(|e| format!("Failed to allocate shadow map descriptor sets: {e}"))?;

        // Write shadow map array image to each descriptor set (both bindings).
        for &ds in &shadow_descriptor_sets {
            // Binding 0: comparison sampler (for PCF)
            let cmp_image_info = vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(depth_array_view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            let cmp_write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&cmp_image_info));
            // Binding 1: non-comparison sampler (for PCSS blocker search)
            let raw_image_info = vk::DescriptorImageInfo::default()
                .sampler(raw_sampler)
                .image_view(depth_array_view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            let raw_write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&raw_image_info));
            unsafe {
                device.update_descriptor_sets(&[cmp_write, raw_write], &[]);
            }
        }

        Ok(Self {
            depth_image,
            depth_allocation,
            depth_layer_views,
            depth_array_view,
            sampler,
            raw_sampler,
            render_pass,
            framebuffers,
            width,
            height,
            light_vp_ubo,
            shadow_ds_layout,
            shadow_descriptor_sets,
            shadow_camera_ds_layout,
            shadow_camera_descriptor_sets,
            device: device.clone(),
            allocator: allocator.clone(),
        })
    }

    // -- Accessors --

    /// Descriptor set layout for the main 3D pass (set 4 = shadow map sampler).
    pub fn ds_layout(&self) -> vk::DescriptorSetLayout {
        self.shadow_ds_layout
    }

    /// Descriptor set layout for the shadow pass (set 0 = light VP UBO).
    pub fn camera_ds_layout(&self) -> vk::DescriptorSetLayout {
        self.shadow_camera_ds_layout
    }

    /// Shadow map descriptor set for the main pass (set 4).
    pub fn descriptor_set(&self, current_frame: usize, viewport_index: usize) -> vk::DescriptorSet {
        self.shadow_descriptor_sets[UniformBuffer::slot(current_frame, viewport_index)]
    }

    /// Shadow camera descriptor set (set 0 in shadow pass).
    /// Retained for API completeness; no longer used since push constants
    /// replaced the UBO for per-cascade light VP.
    #[allow(dead_code)]
    pub fn camera_descriptor_set(
        &self,
        current_frame: usize,
        viewport_index: usize,
    ) -> vk::DescriptorSet {
        self.shadow_camera_descriptor_sets[UniformBuffer::slot(current_frame, viewport_index)]
    }

    /// The shadow depth-only render pass.
    pub fn render_pass(&self) -> vk::RenderPass {
        self.render_pass
    }

    /// The shadow framebuffer for a specific cascade.
    pub fn framebuffer(&self, cascade: usize) -> vk::Framebuffer {
        self.framebuffers[cascade]
    }

    /// Shadow map width in texels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Shadow map height in texels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Write the light-space VP matrix to the UBO for the given slot.
    /// Retained for API completeness; no longer called since push constants
    /// replaced the UBO for per-cascade light VP.
    #[allow(dead_code)]
    pub fn write_light_vp(&self, light_vp: &Mat4, current_frame: usize, viewport_index: usize) {
        let slot = UniformBuffer::slot(current_frame, viewport_index);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                light_vp as *const Mat4 as *const u8,
                std::mem::size_of::<Mat4>(),
            )
        };
        self.light_vp_ubo.update(slot, bytes);
    }

    // -- Resource creation helpers --

    fn create_depth_resources(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        depth_format: vk::Format,
        width: u32,
        height: u32,
    ) -> Result<
        (
            vk::Image,
            GpuAllocation,
            [vk::ImageView; NUM_SHADOW_CASCADES],
            vk::ImageView,
        ),
        String,
    > {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(depth_format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(NUM_SHADOW_CASCADES as u32)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let depth_image = unsafe { device.create_image(&image_info, None) }
            .map_err(|e| format!("Failed to create shadow depth image: {e}"))?;

        let depth_allocation = GpuAllocator::allocate_for_image(
            allocator,
            device,
            depth_image,
            "ShadowMapDepth",
            MemoryLocation::GpuOnly,
        )?;

        // Per-layer views (TYPE_2D) — used as framebuffer attachments.
        let mut layer_views = [vk::ImageView::null(); NUM_SHADOW_CASCADES];
        for (i, view) in layer_views.iter_mut().enumerate() {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(depth_image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(depth_format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: i as u32,
                    layer_count: 1,
                });
            *view = unsafe { device.create_image_view(&view_info, None) }
                .map_err(|e| format!("Failed to create shadow layer {i} image view: {e}"))?;
        }

        // Full-array view (TYPE_2D_ARRAY) — used for sampling in the main pass.
        let array_view_info = vk::ImageViewCreateInfo::default()
            .image(depth_image)
            .view_type(vk::ImageViewType::TYPE_2D_ARRAY)
            .format(depth_format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::DEPTH,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: NUM_SHADOW_CASCADES as u32,
            });
        let array_view = unsafe { device.create_image_view(&array_view_info, None) }
            .map_err(|e| format!("Failed to create shadow array image view: {e}"))?;

        Ok((depth_image, depth_allocation, layer_views, array_view))
    }

    /// One-shot layout transition: UNDEFINED → DEPTH_STENCIL_READ_ONLY_OPTIMAL.
    ///
    /// This ensures the shadow map image is in a valid layout for sampling
    /// even when no shadow pass has been executed yet.
    fn transition_depth_initial(
        device: &ash::Device,
        command_pool: vk::CommandPool,
        queue: vk::Queue,
        image: vk::Image,
    ) -> Result<(), String> {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_pool(command_pool)
            .command_buffer_count(1);

        let cmd_buf = unsafe { device.allocate_command_buffers(&alloc_info) }
            .map_err(|e| format!("Failed to allocate one-shot command buffer: {e}"))?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        unsafe {
            device
                .begin_command_buffer(cmd_buf, &begin_info)
                .map_err(|e| format!("Failed to begin one-shot command buffer: {e}"))?;

            let barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::DEPTH,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: NUM_SHADOW_CASCADES as u32,
                })
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::SHADER_READ);

            device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );

            device
                .end_command_buffer(cmd_buf)
                .map_err(|e| format!("Failed to end one-shot command buffer: {e}"))?;

            let cmd_bufs = [cmd_buf];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);
            device
                .queue_submit(queue, &[submit_info], vk::Fence::null())
                .map_err(|e| format!("Failed to submit layout transition: {e}"))?;
            device
                .queue_wait_idle(queue)
                .map_err(|e| format!("Failed to wait for queue idle: {e}"))?;

            device.free_command_buffers(command_pool, &[cmd_buf]);
        }

        Ok(())
    }

    fn create_render_pass(
        device: &ash::Device,
        depth_format: vk::Format,
    ) -> Result<vk::RenderPass, String> {
        let depth_attachment = vk::AttachmentDescription::default()
            .format(depth_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);

        let depth_ref = vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
        };

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .depth_stencil_attachment(&depth_ref);

        // External dependency: ensure depth writes finish before fragment shader reads.
        let dependency = vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::LATE_FRAGMENT_TESTS)
            .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        let rp_info = vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&depth_attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(std::slice::from_ref(&dependency));

        unsafe { device.create_render_pass(&rp_info, None) }
            .map_err(|e| format!("Failed to create shadow render pass: {e}"))
    }

    fn create_framebuffer(
        device: &ash::Device,
        render_pass: vk::RenderPass,
        depth_view: vk::ImageView,
        width: u32,
        height: u32,
    ) -> Result<vk::Framebuffer, String> {
        let fb_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(std::slice::from_ref(&depth_view))
            .width(width)
            .height(height)
            .layers(1);

        unsafe { device.create_framebuffer(&fb_info, None) }
            .map_err(|e| format!("Failed to create shadow framebuffer: {e}"))
    }
}

impl Drop for ShadowMapSystem {
    fn drop(&mut self) {
        unsafe {
            for fb in &self.framebuffers {
                self.device.destroy_framebuffer(*fb, None);
            }
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_sampler(self.raw_sampler, None);
            self.device.destroy_image_view(self.depth_array_view, None);
            for view in &self.depth_layer_views {
                self.device.destroy_image_view(*view, None);
            }
            self.device.destroy_image(self.depth_image, None);
            self.device
                .destroy_descriptor_set_layout(self.shadow_ds_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.shadow_camera_ds_layout, None);
        }
        // depth_allocation is dropped automatically (GpuAllocation::Drop frees memory).
    }
}

// ---------------------------------------------------------------------------
// Light-space matrix computation
// ---------------------------------------------------------------------------

/// Maximum shadow frustum half-extent (world units). Prevents unbounded growth
/// when physics objects fall far from the scene.
const MAX_SHADOW_EXTENT: f32 = 200.0;
/// Minimum shadow frustum half-extent to avoid degenerate projections.
const MIN_SHADOW_EXTENT: f32 = 1.0;

/// Compute an orthographic light-space view-projection matrix for a
/// directional light, fitted to the given scene AABB.
///
/// The extent is clamped to prevent unbounded frustum growth (e.g. when
/// physics objects fall far away). Texel snapping is applied to prevent
/// shadow shimmer from sub-texel jitter as objects move.
///
/// The Vulkan Y-flip is applied so the result can be used directly
/// as a VP matrix in the shadow pass and for fragment-shader projection.
pub fn compute_directional_light_vp(
    light_direction: Vec3,
    scene_min: Vec3,
    scene_max: Vec3,
) -> Mat4 {
    let light_dir = light_direction.normalize();
    let center = (scene_min + scene_max) * 0.5;
    let extent =
        ((scene_max - scene_min).length() * 0.5).clamp(MIN_SHADOW_EXTENT, MAX_SHADOW_EXTENT);

    // Position the light camera behind the scene center, looking along the light direction.
    let light_pos = center - light_dir * extent;

    // Choose an up vector that isn't parallel to the light direction.
    let up = if light_dir.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    let light_view = Mat4::look_at_lh(light_pos, center, up);
    let mut light_proj = Mat4::orthographic_lh(-extent, extent, -extent, extent, 0.0, extent * 2.0);

    // Vulkan Y-flip: applied to the projection BEFORE view multiplication,
    // matching the convention used by EditorCamera, SceneCamera, and sandbox.
    light_proj.y_axis.y *= -1.0;

    // Shadow map texel snapping: quantize the VP translation to shadow map
    // texel boundaries, preventing sub-texel jitter as objects move.
    let shadow_vp = light_proj * light_view;
    let half_texels = DEFAULT_SHADOW_MAP_SIZE as f32 * 0.5;

    // Transform the origin into clip space to find the sub-texel offset.
    let origin_clip = shadow_vp.transform_point3(Vec3::ZERO);
    let tx = origin_clip.x * half_texels;
    let ty = origin_clip.y * half_texels;
    let offset_x = (tx.round() - tx) / half_texels;
    let offset_y = (ty.round() - ty) / half_texels;

    // Apply the snap offset to the projection matrix.
    light_proj.w_axis.x += offset_x;
    light_proj.w_axis.y += offset_y;

    light_proj * light_view
}

// ---------------------------------------------------------------------------
// Camera-frustum-fitted cascaded shadow maps
// ---------------------------------------------------------------------------

/// Camera frustum data needed for per-cascade shadow map frustum fitting.
///
/// Pass this to [`compute_cascade_vps`] (or indirectly via
/// `Scene::render_shadow_pass`) so that each cascade covers a different
/// depth slice of the camera frustum instead of the full scene AABB.
pub struct ShadowCameraInfo {
    /// The camera's full view-projection matrix (including reverse-Z and Y-flip).
    pub view_projection: Mat4,
    /// Camera near clip distance (positive, world units).
    pub near: f32,
    /// Camera far clip distance (positive, world units).
    pub far: f32,
    /// Camera position in world space. Used as the bounding sphere center
    /// so that cascades are rotation-invariant (only translation moves them).
    pub camera_position: Vec3,
    /// Maximum shadow distance in world units. Cascades are distributed within
    /// this range. Defaults to 100.0 if not specified.
    pub shadow_distance: f32,
}

/// Blend factor between uniform and logarithmic cascade splits.
/// 0.0 = fully uniform, 1.0 = fully logarithmic.
/// Higher values allocate more shadow resolution to near-field geometry.
const CASCADE_SPLIT_LAMBDA: f32 = 0.75;

/// Compute per-cascade orthographic light-space VP matrices by fitting
/// each cascade's frustum to a sub-frustum of the camera.
///
/// Returns `(cascade_vps, split_ndcs, shadow_far, texel_sizes)` where
/// `split_ndcs` contains 3 cascade split depths in Vulkan NDC (reverse-Z)
/// for the fragment shader, `shadow_far` is the effective shadow distance
/// (for shader distance fade), and `texel_sizes` gives the world-units-per-
/// texel for each cascade (used for per-cascade bias scaling).
///
/// The camera sub-frustum corners define the XY bounds of each cascade's
/// orthographic projection, while the scene AABB extends the Z range to
/// include shadow casters that might be outside the camera frustum.
pub fn compute_cascade_vps(
    camera: &ShadowCameraInfo,
    light_direction: Vec3,
    scene_min: Vec3,
    scene_max: Vec3,
) -> (
    [Mat4; NUM_SHADOW_CASCADES],
    [f32; 3],
    f32,
    [f32; NUM_SHADOW_CASCADES],
) {
    let inv_vp = camera.view_projection.inverse();
    let near = camera.near;
    let actual_far = camera.far;

    // Use the configurable shadow distance, clamped only to the camera far plane.
    // The scene AABB still contributes to the Z-range (depth extent) of each
    // cascade's orthographic projection, ensuring shadow casters are captured.
    let shadow_far = camera.shadow_distance.min(actual_far).max(near * 10.0);

    // 1. Extract 8 frustum corners from inv_VP.
    //    Vulkan reverse-Z NDC: near plane at z=1, far plane at z=0.
    let ndc_corners: [Vec4; 8] = [
        // Near plane (z = 1)
        Vec4::new(-1.0, -1.0, 1.0, 1.0),
        Vec4::new(1.0, -1.0, 1.0, 1.0),
        Vec4::new(1.0, 1.0, 1.0, 1.0),
        Vec4::new(-1.0, 1.0, 1.0, 1.0),
        // Far plane (z = 0)
        Vec4::new(-1.0, -1.0, 0.0, 1.0),
        Vec4::new(1.0, -1.0, 0.0, 1.0),
        Vec4::new(1.0, 1.0, 0.0, 1.0),
        Vec4::new(-1.0, 1.0, 0.0, 1.0),
    ];

    let mut world_corners = [Vec3::ZERO; 8];
    for (i, ndc) in ndc_corners.iter().enumerate() {
        let world = inv_vp * *ndc;
        world_corners[i] = world.xyz() / world.w;
    }

    // Near corners = [0..4], Far corners = [4..8].
    // The frustum edges go from near_corner[i] (at camera.near) to
    // far_corner[i] (at camera.far). Lerp parameter t ∈ [0, 1] maps to
    // view-space distance = near + t * (actual_far - near).

    // 2. Compute 3 split distances (PSSM blend) for 4 cascades.
    let num_splits = NUM_SHADOW_CASCADES - 1; // 3
    let mut split_distances = [0.0_f32; 3];
    for (i, dist) in split_distances.iter_mut().enumerate().take(num_splits) {
        let frac = (i + 1) as f32 / NUM_SHADOW_CASCADES as f32;
        let uniform_split = near + (shadow_far - near) * frac;
        let log_split = near * (shadow_far / near).powf(frac);
        *dist = uniform_split * (1.0 - CASCADE_SPLIT_LAMBDA) + log_split * CASCADE_SPLIT_LAMBDA;
    }

    // Convert splits and shadow_far to lerp parameters along the actual frustum
    // edges (which span from camera.near to camera.far, NOT shadow_far).
    let range = actual_far - near;
    let t_shadow_far = (shadow_far - near) / range;
    let t_splits: [f32; 3] = std::array::from_fn(|i| (split_distances[i] - near) / range);

    // 3. Compute split depths in NDC using the camera's actual near/far
    //    (reverse-Z: near=1, far=0), since the shader reads from the camera's
    //    depth buffer which uses the real projection.
    let split_ndcs: [f32; 3] = std::array::from_fn(|i| {
        near * (actual_far - split_distances[i]) / (range * split_distances[i])
    });

    // 4. For each cascade, compute sub-frustum and fit orthographic projection.
    //    Cascade 0 = nearest, Cascade 3 = farthest.
    let mut cascade_ranges = [(0.0_f32, 0.0_f32); NUM_SHADOW_CASCADES];
    cascade_ranges[0] = (0.0, t_splits[0]);
    for i in 1..num_splits {
        cascade_ranges[i] = (t_splits[i - 1], t_splits[i]);
    }
    cascade_ranges[NUM_SHADOW_CASCADES - 1] = (t_splits[num_splits - 1], t_shadow_far);

    let mut cascade_vps = [Mat4::IDENTITY; NUM_SHADOW_CASCADES];
    let mut texel_sizes = [1.0_f32; NUM_SHADOW_CASCADES];

    let light_dir = light_direction.normalize();
    let up = if light_dir.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    // Pre-compute scene AABB corners for light-space projection.
    let scene_aabb_corners: [Vec3; 8] = std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 != 0 { scene_max.x } else { scene_min.x },
            if i & 2 != 0 { scene_max.y } else { scene_min.y },
            if i & 4 != 0 { scene_max.z } else { scene_min.z },
        )
    });

    for (cascade_idx, &(t_near, t_far)) in cascade_ranges.iter().enumerate() {
        // Sub-frustum corners: interpolate between near and far frustum corners.
        let mut sub_corners = [Vec3::ZERO; 8];
        for i in 0..4 {
            sub_corners[i] = world_corners[i].lerp(world_corners[i + 4], t_near);
            sub_corners[i + 4] = world_corners[i].lerp(world_corners[i + 4], t_far);
        }

        // Bounding sphere centered on the sub-frustum centroid.
        // This is much tighter than using camera position (which wastes
        // texels on the half-sphere behind the camera).
        let centroid = sub_corners.iter().copied().sum::<Vec3>() / 8.0;
        let center = centroid;
        let raw_radius = sub_corners
            .iter()
            .map(|c| (*c - center).length())
            .fold(0.0_f32, f32::max);

        // Round radius up to the next texel-aligned increment so the
        // ortho extent stays constant as the camera rotates (prevents
        // shadow shimmer from the sphere growing/shrinking by fractions
        // of a texel).
        let texels_per_unit = DEFAULT_SHADOW_MAP_SIZE as f32 / (raw_radius * 2.0).max(0.001);
        let radius = (raw_radius * texels_per_unit).ceil() / texels_per_unit;

        // Light view matrix — eye behind the cascade center, looking along
        // the light direction.
        let light_view = Mat4::look_at_lh(center - light_dir * MAX_SHADOW_EXTENT, center, up);

        // Z range: start from the scene AABB (captures shadow casters
        // outside the camera frustum), then clamp to avoid a single
        // far-away object stretching all cascades and destroying depth
        // precision.
        let mut z_min = f32::MAX;
        let mut z_max = f32::NEG_INFINITY;

        for &corner in &scene_aabb_corners {
            let ls = light_view.transform_point3(corner);
            z_min = z_min.min(ls.z);
            z_max = z_max.max(ls.z);
        }

        // Sub-frustum Z extents in light space.
        let mut sub_z_min = f32::MAX;
        let mut sub_z_max = f32::NEG_INFINITY;
        for corner in &sub_corners {
            let ls = light_view.transform_point3(*corner);
            sub_z_min = sub_z_min.min(ls.z);
            sub_z_max = sub_z_max.max(ls.z);
            z_min = z_min.min(ls.z);
            z_max = z_max.max(ls.z);
        }

        // Clamp z_max so distant objects don't dilute depth precision.
        // Allow up to 1× the sub-frustum depth range (or 10 units min)
        // beyond the sub-frustum's far extent to capture nearby casters.
        let sub_depth = (sub_z_max - sub_z_min).abs().max(10.0);
        z_max = z_max.min(sub_z_max + sub_depth);
        // Ensure a valid range even when scene AABB is degenerate.
        if z_max - z_min < 1.0 {
            z_min = z_max - 1.0;
        }

        // Build orthographic projection from the bounding sphere XY + scene Z.
        let mut light_proj = Mat4::orthographic_lh(-radius, radius, -radius, radius, z_min, z_max);
        light_proj.y_axis.y *= -1.0; // Vulkan Y-flip

        // Texel snapping: prevent shadow shimmer from sub-texel jitter.
        let shadow_vp = light_proj * light_view;
        let half_texels = DEFAULT_SHADOW_MAP_SIZE as f32 * 0.5;
        let origin_clip = shadow_vp.transform_point3(Vec3::ZERO);
        let tx = origin_clip.x * half_texels;
        let ty = origin_clip.y * half_texels;
        let offset_x = (tx.round() - tx) / half_texels;
        let offset_y = (ty.round() - ty) / half_texels;
        light_proj.w_axis.x += offset_x;
        light_proj.w_axis.y += offset_y;

        cascade_vps[cascade_idx] = light_proj * light_view;

        // World-units-per-texel for this cascade (ortho covers [-radius, radius]).
        texel_sizes[cascade_idx] = (radius * 2.0) / DEFAULT_SHADOW_MAP_SIZE as f32;
    }

    (cascade_vps, split_ndcs, shadow_far, texel_sizes)
}

// ---------------------------------------------------------------------------
// Shadow pipeline creation
// ---------------------------------------------------------------------------

use super::mesh::Mesh;
use super::pipeline::Pipeline;
use super::shader::Shader;

/// Create a depth-only pipeline for the shadow pass.
///
/// Slope-scaled depth bias for angled surfaces. Constant bias is handled
/// entirely on the receiver side (fragment shader) via world-space normal +
/// light-direction offsets. Shadow pancaking in the vertex shader clamps to
/// near plane. No color attachments.
/// Push constants: light VP matrix (64 bytes) + model matrix (64 bytes) = 128 bytes, vertex stage.
/// The light VP is passed via push constants (not UBO) so that per-cascade values
/// are correctly recorded into the command buffer instead of racing on a shared buffer.
///
/// When `front_face_cull` is `true`, front faces are culled instead of back faces.
/// This eliminates self-shadowing acne on front faces at the cost of light leaking
/// on thin single-sided geometry.
pub(crate) fn create_shadow_pipeline(
    device: &ash::Device,
    shader: &Shader,
    render_pass: vk::RenderPass,
    _shadow_camera_ds_layout: vk::DescriptorSetLayout,
    pipeline_cache: vk::PipelineCache,
    front_face_cull: bool,
) -> Result<Pipeline, String> {
    let entry_point = c"main";

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(shader.vert_module())
        .name(entry_point);

    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(shader.frag_module())
        .name(entry_point);

    let shader_stages = [vert_stage, frag_stage];

    let vertex_layout = Mesh::vertex_layout();
    let binding = vertex_layout.vk_binding_description(0);
    let attributes = vertex_layout.vk_attribute_descriptions(0);
    let bindings = [binding];

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    // Slope-scaled depth bias for angled surfaces. All constant bias is
    // handled on the receiver side (fragment shader) via world-space normal
    // + light-direction offsets.
    let cull_mode = if front_face_cull {
        vk::CullModeFlags::FRONT
    } else {
        vk::CullModeFlags::BACK
    };
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(cull_mode)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0)
        .depth_bias_enable(true)
        .depth_bias_constant_factor(0.0)
        .depth_bias_slope_factor(2.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    // No color attachments.
    let color_blending = vk::PipelineColorBlendStateCreateInfo::default();

    // Push constants: light VP (64 bytes) + model matrix (64 bytes) = 128 bytes, vertex stage.
    let push_constant_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: (std::mem::size_of::<[f32; 16]>() * 2) as u32, // 128 bytes
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .push_constant_ranges(std::slice::from_ref(&push_constant_range));
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| format!("Failed to create shadow pipeline layout: {e}"))?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
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
                format!("Failed to create shadow pipeline: {e}")
            })?[0];

    Ok(Pipeline::from_raw(
        pipeline,
        pipeline_layout,
        device.clone(),
    ))
}

/// Create an alpha-tested depth pipeline for the shadow pass.
///
/// Like the opaque variant but includes a fragment shader that samples the
/// albedo texture from the bindless array and discards fragments below the
/// alpha cutoff. Used for foliage, fences, and other alpha-masked geometry.
///
/// Push constants: light VP (64B) + model (64B) + alpha_cutoff (4B) + tex_index (4B) = 136B.
/// Descriptor set: set 1 = bindless texture array (same as main pass).
pub(crate) fn create_shadow_alpha_pipeline(
    device: &ash::Device,
    shader: &Shader,
    render_pass: vk::RenderPass,
    bindless_ds_layout: vk::DescriptorSetLayout,
    pipeline_cache: vk::PipelineCache,
) -> Result<Pipeline, String> {
    let entry_point = c"main";

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(shader.vert_module())
        .name(entry_point);

    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(shader.frag_module())
        .name(entry_point);

    let shader_stages = [vert_stage, frag_stage];

    let vertex_layout = Mesh::vertex_layout();
    let binding = vertex_layout.vk_binding_description(0);
    let attributes = vertex_layout.vk_attribute_descriptions(0);
    let bindings = [binding];

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);

    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    // Back-face culling + slope-scaled depth bias (same as opaque variant).
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::BACK)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0)
        .depth_bias_enable(true)
        .depth_bias_constant_factor(0.0)
        .depth_bias_slope_factor(2.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blending = vk::PipelineColorBlendStateCreateInfo::default();

    // Push constants: light VP (64) + model (64) + alpha_cutoff (4) + tex_index (4) = 136 bytes.
    // Both vertex and fragment stages read push constants.
    let push_constant_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: 136,
    };

    // Descriptor set layout: set 1 = bindless texture array (for alpha sampling).
    // Set 0 is unused in this pipeline (the opaque shadow pass used UBO here,
    // but this pipeline only needs textures).
    let ds_layouts = [bindless_ds_layout];

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&ds_layouts)
        .push_constant_ranges(std::slice::from_ref(&push_constant_range));
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| format!("Failed to create shadow alpha pipeline layout: {e}"))?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
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
                format!("Failed to create shadow alpha pipeline: {e}")
            })?[0];

    Ok(Pipeline::from_raw(
        pipeline,
        pipeline_layout,
        device.clone(),
    ))
}
