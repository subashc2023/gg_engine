use ash::vk;

use super::vertex_array::VertexArray;
use super::{
    BufferElement, BufferLayout, IndexBuffer, Shader, ShaderDataType, VertexBuffer, VulkanContext,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Cast a typed slice to raw bytes for uploading into a vertex buffer.
fn as_bytes<T: Copy>(data: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

/// Create a Vulkan graphics pipeline + layout from a shader and vertex array.
fn create_pipeline(
    device: &ash::Device,
    shader: &Shader,
    va: &VertexArray,
    render_pass: vk::RenderPass,
) -> (vk::PipelineLayout, vk::Pipeline) {
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

    let layout_info = vk::PipelineLayoutCreateInfo::default();
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

    (pipeline_layout, pipeline)
}

// ---------------------------------------------------------------------------
// Triangle vertex data (position + color)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct TriangleVertex {
    position: [f32; 3],
    color: [f32; 4],
}

// Y-flipped for Vulkan NDC (Y+ is downward).
const TRIANGLE_VERTICES: [TriangleVertex; 3] = [
    TriangleVertex { position: [-0.5, 0.5, 0.0], color: [0.8, 0.2, 0.3, 1.0] },  // bottom-left  (red-ish)
    TriangleVertex { position: [0.5, 0.5, 0.0],  color: [0.2, 0.3, 0.8, 1.0] },  // bottom-right (blue-ish)
    TriangleVertex { position: [0.0, -0.5, 0.0], color: [0.8, 0.8, 0.2, 1.0] },  // top-center   (yellow-ish)
];

const TRIANGLE_INDICES: [u32; 3] = [0, 1, 2];

// ---------------------------------------------------------------------------
// Square vertex data (position only — uses flat color shader)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct SquareVertex {
    position: [f32; 3],
}

const SQUARE_VERTICES: [SquareVertex; 4] = [
    SquareVertex { position: [-0.75,  0.75, 0.0] }, // bottom-left
    SquareVertex { position: [ 0.75,  0.75, 0.0] }, // bottom-right
    SquareVertex { position: [ 0.75, -0.75, 0.0] }, // top-right
    SquareVertex { position: [-0.75, -0.75, 0.0] }, // top-left
];

const SQUARE_INDICES: [u32; 6] = [0, 1, 2, 2, 3, 0];

// ---------------------------------------------------------------------------
// TriangleRenderer
// ---------------------------------------------------------------------------

pub(crate) struct TriangleRenderer {
    // Square (drawn first, behind the triangle).
    _square_shader: Shader,
    square_pipeline_layout: vk::PipelineLayout,
    square_pipeline: vk::Pipeline,
    square_va: VertexArray,

    // Triangle (drawn on top).
    _triangle_shader: Shader,
    triangle_pipeline_layout: vk::PipelineLayout,
    triangle_pipeline: vk::Pipeline,
    triangle_va: VertexArray,

    device: ash::Device,
}

impl TriangleRenderer {
    pub fn new(vk_ctx: &VulkanContext, render_pass: vk::RenderPass) -> Self {
        let device = vk_ctx.device();

        // ==== Square (flat blue) ============================================
        let square_shader = Shader::new(
            device,
            "flat_color",
            include_bytes!("shaders/flat_color_vert.spv"),
            include_bytes!("shaders/flat_color_frag.spv"),
        );

        let mut square_vb = VertexBuffer::new(vk_ctx, as_bytes(&SQUARE_VERTICES));
        square_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
        ]));

        let square_ib = IndexBuffer::new(vk_ctx, &SQUARE_INDICES);

        let mut square_va = VertexArray::new(device);
        square_va.add_vertex_buffer(square_vb);
        square_va.set_index_buffer(square_ib);

        let (square_pipeline_layout, square_pipeline) =
            create_pipeline(device, &square_shader, &square_va, render_pass);

        // ==== Triangle (vertex colors) ======================================
        let triangle_shader = Shader::new(
            device,
            "triangle",
            include_bytes!("shaders/triangle_vert.spv"),
            include_bytes!("shaders/triangle_frag.spv"),
        );

        let mut triangle_vb = VertexBuffer::new(vk_ctx, as_bytes(&TRIANGLE_VERTICES));
        triangle_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float4, "a_color"),
        ]));

        let triangle_ib = IndexBuffer::new(vk_ctx, &TRIANGLE_INDICES);

        let mut triangle_va = VertexArray::new(device);
        triangle_va.add_vertex_buffer(triangle_vb);
        triangle_va.set_index_buffer(triangle_ib);

        let (triangle_pipeline_layout, triangle_pipeline) =
            create_pipeline(device, &triangle_shader, &triangle_va, render_pass);

        log::info!(target: "gg_engine", "Triangle renderer initialized");

        Self {
            _square_shader: square_shader,
            square_pipeline_layout,
            square_pipeline,
            square_va,

            _triangle_shader: triangle_shader,
            triangle_pipeline_layout,
            triangle_pipeline,
            triangle_va,

            device: device.clone(),
        }
    }

    /// Record draw commands into an already-begun render pass.
    pub fn cmd_draw(
        &self,
        device: &ash::Device,
        cmd_buf: vk::CommandBuffer,
        extent: vk::Extent2D,
    ) {
        unsafe {
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width: extent.width as f32,
                height: extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            device.cmd_set_viewport(cmd_buf, 0, &[viewport]);

            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            };
            device.cmd_set_scissor(cmd_buf, 0, &[scissor]);

            // Draw square (background).
            device.cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, self.square_pipeline);
            self.square_va.bind(cmd_buf);
            let square_count = self.square_va.index_buffer().unwrap().count();
            device.cmd_draw_indexed(cmd_buf, square_count, 1, 0, 0, 0);

            // Draw triangle (foreground).
            device.cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, self.triangle_pipeline);
            self.triangle_va.bind(cmd_buf);
            let triangle_count = self.triangle_va.index_buffer().unwrap().count();
            device.cmd_draw_indexed(cmd_buf, triangle_count, 1, 0, 0, 0);
        }
    }
}

impl Drop for TriangleRenderer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.triangle_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.triangle_pipeline_layout, None);
            self.device.destroy_pipeline(self.square_pipeline, None);
            self.device
                .destroy_pipeline_layout(self.square_pipeline_layout, None);
        }
    }
}
