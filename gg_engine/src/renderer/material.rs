use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ash::vk;
use glam::{Vec3, Vec4};

use super::gpu_allocation::GpuAllocator;
use super::texture::Texture2D;
use super::uniform_buffer::UniformBuffer;
use super::{MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS};
use crate::error::{EngineError, EngineResult};
use crate::uuid::Uuid;
use crate::Ref;

// ---------------------------------------------------------------------------
// BlendMode
// ---------------------------------------------------------------------------

/// How a material's fragments are blended with the framebuffer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// Fully opaque — no alpha blending, writes depth.
    #[default]
    Opaque,
    /// Standard alpha blending (src_alpha / one_minus_src_alpha).
    AlphaBlend,
    /// Additive blending (src_alpha / one). Good for particles, glow.
    Additive,
}

impl BlendMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opaque => "Opaque",
            Self::AlphaBlend => "AlphaBlend",
            Self::Additive => "Additive",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "Opaque" => Self::Opaque,
            "AlphaBlend" => Self::AlphaBlend,
            "Additive" => Self::Additive,
            _ => Self::Opaque,
        }
    }
}

impl std::fmt::Display for BlendMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// MaterialHandle
// ---------------------------------------------------------------------------

/// Handle to a material in the material library.
pub type MaterialHandle = Uuid;

// ---------------------------------------------------------------------------
// Material
// ---------------------------------------------------------------------------

/// Describes the visual properties of a surface.
///
/// Designed around the PBR metallic-roughness workflow for 3D readiness,
/// but works fine for simple unlit 2D/3D rendering (just use albedo color/texture).
#[derive(Clone)]
pub struct Material {
    pub name: String,

    // -- Surface color --------------------------------------------------------
    /// Base color tint, multiplied with the albedo texture. Default: white.
    pub albedo_color: Vec4,
    /// Albedo (base color) texture. `None` = solid `albedo_color`.
    pub albedo_texture: Option<Ref<Texture2D>>,
    /// Asset handle for serialization. 0 = no texture.
    pub albedo_texture_handle: Uuid,

    // -- PBR metallic-roughness -----------------------------------------------
    /// 0.0 = dielectric (plastic, wood), 1.0 = metal (gold, steel).
    pub metallic: f32,
    /// 0.0 = mirror-smooth, 1.0 = fully rough/matte.
    pub roughness: f32,

    // -- Normal map -----------------------------------------------------------
    /// Tangent-space normal map. `None` = flat normals.
    pub normal_texture: Option<Ref<Texture2D>>,
    /// Asset handle for serialization. 0 = no normal map.
    pub normal_texture_handle: Uuid,

    // -- Emissive -------------------------------------------------------------
    /// Emissive color (HDR). Black = no emission.
    pub emissive_color: Vec3,
    /// Multiplier on emissive color for HDR bloom intensity.
    pub emissive_strength: f32,

    // -- Rendering state ------------------------------------------------------
    /// How fragments are blended with the framebuffer.
    pub blend_mode: BlendMode,
    /// When `true`, both front and back faces are rendered (no backface culling).
    pub double_sided: bool,
    /// Alpha threshold for alpha-test cutout. Fragments below this are discarded.
    pub alpha_cutoff: f32,
    /// Whether depth testing is enabled for this material.
    pub depth_test: bool,
    /// Whether depth writing is enabled for this material.
    pub depth_write: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            albedo_color: Vec4::ONE,
            albedo_texture: None,
            albedo_texture_handle: Uuid::default(),
            metallic: 0.0,
            roughness: 0.5,
            normal_texture: None,
            normal_texture_handle: Uuid::default(),
            emissive_color: Vec3::ZERO,
            emissive_strength: 1.0,
            blend_mode: BlendMode::Opaque,
            double_sided: false,
            alpha_cutoff: 0.5,
            depth_test: true,
            depth_write: true,
        }
    }
}

// ---------------------------------------------------------------------------
// MaterialData — GPU-side UBO struct (std140 layout)
// ---------------------------------------------------------------------------

/// GPU representation of a material, written to a UBO (descriptor set 2, binding 0).
///
/// Matches the `MaterialUBO` struct in 3D shaders. std140-aligned.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MaterialGpuData {
    pub albedo_color: [f32; 4],   // 16 bytes (offset  0)
    pub emissive_color: [f32; 3], // 12 bytes (offset 16)
    pub metallic: f32,            //  4 bytes (offset 28)
    pub roughness: f32,           //  4 bytes (offset 32)
    pub emissive_strength: f32,   //  4 bytes (offset 36)
    pub alpha_cutoff: f32,        //  4 bytes (offset 40)
    pub albedo_tex_index: i32,    //  4 bytes (offset 44) — bindless slot, -1 = none
    pub normal_tex_index: i32,    //  4 bytes (offset 48) — bindless slot, -1 = none
    pub _pad: [f32; 3],           // 12 bytes (offset 52) — pad to 64 bytes
}

