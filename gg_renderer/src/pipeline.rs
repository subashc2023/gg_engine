use ash::vk;

use super::buffer::BufferLayout;
use super::material::BlendMode;
use super::shader::Shader;
use super::vertex_array::VertexArray;

use gg_core::error::{EngineError, EngineResult};
use gg_core::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// CullMode — backface culling configuration
// ---------------------------------------------------------------------------

/// Which faces to cull during rasterization.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CullMode {
    /// No culling — both faces are rendered (2D default).
    #[default]
    None,
    /// Cull back faces (standard for closed 3D meshes).
    Back,
    /// Cull front faces.
    Front,
}

impl CullMode {
    fn to_vk(self) -> vk::CullModeFlags {
        match self {
            Self::None => vk::CullModeFlags::NONE,
            Self::Back => vk::CullModeFlags::BACK,
            Self::Front => vk::CullModeFlags::FRONT,
        }
    }
}

// ---------------------------------------------------------------------------
// DepthConfig — depth test/write configuration
// ---------------------------------------------------------------------------

/// Depth buffer test and write configuration for pipeline creation.
#[derive(Debug, Clone, Copy)]
pub struct DepthConfig {
    /// Whether to test fragments against the depth buffer.
    pub test: bool,
    /// Whether to write fragment depth to the depth buffer.
    pub write: bool,
    /// Comparison operator for depth test (default: LESS_OR_EQUAL).
    pub compare_op: vk::CompareOp,
}

impl Default for DepthConfig {
    fn default() -> Self {
        Self {
            test: true,
            write: true,
            compare_op: vk::CompareOp::GREATER_OR_EQUAL,
        }
    }
}

impl DepthConfig {
    /// Depth testing disabled (2D painter's algorithm).
    pub const DISABLED: Self = Self {
        test: false,
        write: false,
        compare_op: vk::CompareOp::GREATER_OR_EQUAL,
    };

    /// Standard 3D depth: test + write with GREATER_OR_EQUAL (reverse-Z).
    pub const STANDARD_3D: Self = Self {
        test: true,
        write: true,
        compare_op: vk::CompareOp::GREATER_OR_EQUAL,
    };

    /// Read-only depth: test but no write (e.g. transparent 3D objects).
    pub const READ_ONLY: Self = Self {
        test: true,
        write: false,
        compare_op: vk::CompareOp::GREATER_OR_EQUAL,
    };
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Wraps a Vulkan graphics pipeline and its layout.
///
/// Created via [`Renderer::create_pipeline`](super::Renderer::create_pipeline).
/// Destroyed automatically on drop.
pub struct Pipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    device: ash::Device,
}

impl Pipeline {
    /// Create a `Pipeline` from raw Vulkan handles. Ownership is transferred;
    /// the pipeline and layout will be destroyed on drop.
    pub fn from_raw(
        pipeline: vk::Pipeline,
        layout: vk::PipelineLayout,
        device: ash::Device,
    ) -> Self {
        Self {
            pipeline,
            layout,
            device,
        }
    }

    pub fn pipeline(&self) -> vk::Pipeline {
        self.pipeline
    }

    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared pipeline helpers
// ---------------------------------------------------------------------------

/// Standard rasterizer state: fill mode, no culling, CCW front face.
fn default_rasterizer() -> vk::PipelineRasterizationStateCreateInfo<'static> {
    rasterizer(CullMode::None, false, false)
}

/// Rasterizer state with configurable face culling and optional wireframe mode.
///
/// When `wireframe` is true, polygon mode is `LINE` and culling is disabled
/// so all edges are visible. No depth bias is applied — slope-based bias
/// causes steep triangles to incorrectly occlude front-facing wireframe edges.
fn rasterizer(
    cull_mode: CullMode,
    wireframe: bool,
    clockwise_front_face: bool,
) -> vk::PipelineRasterizationStateCreateInfo<'static> {
    let front_face = if clockwise_front_face {
        vk::FrontFace::CLOCKWISE
    } else {
        vk::FrontFace::COUNTER_CLOCKWISE
    };
    let info = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(if wireframe {
            vk::PolygonMode::LINE
        } else {
            vk::PolygonMode::FILL
        })
        .cull_mode(if wireframe {
            vk::CullModeFlags::NONE
        } else {
            cull_mode.to_vk()
        })
        .front_face(front_face)
        .line_width(1.0);
    info
}

/// Standard multisampling state for the given sample count.
fn default_multisampling(
    samples: vk::SampleCountFlags,
) -> vk::PipelineMultisampleStateCreateInfo<'static> {
    vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(samples)
}

