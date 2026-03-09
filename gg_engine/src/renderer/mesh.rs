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
        type CubeFace = ([f32; 3], [[f32; 3]; 4], [[f32; 2]; 4]);
        #[rustfmt::skip]
        let faces: [CubeFace; 6] = [
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

    /// Compute the axis-aligned bounding box of the mesh vertices.
    ///
    /// Returns `(min, max)` in local space. Returns zeros for empty meshes.
    pub fn compute_bounds(&self) -> (glam::Vec3, glam::Vec3) {
        if self.vertices.is_empty() {
            return (glam::Vec3::ZERO, glam::Vec3::ZERO);
        }
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        for v in &self.vertices {
            let p = glam::Vec3::from(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        (min, max)
    }

    /// Merge multiple meshes into a single mesh, concatenating vertices and
    /// adjusting indices. Used to combine all primitives from a glTF file.
    pub fn merge(meshes: Vec<Mesh>, name: String) -> Self {
        let total_verts: usize = meshes.iter().map(|m| m.vertices.len()).sum();
        let total_idx: usize = meshes.iter().map(|m| m.indices.len()).sum();

        let mut vertices = Vec::with_capacity(total_verts);
        let mut indices = Vec::with_capacity(total_idx);

        for mesh in meshes {
            let base = vertices.len() as u32;
            vertices.extend(mesh.vertices);
            indices.extend(mesh.indices.iter().map(|&i| i + base));
        }

        Self {
            vertices,
            indices,
            name,
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

            // glTF uses CCW winding; our pipeline (with Vulkan Y-flip)
            // expects CW in model space. Swap each triangle's winding.
            let indices: Vec<u32> = reader
                .read_indices()
                .map(|idx| {
                    let raw: Vec<u32> = idx.into_u32().collect();
                    let mut flipped = Vec::with_capacity(raw.len());
                    for tri in raw.chunks(3) {
                        if tri.len() == 3 {
                            flipped.push(tri[0]);
                            flipped.push(tri[2]);
                            flipped.push(tri[1]);
                        }
                    }
                    flipped
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_gltf_triangle() {
        let path = Path::new("../assets/meshes/triangle.gltf");
        if !path.exists() {
            return; // Skip if assets not available (e.g. CI).
        }
        let meshes = load_gltf(path).expect("Failed to load triangle.gltf");
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
        // Vertex colors should be present (red, green, blue).
        assert!(mesh.vertices[0].color[0] > 0.9); // red
        assert!(mesh.vertices[1].color[1] > 0.9); // green
        assert!(mesh.vertices[2].color[2] > 0.9); // blue
    }

    #[test]
    fn load_gltf_quad() {
        let path = Path::new("../assets/meshes/quad.gltf");
        if !path.exists() {
            return;
        }
        let meshes = load_gltf(path).expect("Failed to load quad.gltf");
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        // Normals should point up (0, 1, 0).
        for v in &mesh.vertices {
            assert!((v.normal[1] - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn load_gltf_icosphere() {
        let path = Path::new("../assets/meshes/suzanne_low.gltf");
        if !path.exists() {
            return;
        }
        let meshes = load_gltf(path).expect("Failed to load suzanne_low.gltf");
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.vertices.len(), 12); // icosahedron
        assert_eq!(mesh.indices.len(), 60); // 20 faces × 3
    }

    #[test]
    fn mesh_merge() {
        let a = Mesh::cube([1.0; 4]);
        let b = Mesh::plane([1.0; 4]);
        let a_verts = a.vertices.len();
        let a_idx = a.indices.len();
        let b_verts = b.vertices.len();
        let b_idx = b.indices.len();
        let merged = Mesh::merge(vec![a, b], "merged".into());
        assert_eq!(merged.vertices.len(), a_verts + b_verts);
        assert_eq!(merged.indices.len(), a_idx + b_idx);
        assert_eq!(merged.name, "merged");
    }

    #[test]
    fn mesh_compute_bounds() {
        let cube = Mesh::cube([1.0; 4]);
        let (min, max) = cube.compute_bounds();
        assert!((min.x - (-0.5)).abs() < 0.01);
        assert!((max.x - 0.5).abs() < 0.01);
        assert!((min.y - (-0.5)).abs() < 0.01);
        assert!((max.y - 0.5).abs() < 0.01);
    }
}