impl MaterialGpuData {
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Build GPU data from a `Material`.
    pub fn from_material(mat: &Material) -> Self {
        let albedo_tex_index = mat
            .albedo_texture
            .as_ref()
            .map(|t| t.bindless_index() as i32)
            .unwrap_or(-1);

        let normal_tex_index = mat
            .normal_texture
            .as_ref()
            .map(|t| t.bindless_index() as i32)
            .unwrap_or(-1);

        Self {
            albedo_color: mat.albedo_color.to_array(),
            emissive_color: mat.emissive_color.to_array(),
            metallic: mat.metallic,
            roughness: mat.roughness,
            emissive_strength: mat.emissive_strength,
            alpha_cutoff: mat.alpha_cutoff,
            albedo_tex_index,
            normal_tex_index,
            _pad: [0.0; 3],
        }
    }

    /// Convert to raw bytes for UBO upload.
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, Self::SIZE) }
    }
}

// ---------------------------------------------------------------------------
// MaterialLibrary — manages materials + GPU resources
// ---------------------------------------------------------------------------

/// Central registry of materials with GPU UBO infrastructure.
///
/// Owns the material descriptor set layout (set 2) and per-slot UBO
/// buffers for uploading material data to the GPU each frame.
pub struct MaterialLibrary {
    materials: HashMap<MaterialHandle, Material>,
    default_handle: MaterialHandle,

    // GPU resources
    material_ubo: UniformBuffer,
    ds_layout: vk::DescriptorSetLayout,
    descriptor_sets: Vec<vk::DescriptorSet>,
    device: ash::Device,
}

impl MaterialLibrary {
    /// Create the material library with GPU infrastructure.
    ///
    /// Allocates the material UBO descriptor set layout (binding 0, UNIFORM_BUFFER,
    /// vertex + fragment stages), per-slot UBO buffers, and descriptor sets.
    pub fn new(
        allocator: &Arc<Mutex<GpuAllocator>>,
        device: &ash::Device,
        descriptor_pool: vk::DescriptorPool,
    ) -> EngineResult<Self> {
        // Descriptor set layout: binding 0, UNIFORM_BUFFER, vertex + fragment stages.
        let ubo_binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
        let ubo_layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(std::slice::from_ref(&ubo_binding));
        let ds_layout = unsafe { device.create_descriptor_set_layout(&ubo_layout_info, None) }
            .map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to create material UBO descriptor set layout: {e}"
                ))
            })?;

        // UBO buffers (one per frame × viewport slot, same as camera).
        let material_ubo = UniformBuffer::new(allocator, device, MaterialGpuData::SIZE)?;

        // Allocate descriptor sets for all (frame, viewport) slots.
        let total_slots = MAX_FRAMES_IN_FLIGHT * MAX_VIEWPORTS;
        let layouts = vec![ds_layout; total_slots];
        let ds_alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&layouts);
        let descriptor_sets =
            unsafe { device.allocate_descriptor_sets(&ds_alloc_info) }.map_err(|e| {
                EngineError::Gpu(format!(
                    "Failed to allocate material UBO descriptor sets: {e}"
                ))
            })?;

        // Write each descriptor set pointing to its UBO buffer.
        for (i, &ds) in descriptor_sets.iter().enumerate() {
            let buffer_info = vk::DescriptorBufferInfo::default()
                .buffer(material_ubo.buffer(i))
                .offset(0)
                .range(MaterialGpuData::SIZE as u64);
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

        // Create the default material.
        let default_handle = Uuid::new();
        let mut materials = HashMap::new();
        materials.insert(default_handle, Material::default());

        Ok(Self {
            materials,
            default_handle,
            material_ubo,
            ds_layout,
            descriptor_sets,
            device: device.clone(),
        })
    }

    /// The descriptor set layout for pipeline creation (set 2 = material UBO).
    pub fn ds_layout(&self) -> vk::DescriptorSetLayout {
        self.ds_layout
    }

    /// Get the descriptor set for the given (frame, viewport) slot.
    pub fn descriptor_set(&self, current_frame: usize, viewport_index: usize) -> vk::DescriptorSet {
        self.descriptor_sets[UniformBuffer::slot(current_frame, viewport_index)]
    }

    /// Handle of the built-in default material (white, opaque, no textures).
    pub fn default_handle(&self) -> MaterialHandle {
        self.default_handle
    }

    /// Create a new material with the given name. Returns its handle.
    pub fn create(&mut self, name: impl Into<String>) -> MaterialHandle {
        let handle = Uuid::new();
        let mat = Material {
            name: name.into(),
            ..Material::default()
        };
        self.materials.insert(handle, mat);
        handle
    }

    /// Insert a material with an existing handle (e.g. when loading from asset).
    pub fn insert(&mut self, handle: MaterialHandle, material: Material) {
        self.materials.insert(handle, material);
    }

    /// Get an immutable reference to a material by handle.
    pub fn get(&self, handle: &MaterialHandle) -> Option<&Material> {
        self.materials.get(handle)
    }

    /// Get a mutable reference to a material by handle.
    pub fn get_mut(&mut self, handle: &MaterialHandle) -> Option<&mut Material> {
        self.materials.get_mut(handle)
    }

    /// Remove a material from the library. Returns `None` if not found.
    /// The default material cannot be removed.
    pub fn remove(&mut self, handle: &MaterialHandle) -> Option<Material> {
        if *handle == self.default_handle {
            return None;
        }
        self.materials.remove(handle)
    }

    /// Number of materials in the library (including the default).
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Whether the library is empty (should never be — always has default).
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    /// Iterate over all materials.
    pub fn iter(&self) -> impl Iterator<Item = (&MaterialHandle, &Material)> {
        self.materials.iter()
    }

    /// Write a material's GPU data to the UBO for the given (frame, viewport) slot.
    ///
    /// Call this before drawing objects that use this material.
    pub fn write_material_ubo(
        &self,
        handle: &MaterialHandle,
        current_frame: usize,
        viewport_index: usize,
    ) {
        let mat = self
            .materials
            .get(handle)
            .unwrap_or_else(|| self.materials.get(&self.default_handle).unwrap());
        let gpu_data = MaterialGpuData::from_material(mat);
        let slot = UniformBuffer::slot(current_frame, viewport_index);
        self.material_ubo.update(slot, gpu_data.as_bytes());
    }
}