/// Build color blend attachment states for batch pipelines.
///
/// Attachment 0: standard alpha blending (RGBA).
/// Attachments 1+: no blending, R channel only (integer entity ID attachment).
fn batch_blend_attachments(
    color_attachment_count: u32,
) -> Vec<vk::PipelineColorBlendAttachmentState> {
    let mut attachments = Vec::with_capacity(color_attachment_count as usize);
    attachments.push(
        vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD),
    );
    if color_attachment_count > 1 {
        // Attachment 1: entity ID — R channel only, no blending.
        attachments.push(
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::R)
                .blend_enable(false),
        );
    }
    for _ in 2..color_attachment_count {
        // Attachment 2+: normal map etc. — RGBA, no blending.
        attachments.push(
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false),
        );
    }
    attachments
}

/// Prepend camera UBO layout (set 0) before caller-provided descriptor set layouts.
fn prepare_descriptor_layouts(
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    extra_layouts: &[vk::DescriptorSetLayout],
) -> Vec<vk::DescriptorSetLayout> {
    let mut all = Vec::with_capacity(1 + extra_layouts.len());
    all.push(camera_ubo_ds_layout);
    all.extend_from_slice(extra_layouts);
    all
}

/// Create the Vulkan pipeline + wrap in [`Pipeline`]. Cleans up layout on failure.
fn create_and_wrap_pipeline(
    device: &ash::Device,
    pipeline_info: &vk::GraphicsPipelineCreateInfo<'_>,
    pipeline_cache: vk::PipelineCache,
    pipeline_layout: vk::PipelineLayout,
) -> EngineResult<Pipeline> {
    let pipeline =
        unsafe { device.create_graphics_pipelines(pipeline_cache, &[*pipeline_info], None) }
            .map_err(|(_, e)| {
                unsafe {
                    device.destroy_pipeline_layout(pipeline_layout, None);
                }
                EngineError::Gpu(format!("Failed to create graphics pipeline: {e}"))
            })?[0];

    Ok(Pipeline {
        pipeline,
        layout: pipeline_layout,
        device: device.clone(),
    })
}

// ---------------------------------------------------------------------------
// Pipeline creation
// ---------------------------------------------------------------------------

/// Create a Vulkan graphics pipeline + layout from a shader and vertex array.
///
/// Uses sensible defaults: triangle list topology, no culling, fill mode,
/// dynamic viewport/scissor, and a push constant range for the VP matrix.
///
/// When `has_material_color` is true, an additional push constant range is
/// added for a `vec4` color at offset 64 (fragment stage, 16 bytes).
///
/// `camera_ubo_ds_layout` is prepended as set 0 (camera UBO).
/// `descriptor_set_layouts` follows as set 1+ (e.g. for texture samplers).
/// `blend_enable` enables standard alpha blending.
#[allow(clippy::too_many_arguments)]
pub fn create_pipeline(
    device: &ash::Device,
    shader: &Shader,
    va: &VertexArray,
    render_pass: vk::RenderPass,
    has_material_color: bool,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    blend_enable: bool,
    pipeline_cache: vk::PipelineCache,
    samples: vk::SampleCountFlags,
    wireframe: bool,
) -> EngineResult<Pipeline> {
    let _timer = ProfileTimer::new("Pipeline::create");
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

    // Vertex input — driven by the VertexArray's buffer layouts.
    let bindings = va.vk_binding_descriptions();
    let attributes = va.vk_attribute_descriptions();

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);

    // Dynamic viewport/scissor (survives swapchain recreation).
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = rasterizer(CullMode::None, wireframe, false);
    let multisampling = default_multisampling(samples);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let color_blend_attachment = if blend_enable {
        vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD)
    } else {
        vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false)
    };

    let blend_attachments = [color_blend_attachment];
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    // Push constant range: transform matrix only (1 × mat4 = 64 bytes).
    let vertex_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: std::mem::size_of::<[f32; 16]>() as u32,
    };

    // Optional: material color + tiling factor (vec4 + float = 20 bytes at offset 64, fragment stage).
    let fragment_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        offset: 64,
        size: (std::mem::size_of::<[f32; 4]>() + std::mem::size_of::<f32>()) as u32,
    };

    let ranges_with_color = [vertex_range, fragment_range];
    let ranges_without = [vertex_range];
    let push_constant_ranges: &[vk::PushConstantRange] = if has_material_color {
        &ranges_with_color
    } else {
        &ranges_without
    };

    let all_layouts = prepare_descriptor_layouts(camera_ubo_ds_layout, descriptor_set_layouts);

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .push_constant_ranges(push_constant_ranges)
        .set_layouts(&all_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create pipeline layout: {e}")))?;

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

    create_and_wrap_pipeline(device, &pipeline_info, pipeline_cache, pipeline_layout)
}

