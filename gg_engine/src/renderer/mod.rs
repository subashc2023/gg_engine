mod buffer;
mod camera_system;
mod compute;
mod draw_context;
mod editor_camera;
mod font;
mod framebuffer;
pub(crate) mod gpu_allocation;
mod gpu_particle_system;
mod gpu_profiling;
mod lighting;
mod material;
mod mesh;
mod msdf;
mod orthographic_camera;
mod pipeline;
mod postprocess;
mod render_command;
#[allow(clippy::module_inception)]
mod renderer;
mod renderer_2d;
mod renderer_api;
mod scene_camera;
mod shader;
mod shader_compiler;
mod shader_library;
pub mod shaders;
pub mod shadow_map;
mod sub_texture;
mod swapchain;
mod texture;
mod uniform_buffer;
mod vertex_array;
mod vulkan_context;

pub use buffer::{
    as_bytes, BufferElement, BufferLayout, IndexBuffer, ShaderDataType, VertexBuffer,
};
pub(crate) use draw_context::DrawContext;
pub use editor_camera::EditorCamera;
pub(crate) use font::generate_font_cpu_data;
pub use font::{Font, FontCpuData, GlyphInfo};
pub(crate) use framebuffer::ClearValues;
pub use framebuffer::{
    Framebuffer, FramebufferSpec, FramebufferTextureFormat, FramebufferTextureSpec, MsaaSamples,
};
pub use gpu_profiling::{GpuProfiler, GpuTimingResult};
pub(crate) use gpu_allocation::GpuAllocator;
pub use lighting::{LightEnvironment, LightGpuData, MAX_POINT_LIGHTS, NUM_SHADOW_CASCADES};
pub use shadow_map::ShadowCameraInfo;
pub use material::{BlendMode, Material, MaterialGpuData, MaterialHandle, MaterialLibrary};
pub use mesh::{load_gltf, Mesh, MeshVertex};
pub use orthographic_camera::OrthographicCamera;
pub use pipeline::{CullMode, DepthConfig, Pipeline};
pub use postprocess::{PostProcessPipeline, TonemappingMode};
pub use renderer::{Renderer, WireframeMode};
pub use renderer_2d::Renderer2DStats;
pub use scene_camera::{ProjectionType, SceneCamera};
pub use shader::Shader;
pub use shader_library::ShaderLibrary;
pub use sub_texture::SubTexture2D;
pub use swapchain::{Swapchain, SwapchainError};
pub use texture::{ImageFormat, Texture2D, TextureCpuData, TextureSpecification};
pub use vertex_array::VertexArray;
pub use vulkan_context::{VulkanContext, VulkanInitError};

// ---------------------------------------------------------------------------
// RendererResources — lightweight view of Renderer-owned Vulkan state
// ---------------------------------------------------------------------------

/// Borrows the Vulkan handles needed by internal factory functions
/// (textures, framebuffers). Avoids passing 7-8 individual parameters
/// through internal APIs.
pub(crate) struct RendererResources<'a> {
    pub device: &'a ash::Device,
    pub graphics_queue: vk::Queue,
    pub command_pool: vk::CommandPool,
    pub descriptor_pool: vk::DescriptorPool,
    pub texture_ds_layout: vk::DescriptorSetLayout,
    pub color_format: vk::Format,
    pub depth_format: vk::Format,
}

use ash::vk;

/// Maximum number of frames that can be in-flight simultaneously.
/// All renderer subsystems (swapchain, batch renderer, uniform buffers) must
/// use this single constant to stay in sync.
pub(crate) const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// Maximum number of viewports (offscreen framebuffers) that can be rendered
/// per frame. Each viewport gets its own camera UBO slot to avoid conflicts
/// when multiple viewports render in the same command buffer.
pub(crate) const MAX_VIEWPORTS: usize = 4;

// ---------------------------------------------------------------------------
// PresentMode
// ---------------------------------------------------------------------------

/// Desired presentation mode for the swapchain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PresentMode {
    /// VSync — guaranteed available on all drivers.
    #[default]
    Fifo,
    /// Triple-buffered, no vsync. Falls back to Immediate, then Fifo.
    Mailbox,
    /// Immediate (tearing allowed). Falls back to Mailbox, then Fifo.
    Immediate,
}

impl std::fmt::Display for PresentMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fifo => write!(f, "Fifo (VSync)"),
            Self::Mailbox => write!(f, "Mailbox (Triple-buffered)"),
            Self::Immediate => write!(f, "Immediate (No VSync)"),
        }
    }
}

// ---------------------------------------------------------------------------
// RendererBackend
// ---------------------------------------------------------------------------

/// Which rendering API the engine is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererBackend {
    None,
    Vulkan,
}

impl RendererBackend {
    pub fn current() -> Self {
        RendererBackend::Vulkan
    }
}

impl std::fmt::Display for RendererBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Vulkan => write!(f, "Vulkan"),
        }
    }
}