impl Drop for MaterialLibrary {
    fn drop(&mut self) {
        unsafe {
            self.device
                .destroy_descriptor_set_layout(self.ds_layout, None);
        }
        // UniformBuffer::Drop handles buffer/memory cleanup.
        // Descriptor sets are freed when the parent descriptor pool is destroyed.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_default_values() {
        let mat = Material::default();
        assert_eq!(mat.name, "Default");
        assert_eq!(mat.albedo_color, Vec4::ONE);
        assert_eq!(mat.metallic, 0.0);
        assert_eq!(mat.roughness, 0.5);
        assert_eq!(mat.emissive_color, Vec3::ZERO);
        assert_eq!(mat.emissive_strength, 1.0);
        assert_eq!(mat.blend_mode, BlendMode::Opaque);
        assert!(!mat.double_sided);
        assert_eq!(mat.alpha_cutoff, 0.5);
        assert!(mat.depth_test);
        assert!(mat.depth_write);
        assert!(mat.albedo_texture.is_none());
        assert!(mat.normal_texture.is_none());
    }

    #[test]
    fn material_gpu_data_size_is_64_bytes() {
        assert_eq!(MaterialGpuData::SIZE, 64);
    }

    #[test]
    fn material_gpu_data_from_default() {
        let mat = Material::default();
        let gpu = MaterialGpuData::from_material(&mat);
        assert_eq!(gpu.albedo_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(gpu.emissive_color, [0.0, 0.0, 0.0]);
        assert_eq!(gpu.metallic, 0.0);
        assert_eq!(gpu.roughness, 0.5);
        assert_eq!(gpu.emissive_strength, 1.0);
        assert_eq!(gpu.alpha_cutoff, 0.5);
        assert_eq!(gpu.albedo_tex_index, -1);
        assert_eq!(gpu.normal_tex_index, -1);
    }

    #[test]
    fn material_gpu_data_as_bytes_length() {
        let gpu = MaterialGpuData::from_material(&Material::default());
        assert_eq!(gpu.as_bytes().len(), 64);
    }

    #[test]
    fn blend_mode_round_trip() {
        for mode in [
            BlendMode::Opaque,
            BlendMode::AlphaBlend,
            BlendMode::Additive,
        ] {
            assert_eq!(BlendMode::parse_str(mode.as_str()), mode);
        }
    }

    #[test]
    fn blend_mode_parse_unknown_defaults_to_opaque() {
        assert_eq!(BlendMode::parse_str("Unknown"), BlendMode::Opaque);
        assert_eq!(BlendMode::parse_str(""), BlendMode::Opaque);
    }

    #[test]
    fn blend_mode_display() {
        assert_eq!(format!("{}", BlendMode::Opaque), "Opaque");
        assert_eq!(format!("{}", BlendMode::AlphaBlend), "AlphaBlend");
        assert_eq!(format!("{}", BlendMode::Additive), "Additive");
    }
}
