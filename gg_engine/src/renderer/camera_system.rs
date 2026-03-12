use std::sync::{Arc, Mutex};

use ash::vk;
use glam::Mat4;

use super::gpu_allocation::GpuAllocator;
use super::uniform_buffer::{CameraData, UniformBuffer};
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};
use crate::error::{EngineError, EngineResult};

/// Manages the per-frame per-viewport camera UBO (view-projection matrix + time).
///
/// Owns the UBO buffers, descriptor set layout, and descriptor sets.
/// Provides `descriptor_set(frame, viewport)` to select the correct slot
/// for draw call binding.
pub(crate) struct CameraSystem {
    camera_ubo: UniformBuffer,
    ds_layout: vk::DescriptorSetLayout,
    descriptor_sets: Vec<vk::DescriptorSet>,
    scene_time: f32,
    view_projection: Mat4,
    device: ash::Device,
}

impl CameraSystem {
    /// Create camera UBO infrastructure: descriptor set layout, per-slot UBO
    /// buffers, and descriptor sets allocated from the given pool.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        descriptor_pool: vk::DescriptorPool,
    ) -> EngineResult<Self> {
        // Descriptor set layout: binding 0, UNIFORM_BUFFER, vertex + fragment stages.
        // Fragment stage needed for cascade shadow map depth comparison (camera VP).
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
        let ubo_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&ubo_binding));
        let ds_layout = unsafe { device.create_descriptor_set_layout(&ubo_layout_info, None) }
            .map_err(|e| EngineError::Gpu(format!("Failed to create camera UBO descriptor set layout: {e}")))?;

        // UBO buffers (one per frame × viewport slot).
        let camera_ubo = UniformBuffer::new(allocator, device, CameraData::SIZE)?;

        // Allocate descriptor sets for all (frame, viewport) slots.
        let total_slots = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;
        let layouts = vec![ds_layout; total_slots];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_sets = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .map_err(|e| EngineError::Gpu(format!("Failed to allocate camera UBO descriptor sets: {e}")))?;

        // Write each descriptor set pointing to its UBO buffer.
        for (i, &ds) in descriptor_sets.iter().enumerate() {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(camera_ubo.buffer(i))
                .offset(0)
                .range(CameraData::SIZE as u64);
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(std::slice::from_ref(&buffer_info));
            unsafe {
                device.update_descriptor_sets(&[write], &[]);
            }
        }

        Ok(Self {
            camera_ubo,
            ds_layout,
            descriptor_sets,
            scene_time: 0.0,
            view_projection: Mat4::IDENTITY,
            device: device.clone(),
        })
    }

    /// The descriptor set layout for pipeline creation (set 0 = camera UBO).
    pub fn ds_layout(&self) -> vk::DescriptorSetLayout {
        self.ds_layout
    }

    /// Get the descriptor set for the given (frame, viewport) slot.
    pub fn descriptor_set(&self, current_frame: usize, viewport_index: usize) -> vk::DescriptorSet {
        self.descriptor_sets[UniformBuffer::slot(current_frame, viewport_index)]
    }

    /// Current view-projection matrix.
    pub fn view_projection(&self) -> Mat4 {
        self.view_projection
    }

    /// Set the scene time written to the UBO as `u_time` (for GPU animation).
    pub fn set_scene_time(&mut self, t: f32) {
        self.scene_time = t;
    }

    /// Write the VP matrix + time to the UBO for the given (frame, viewport) slot.
    pub fn write_ubo(&self, vp: Mat4, current_frame: usize, viewport_index: usize) {
        let camera_data = CameraData {
            view_projection: vp,
            time: self.scene_time,
            _pad: [0.0; 3],
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &camera_data as *const CameraData as *const u8,
                CameraData::SIZE,
            )
        };
        let slot = UniformBuffer::slot(current_frame, viewport_index);
        self.camera_ubo.update(slot, bytes);
    }

    /// Update the stored VP matrix and write it to the UBO.
    pub fn set_view_projection(&mut self, vp: Mat4, current_frame: usize, viewport_index: usize) {
        self.view_projection = vp;
        self.write_ubo(vp, current_frame, viewport_index);
    }
}

impl Drop for CameraSystem {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_descriptor_set_layout(self.ds_layout, None);
        }
        // UniformBuffer::Drop handles buffer/memory cleanup.
        // Descriptor sets are freed when the parent descriptor pool is destroyed.
    }
}
