use std::sync::{Arc, Mutex};

use ash::vk;
use glam::Mat4;

use super::buffer::create_buffer_with_allocation;
use super::gpu_allocation::{GpuAllocation, GpuAllocator};
use gg_core::error::EngineResult;

// ---------------------------------------------------------------------------
// CameraData — the data written to the camera UBO each frame
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct CameraData {
    pub view_projection: Mat4,
    pub time: f32,
    pub _pad: [f32; 3],
}
// std140 layout: mat4 (64) + float (4) + pad (12) = 80 bytes

impl CameraData {
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

// ---------------------------------------------------------------------------
// UniformBuffer — per-frame-in-flight double-buffered UBO
// ---------------------------------------------------------------------------

use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};

/// Total number of UBO slots: one per (frame, viewport) pair.
const TOTAL_SLOTS: usize = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;

pub struct UniformBuffer {
    buffers: [vk::Buffer; TOTAL_SLOTS],
    allocations: [Option<GpuAllocation>; TOTAL_SLOTS],
    device: ash::Device,
}

impl UniformBuffer {
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        size: usize,
    ) -> EngineResult<Self> {
        let mut buffers = [vk::Buffer::null(); TOTAL_SLOTS];
        let mut allocations: [Option<GpuAllocation>; TOTAL_SLOTS] = std::array::from_fn(|_| None);

        for i in 0..TOTAL_SLOTS {
            let (buffer, allocation) = create_buffer_with_allocation(
                allocator,
                device,
                size as vk::DeviceSize,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                "UniformBuffer",
            )?;

            buffers[i] = buffer;
            allocations[i] = Some(allocation);
        }

        Ok(Self {
            buffers,
            allocations,
            device: device.clone(),
        })
    }

    /// Compute the slot index for a (frame, viewport) pair.
    pub fn slot(current_frame: usize, viewport_index: usize) -> usize {
        current_frame * MAX_VIEWPORTS + viewport_index
    }

    /// Write data to the UBO for the given slot.
    pub fn update(&self, slot: usize, data: &[u8]) {
        let ptr = self.allocations[slot]
            .as_ref()
            .unwrap()
            .mapped_ptr()
            .expect("UniformBuffer must be persistently mapped");
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        }
    }

    /// Get the Vulkan buffer handle for the given slot.
    pub fn buffer(&self, slot: usize) -> vk::Buffer {
        self.buffers[slot]
    }
}

impl Drop for UniformBuffer {
    fn drop(&mut self) {
        for i in 0..TOTAL_SLOTS {
            // Destroy buffer first, then free memory (Vulkan spec requirement).
            unsafe {
                self.device.destroy_buffer(self.buffers[i], None);
            }
            self.allocations[i].take();
        }
    }
}
