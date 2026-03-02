use std::sync::Arc;

use ash::vk;

use super::buffer::{as_bytes, BufferElement, BufferLayout, ShaderDataType};
use super::pipeline::{self, Pipeline};
use super::shader::Shader;
use super::texture::Texture2D;
use super::vertex_array::VertexArray;
use crate::profiling::ProfileTimer;
use crate::shaders;

// ---------------------------------------------------------------------------
// Unit quad geometry (position + texture coordinates)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct QuadVertex {
    position: [f32; 3],
    tex_coord: [f32; 2],
}

const QUAD_VERTICES: [QuadVertex; 4] = [
    QuadVertex {
        position: [-0.5, 0.5, 0.0],
        tex_coord: [0.0, 0.0],
    },
    QuadVertex {
        position: [0.5, 0.5, 0.0],
        tex_coord: [1.0, 0.0],
    },
    QuadVertex {
        position: [0.5, -0.5, 0.0],
        tex_coord: [1.0, 1.0],
    },
    QuadVertex {
        position: [-0.5, -0.5, 0.0],
        tex_coord: [0.0, 1.0],
    },
];

const QUAD_INDICES: [u32; 6] = [0, 1, 2, 2, 3, 0];

// ---------------------------------------------------------------------------
// Renderer2DData — internal resources for the 2D renderer
// ---------------------------------------------------------------------------

pub(super) struct Renderer2DData {
    // Kept alive so the shader modules aren't destroyed while the pipeline exists.
    _shader: Arc<Shader>,
    pipeline: Arc<Pipeline>,
    quad_vertex_array: VertexArray,
    white_texture: Texture2D,
}

impl Renderer2DData {
    pub(super) fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        render_pass: vk::RenderPass,
        texture_descriptor_set_layout: vk::DescriptorSetLayout,
        white_texture: Texture2D,
    ) -> Self {
        let _timer = ProfileTimer::new("Renderer2D::init");
        let shader = Arc::new(Shader::new(
            device,
            "texture",
            shaders::TEXTURE_VERT_SPV,
            shaders::TEXTURE_FRAG_SPV,
        ));

        let mut vb = super::buffer::VertexBuffer::new(
            instance,
            physical_device,
            device,
            as_bytes(&QUAD_VERTICES),
        );
        vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float2, "a_tex_coord"),
        ]));

        let ib = super::buffer::IndexBuffer::new(instance, physical_device, device, &QUAD_INDICES);

        let mut quad_vertex_array = VertexArray::new(device);
        quad_vertex_array.add_vertex_buffer(vb);
        quad_vertex_array.set_index_buffer(ib);

        // Unified pipeline: material color push constant + texture descriptor set.
        let pipeline = Arc::new(pipeline::create_pipeline(
            device,
            &shader,
            &quad_vertex_array,
            render_pass,
            true,                             // has_material_color
            &[texture_descriptor_set_layout], // texture descriptor set
            true,                             // blend_enable
        ));

        Self {
            _shader: shader,
            pipeline,
            quad_vertex_array,
            white_texture,
        }
    }

    pub(super) fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    pub(super) fn vertex_array(&self) -> &VertexArray {
        &self.quad_vertex_array
    }

    pub(super) fn white_texture(&self) -> &Texture2D {
        &self.white_texture
    }
}
