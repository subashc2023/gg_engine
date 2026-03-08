use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Vec3};

use super::gpu_allocation::GpuAllocator;
use super::uniform_buffer::UniformBuffer;
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of point lights the shader supports.
pub const MAX_POINT_LIGHTS: usize = 16;

// ---------------------------------------------------------------------------
// LightGpuData — the UBO struct written to the GPU (std140 layout)
// ---------------------------------------------------------------------------

/// GPU-side lighting UBO (descriptor set 3, binding 0).
///
/// Matches the `LightingUBO` struct in `mesh3d.glsl`. std140-aligned.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LightGpuData {
    // Directional light
    pub dir_direction: [f32; 4], // xyz = direction, w = unused
    pub dir_color: [f32; 4],     // xyz = color, w = intensity

    // Point lights (MAX_POINT_LIGHTS entries)
    pub point_positions: [[f32; 4]; MAX_POINT_LIGHTS], // xyz = position, w = radius
    pub point_colors: [[f32; 4]; MAX_POINT_LIGHTS],    // xyz = color, w = intensity

    // Scene-wide data
    pub ambient_color: [f32; 4],   // xyz = color, w = intensity
    pub camera_position: [f32; 4], // xyz = eye position, w = unused
    pub counts: [i32; 4], // x = num_point_lights, y = has_directional, z = has_shadow, w = unused

    // Shadow mapping
    pub shadow_light_vp: [f32; 16], // mat4 = light-space VP matrix (64 bytes)
}

impl LightGpuData {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Default: no lights, dark ambient.
    pub fn empty() -> Self {
        Self {
            dir_direction: [0.0, -1.0, 0.0, 0.0],
            dir_color: [1.0, 1.0, 1.0, 1.0],
            point_positions: [[0.0; 4]; MAX_POINT_LIGHTS],
            point_colors: [[0.0; 4]; MAX_POINT_LIGHTS],
            ambient_color: [0.03, 0.03, 0.03, 1.0],
            camera_position: [0.0; 4],
            counts: [0, 0, 0, 0],
            shadow_light_vp: [0.0; 16],
        }
    }

    /// Convert to raw bytes for UBO upload.
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, Self::SIZE) }
    }
}

// ---------------------------------------------------------------------------
// LightingSystem — manages the light UBO + descriptor sets
// ---------------------------------------------------------------------------

/// Manages per-frame per-viewport lighting UBO (descriptor set 3).
///
/// Follows the same slot pattern as CameraSystem and MaterialLibrary.
pub(crate) struct LightingSystem {
    light_ubo: UniformBuffer,
    ds_layout: vk::DescriptorSetLayout,
    descriptor_sets: Vec<vk::DescriptorSet>,
    device: ash::Device,
}

impl LightingSystem {
    /// Create lighting UBO infrastructure: descriptor set layout, per-slot UBO
    /// buffers, and descriptor sets.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        descriptor_pool: vk::DescriptorPool,
    ) -> Result<Self, String> {
        // Descriptor set layout: binding 0, UNIFORM_BUFFER, vertex + fragment stages.
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
        let ubo_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&ubo_binding));
        let ds_layout = unsafe { device.create_descriptor_set_layout(&ubo_layout_info, None) }
            .map_err(|e| format!("Failed to create lighting UBO descriptor set layout: {e}"))?;

        // UBO buffers (one per frame × viewport slot).
        let light_ubo = UniformBuffer::new(allocator, device, LightGpuData::SIZE)?;

        // Allocate descriptor sets for all (frame, viewport) slots.
        let total_slots = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;
        let layouts = vec![ds_layout; total_slots];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_sets = unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }
            .map_err(|e| format!("Failed to allocate lighting UBO descriptor sets: {e}"))?;

        // Write each descriptor set pointing to its UBO buffer.
        for (i, &ds) in descriptor_sets.iter().enumerate() {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(light_ubo.buffer(i))
                .offset(0)
                .range(LightGpuData::SIZE as u64);
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
            light_ubo,
            ds_layout,
            descriptor_sets,
            device: device.clone(),
        })
    }

    /// The descriptor set layout for pipeline creation (set 3 = lighting UBO).
    pub fn ds_layout(&self) -> vk::DescriptorSetLayout {
        self.ds_layout
    }

    /// Get the descriptor set for the given (frame, viewport) slot.
    pub fn descriptor_set(&self, current_frame: usize, viewport_index: usize) -> vk::DescriptorSet {
        self.descriptor_sets[UniformBuffer::slot(current_frame, viewport_index)]
    }

    /// Write lighting data to the UBO for the given (frame, viewport) slot.
    pub fn write_ubo(&self, data: &LightGpuData, current_frame: usize, viewport_index: usize) {
        let slot = UniformBuffer::slot(current_frame, viewport_index);
        self.light_ubo.update(slot, data.as_bytes());
    }
}

impl Drop for LightingSystem {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_descriptor_set_layout(self.ds_layout, None);
        }
    }
}

