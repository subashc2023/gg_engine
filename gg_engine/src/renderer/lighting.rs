use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Mat4, Vec3};

use super::gpu_allocation::GpuAllocator;
use super::uniform_buffer::UniformBuffer;
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};
use crate::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of point lights the shader supports.
pub const MAX_POINT_LIGHTS: usize = 16;

/// Number of shadow map cascades for directional light CSM.
pub const NUM_SHADOW_CASCADES: usize = 4;

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
    pub counts: [i32; 4], // x = num_point_lights, y = has_directional, z = has_shadow, w = csm_debug

    // Shadow mapping (cascaded)
    pub shadow_light_vp: [[f32; 16]; NUM_SHADOW_CASCADES], // 4 × mat4 = 256 bytes
    pub cascade_split_depth: [f32; 4], // xyz = 3 split depths (NDC), w = shadow_distance
    pub cascade_texel_size: [f32; 4],  // world-units-per-texel per cascade (for bias scaling)
    pub shadow_settings: [i32; 4],     // x = quality (0-3), yzw = reserved
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
            shadow_light_vp: [[0.0; 16]; NUM_SHADOW_CASCADES],
            cascade_split_depth: [0.0; 4],
            cascade_texel_size: [1.0; 4],
            shadow_settings: [0; 4],
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
/// Descriptor set 3 layout:
///   binding 0: UNIFORM_BUFFER  — LightGpuData (lighting UBO)
///   binding 1: COMBINED_IMAGE_SAMPLER — irradiance cubemap (IBL diffuse)
///   binding 2: COMBINED_IMAGE_SAMPLER — pre-filtered specular cubemap (IBL specular)
///   binding 3: COMBINED_IMAGE_SAMPLER — BRDF integration LUT (IBL split-sum)
///   binding 4: COMBINED_IMAGE_SAMPLER — source environment cubemap (skybox)
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
    ) -> EngineResult<Self> {
        // Descriptor set layout: 5 bindings.
        //   0: Lighting UBO (vertex + fragment)
        //   1: Irradiance cubemap (fragment)
        //   2: Pre-filtered specular cubemap (fragment)
        //   3: BRDF integration LUT (fragment)
        //   4: Source environment cubemap (fragment, for skybox)
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(4)
                .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let ubo_layout_info =
            vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let ds_layout = unsafe { device.create_descriptor_set_layout(&ubo_layout_info, None) }
            .map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to create lighting UBO descriptor set layout: {e}"
                ))
            })?;

        // UBO buffers (one per frame × viewport slot).
        let light_ubo = UniformBuffer::new(allocator, device, LightGpuData::SIZE)?;

        // Allocate descriptor sets for all (frame, viewport) slots.
        let total_slots = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;
        let layouts = vec![ds_layout; total_slots];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_sets =
            unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }.map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to allocate lighting UBO descriptor sets: {e}"
                ))
            })?;

        // Write each descriptor set pointing to its UBO buffer (binding 0 only).
        // IBL bindings (1-4) are written later by write_ibl_descriptors().
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

    /// The descriptor set layout for pipeline creation (set 3 = lighting + IBL).
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

    /// Write IBL cubemap and BRDF LUT descriptors to bindings 1-4 for ALL slots.
    ///
    /// Called once when the environment map changes (or at startup with fallbacks).
    pub fn write_ibl_descriptors(
        &self,
        irradiance_view: vk::ImageView,
        irradiance_sampler: vk::Sampler,
        prefiltered_view: vk::ImageView,
        prefiltered_sampler: vk::Sampler,
        brdf_lut_view: vk::ImageView,
        brdf_lut_sampler: vk::Sampler,
        env_cubemap_view: vk::ImageView,
        env_cubemap_sampler: vk::Sampler,
    ) {
        for &ds in &self.descriptor_sets {
            let irradiance_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(irradiance_view)
                .sampler(irradiance_sampler);
            let prefiltered_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(prefiltered_view)
                .sampler(prefiltered_sampler);
            let brdf_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(brdf_lut_view)
                .sampler(brdf_lut_sampler);
            let env_info = vk::DescriptorImageInfo::default()
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image_view(env_cubemap_view)
                .sampler(env_cubemap_sampler);

            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(ds)
                    .dst_binding(1)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(&irradiance_info)),
                vk::WriteDescriptorSet::default()
                    .dst_set(ds)
                    .dst_binding(2)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(&prefiltered_info)),
                vk::WriteDescriptorSet::default()
                    .dst_set(ds)
                    .dst_binding(3)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(&brdf_info)),
                vk::WriteDescriptorSet::default()
                    .dst_set(ds)
                    .dst_binding(4)
                    .dst_array_element(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(std::slice::from_ref(&env_info)),
            ];

            unsafe {
                self.device.update_descriptor_sets(&writes, &[]);
            }
        }
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
    /// Per-cascade light-space VP matrices. `Some` = shadows enabled.
    pub shadow_cascade_vps: Option<[Mat4; NUM_SHADOW_CASCADES]>,
    /// Cascade split depths in Vulkan NDC (3 splits for 4 cascades).
    pub cascade_split_depths: [f32; 3],
    /// Shadow distance in world units (for shader fade-out). Packed into cascade_split_depth.w.
    pub shadow_distance: f32,
    /// World-units-per-texel for each cascade (used for per-cascade bias scaling).
    pub cascade_texel_sizes: [f32; 4],
    /// Whether IBL environment maps are active.
    pub has_ibl: bool,
    /// IBL intensity multiplier.
    pub ibl_intensity: f32,
    /// Number of pre-filtered specular mip levels minus one (for roughness LOD).
    pub max_prefilter_mip: i32,
}

