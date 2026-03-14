use std::sync::{Arc, Mutex};

use ash::vk;
use glam::Mat4;

use super::gpu_allocation::{GpuAllocation, GpuAllocator, MemoryLocation};
use super::skeleton::MAX_SKINNED_BONES_PER_FRAME;
use super::MAX_FRAMES_IN_FLIGHT;
use crate::error::{EngineError, EngineResult};

/// Size of one bone matrix in bytes (mat4 = 64 bytes).
const BONE_MATRIX_SIZE: usize = std::mem::size_of::<Mat4>();

/// Total SSBO size per frame.
const SSBO_SIZE: u64 = (MAX_SKINNED_BONES_PER_FRAME * BONE_MATRIX_SIZE) as u64;

/// Manages a per-frame Storage Buffer Object (SSBO) that holds bone matrices
/// for all skinned meshes rendered in a single frame.
///
/// The SSBO is bound at descriptor set 5 in the skinned mesh pipeline.
/// Each draw call uses a `bone_offset` push constant to index into the SSBO.
pub struct BonePaletteSystem {
    buffers: Vec<vk::Buffer>,
    _allocations: Vec<GpuAllocation>,
    mapped_ptrs: Vec<*mut u8>,
    ds_layout: vk::DescriptorSetLayout,
    descriptor_sets: Vec<vk::DescriptorSet>,
    /// Current write offset (in number of mat4s) for the current frame.
    current_offset: usize,
}

// Safety: mapped_ptrs are only accessed from the main thread during rendering.
unsafe impl Send for BonePaletteSystem {}
unsafe impl Sync for BonePaletteSystem {}

impl BonePaletteSystem {
    /// Create the bone palette system: per-frame SSBOs, descriptor set layout,
    /// and descriptor sets.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        pool: vk::DescriptorPool,
    ) -> EngineResult<Self> {
        // Descriptor set layout: one SSBO at binding 0, vertex stage only.
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let ds_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None) }
            .map_err(|e| {
                EngineError::Gpu(format!("Failed to create bone palette DS layout: {e}"))
            })?;

        let mut buffers = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut gpu_allocations = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut mapped_ptrs = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut descriptor_sets = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);

        for i in 0..MAX_FRAMES_IN_FLIGHT {
            // Create host-visible SSBO.
            let buffer_info = vk::BufferCreateInfo::default()
                .size(SSBO_SIZE)
                .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = unsafe { device.create_buffer(&buffer_info, None) }.map_err(|e| {
                EngineError::Gpu(format!("Failed to create bone palette buffer: {e}"))
            })?;

            let allocation = GpuAllocator::allocate_for_buffer(
                allocator,
                device,
                buffer,
                &format!("bone_palette_ssbo_{i}"),
                MemoryLocation::CpuToGpu,
            )?;

            let ptr = allocation.mapped_ptr().ok_or_else(|| {
                EngineError::Gpu("Bone palette buffer not host-visible".to_string())
            })?;

            // Initialize to identity matrices.
            unsafe {
                let identity = Mat4::IDENTITY;
                let identity_bytes = std::slice::from_raw_parts(
                    &identity as *const Mat4 as *const u8,
                    BONE_MATRIX_SIZE,
                );
                for k in 0..MAX_SKINNED_BONES_PER_FRAME {
                    std::ptr::copy_nonoverlapping(
                        identity_bytes.as_ptr(),
                        ptr.add(k * BONE_MATRIX_SIZE),
                        BONE_MATRIX_SIZE,
                    );
                }
            }

            // Allocate descriptor set.
            let layouts = [ds_layout];
            let alloc_info = vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&layouts);
            let sets = unsafe { device.allocate_descriptor_sets(&alloc_info) }.map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to allocate bone palette descriptor set: {e}"
                ))
            })?;
            let ds = sets[0];

            // Write descriptor.
            let buffer_info_desc = vk::DescriptorBufferInfo::default()
                .buffer(buffer)
                .offset(0)
                .range(SSBO_SIZE);
            let buffer_infos = [buffer_info_desc];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(ds)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_infos);
            unsafe { device.update_descriptor_sets(&[write], &[]) };

            buffers.push(buffer);
            mapped_ptrs.push(ptr);
            gpu_allocations.push(allocation);
            descriptor_sets.push(ds);
        }

        log::info!(
            "Bone palette system initialized: {} KB per frame, max {} bones/frame",
            SSBO_SIZE / 1024,
            MAX_SKINNED_BONES_PER_FRAME,
        );

        Ok(Self {
            buffers,
            _allocations: gpu_allocations,
            mapped_ptrs,
            ds_layout,
            descriptor_sets,
            current_offset: 0,
        })
    }

    /// Reset the write offset for a new frame.
    pub fn begin_frame(&mut self) {
        self.current_offset = 0;
    }

    /// Write bone matrices for one skinned entity. Returns the offset (in number
    /// of mat4s) where the matrices were written.
    pub fn write_bones(&mut self, matrices: &[Mat4], current_frame: usize) -> Option<u32> {
        if matrices.is_empty() {
            return Some(0);
        }
        let count = matrices.len();
        if self.current_offset + count > MAX_SKINNED_BONES_PER_FRAME {
            log::warn!(
                "Bone palette SSBO full: need {} more, only {} remaining",
                count,
                MAX_SKINNED_BONES_PER_FRAME - self.current_offset
            );
            return None;
        }

        let offset = self.current_offset;
        let ptr = self.mapped_ptrs[current_frame];
        let byte_offset = offset * BONE_MATRIX_SIZE;
        let byte_count = count * BONE_MATRIX_SIZE;

        unsafe {
            std::ptr::copy_nonoverlapping(
                matrices.as_ptr() as *const u8,
                ptr.add(byte_offset),
                byte_count,
            );
        }

        self.current_offset += count;
        Some(offset as u32)
    }

    /// Descriptor set layout for pipeline creation (set 5).
    pub fn ds_layout(&self) -> vk::DescriptorSetLayout {
        self.ds_layout
    }

    /// Descriptor set for the given frame-in-flight.
    pub fn descriptor_set(&self, current_frame: usize) -> vk::DescriptorSet {
        self.descriptor_sets[current_frame]
    }

    /// Destroy Vulkan resources. Buffers are destroyed here; allocations auto-free on drop.
    pub fn destroy(&mut self, device: &ash::Device) {
        for buffer in self.buffers.drain(..) {
            unsafe { device.destroy_buffer(buffer, None) };
        }
        unsafe {
            device.destroy_descriptor_set_layout(self.ds_layout, None);
        }
        self.descriptor_sets.clear();
    }
}
