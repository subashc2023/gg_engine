use ash::vk;
use glam::Mat4;

use super::buffer::create_buffer_and_memory;

// ---------------------------------------------------------------------------
// CameraData — the data written to the camera UBO each frame
// ---------------------------------------------------------------------------

#[repr(C)]
pub(crate) struct CameraData {
    pub view_projection: Mat4,
}

impl CameraData {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

// ---------------------------------------------------------------------------
// UniformBuffer — per-frame-in-flight double-buffered UBO
// ---------------------------------------------------------------------------

const FRAMES_IN_FLIGHT: usize = 2;

pub(crate) struct UniformBuffer {
    buffers: [vk::Buffer; FRAMES_IN_FLIGHT],
    memories: [vk::DeviceMemory; FRAMES_IN_FLIGHT],
    mapped_ptrs: [*mut u8; FRAMES_IN_FLIGHT],
    device: ash::Device,
}

// Safety: Same contract as DynamicVertexBuffer — mapped_ptr per frame is only
// written by one frame at a time (guarded by frame-in-flight fencing).
unsafe impl Send for UniformBuffer {}
unsafe impl Sync for UniformBuffer {}

impl UniformBuffer {
    pub fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        size: usize,
    ) -> Self {
        let mut buffers = [vk::Buffer::null(); FRAMES_IN_FLIGHT];
        let mut memories = [vk::DeviceMemory::null(); FRAMES_IN_FLIGHT];
        let mut mapped_ptrs = [std::ptr::null_mut(); FRAMES_IN_FLIGHT];

        for i in 0..FRAMES_IN_FLIGHT {
            let (buffer, memory) = create_buffer_and_memory(
                instance,
                physical_device,
                device,
                size as vk::DeviceSize,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
            );

            let ptr = unsafe {
                device
                    .map_memory(
                        memory,
                        0,
                        size as vk::DeviceSize,
                        vk::MemoryMapFlags::empty(),
                    )
                    .expect("Failed to map uniform buffer memory") as *mut u8
            };

            buffers[i] = buffer;
            memories[i] = memory;
            mapped_ptrs[i] = ptr;
        }

        Self {
            buffers,
            memories,
            mapped_ptrs,
            device: device.clone(),
        }
    }

    /// Write data to the UBO for the given frame-in-flight index.
    pub fn update(&self, current_frame: usize, data: &[u8]) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.mapped_ptrs[current_frame],
                data.len(),
            );
        }
    }

    /// Get the Vulkan buffer handle for the given frame-in-flight index.
    pub fn buffer(&self, frame: usize) -> vk::Buffer {
        self.buffers[frame]
    }
}

impl Drop for UniformBuffer {
    fn drop(&mut self) {
        unsafe {
            for i in 0..FRAMES_IN_FLIGHT {
                self.device.unmap_memory(self.memories[i]);
                self.device.free_memory(self.memories[i], None);
                self.device.destroy_buffer(self.buffers[i], None);
            }
        }
    }
}
