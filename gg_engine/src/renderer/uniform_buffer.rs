use std::sync::{Arc, Mutex};

use ash::vk;
use glam::Mat4;

use super::buffer::create_buffer_with_allocation;
use super::gpu_allocation::{GpuAllocation, GpuAllocator};

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
    allocations: [Option<GpuAllocation>; FRAMES_IN_FLIGHT],
    device: ash::Device,
}

impl UniformBuffer {
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        size: usize,
    ) -> Self {
        let mut buffers = [vk::Buffer::null(); FRAMES_IN_FLIGHT];
        let mut allocations: [Option<GpuAllocation>; FRAMES_IN_FLIGHT] = [None, None];

        for i in 0..FRAMES_IN_FLIGHT {
            let (buffer, allocation) = create_buffer_with_allocation(
                allocator,
                device,
                size as vk::DeviceSize,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                "UniformBuffer",
            );

            buffers[i] = buffer;
            allocations[i] = Some(allocation);
        }

        Self {
            buffers,
            allocations,
            device: device.clone(),
        }
    }

    /// Write data to the UBO for the given frame-in-flight index.
    pub fn update(&self, current_frame: usize, data: &[u8]) {
        let ptr = self.allocations[current_frame]
            .as_ref()
            .unwrap()
            .mapped_ptr()
            .expect("UniformBuffer must be persistently mapped");
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        }
    }

    /// Get the Vulkan buffer handle for the given frame-in-flight index.
    pub fn buffer(&self, frame: usize) -> vk::Buffer {
        self.buffers[frame]
    }
}

impl Drop for UniformBuffer {
    fn drop(&mut self) {
        for i in 0..FRAMES_IN_FLIGHT {
            // Destroy buffer first, then free memory (Vulkan spec requirement).
            unsafe {
                self.device.destroy_buffer(self.buffers[i], None);
            }
            self.allocations[i].take();
        }
    }
}
