use ash::vk;

use super::buffer::BufferLayout;
use super::shader::Shader;
use super::vertex_array::VertexArray;

use crate::profiling::ProfileTimer;

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
// Pipeline creation
// ---------------------------------------------------------------------------

/// Create a Vulkan graphics pipeline + layout from a shader and vertex array.
///
/// Uses sensible defaults: triangle list topology, no culling, fill mode,
/// dynamic viewport/scissor, and a push constant range for the VP matrix.
///
/// When `has_material_color` is true, an additional push constant range is
/// added for a `vec4` color at offset 128 (fragment stage, 16 bytes).
///
/// `descriptor_set_layouts` is passed to the pipeline layout (e.g. for texture
/// samplers). `blend_enable` enables standard alpha blending.
pub(crate) fn create_pipeline(
    device: &ash::Device,
    shader: &Shader,
    va: &VertexArray,
    render_pass: vk::RenderPass,
    has_material_color: bool,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    blend_enable: bool,
) -> Pipeline {
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

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
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

    // Push constant range: VP matrix + transform matrix (2 × mat4 = 128 bytes).
    let vertex_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: (std::mem::size_of::<[f32; 16]>() * 2) as u32,
    };

    // Optional: material color + tiling factor (vec4 + float = 20 bytes at offset 128, fragment stage).
    let fragment_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        offset: 128,
        size: (std::mem::size_of::<[f32; 4]>() + std::mem::size_of::<f32>()) as u32,
    };

    let ranges_with_color = [vertex_range, fragment_range];
    let ranges_without = [vertex_range];
    let push_constant_ranges: &[vk::PushConstantRange] = if has_material_color {
        &ranges_with_color
    } else {
        &ranges_without
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .push_constant_ranges(push_constant_ranges)
        .set_layouts(descriptor_set_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .expect("Failed to create pipeline layout");

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

    let pipeline = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    }
    .expect("Failed to create graphics pipeline")[0];

    Pipeline {
        pipeline,
        layout: pipeline_layout,
        device: device.clone(),
    }
}

/// Create a Vulkan graphics pipeline for batch rendering.
///
/// Uses a `BufferLayout` directly instead of a `VertexArray` for vertex input.
/// Push constant: VP matrix only (64 bytes, vertex stage).
/// Descriptor set layout: sampler array (16 combined image samplers).
pub(crate) fn create_batch_pipeline(
    device: &ash::Device,
    shader: &Shader,
    vertex_layout: &BufferLayout,
    render_pass: vk::RenderPass,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    color_attachment_count: u32,
) -> Pipeline {
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

    let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0);

    let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL)
        .depth_bounds_test_enable(false)
        .stencil_test_enable(false);

    // Attachment 0: standard alpha blending (RGBA).
    // Attachments 1+: blend disabled, R write mask only (integer formats).
    let mut blend_attachments = Vec::with_capacity(color_attachment_count as usize);
    blend_attachments.push(
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
    for _ in 1..color_attachment_count {
        blend_attachments.push(
            vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::R)
                .blend_enable(false),
        );
    }

    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    // Push constant: VP matrix only (1 × mat4 = 64 bytes, vertex stage).
    let vertex_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: std::mem::size_of::<[f32; 16]>() as u32,
    };

    let push_constant_ranges = [vertex_range];

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .push_constant_ranges(&push_constant_ranges)
        .set_layouts(descriptor_set_layouts);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .expect("Failed to create batch pipeline layout");

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

    let pipeline = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
    }
    .expect("Failed to create batch graphics pipeline")[0];

    Pipeline {
        pipeline,
        layout: pipeline_layout,
        device: device.clone(),
    }
}