/// Create a Vulkan graphics pipeline for batch rendering.
///
/// Uses a `BufferLayout` directly instead of a `VertexArray` for vertex input.
/// Push constant: VP matrix only (64 bytes, vertex stage).
/// Descriptor set layout: sampler array (16 combined image samplers).
#[allow(clippy::too_many_arguments)]
pub fn create_batch_pipeline(
    device: &ash::Device,
    shader: &Shader,
    vertex_layout: &BufferLayout,
    render_pass: vk::RenderPass,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    color_attachment_count: u32,
    pipeline_cache: vk::PipelineCache,
    samples: vk::SampleCountFlags,
    wireframe: bool,
) -> EngineResult<Pipeline> {
    let _timer = ProfileTimer::new("Pipeline::create_batch");
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

    let rast = rasterizer(CullMode::None, wireframe, false);
    let multisampling = default_multisampling(samples);

    // 2D batch rendering uses painter's algorithm (draw order); no depth test needed.
    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(false)
        .depth_write_enable(false)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let blend_attachments = batch_blend_attachments(color_attachment_count);
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    let all_layouts = prepare_descriptor_layouts(camera_ubo_ds_layout, descriptor_set_layouts);

    let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&all_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create batch pipeline layout: {e}")))?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rast)
        .multisample_state(&multisampling)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    create_and_wrap_pipeline(device, &pipeline_info, pipeline_cache, pipeline_layout)
}

/// Create a Vulkan graphics pipeline for **instanced** sprite rendering.
///
/// Two vertex bindings:
/// - Binding 0 (per-vertex, rate VERTEX): static unit quad geometry
/// - Binding 1 (per-instance, rate INSTANCE): per-sprite instance data
///
/// Uses the same descriptor set layout as the batch pipeline (camera UBO at
/// set 0, bindless textures at set 1). No push constants.
#[allow(clippy::too_many_arguments)]
pub fn create_instanced_batch_pipeline(
    device: &ash::Device,
    shader: &Shader,
    vertex_layout: &BufferLayout,
    instance_layout: &BufferLayout,
    render_pass: vk::RenderPass,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    color_attachment_count: u32,
    pipeline_cache: vk::PipelineCache,
    samples: vk::SampleCountFlags,
    wireframe: bool,
) -> EngineResult<Pipeline> {
    let _timer = ProfileTimer::new("Pipeline::create_instanced_batch");
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

    // Binding 0: per-vertex (unit quad), Binding 1: per-instance (sprite data).
    let vertex_binding = vertex_layout.vk_binding_description(0);
    let instance_binding = instance_layout.vk_binding_description_instanced(1);
    let bindings = [vertex_binding, instance_binding];

    let vertex_location_count = vertex_layout.elements().len() as u32;
    let mut attributes = vertex_layout.vk_attribute_descriptions(0);
    attributes.extend(instance_layout.vk_attribute_descriptions_at(1, vertex_location_count));

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

    let rast = rasterizer(CullMode::None, wireframe, false);
    let multisampling = default_multisampling(samples);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(false)
        .depth_write_enable(false)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let blend_attachments = batch_blend_attachments(color_attachment_count);
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    let all_layouts = prepare_descriptor_layouts(camera_ubo_ds_layout, descriptor_set_layouts);

    let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&all_layouts);
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|e| {
            EngineError::Gpu(format!(
                "Failed to create instanced batch pipeline layout: {e}"
            ))
        })?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rast)
        .multisample_state(&multisampling)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    create_and_wrap_pipeline(device, &pipeline_info, pipeline_cache, pipeline_layout)
}

/// Create a Vulkan graphics pipeline for batch **line** rendering.
///
/// Like [`create_batch_pipeline`] but uses `LINE_LIST` topology and adds
/// `LINE_WIDTH` as a dynamic state so callers can set it per-draw via
/// `vkCmdSetLineWidth`. No index buffer is needed — lines are drawn with
/// `vkCmdDraw` (2 vertices per line segment).
#[allow(clippy::too_many_arguments)]
pub fn create_line_batch_pipeline(
    device: &ash::Device,
    shader: &Shader,
    vertex_layout: &BufferLayout,
    render_pass: vk::RenderPass,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    color_attachment_count: u32,
    pipeline_cache: vk::PipelineCache,
    samples: vk::SampleCountFlags,
) -> EngineResult<Pipeline> {
    let _timer = ProfileTimer::new("Pipeline::create_line_batch");
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

    let binding = vertex_layout.vk_binding_description(0);
    let attributes = vertex_layout.vk_attribute_descriptions(0);
    let bindings = [binding];

    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&bindings)
        .vertex_attribute_descriptions(&attributes);

    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::LINE_LIST)
        .primitive_restart_enable(false);

    // Dynamic viewport/scissor + line width.
    let dynamic_states = [
        vk::DynamicState::VIEWPORT,
        vk::DynamicState::SCISSOR,
        vk::DynamicState::LINE_WIDTH,
    ];
    let dynamic_state =
        vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

    let viewport_state = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);

    let rasterizer = default_rasterizer();
    let multisampling = default_multisampling(samples);

    // 2D batch rendering uses painter's algorithm (draw order); no depth test needed.
    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(false)
        .depth_write_enable(false)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    let blend_attachments = batch_blend_attachments(color_attachment_count);
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    // No push constants — VP is in UBO (set 0), positions are baked into vertices.
    let all_layouts = [camera_ubo_ds_layout];

    let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&all_layouts);
    let pipeline_layout =
        unsafe { device.create_pipeline_layout(&layout_info, None) }.map_err(|e| {
            EngineError::Gpu(format!("Failed to create line batch pipeline layout: {e}"))
        })?;

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

    create_and_wrap_pipeline(device, &pipeline_info, pipeline_cache, pipeline_layout)
}

