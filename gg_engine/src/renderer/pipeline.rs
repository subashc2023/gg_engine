use ash::vk;

use super::shader::Shader;
use super::vertex_array::VertexArray;

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
pub(crate) fn create_pipeline(
    device: &ash::Device,
    shader: &Shader,
    va: &VertexArray,
    render_pass: vk::RenderPass,
    has_material_color: bool,
) -> Pipeline {
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

    let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(false);

    let blend_attachments = [color_blend_attachment];
    let color_blending =
        vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

    // Push constant range: VP matrix + transform matrix (2 × mat4 = 128 bytes).
    let vertex_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX,
        offset: 0,
        size: (std::mem::size_of::<[f32; 16]>() * 2) as u32,
    };

    // Optional: material color (vec4 = 16 bytes at offset 128, fragment stage).
    let fragment_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::FRAGMENT,
        offset: 128,
        size: std::mem::size_of::<[f32; 4]>() as u32,
    };

    let ranges_with_color = [vertex_range, fragment_range];
    let ranges_without = [vertex_range];
    let push_constant_ranges: &[vk::PushConstantRange] = if has_material_color {
        &ranges_with_color
    } else {
        &ranges_without
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .push_constant_ranges(push_constant_ranges);
    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .expect("Failed to create pipeline layout");

    let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&shader_stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisampling)
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