impl Default for LightEnvironment {
    fn default() -> Self {
        Self {
            directional: None,
            point_lights: Vec::new(),
            ambient_color: Vec3::new(0.03, 0.03, 0.03),
            ambient_intensity: 1.0,
            camera_position: Vec3::ZERO,
            shadow_cascade_vps: None,
            cascade_split_depths: [0.0; 3],
            shadow_distance: 100.0,
            cascade_texel_sizes: [1.0; 4],
            has_ibl: false,
            ibl_intensity: 1.0,
            max_prefilter_mip: 8,
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

        // Shadow mapping (cascaded).
        if let Some(cascade_vps) = self.shadow_cascade_vps {
            for (i, vp) in cascade_vps.iter().enumerate() {
                data.shadow_light_vp[i] = vp.to_cols_array();
            }
            data.cascade_split_depth[0] = self.cascade_split_depths[0];
            data.cascade_split_depth[1] = self.cascade_split_depths[1];
            data.cascade_split_depth[2] = self.cascade_split_depths[2];
            data.cascade_split_depth[3] = self.shadow_distance;
            data.cascade_texel_size = self.cascade_texel_sizes;
            data.counts[2] = 1; // has_shadow = true
        }

        // IBL (packed into shadow_settings.yzw which are reserved).
        if self.has_ibl {
            data.shadow_settings[1] = 1; // has_ibl
            data.shadow_settings[2] = self.ibl_intensity.to_bits() as i32; // intBitsToFloat in shader
            data.shadow_settings[3] = self.max_prefilter_mip;
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
        // shadow_light_vp: 4 × mat4 = 256 bytes
        // cascade_split_depth: vec4 = 16 bytes
        // cascade_texel_size: vec4 = 16 bytes
        // shadow_settings: ivec4 = 16 bytes
        // Total = 32 + 512 + 48 + 256 + 16 + 16 + 16 = 896 bytes
        assert_eq!(LightGpuData::SIZE, 896);
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
            shadow_cascade_vps: None,
            cascade_split_depths: [0.0; 3],
            shadow_distance: 100.0,
            cascade_texel_sizes: [1.0; 4],
            ..Default::default()
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
