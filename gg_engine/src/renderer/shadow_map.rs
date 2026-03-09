use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Vec3};

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::uniform_buffer::UniformBuffer;
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default shadow map resolution (width and height in texels).
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
    // Depth image + view
    depth_image: vk::Image,
    #[allow(dead_code)] // Owned for memory lifetime; freed on drop.
    depth_allocation: GpuAllocation,
    depth_view: vk::ImageView,

    // Comparison sampler for hardware PCF
    sampler: vk::Sampler,

    // Depth-only render pass + framebuffer
    render_pass: vk::RenderPass,
    framebuffer: vk::Framebuffer,

    // Resolution
    width: u32,
    height: u32,

    // Light VP UBO (one per frame × viewport slot) — used by the shadow
    // pass vertex shader (set 0) to transform geometry into light space.
    light_vp_ubo: UniformBuffer,

    // Descriptor set layout for the main pass (set 4):
    //   binding 0 = combined image sampler (shadow map depth texture)
    shadow_ds_layout: vk::DescriptorSetLayout,
    shadow_descriptor_sets: Vec<vk::DescriptorSet>,

    // Descriptor set layout for the shadow pass itself (set 0):
    //   binding 0 = UBO (light VP matrix)
    shadow_camera_ds_layout: vk::DescriptorSetLayout,
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
        // --- Depth image ---
        let (depth_image, depth_allocation, depth_view) =
            Self::create_depth_resources(allocator, device, depth_format, width, height)?;

        // Transition depth image from UNDEFINED to DEPTH_STENCIL_READ_ONLY_OPTIMAL
        // so it's valid for sampling in the main pass even before any shadow pass runs.
        Self::transition_depth_initial(device, command_pool, graphics_queue, depth_image)?;

        // --- Comparison sampler (for hardware PCF via sampler2DShadow) ---
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

        // --- Depth-only render pass ---
        let render_pass = Self::create_render_pass(device, depth_format)?;

        // --- Framebuffer ---
        let framebuffer = Self::create_framebuffer(device, render_pass, depth_view, width, height)?;

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
        //     binding 0: combined image sampler (shadow depth texture), fragment stage
        let shadow_sampler_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let shadow_ds_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&shadow_sampler_binding));
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

        // Write shadow map image to each descriptor set
        for &ds in &shadow_descriptor_sets {
            let image_info = vk::DescriptorImageInfo::default()
                .sampler(sampler)
                .image_view(depth_view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL);
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .image_info(std::slice::from_ref(&image_info));
            unsafe {
                device.update_descriptor_sets(&[write], &[]);
            }
        }

        Ok(Self {
            depth_image,
            depth_allocation,
            depth_view,
            sampler,
            render_pass,
            framebuffer,
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

    /// The shadow framebuffer.
    pub fn framebuffer(&self) -> vk::Framebuffer {
        self.framebuffer
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
    ) -> Result<(vk::Image, GpuAllocation, vk::ImageView), String> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(depth_format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
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

        let depth_view = unsafe { device.create_image_view(&view_info, None) }
            .map_err(|e| format!("Failed to create shadow depth image view: {e}"))?;

        Ok((depth_image, depth_allocation, depth_view))
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
                    layer_count: 1,
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
            self.device.destroy_framebuffer(self.framebuffer, None);
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_sampler(self.sampler, None);
            self.device.destroy_image_view(self.depth_view, None);
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
// Shadow pipeline creation
// ---------------------------------------------------------------------------

use super::mesh::Mesh;
use super::pipeline::Pipeline;
use super::shader::Shader;

/// Create a depth-only pipeline for the shadow pass.
///
/// Front-face culling to reduce peter-panning. No color attachments.
/// Push constant: model matrix (64 bytes, vertex stage).
pub(crate) fn create_shadow_pipeline(
    device: &ash::Device,
    shader: &Shader,
    render_pass: vk::RenderPass,
    shadow_camera_ds_layout: vk::DescriptorSetLayout,
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

    // Front-face culling to reduce peter-panning artifact.
    // Depth bias to prevent shadow acne (self-shadowing artifacts).
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::FRONT)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0)
        .depth_bias_enable(true)
        .depth_bias_constant_factor(2.0)
        .depth_bias_slope_factor(3.0);

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

    // Push constant: model matrix (mat4 = 64 bytes), vertex stage.
    let push_constant_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: std::mem::size_of::<[f32; 16]>() as u32,
    };

    let set_layouts = [shadow_camera_ds_layout];
    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&set_layouts)
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