// ---------------------------------------------------------------------------
// 3D pipeline creation
// ---------------------------------------------------------------------------

/// Convert a [`BlendMode`] to a Vulkan color blend attachment state.
fn blend_mode_attachment(blend_mode: BlendMode) -> vk::PipelineColorBlendAttachmentState {
    match blend_mode {
        BlendMode::Opaque => vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false),
        BlendMode::AlphaBlend => vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD),
        BlendMode::Additive => vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
            .alpha_blend_op(vk::BlendOp::ADD),
    }
}

/// Create a Vulkan graphics pipeline for **3D mesh** rendering.
///
/// Supports configurable face culling, depth testing, and blend modes.
/// Pipeline layout: set 0 = camera UBO, then `extra_descriptor_set_layouts`
/// in order (typically set 1 = bindless textures, set 2 = material UBO).
/// Push constant: model matrix (64 bytes, vertex stage, offset 0).
#[allow(clippy::too_many_arguments)]
pub fn create_3d_pipeline(
    device: &ash::Device,
    shader: &Shader,
    vertex_layout: &BufferLayout,
    render_pass: vk::RenderPass,
    camera_ubo_ds_layout: vk::DescriptorSetLayout,
    extra_descriptor_set_layouts: &[vk::DescriptorSetLayout],
    cull_mode: CullMode,
    depth_config: DepthConfig,
    blend_mode: BlendMode,
    color_attachment_count: u32,
    pipeline_cache: vk::PipelineCache,
    samples: vk::SampleCountFlags,
    wireframe: bool,
    clockwise_front_face: bool,
) -> EngineResult<Pipeline> {
    let _timer = ProfileTimer::new("Pipeline::create_3d");
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

    let rast = rasterizer(cull_mode, wireframe, clockwise_front_face);
    let multisampling = default_multisampling(samples);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(depth_config.test)
        .depth_write_enable(depth_config.write)
        .depth_compare_op(depth_config.compare_op)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    // Build blend attachments: first uses the material's blend mode,
    // attachment 1 = entity ID (R-only), attachment 2+ = normals etc. (RGBA).
    let mut blend_attachments = Vec::with_capacity(color_attachment_count as usize);
    blend_attachments.push(blend_mode_attachment(blend_mode));
    if color_attachment_count > 1 {
        blend_attachments.push(
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::R)
                .blend_enable(false),
        );
    }
    for _ in 2..color_attachment_count {
        blend_attachments.push(
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA)
                .blend_enable(false),
        );
    }
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    let all_layouts =
        prepare_descriptor_layouts(camera_ubo_ds_layout, extra_descriptor_set_layouts);

    // Push constants: both stages declare the same block in SPIR-V.
    // Static mesh: 168 bytes. Skinned mesh: 172 bytes (+ bone_offset u32).
    // Size is inferred from the extra descriptor set layouts:
    //   5 extras (sets 1-5 = bindless + material + lighting + shadow + bone) → 172
    //   4 extras (sets 1-4 = bindless + material + lighting + shadow)        → 168
    let push_size: u32 = if extra_descriptor_set_layouts.len() >= 5 {
        172
    } else {
        168
    };
    let push_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: push_size,
    };
    let push_ranges = [push_range];

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(&all_layouts)
        .push_constant_ranges(&push_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| EngineError::Gpu(format!("Failed to create 3D pipeline layout: {e}")))?;

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rast)
        .multisample_state(&multisampling)
        .depth_stencil_state(&depth_stencil)
        .color_blend_state(&color_blending)
        .dynamic_state(&dynamic_state)
        .layout(pipeline_layout)
        .render_pass(render_pass)
        .subpass(0);

    create_and_wrap_pipeline(device, &pipeline_info, pipeline_cache, pipeline_layout)
}