// ---------------------------------------------------------------------------
// LightEnvironment — CPU-side collected light data for a frame
// ---------------------------------------------------------------------------

/// Collected light data from the scene, ready to be uploaded to the GPU.
///
/// Built by `Scene::collect_lights()` each frame before 3D rendering.
pub struct LightEnvironment {
    pub directional: Option<(Vec3, Vec3, f32)>, // (direction, color, intensity)
    pub point_lights: Vec<(Vec3, Vec3, f32, f32)>, // (position, color, intensity, radius)
    pub ambient_color: Vec3,
    pub ambient_intensity: f32,
    pub camera_position: Vec3,
    /// Light-space VP matrix for shadow mapping. `Some` = shadows enabled.
    pub shadow_light_vp: Option<Mat4>,
}

impl Default for LightEnvironment {
    fn default() -> Self {
        Self {
            directional: None,
            point_lights: Vec::new(),
            ambient_color: Vec3::new(0.03, 0.03, 0.03),
            ambient_intensity: 1.0,
            camera_position: Vec3::ZERO,
            shadow_light_vp: None,
        }
    }
}

impl LightEnvironment {
    /// Convert to GPU UBO struct.
    pub fn to_gpu_data(&self) -> LightGpuData {
        let mut data = LightGpuData::empty();

        // Directional light.
        if let Some((dir, color, intensity)) = self.directional {
            data.dir_direction = [dir.x, dir.y, dir.z, 0.0];
            data.dir_color = [color.x, color.y, color.z, intensity];
            data.counts[1] = 1;
        }

        // Point lights (clamped to MAX_POINT_LIGHTS).
        let count = self.point_lights.len().min(MAX_POINT_LIGHTS);
        for (i, &(pos, color, intensity, radius)) in
            self.point_lights.iter().take(count).enumerate()
        {
            data.point_positions[i] = [pos.x, pos.y, pos.z, radius];
            data.point_colors[i] = [color.x, color.y, color.z, intensity];
        }
        data.counts[0] = count as i32;

        // Ambient.
        data.ambient_color = [
            self.ambient_color.x,
            self.ambient_color.y,
            self.ambient_color.z,
            self.ambient_intensity,
        ];

        // Camera position.
        data.camera_position = [
            self.camera_position.x,
            self.camera_position.y,
            self.camera_position.z,
            0.0,
        ];

        // Shadow mapping.
        if let Some(light_vp) = self.shadow_light_vp {
            data.shadow_light_vp = light_vp.to_cols_array();
            data.counts[2] = 1; // has_shadow = true
        }

        data
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_gpu_data_size() {
        // DirectionalLight: 2 × vec4 = 32 bytes
        // PointLights: 16 × 2 × vec4 = 512 bytes
        // ambient + camera_pos + counts = 3 × vec4 = 48 bytes
        // shadow_light_vp: mat4 = 64 bytes
        // Total = 32 + 512 + 48 + 64 = 656 bytes
        assert_eq!(LightGpuData::SIZE, 656);
    }

    #[test]
    fn light_gpu_data_as_bytes_length() {
        let data = LightGpuData::empty();
        assert_eq!(data.as_bytes().len(), LightGpuData::SIZE);
    }

    #[test]
    fn light_environment_to_gpu_no_lights() {
        let env = LightEnvironment::default();
        let gpu = env.to_gpu_data();
        assert_eq!(gpu.counts[0], 0); // no point lights
        assert_eq!(gpu.counts[1], 0); // no directional
    }

    #[test]
    fn light_environment_to_gpu_with_lights() {
        let env = LightEnvironment {
            directional: Some((Vec3::new(0.0, -1.0, 0.0), Vec3::ONE, 2.0)),
            point_lights: vec![(
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(1.0, 0.0, 0.0),
                5.0,
                10.0,
            )],
            ambient_color: Vec3::new(0.1, 0.1, 0.1),
            ambient_intensity: 0.5,
            camera_position: Vec3::new(0.0, 5.0, -10.0),
            shadow_light_vp: None,
        };
        let gpu = env.to_gpu_data();
        assert_eq!(gpu.counts[0], 1); // 1 point light
        assert_eq!(gpu.counts[1], 1); // has directional
        assert_eq!(gpu.dir_color[3], 2.0); // intensity in w
        assert_eq!(gpu.point_positions[0][3], 10.0); // radius in w
        assert_eq!(gpu.point_colors[0][3], 5.0); // intensity in w
        assert_eq!(gpu.ambient_color[3], 0.5); // ambient intensity
    }

    #[test]
    fn light_environment_clamps_point_lights() {
        let mut env = LightEnvironment::default();
        for i in 0..20 {
            env.point_lights
                .push((Vec3::new(i as f32, 0.0, 0.0), Vec3::ONE, 1.0, 5.0));
        }
        let gpu = env.to_gpu_data();
        assert_eq!(gpu.counts[0], MAX_POINT_LIGHTS as i32);
    }
}
