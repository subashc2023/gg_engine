use ash::vk;

use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// ComputeShader — single compute stage
// ---------------------------------------------------------------------------

pub(crate) struct ComputeShader {
    module: vk::ShaderModule,
    device: ash::Device,
}

impl ComputeShader {
    pub fn new(device: &ash::Device, name: &str, comp_spv: &[u8]) -> Result<Self, String> {
        let _timer = ProfileTimer::new("ComputeShader::new");
        let module = create_shader_module(device, comp_spv)
            .map_err(|e| format!("Failed to create compute shader module for '{name}': {e}"))?;

        log::info!(target: "gg_engine", "Compute shader '{name}' created");

        Ok(Self {
            module,
            device: device.clone(),
        })
    }

    pub fn module(&self) -> vk::ShaderModule {
        self.module
    }
}

impl Drop for ComputeShader {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.module, None);
        }
    }
}

// ---------------------------------------------------------------------------
// ComputePipeline
// ---------------------------------------------------------------------------

pub(crate) struct ComputePipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    device: ash::Device,
}

impl ComputePipeline {
    pub fn pipeline(&self) -> vk::Pipeline {
        self.pipeline
    }

    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }
}

impl Drop for ComputePipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device
                .destroy_pipeline_layout(self.layout, None);
        }
    }
}

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

pub(crate) fn create_compute_pipeline(
    device: &ash::Device,
    shader: &ComputeShader,
    descriptor_set_layouts: &[vk::DescriptorSetLayout],
    push_constant_size: u32,
    pipeline_cache: vk::PipelineCache,
) -> Result<ComputePipeline, String> {
    let _timer = ProfileTimer::new("create_compute_pipeline");

    let push_constant_range = vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::COMPUTE,
        offset: 0,
        size: push_constant_size,
    };
    let push_ranges = if push_constant_size > 0 {
        vec![push_constant_range]
    } else {
        vec![]
    };

    let layout_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(descriptor_set_layouts)
        .push_constant_ranges(&push_ranges);

    let layout = unsafe { device.create_pipeline_layout(&layout_info, None) }
        .map_err(|e| format!("Failed to create compute pipeline layout: {e}"))?;

    let entry_point = c"main";
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader.module())
        .name(entry_point);

    let create_info = vk::ComputePipelineCreateInfo::default()
        .stage(stage)
        .layout(layout);

    let pipelines = unsafe {
        device.create_compute_pipelines(pipeline_cache, &[create_info], None)
    }
    .map_err(|(_pipelines, e)| format!("Failed to create compute pipeline: {e}"))?;

    Ok(ComputePipeline {
        pipeline: pipelines[0],
        layout,
        device: device.clone(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_shader_module(device: &ash::Device, spv_bytes: &[u8]) -> Result<vk::ShaderModule, vk::Result> {
    let spv_u32: Vec<u32> = spv_bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let info = vk::ShaderModuleCreateInfo::default().code(&spv_u32);
    unsafe { device.create_shader_module(&info, None) }
}
