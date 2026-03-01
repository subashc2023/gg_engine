use ash::vk;

use super::VulkanContext;

// ---------------------------------------------------------------------------
// VertexBuffer
// ---------------------------------------------------------------------------

pub(crate) struct VertexBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    device: ash::Device,
}

impl VertexBuffer {
    pub fn new(vk_ctx: &VulkanContext, data: &[u8]) -> Self {
        let device = vk_ctx.device();
        let size = data.len() as vk::DeviceSize;

        let (buffer, memory) =
            create_buffer_and_memory(vk_ctx, size, vk::BufferUsageFlags::VERTEX_BUFFER);

        unsafe {
            let ptr = device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                .expect("Failed to map vertex buffer memory");
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            device.unmap_memory(memory);
        }

        Self {
            buffer,
            memory,
            device: device.clone(),
        }
    }

    pub fn bind(&self, device: &ash::Device, cmd_buf: vk::CommandBuffer) {
        unsafe {
            device.cmd_bind_vertex_buffers(cmd_buf, 0, &[self.buffer], &[0]);
        }
    }
}

impl Drop for VertexBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

// ---------------------------------------------------------------------------
// IndexBuffer
// ---------------------------------------------------------------------------

pub(crate) struct IndexBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    count: u32,
    device: ash::Device,
}

impl IndexBuffer {
    pub fn new(vk_ctx: &VulkanContext, indices: &[u32]) -> Self {
        let device = vk_ctx.device();
        let size = std::mem::size_of_val(indices) as vk::DeviceSize;

        let (buffer, memory) =
            create_buffer_and_memory(vk_ctx, size, vk::BufferUsageFlags::INDEX_BUFFER);

        unsafe {
            let ptr = device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                .expect("Failed to map index buffer memory") as *mut u32;
            ptr.copy_from_nonoverlapping(indices.as_ptr(), indices.len());
            device.unmap_memory(memory);
        }

        Self {
            buffer,
            memory,
            count: indices.len() as u32,
            device: device.clone(),
        }
    }

    pub fn bind(&self, device: &ash::Device, cmd_buf: vk::CommandBuffer) {
        unsafe {
            device.cmd_bind_index_buffer(cmd_buf, self.buffer, 0, vk::IndexType::UINT32);
        }
    }

    pub fn count(&self) -> u32 {
        self.count
    }
}

impl Drop for IndexBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_buffer_and_memory(
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
