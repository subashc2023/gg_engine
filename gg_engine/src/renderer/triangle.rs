use ash::vk;

use super::VulkanContext;

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
    pipeline_layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    vertex_buffer: vk::Buffer,
    vertex_buffer_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    index_buffer_memory: vk::DeviceMemory,
    device: ash::Device,
}

impl TriangleRenderer {
    pub fn new(vk_ctx: &VulkanContext, render_pass: vk::RenderPass) -> Self {
        let device = vk_ctx.device();

        // -- Shader modules --------------------------------------------------
        let vert_spv = include_bytes!("shaders/triangle_vert.spv");
        let frag_spv = include_bytes!("shaders/triangle_frag.spv");

        let vert_module = create_shader_module(device, vert_spv);
        let frag_module = create_shader_module(device, frag_spv);

        let entry_point = c"main";

        let vert_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vert_module)
            .name(entry_point);

        let frag_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(frag_module)
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

        // Shader modules are baked into the pipeline — no longer needed.
        unsafe {
            device.destroy_shader_module(vert_module, None);
            device.destroy_shader_module(frag_module, None);
        }

        // -- Vertex buffer ---------------------------------------------------
        let vb_size = (std::mem::size_of::<Vertex>() * VERTICES.len()) as vk::DeviceSize;
        let (vertex_buffer, vertex_buffer_memory) =
            create_buffer(vk_ctx, vb_size, vk::BufferUsageFlags::VERTEX_BUFFER);

        unsafe {
            let ptr = device
                .map_memory(
                    vertex_buffer_memory,
                    0,
                    vb_size,
                    vk::MemoryMapFlags::empty(),
                )
                .expect("Failed to map vertex buffer memory") as *mut Vertex;
            ptr.copy_from_nonoverlapping(VERTICES.as_ptr(), VERTICES.len());
            device.unmap_memory(vertex_buffer_memory);
        }

        // -- Index buffer ----------------------------------------------------
        let ib_size = (std::mem::size_of::<u32>() * INDICES.len()) as vk::DeviceSize;
        let (index_buffer, index_buffer_memory) =
            create_buffer(vk_ctx, ib_size, vk::BufferUsageFlags::INDEX_BUFFER);

        unsafe {
            let ptr = device
                .map_memory(
                    index_buffer_memory,
                    0,
                    ib_size,
                    vk::MemoryMapFlags::empty(),
                )
                .expect("Failed to map index buffer memory") as *mut u32;
            ptr.copy_from_nonoverlapping(INDICES.as_ptr(), INDICES.len());
            device.unmap_memory(index_buffer_memory);
        }

        log::info!(target: "gg_engine", "Triangle renderer initialized");

        Self {
            pipeline_layout,
            pipeline,
            vertex_buffer,
            vertex_buffer_memory,
            index_buffer,
            index_buffer_memory,
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
            device.cmd_bind_vertex_buffers(cmd_buf, 0, &[self.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(cmd_buf, self.index_buffer, 0, vk::IndexType::UINT32);
            device.cmd_draw_indexed(cmd_buf, INDICES.len() as u32, 1, 0, 0, 0);
        }
    }
}

impl Drop for TriangleRenderer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device.free_memory(self.index_buffer_memory, None);
            self.device.destroy_buffer(self.index_buffer, None);
            self.device.free_memory(self.vertex_buffer_memory, None);
            self.device.destroy_buffer(self.vertex_buffer, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_shader_module(device: &ash::Device, spv_bytes: &[u8]) -> vk::ShaderModule {
    // SPIR-V is a stream of u32 words. ash requires &[u32].
    let spv_u32: Vec<u32> = spv_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let info = vk::ShaderModuleCreateInfo::default().code(&spv_u32);
    unsafe { device.create_shader_module(&info, None) }.expect("Failed to create shader module")
}

fn create_buffer(
    vk_ctx: &VulkanContext,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
) -> (vk::Buffer, vk::DeviceMemory) {
    let device = vk_ctx.device();

    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer =
        unsafe { device.create_buffer(&buffer_info, None) }.expect("Failed to create buffer");

    let mem_req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem_type_index = find_memory_type(
        vk_ctx,
        mem_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );

    let alloc_info = vk::MemoryAllocateInfo::default()
        .allocation_size(mem_req.size)
        .memory_type_index(mem_type_index);

    let memory =
        unsafe { device.allocate_memory(&alloc_info, None) }.expect("Failed to allocate memory");

    unsafe { device.bind_buffer_memory(buffer, memory, 0) }
        .expect("Failed to bind buffer memory");

    (buffer, memory)
}

fn find_memory_type(
    vk_ctx: &VulkanContext,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> u32 {
    let mem_props = unsafe {
        vk_ctx
            .instance()
            .get_physical_device_memory_properties(vk_ctx.physical_device())
    };

    for i in 0..mem_props.memory_type_count {
        let type_matches = (type_filter & (1 << i)) != 0;
        let props_match = mem_props.memory_types[i as usize]
            .property_flags
            .contains(properties);
        if type_matches && props_match {
            return i;
        }
    }

    panic!("Failed to find suitable memory type");
}
