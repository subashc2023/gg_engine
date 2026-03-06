use ash::vk;

use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// Shader
// ---------------------------------------------------------------------------

/// A compiled shader program (vertex + fragment) loaded from SPIR-V bytecode.
///
/// Created via [`Renderer::create_shader`](super::Renderer::create_shader).
/// Owns the Vulkan shader modules; destroyed on drop.
pub struct Shader {
    name: String,
    vert_module: vk::ShaderModule,
    frag_module: vk::ShaderModule,
    device: ash::Device,
}

impl Shader {
    /// Create a shader from pre-compiled SPIR-V bytecode.
    pub(crate) fn new(device: &ash::Device, name: &str, vert_spv: &[u8], frag_spv: &[u8]) -> Result<Self, String> {
        let _timer = ProfileTimer::new("Shader::new");
        let vert_module = create_shader_module(device, vert_spv)
            .map_err(|e| format!("Failed to create vertex shader module for '{name}': {e}"))?;
        let frag_module = create_shader_module(device, frag_spv)
            .map_err(|e| format!("Failed to create fragment shader module for '{name}': {e}"))?;

        log::info!(target: "gg_engine", "Shader '{name}' created");

        Ok(Self {
            name: name.to_string(),
            vert_module,
            frag_module,
            device: device.clone(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn vert_module(&self) -> vk::ShaderModule {
        self.vert_module
    }

    pub fn frag_module(&self) -> vk::ShaderModule {
        self.frag_module
    }
}

impl Drop for Shader {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.vert_module, None);
            self.device.destroy_shader_module(self.frag_module, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_shader_module(device: &ash::Device, spv_bytes: &[u8]) -> Result<vk::ShaderModule, vk::Result> {
    // SPIR-V is a stream of u32 words. ash requires &[u32].
    let spv_u32: Vec<u32> = spv_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let info = vk::ShaderModuleCreateInfo::default().code(&spv_u32);
    unsafe { device.create_shader_module(&info, None) }
}
