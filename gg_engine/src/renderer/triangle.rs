use ash::vk;

use super::{IndexBuffer, Shader, VertexBuffer, VulkanContext};

// ---------------------------------------------------------------------------
// Vertex layout
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
}

// Cherno's triangle positions, Y-flipped for Vulkan NDC (Y+ is downward).
const VERTICES: [Vertex; 3] = [
    Vertex { position: [-0.5, 0.5, 0.0] },  // bottom-left
    Vertex { position: [0.5, 0.5, 0.0] },   // bottom-right
    Vertex { position: [0.0, -0.5, 0.0] },  // top-center
];

const INDICES: [u32; 3] = [0, 1, 2];

// ---------------------------------------------------------------------------
// TriangleRenderer
// ---------------------------------------------------------------------------

pub(crate) struct TriangleRenderer {
    _shader: Shader,
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    vertex_buffer: VertexBuffer,
    index_buffer: IndexBuffer,
    device: ash::Device,
}

impl TriangleRenderer {
    pub fn new(vk_ctx: &VulkanContext, render_pass: vk::RenderPass) -> Self {
        let device = vk_ctx.device();

        // -- Shader ----------------------------------------------------------
        let shader = Shader::new(
            device,
            "triangle",
            include_bytes!("shaders/triangle_vert.spv"),
            include_bytes!("shaders/triangle_frag.spv"),
        );

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

        // -- Vertex input ----------------------------------------------------
        let binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        let attribute = vk::VertexInputAttributeDescription::default()
            .location(0)
            .binding(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0);

        let bindings = [binding];
        let attributes = [attribute];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        // -- Dynamic viewport/scissor (survives swapchain recreation) --------
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        // -- Rasterizer ------------------------------------------------------
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);

        // -- Multisampling ---------------------------------------------------
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        // -- Color blending --------------------------------------------------
        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let attachments = [color_blend_attachment];
        let color_blending =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);

        // -- Pipeline layout (empty) -----------------------------------------
        let layout_info = vk::PipelineLayoutCreateInfo::default();
        let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
            .expect("Failed to create pipeline layout");

        // -- Graphics pipeline -----------------------------------------------
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

        // -- Vertex buffer ---------------------------------------------------
        let vertex_data: &[u8] = unsafe {
            std::slice::from_raw_parts(
                VERTICES.as_ptr() as *const u8,
                std::mem::size_of_val(&VERTICES),
            )
        };
        let vertex_buffer = VertexBuffer::new(vk_ctx, vertex_data);

        // -- Index buffer ----------------------------------------------------
        let index_buffer = IndexBuffer::new(vk_ctx, &INDICES);

        log::info!(target: "gg_engine", "Triangle renderer initialized");

        Self {
            _shader: shader,
            pipeline_layout,
            pipeline,
            vertex_buffer,
            index_buffer,
            device: device.clone(),
        }
    }

    /// Record triangle draw commands into an already-begun render pass.
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

            device.cmd_bind_pipeline(cmd_buf, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            self.vertex_buffer.bind(device, cmd_buf);
            self.index_buffer.bind(device, cmd_buf);
            device.cmd_draw_indexed(cmd_buf, self.index_buffer.count(), 1, 0, 0, 0);
        }
    }
}

impl Drop for TriangleRenderer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
