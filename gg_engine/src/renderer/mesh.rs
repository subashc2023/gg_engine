use std::path::Path;

use super::buffer::as_bytes;
use super::{BufferElement, BufferLayout, ShaderDataType, VertexArray};
use crate::renderer::Renderer;

// ---------------------------------------------------------------------------
// MeshVertex
// ---------------------------------------------------------------------------

/// Standard 3D vertex: position, normal, texture coordinates, vertex color.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

// ---------------------------------------------------------------------------
// Mesh (CPU-side)
// ---------------------------------------------------------------------------

/// CPU-side mesh data ready for GPU upload.
pub struct Mesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub name: String,
}

impl Mesh {
    /// Vertex layout matching `MeshVertex` fields (for pipeline creation).
    pub fn vertex_layout() -> BufferLayout {
        BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float3, "a_normal"),
            BufferElement::new(ShaderDataType::Float2, "a_uv"),
            BufferElement::new(ShaderDataType::Float4, "a_color"),
        ])
    }

    /// Upload mesh data to the GPU, returning a ready-to-draw `VertexArray`.
    pub fn upload(&self, renderer: &mut Renderer) -> Result<VertexArray, String> {
        let vertex_bytes = unsafe { as_bytes(&self.vertices) };
        let mut vb = renderer.create_vertex_buffer(vertex_bytes)?;
        vb.set_layout(Self::vertex_layout());

        let ib = renderer.create_index_buffer(&self.indices)?;

        let mut va = renderer.create_vertex_array();
        va.add_vertex_buffer(vb);
        va.set_index_buffer(ib);
        Ok(va)
    }

    // -----------------------------------------------------------------------
    // Built-in primitives
    // -----------------------------------------------------------------------

    /// Unit cube centered at origin (side length 1). 24 vertices, 36 indices.
    pub fn cube(color: [f32; 4]) -> Self {
        #[rustfmt::skip]
        let faces: [([f32; 3], [[f32; 3]; 4], [[f32; 2]; 4]); 6] = [
            // normal,        positions (CCW from front),                          UVs
            // +Z (front in LH)
            ([0.0, 0.0, 1.0], [[-0.5, -0.5,  0.5], [ 0.5, -0.5,  0.5], [ 0.5,  0.5,  0.5], [-0.5,  0.5,  0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
            // -Z (back)
            ([0.0, 0.0,-1.0], [[ 0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5,  0.5, -0.5], [ 0.5,  0.5, -0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
            // +X (right)
            ([1.0, 0.0, 0.0], [[ 0.5, -0.5,  0.5], [ 0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5,  0.5,  0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
            // -X (left)
            ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5,  0.5], [-0.5,  0.5,  0.5], [-0.5,  0.5, -0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
            // +Y (top)
            ([0.0, 1.0, 0.0], [[-0.5,  0.5,  0.5], [ 0.5,  0.5,  0.5], [ 0.5,  0.5, -0.5], [-0.5,  0.5, -0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
            // -Y (bottom)
            ([0.0,-1.0, 0.0], [[-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5], [-0.5, -0.5,  0.5]], [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
        ];

        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);

        for (normal, positions, uvs) in &faces {
            let base = vertices.len() as u32;
            for i in 0..4 {
                vertices.push(MeshVertex {
                    position: positions[i],
                    normal: *normal,
                    uv: uvs[i],
                    color,
                });
            }
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }

        Self {
            vertices,
            indices,
            name: "Cube".into(),
        }
    }

    /// UV sphere centered at origin (radius 0.5).
    pub fn sphere(segments: u32, rings: u32, color: [f32; 4]) -> Self {
        let segments = segments.max(3);
        let rings = rings.max(2);

        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let phi = v * std::f32::consts::PI;
            let sin_phi = phi.sin();
            let cos_phi = phi.cos();

            for seg in 0..=segments {
                let u = seg as f32 / segments as f32;
                let theta = u * 2.0 * std::f32::consts::PI;

                let x = sin_phi * theta.cos();
                let y = cos_phi;
                let z = sin_phi * theta.sin();

                vertices.push(MeshVertex {
                    position: [x * 0.5, y * 0.5, z * 0.5],
                    normal: [x, y, z],
                    uv: [u, v],
                    color,
                });
            }
        }

        for ring in 0..rings {
            for seg in 0..segments {
                let curr = ring * (segments + 1) + seg;
                let next = curr + segments + 1;

                indices.extend_from_slice(&[curr, next, curr + 1]);
                indices.extend_from_slice(&[curr + 1, next, next + 1]);
            }
        }

        Self {
            vertices,
            indices,
            name: "Sphere".into(),
        }
    }

    /// Flat plane on the XZ plane (Y = 0), centered at origin (side length 1).
    pub fn plane(color: [f32; 4]) -> Self {
        let normal = [0.0, 1.0, 0.0];
        let vertices = vec![
            MeshVertex {
                position: [-0.5, 0.0, -0.5],
                normal,
                uv: [0.0, 0.0],
                color,
            },
            MeshVertex {
                position: [0.5, 0.0, -0.5],
                normal,
                uv: [1.0, 0.0],
                color,
            },
            MeshVertex {
                position: [0.5, 0.0, 0.5],
                normal,
                uv: [1.0, 1.0],
                color,
            },
            MeshVertex {
                position: [-0.5, 0.0, 0.5],
                normal,
                uv: [0.0, 1.0],
                color,
            },
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];

        Self {
            vertices,
            indices,
            name: "Plane".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// glTF loading
// ---------------------------------------------------------------------------

/// Load all meshes from a glTF / GLB file.
pub fn load_gltf(path: &Path) -> Result<Vec<Mesh>, String> {
    let (document, buffers, _images) = gltf::import(path)
        .map_err(|e| format!("Failed to load glTF '{}': {}", path.display(), e))?;

    let mut meshes = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| format!("Mesh '{}' primitive has no positions", mesh.index()))?
                .collect();

            let vert_count = positions.len();

            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|n| n.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; vert_count]);

            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|tc| tc.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; vert_count]);

            let colors: Vec<[f32; 4]> = reader
                .read_colors(0)
                .map(|c| c.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.0, 1.0, 1.0, 1.0]; vert_count]);

            let vertices: Vec<MeshVertex> = (0..vert_count)
                .map(|i| MeshVertex {
                    position: positions[i],
                    normal: normals[i],
                    uv: uvs[i],
                    color: colors[i],
                })
                .collect();

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|idx| idx.into_u32().collect())
                .unwrap_or_else(|| (0..vertices.len() as u32).collect());

            let name = mesh
                .name()
                .map(String::from)
                .unwrap_or_else(|| format!("mesh_{}", mesh.index()));

            meshes.push(Mesh {
                vertices,
                indices,
                name,
            });
        }
    }

    if meshes.is_empty() {
        return Err(format!("No meshes found in '{}'", path.display()));
    }

    log::info!("Loaded {} mesh(es) from '{}'", meshes.len(), path.display());

    Ok(meshes)
}
