use std::path::Path;

use super::buffer::as_bytes;
use super::skeleton::{JointChannel, Keyframe, SkeletalAnimationClip, Skeleton};
use super::{BufferElement, BufferLayout, ShaderDataType, VertexArray};
use gg_core::error::{EngineError, EngineResult};
use crate::Renderer;

// ---------------------------------------------------------------------------
// MeshVertex
// ---------------------------------------------------------------------------

/// Standard 3D vertex: position, normal, texture coordinates, vertex color, tangent.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Tangent vector for normal mapping. xyz = tangent direction,
    /// w = bitangent sign (+1 or -1) for reconstructing the bitangent
    /// via `cross(normal, tangent.xyz) * tangent.w`.
    pub tangent: [f32; 4],
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
            BufferElement::new(ShaderDataType::Float4, "a_tangent"),
        ])
    }

    /// Upload mesh data to the GPU, returning a ready-to-draw `VertexArray`.
    pub fn upload(&self, renderer: &mut Renderer) -> EngineResult<VertexArray> {
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
                    tangent: [0.0; 4],
                });
            }
            indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
        }

        compute_tangents(&mut vertices, &indices);
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
                    tangent: [0.0; 4],
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

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Sphere".into(),
        }
    }

    /// Flat plane on the XZ plane (Y = 0), centered at origin (side length 1).
    pub fn plane(color: [f32; 4]) -> Self {
        let normal = [0.0, 1.0, 0.0];
        let mut vertices = vec![
            MeshVertex {
                position: [-0.5, 0.0, -0.5],
                normal,
                uv: [0.0, 0.0],
                color,
                tangent: [0.0; 4],
            },
            MeshVertex {
                position: [0.5, 0.0, -0.5],
                normal,
                uv: [1.0, 0.0],
                color,
                tangent: [0.0; 4],
            },
            MeshVertex {
                position: [0.5, 0.0, 0.5],
                normal,
                uv: [1.0, 1.0],
                color,
                tangent: [0.0; 4],
            },
            MeshVertex {
                position: [-0.5, 0.0, 0.5],
                normal,
                uv: [0.0, 1.0],
                color,
                tangent: [0.0; 4],
            },
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Plane".into(),
        }
    }

    /// Cylinder centered at origin, axis along Y, radius 0.5, height 1.0.
    pub fn cylinder(segments: u32, color: [f32; 4]) -> Self {
        let segments = segments.max(3);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let t0 = [0.0; 4];

        let two_pi = 2.0 * std::f32::consts::PI;

        // Side vertices: two rings (top and bottom).
        for i in 0..=segments {
            let u = i as f32 / segments as f32;
            let theta = u * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();
            let nx = sin_t;
            let nz = cos_t;

            // Bottom ring.
            vertices.push(MeshVertex {
                position: [nx * 0.5, -0.5, nz * 0.5],
                normal: [nx, 0.0, nz],
                uv: [u, 1.0],
                color,
                tangent: t0,
            });
            // Top ring.
            vertices.push(MeshVertex {
                position: [nx * 0.5, 0.5, nz * 0.5],
                normal: [nx, 0.0, nz],
                uv: [u, 0.0],
                color,
                tangent: t0,
            });
        }

        // Side indices (CW winding from outside — matches cube/sphere convention).
        for i in 0..segments {
            let base = i * 2;
            indices.extend_from_slice(&[base, base + 1, base + 2]);
            indices.extend_from_slice(&[base + 1, base + 3, base + 2]);
        }

        // Top cap.
        let top_center = vertices.len() as u32;
        vertices.push(MeshVertex {
            position: [0.0, 0.5, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.5, 0.5],
            color,
            tangent: t0,
        });
        for i in 0..=segments {
            let theta = i as f32 / segments as f32 * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();
            vertices.push(MeshVertex {
                position: [sin_t * 0.5, 0.5, cos_t * 0.5],
                normal: [0.0, 1.0, 0.0],
                uv: [sin_t * 0.5 + 0.5, cos_t * 0.5 + 0.5],
                color,
                tangent: t0,
            });
        }
        for i in 0..segments {
            indices.extend_from_slice(&[top_center, top_center + 2 + i, top_center + 1 + i]);
        }

        // Bottom cap.
        let bot_center = vertices.len() as u32;
        vertices.push(MeshVertex {
            position: [0.0, -0.5, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            color,
            tangent: t0,
        });
        for i in 0..=segments {
            let theta = i as f32 / segments as f32 * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();
            vertices.push(MeshVertex {
                position: [sin_t * 0.5, -0.5, cos_t * 0.5],
                normal: [0.0, -1.0, 0.0],
                uv: [sin_t * 0.5 + 0.5, cos_t * 0.5 + 0.5],
                color,
                tangent: t0,
            });
        }
        for i in 0..segments {
            indices.extend_from_slice(&[bot_center, bot_center + 1 + i, bot_center + 2 + i]);
        }

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Cylinder".into(),
        }
    }

    /// Cone centered at origin, apex at Y=0.5, base at Y=-0.5, radius 0.5.
    pub fn cone(segments: u32, color: [f32; 4]) -> Self {
        let segments = segments.max(3);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let t0 = [0.0; 4];

        let two_pi = 2.0 * std::f32::consts::PI;
        // The slope angle for normals: atan(radius / height) = atan(0.5/1.0).
        let slope = (0.5_f32).atan2(1.0);
        let ny = slope.sin();
        let nr = slope.cos();

        // Side vertices: apex duplicated per-segment + base ring.
        for i in 0..=segments {
            let u = i as f32 / segments as f32;
            let theta = u * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();

            // Normal for this side slice.
            let nx = sin_t * nr;
            let nz = cos_t * nr;

            // Apex vertex (per-segment for correct normals + UVs).
            vertices.push(MeshVertex {
                position: [0.0, 0.5, 0.0],
                normal: [nx, ny, nz],
                uv: [u, 0.0],
                color,
                tangent: t0,
            });
            // Base vertex.
            vertices.push(MeshVertex {
                position: [sin_t * 0.5, -0.5, cos_t * 0.5],
                normal: [nx, ny, nz],
                uv: [u, 1.0],
                color,
                tangent: t0,
            });
        }

        // Side triangles (CW winding from outside).
        for i in 0..segments {
            let base = i * 2;
            // Each triangle: apex_i, base_{i+1}, base_i.
            indices.extend_from_slice(&[base, base + 3, base + 1]);
        }

        // Bottom cap.
        let bot_center = vertices.len() as u32;
        vertices.push(MeshVertex {
            position: [0.0, -0.5, 0.0],
            normal: [0.0, -1.0, 0.0],
            uv: [0.5, 0.5],
            color,
            tangent: t0,
        });
        for i in 0..=segments {
            let theta = i as f32 / segments as f32 * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();
            vertices.push(MeshVertex {
                position: [sin_t * 0.5, -0.5, cos_t * 0.5],
                normal: [0.0, -1.0, 0.0],
                uv: [sin_t * 0.5 + 0.5, cos_t * 0.5 + 0.5],
                color,
                tangent: t0,
            });
        }
        for i in 0..segments {
            indices.extend_from_slice(&[bot_center, bot_center + 1 + i, bot_center + 2 + i]);
        }

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Cone".into(),
        }
    }

    /// Torus centered at origin, lying in the XZ plane.
    /// Major radius 0.35, minor (tube) radius 0.15 — fits in a unit bounding box.
    pub fn torus(radial_segments: u32, tubular_segments: u32, color: [f32; 4]) -> Self {
        let radial = radial_segments.max(3);
        let tubular = tubular_segments.max(3);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let t0 = [0.0; 4];

        let major_r = 0.35;
        let minor_r = 0.15;
        let two_pi = 2.0 * std::f32::consts::PI;

        for i in 0..=radial {
            let u = i as f32 / radial as f32;
            let phi = u * two_pi;
            let (sin_phi, cos_phi) = phi.sin_cos();

            // Center of the tube circle at this radial step.
            let cx = major_r * cos_phi;
            let cz = major_r * sin_phi;

            for j in 0..=tubular {
                let v = j as f32 / tubular as f32;
                let theta = v * two_pi;
                let (sin_theta, cos_theta) = theta.sin_cos();

                let px = (major_r + minor_r * cos_theta) * cos_phi;
                let py = minor_r * sin_theta;
                let pz = (major_r + minor_r * cos_theta) * sin_phi;

                // Normal = (position - tube center), normalized.
                let nx = px - cx;
                let ny = py;
                let nz = pz - cz;
                let len = (nx * nx + ny * ny + nz * nz).sqrt();

                vertices.push(MeshVertex {
                    position: [px, py, pz],
                    normal: [nx / len, ny / len, nz / len],
                    uv: [u, v],
                    color,
                    tangent: t0,
                });
            }
        }

        // Indices.
        for i in 0..radial {
            for j in 0..tubular {
                let a = i * (tubular + 1) + j;
                let b = a + tubular + 1;
                indices.extend_from_slice(&[a, b, a + 1]);
                indices.extend_from_slice(&[a + 1, b, b + 1]);
            }
        }

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Torus".into(),
        }
    }

    /// Capsule centered at origin, axis along Y, total height 1.0 (0.5 cylinder + two 0.25-radius hemispheres).
    pub fn capsule(segments: u32, rings: u32, color: [f32; 4]) -> Self {
        let segments = segments.max(3);
        let rings = rings.max(2);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let t0 = [0.0; 4];

        let radius = 0.25;
        let half_height = 0.25; // Cylinder half-height (total capsule = 1.0).
        let two_pi = 2.0 * std::f32::consts::PI;
        let half_pi = std::f32::consts::FRAC_PI_2;

        // Top hemisphere (from pole down to equator).
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let phi = v * half_pi; // 0 (pole) to PI/2 (equator).
            let sin_phi = phi.sin();
            let cos_phi = phi.cos();

            for seg in 0..=segments {
                let u = seg as f32 / segments as f32;
                let theta = u * two_pi;

                let x = sin_phi * theta.cos();
                let y = cos_phi;
                let z = sin_phi * theta.sin();

                vertices.push(MeshVertex {
                    position: [x * radius, y * radius + half_height, z * radius],
                    normal: [x, y, z],
                    uv: [u, v * 0.25], // Top quarter of UV.
                    color,
                    tangent: t0,
                });
            }
        }

        // Top hemisphere indices.
        for ring in 0..rings {
            for seg in 0..segments {
                let curr = ring * (segments + 1) + seg;
                let next = curr + segments + 1;
                indices.extend_from_slice(&[curr, next, curr + 1]);
                indices.extend_from_slice(&[curr + 1, next, next + 1]);
            }
        }

        // Cylinder body (two rings).
        let cyl_base = vertices.len() as u32;
        for seg in 0..=segments {
            let u = seg as f32 / segments as f32;
            let theta = u * two_pi;
            let (sin_t, cos_t) = theta.sin_cos();
            let nx = sin_t;
            let nz = cos_t;

            // Top ring of cylinder (= equator of top hemisphere).
            vertices.push(MeshVertex {
                position: [nx * radius, half_height, nz * radius],
                normal: [nx, 0.0, nz],
                uv: [u, 0.25],
                color,
                tangent: t0,
            });
            // Bottom ring of cylinder (= equator of bottom hemisphere).
            vertices.push(MeshVertex {
                position: [nx * radius, -half_height, nz * radius],
                normal: [nx, 0.0, nz],
                uv: [u, 0.75],
                color,
                tangent: t0,
            });
        }

        for seg in 0..segments {
            let base = cyl_base + seg * 2;
            indices.extend_from_slice(&[base, base + 2, base + 1]);
            indices.extend_from_slice(&[base + 1, base + 2, base + 3]);
        }

        // Bottom hemisphere (from equator down to pole).
        let bot_base = vertices.len() as u32;
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let phi = half_pi + v * half_pi; // PI/2 (equator) to PI (pole).
            let sin_phi = phi.sin();
            let cos_phi = phi.cos();

            for seg in 0..=segments {
                let u = seg as f32 / segments as f32;
                let theta = u * two_pi;

                let x = sin_phi * theta.cos();
                let y = cos_phi;
                let z = sin_phi * theta.sin();

                vertices.push(MeshVertex {
                    position: [x * radius, y * radius - half_height, z * radius],
                    normal: [x, y, z],
                    uv: [u, 0.75 + v * 0.25], // Bottom quarter of UV.
                    color,
                    tangent: t0,
                });
            }
        }

        // Bottom hemisphere indices.
        for ring in 0..rings {
            for seg in 0..segments {
                let curr = bot_base + ring * (segments + 1) + seg;
                let next = curr + segments + 1;
                indices.extend_from_slice(&[curr, next, curr + 1]);
                indices.extend_from_slice(&[curr + 1, next, next + 1]);
            }
        }

        compute_tangents(&mut vertices, &indices);
        Self {
            vertices,
            indices,
            name: "Capsule".into(),
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
// SkinnedMeshVertex
// ---------------------------------------------------------------------------

/// 3D vertex with skeletal skinning data: standard attributes + bone
/// indices/weights for up to 4 bone influences per vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SkinnedMeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Tangent vector for normal mapping (xyz = direction, w = bitangent sign).
    pub tangent: [f32; 4],
    /// Indices into the bone matrix palette (up to 4 influences).
    pub bone_indices: [i32; 4],
    /// Blend weights (should sum to 1.0).
    pub bone_weights: [f32; 4],
}

// ---------------------------------------------------------------------------
// SkinnedMesh (CPU-side)
// ---------------------------------------------------------------------------

/// CPU-side skinned mesh data ready for GPU upload.
#[derive(Clone)]
pub struct SkinnedMesh {
    pub vertices: Vec<SkinnedMeshVertex>,
    pub indices: Vec<u32>,
    pub name: String,
}

impl SkinnedMesh {
    /// Vertex layout matching `SkinnedMeshVertex` fields.
    pub fn vertex_layout() -> BufferLayout {
        BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float3, "a_normal"),
            BufferElement::new(ShaderDataType::Float2, "a_uv"),
            BufferElement::new(ShaderDataType::Float4, "a_color"),
            BufferElement::new(ShaderDataType::Float4, "a_tangent"),
            BufferElement::new(ShaderDataType::Int4, "a_bone_indices"),
            BufferElement::new(ShaderDataType::Float4, "a_bone_weights"),
        ])
    }

    /// Upload skinned mesh data to the GPU.
    pub fn upload(&self, renderer: &mut Renderer) -> EngineResult<VertexArray> {
        let vertex_bytes = unsafe { as_bytes(&self.vertices) };
        let mut vb = renderer.create_vertex_buffer(vertex_bytes)?;
        vb.set_layout(Self::vertex_layout());

        let ib = renderer.create_index_buffer(&self.indices)?;

        let mut va = renderer.create_vertex_array();
        va.add_vertex_buffer(vb);
        va.set_index_buffer(ib);
        Ok(va)
    }

    /// Compute the axis-aligned bounding box of the mesh vertices.
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
}

// ---------------------------------------------------------------------------
// GltfSkinData — result of loading a skinned glTF model
// ---------------------------------------------------------------------------

/// All data extracted from a glTF file that contains a skin (skeleton).
pub struct GltfSkinData {
    pub mesh: SkinnedMesh,
    pub skeleton: Skeleton,
    pub clips: Vec<SkeletalAnimationClip>,
}

// ---------------------------------------------------------------------------
// Tangent generation (Mikktspace-style)
// ---------------------------------------------------------------------------

/// Compute tangent vectors for indexed triangle geometry from positions,
/// normals, and UVs. Uses the standard Mikktspace algorithm: accumulate
/// per-triangle tangent/bitangent from UV deltas, then Gram-Schmidt
/// orthogonalize against the vertex normal and store the bitangent sign
/// in the `.w` component.
///
/// Operates in-place on a `MeshVertex` slice using the given index buffer.
pub fn compute_tangents(vertices: &mut [MeshVertex], indices: &[u32]) {
    let n = vertices.len();
    let mut tan1 = vec![[0.0_f32; 3]; n]; // accumulated tangent
    let mut tan2 = vec![[0.0_f32; 3]; n]; // accumulated bitangent

    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= n || i1 >= n || i2 >= n {
            continue;
        }

        let p0 = vertices[i0].position;
        let p1 = vertices[i1].position;
        let p2 = vertices[i2].position;
        let uv0 = vertices[i0].uv;
        let uv1 = vertices[i1].uv;
        let uv2 = vertices[i2].uv;

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let duv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let duv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];

        let denom = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        if denom.abs() < 1e-8 {
            continue; // Degenerate UV triangle — skip.
        }
        let r = 1.0 / denom;

        // Tangent = (e1 * duv2.y - e2 * duv1.y) * r
        let t = [
            (e1[0] * duv2[1] - e2[0] * duv1[1]) * r,
            (e1[1] * duv2[1] - e2[1] * duv1[1]) * r,
            (e1[2] * duv2[1] - e2[2] * duv1[1]) * r,
        ];
        // Bitangent = (e2 * duv1.x - e1 * duv2.x) * r
        let b = [
            (e2[0] * duv1[0] - e1[0] * duv2[0]) * r,
            (e2[1] * duv1[0] - e1[1] * duv2[0]) * r,
            (e2[2] * duv1[0] - e1[2] * duv2[0]) * r,
        ];

        for &idx in &[i0, i1, i2] {
            tan1[idx][0] += t[0];
            tan1[idx][1] += t[1];
            tan1[idx][2] += t[2];
            tan2[idx][0] += b[0];
            tan2[idx][1] += b[1];
            tan2[idx][2] += b[2];
        }
    }

    // Gram-Schmidt orthogonalize and compute bitangent sign.
    for i in 0..n {
        let n_vec = glam::Vec3::from(vertices[i].normal);
        let t_vec = glam::Vec3::from(tan1[i]);

        // Orthogonalize: T' = normalize(T - N * dot(N, T))
        let tangent = (t_vec - n_vec * n_vec.dot(t_vec)).normalize_or_zero();

        // Bitangent sign: determines handedness of the TBN basis.
        let b_vec = glam::Vec3::from(tan2[i]);
        let w = if n_vec.cross(t_vec).dot(b_vec) < 0.0 {
            -1.0
        } else {
            1.0
        };

        if tangent.length_squared() > 0.0 {
            vertices[i].tangent = [tangent.x, tangent.y, tangent.z, w];
        } else {
            // Fallback for degenerate vertices.
            vertices[i].tangent = [1.0, 0.0, 0.0, 1.0];
        }
    }
}

/// Compute tangent vectors for a skinned mesh vertex slice (same algorithm
/// as [`compute_tangents`] but operating on [`SkinnedMeshVertex`]).
pub fn compute_tangents_skinned(vertices: &mut [SkinnedMeshVertex], indices: &[u32]) {
    let n = vertices.len();
    let mut tan1 = vec![[0.0_f32; 3]; n];
    let mut tan2 = vec![[0.0_f32; 3]; n];

    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= n || i1 >= n || i2 >= n {
            continue;
        }

        let p0 = vertices[i0].position;
        let p1 = vertices[i1].position;
        let p2 = vertices[i2].position;
        let uv0 = vertices[i0].uv;
        let uv1 = vertices[i1].uv;
        let uv2 = vertices[i2].uv;

        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let duv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let duv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];

        let denom = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        if denom.abs() < 1e-8 {
            continue;
        }
        let r = 1.0 / denom;

        let t = [
            (e1[0] * duv2[1] - e2[0] * duv1[1]) * r,
            (e1[1] * duv2[1] - e2[1] * duv1[1]) * r,
            (e1[2] * duv2[1] - e2[2] * duv1[1]) * r,
        ];
        let b = [
            (e2[0] * duv1[0] - e1[0] * duv2[0]) * r,
            (e2[1] * duv1[0] - e1[1] * duv2[0]) * r,
            (e2[2] * duv1[0] - e1[2] * duv2[0]) * r,
        ];

        for &idx in &[i0, i1, i2] {
            tan1[idx][0] += t[0];
            tan1[idx][1] += t[1];
            tan1[idx][2] += t[2];
            tan2[idx][0] += b[0];
            tan2[idx][1] += b[1];
            tan2[idx][2] += b[2];
        }
    }

    for i in 0..n {
        let n_vec = glam::Vec3::from(vertices[i].normal);
        let t_vec = glam::Vec3::from(tan1[i]);
        let tangent = (t_vec - n_vec * n_vec.dot(t_vec)).normalize_or_zero();
        let b_vec = glam::Vec3::from(tan2[i]);
        let w = if n_vec.cross(t_vec).dot(b_vec) < 0.0 {
            -1.0
        } else {
            1.0
        };
        if tangent.length_squared() > 0.0 {
            vertices[i].tangent = [tangent.x, tangent.y, tangent.z, w];
        } else {
            vertices[i].tangent = [1.0, 0.0, 0.0, 1.0];
        }
    }
}

// ---------------------------------------------------------------------------
// Normal generation for meshes without authored normals
// ---------------------------------------------------------------------------

/// Compute smooth (area-weighted) normals from indexed triangle geometry.
///
/// Accumulates area-weighted face normals per vertex (larger triangles contribute
/// more), then normalizes. Falls back to `(0, 1, 0)` for degenerate vertices
/// with no contributing faces.
fn compute_normals_from_geometry(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0_f32; 3]; positions.len()];

    for tri in indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }

        let p0 = positions[i0];
        let p1 = positions[i1];
        let p2 = positions[i2];

        // Edge vectors.
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];

        // Cross product (area-weighted face normal — magnitude = 2× triangle area).
        let cx = e1[1] * e2[2] - e1[2] * e2[1];
        let cy = e1[2] * e2[0] - e1[0] * e2[2];
        let cz = e1[0] * e2[1] - e1[1] * e2[0];

        // Accumulate into each vertex of the triangle.
        for &idx in &[i0, i1, i2] {
            normals[idx][0] += cx;
            normals[idx][1] += cy;
            normals[idx][2] += cz;
        }
    }

    // Normalize accumulated normals.
    for n in &mut normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-8 {
            n[0] /= len;
            n[1] /= len;
            n[2] /= len;
        } else {
            // Degenerate vertex (no contributing faces) — default to up.
            *n = [0.0, 1.0, 0.0];
        }
    }

    normals
}

// ---------------------------------------------------------------------------
// glTF loading
// ---------------------------------------------------------------------------

/// Load all meshes from a glTF / GLB file.
pub fn load_gltf(path: &Path) -> EngineResult<Vec<Mesh>> {
    let (document, buffers, _images) = gltf::import(path).map_err(|e| {
        EngineError::Gpu(format!("Failed to load glTF '{}': {}", path.display(), e))
    })?;

    let mut meshes = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .ok_or_else(|| {
                    EngineError::Gpu(format!(
                        "Mesh '{}' primitive has no positions",
                        mesh.index()
                    ))
                })?
                .collect();

            let vert_count = positions.len();

            // Read indices early — needed for normal generation when normals
            // are missing. The ORIGINAL (pre-flip) winding is used so that
            // computed face normals point outward in glTF convention.
            let raw_indices: Vec<u32> = reader
                .read_indices()
                .map(|idx| idx.into_u32().collect())
                .unwrap_or_else(|| (0..vert_count as u32).collect());

            let normals: Vec<[f32; 3]> =
                reader
                    .read_normals()
                    .map(|n| n.collect())
                    .unwrap_or_else(|| {
                        // No authored normals — compute smooth normals from the
                        // original (CCW) winding so they point outward correctly.
                        log::warn!(
                            "Mesh '{}' has no normals — generating from geometry",
                            mesh.name().unwrap_or("unnamed")
                        );
                        compute_normals_from_geometry(&positions, &raw_indices)
                    });

            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|tc| tc.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; vert_count]);

            let colors: Vec<[f32; 4]> = reader
                .read_colors(0)
                .map(|c| c.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.0, 1.0, 1.0, 1.0]; vert_count]);

            // Read tangents from glTF if present (vec4: xyz = direction, w = sign).
            let gltf_tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|t| t.collect());

            let mut vertices: Vec<MeshVertex> = (0..vert_count)
                .map(|i| MeshVertex {
                    position: positions[i],
                    normal: normals[i],
                    uv: uvs[i],
                    color: colors[i],
                    tangent: gltf_tangents.as_ref().map(|t| t[i]).unwrap_or([0.0; 4]),
                })
                .collect();

            // glTF uses CCW winding; our pipeline (with Vulkan Y-flip)
            // expects CW in model space. Swap each triangle's winding.
            let indices: Vec<u32> = {
                let mut flipped = Vec::with_capacity(raw_indices.len());
                for tri in raw_indices.chunks(3) {
                    if tri.len() == 3 {
                        flipped.push(tri[0]);
                        flipped.push(tri[2]);
                        flipped.push(tri[1]);
                    }
                }
                flipped
            };

            // Compute tangents from geometry if the glTF file didn't provide them.
            if gltf_tangents.is_none() {
                compute_tangents(&mut vertices, &indices);
            } else {
                // glTF tangents were authored for CCW winding. After the CW
                // flip above, the bitangent sign is inverted, so negate .w.
                for v in &mut vertices {
                    v.tangent[3] = -v.tangent[3];
                }
            }

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
        return Err(EngineError::Gpu(format!(
            "No meshes found in '{}'",
            path.display()
        )));
    }

    log::info!("Loaded {} mesh(es) from '{}'", meshes.len(), path.display());

    Ok(meshes)
}

/// Load a glTF/GLB file that contains a skin (skeleton + bone weights) and
/// optionally animations. Returns the skinned mesh, skeleton hierarchy, and
/// animation clips.
///
/// If the file has no skin, returns an error — use [`load_gltf`] instead.
pub fn load_gltf_skinned(path: &Path) -> EngineResult<GltfSkinData> {
    let (document, buffers, _images) = gltf::import(path).map_err(|e| {
        EngineError::Gpu(format!("Failed to load glTF '{}': {}", path.display(), e))
    })?;

    // --- Find the first skin -------------------------------------------------
    let skin = document
        .skins()
        .next()
        .ok_or_else(|| EngineError::Gpu(format!("No skin found in '{}'", path.display())))?;

    // Build joint node-index → skeleton joint-index mapping.
    let joint_nodes: Vec<gltf::Node> = skin.joints().collect();
    let joint_count = joint_nodes.len();

    // Map from glTF node index to our 0..N joint index.
    let mut node_to_joint: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    for (j, node) in joint_nodes.iter().enumerate() {
        node_to_joint.insert(node.index(), j);
    }

    // Joint names and parent indices.
    let joint_names: Vec<String> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            n.name()
                .map(String::from)
                .unwrap_or_else(|| format!("joint_{}", i))
        })
        .collect();

    // Determine parent indices by walking the scene hierarchy.
    let mut parent_indices = vec![-1i32; joint_count];
    fn find_parents(
        node: &gltf::Node,
        parent_joint: i32,
        node_to_joint: &std::collections::HashMap<usize, usize>,
        parent_indices: &mut Vec<i32>,
    ) {
        let my_joint = node_to_joint.get(&node.index()).copied();
        let next_parent = if let Some(j) = my_joint {
            parent_indices[j] = parent_joint;
            j as i32
        } else {
            parent_joint
        };
        for child in node.children() {
            find_parents(&child, next_parent, node_to_joint, parent_indices);
        }
    }
    for scene in document.scenes() {
        for root in scene.nodes() {
            find_parents(&root, -1, &node_to_joint, &mut parent_indices);
        }
    }

    // Inverse bind matrices.
    let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));
    let inverse_bind_matrices: Vec<glam::Mat4> = reader
        .read_inverse_bind_matrices()
        .map(|ibm| ibm.map(|m| glam::Mat4::from_cols_array_2d(&m)).collect())
        .unwrap_or_else(|| vec![glam::Mat4::IDENTITY; joint_count]);

    // Extract rest pose local transforms from each joint node.
    let rest_local_transforms: Vec<glam::Mat4> = joint_nodes
        .iter()
        .map(|node| {
            let (t, r, s) = node.transform().decomposed();
            let translation = glam::Vec3::from(t);
            let rotation = glam::Quat::from_array(r);
            let scale = glam::Vec3::from(s);
            glam::Mat4::from_scale_rotation_translation(scale, rotation, translation)
        })
        .collect();

    // --- Build global transforms for all nodes in the scene graph -------------
    let mut node_globals: std::collections::HashMap<usize, glam::Mat4> =
        std::collections::HashMap::new();
    fn compute_node_globals(
        node: &gltf::Node,
        parent_global: glam::Mat4,
        node_globals: &mut std::collections::HashMap<usize, glam::Mat4>,
    ) {
        let (t, r, s) = node.transform().decomposed();
        let local = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::from(s),
            glam::Quat::from_array(r),
            glam::Vec3::from(t),
        );
        let global = parent_global * local;
        node_globals.insert(node.index(), global);
        for child in node.children() {
            compute_node_globals(&child, global, node_globals);
        }
    }
    for scene in document.scenes() {
        for root in scene.nodes() {
            compute_node_globals(&root, glam::Mat4::IDENTITY, &mut node_globals);
        }
    }

    // --- Extract mesh with skin weights --------------------------------------
    // Find the mesh node that references this skin.
    let skin_index = skin.index();
    let mesh_node = document
        .nodes()
        .find(|n| n.skin().map(|s| s.index()) == Some(skin_index) && n.mesh().is_some())
        .ok_or_else(|| {
            EngineError::Gpu(format!(
                "No mesh node with skin found in '{}'",
                path.display()
            ))
        })?;
    let gltf_mesh = mesh_node.mesh().unwrap();

    // --- Compute bind-space correction per glTF spec -------------------------
    // glTF skinning: jointMatrix = inverse(meshNodeGlobal) × jointGlobal × IBM
    // Our FK computes joint globals relative to the root joint. The true joint
    // global includes any ancestor transforms above the root joint in the scene
    // graph. So: trueJointGlobal[j] = rootJointParentGlobal × FK_world[j].
    // Correction = inverse(meshNodeGlobal) × rootJointParentGlobal.
    let mesh_node_global = node_globals
        .get(&mesh_node.index())
        .copied()
        .unwrap_or(glam::Mat4::IDENTITY);

    // Find root joint's scene-graph parent global by dividing out the root
    // joint's own local transform from its scene-graph global.
    let root_joint_idx = parent_indices.iter().position(|&p| p == -1).unwrap_or(0);
    let root_joint_node_index = joint_nodes[root_joint_idx].index();
    let root_joint_global = node_globals
        .get(&root_joint_node_index)
        .copied()
        .unwrap_or(glam::Mat4::IDENTITY);
    let root_joint_local = rest_local_transforms[root_joint_idx];
    let root_joint_parent_global = root_joint_global * root_joint_local.inverse();

    let bind_space_correction = mesh_node_global.inverse() * root_joint_parent_global;

    log::info!(
        "Skinned mesh bind-space correction for '{}': mesh_node_global = {:?}, root_joint_parent_global = {:?}, correction = {:?}",
        path.display(),
        mesh_node_global,
        root_joint_parent_global,
        bind_space_correction,
    );

    let skeleton = Skeleton {
        joint_names,
        parent_indices,
        inverse_bind_matrices,
        rest_local_transforms,
        bind_space_correction,
    };

    let mut all_vertices = Vec::new();
    let mut all_indices = Vec::new();

    for primitive in gltf_mesh.primitives() {
        let prim_reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

        let positions: Vec<[f32; 3]> = prim_reader
            .read_positions()
            .ok_or_else(|| {
                EngineError::Gpu(format!(
                    "Skinned mesh '{}' primitive has no positions",
                    gltf_mesh.index()
                ))
            })?
            .collect();

        let vert_count = positions.len();

        // Read indices before normals — needed for generating normals
        // when the mesh has none.
        let prim_raw_indices: Vec<u32> = prim_reader
            .read_indices()
            .map(|idx| idx.into_u32().collect())
            .unwrap_or_else(|| (0..vert_count as u32).collect());

        let normals: Vec<[f32; 3]> = prim_reader
            .read_normals()
            .map(|n| n.collect())
            .unwrap_or_else(|| {
                log::warn!(
                    "Skinned mesh '{}' has no normals — generating from geometry",
                    gltf_mesh.name().unwrap_or("unnamed")
                );
                compute_normals_from_geometry(&positions, &prim_raw_indices)
            });

        let uvs: Vec<[f32; 2]> = prim_reader
            .read_tex_coords(0)
            .map(|tc| tc.into_f32().collect())
            .unwrap_or_else(|| vec![[0.0, 0.0]; vert_count]);

        let colors: Vec<[f32; 4]> = prim_reader
            .read_colors(0)
            .map(|c| c.into_rgba_f32().collect())
            .unwrap_or_else(|| vec![[1.0, 1.0, 1.0, 1.0]; vert_count]);

        // Bone indices (JOINTS_0).
        let joints: Vec<[i32; 4]> = prim_reader
            .read_joints(0)
            .map(|j| {
                j.into_u16()
                    .map(|[a, b, c, d]| [a as i32, b as i32, c as i32, d as i32])
                    .collect()
            })
            .unwrap_or_else(|| vec![[0, 0, 0, 0]; vert_count]);

        // Bone weights (WEIGHTS_0).
        let weights: Vec<[f32; 4]> = prim_reader
            .read_weights(0)
            .map(|w| w.into_f32().collect())
            .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 0.0]; vert_count]);

        // Tangents from glTF (vec4: xyz = direction, w = bitangent sign).
        let gltf_tangents: Option<Vec<[f32; 4]>> = prim_reader.read_tangents().map(|t| t.collect());

        let base = all_vertices.len() as u32;
        for i in 0..vert_count {
            all_vertices.push(SkinnedMeshVertex {
                position: positions[i],
                normal: normals[i],
                uv: uvs[i],
                color: colors[i],
                tangent: gltf_tangents.as_ref().map(|t| t[i]).unwrap_or([0.0; 4]),
                bone_indices: joints[i],
                bone_weights: weights[i],
            });
        }

        // Keep glTF's native CCW winding order for skinned meshes.
        // The skinned pipeline uses FrontFace::CLOCKWISE to account for
        // the Vulkan Y-flip (CCW model → CW clip → front). This avoids
        // a winding flip that would desynchronize vertex normals from the
        // rasterizer's front-face classification after bone skinning.
        let indices: Vec<u32> = prim_raw_indices.iter().map(|&i| i + base).collect();

        all_indices.extend(indices);

        // Compute tangents if glTF didn't provide them.
        if gltf_tangents.is_none() {
            compute_tangents_skinned(&mut all_vertices[base as usize..], &prim_raw_indices);
        }
    }

    let mesh_name = gltf_mesh
        .name()
        .map(String::from)
        .unwrap_or_else(|| format!("skinned_mesh_{}", gltf_mesh.index()));

    let skinned_mesh = SkinnedMesh {
        vertices: all_vertices,
        indices: all_indices,
        name: mesh_name,
    };

    // --- Extract animations --------------------------------------------------
    let mut clips = Vec::new();

    for anim in document.animations() {
        let clip_name = anim
            .name()
            .map(String::from)
            .unwrap_or_else(|| format!("animation_{}", anim.index()));

        let mut duration = 0.0f32;
        // Collect channels per joint.
        let mut joint_channels: std::collections::HashMap<usize, JointChannel> =
            std::collections::HashMap::new();

        for channel in anim.channels() {
            let target_node = channel.target().node().index();
            let joint_index = match node_to_joint.get(&target_node) {
                Some(&j) => j,
                None => continue, // Channel targets a non-joint node.
            };

            let ch_reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
            let inputs: Vec<f32> = ch_reader
                .read_inputs()
                .map(|i| i.collect())
                .unwrap_or_default();

            if let Some(&last) = inputs.last() {
                duration = duration.max(last);
            }

            let entry = joint_channels
                .entry(joint_index)
                .or_insert_with(|| JointChannel {
                    joint_index,
                    translations: Vec::new(),
                    rotations: Vec::new(),
                    scales: Vec::new(),
                });

            match ch_reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(values)) => {
                    for (t, v) in inputs.iter().zip(values) {
                        entry.translations.push(Keyframe {
                            time: *t,
                            value: glam::Vec3::from(v),
                        });
                    }
                }
                Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                    for (t, v) in inputs.iter().zip(values.into_f32()) {
                        // glTF quaternion order: [x, y, z, w]
                        entry.rotations.push(Keyframe {
                            time: *t,
                            value: glam::Quat::from_xyzw(v[0], v[1], v[2], v[3]),
                        });
                    }
                }
                Some(gltf::animation::util::ReadOutputs::Scales(values)) => {
                    for (t, v) in inputs.iter().zip(values) {
                        entry.scales.push(Keyframe {
                            time: *t,
                            value: glam::Vec3::from(v),
                        });
                    }
                }
                _ => {}
            }
        }

        let channels: Vec<JointChannel> = joint_channels.into_values().collect();

        clips.push(SkeletalAnimationClip {
            name: clip_name,
            duration,
            channels,
        });
    }

    log::info!(
        "Loaded skinned mesh '{}' from '{}': {} verts, {} indices, {} joints, {} clips",
        skinned_mesh.name,
        path.display(),
        skinned_mesh.vertices.len(),
        skinned_mesh.indices.len(),
        skeleton.joint_count(),
        clips.len(),
    );

    Ok(GltfSkinData {
        mesh: skinned_mesh,
        skeleton,
        clips,
    })
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

    #[test]
    fn load_fox_glb_static() {
        let path = Path::new("../test_assets/Fox.glb");
        if !path.exists() {
            return;
        }
        // Fox.glb should load as static geometry (skin data ignored).
        let meshes = load_gltf(path).expect("Failed to load Fox.glb");
        assert!(!meshes.is_empty());
        assert!(meshes[0].vertices.len() > 100);
    }

    #[test]
    fn load_fox_glb_skinned() {
        let path = Path::new("../test_assets/Fox.glb");
        if !path.exists() {
            return;
        }
        let data = load_gltf_skinned(path).expect("Failed to load Fox.glb skinned");
        assert!(
            data.mesh.vertices.len() > 100,
            "Fox mesh should have vertices"
        );
        assert!(data.skeleton.joint_count() > 10, "Fox should have joints");
        assert!(!data.clips.is_empty(), "Fox should have animation clips");

        // Verify bone weights are normalized.
        for v in &data.mesh.vertices {
            let sum: f32 = v.bone_weights.iter().sum();
            assert!(
                (sum - 1.0).abs() < 0.01,
                "Bone weights should sum to ~1.0, got {}",
                sum
            );
        }

        // Print info for debugging.
        println!(
            "Fox: {} verts, {} indices, {} joints, {} clips",
            data.mesh.vertices.len(),
            data.mesh.indices.len(),
            data.skeleton.joint_count(),
            data.clips.len(),
        );
        for clip in &data.clips {
            println!("  clip '{}': {:.2}s", clip.name, clip.duration);
        }
    }

    #[test]
    fn fox_glb_scene_graph_diagnostic() {
        let path = Path::new("../test_assets/Fox.glb");
        if !path.exists() {
            return;
        }
        let (document, _buffers, _) = gltf::import(path).expect("Failed to import Fox.glb");

        // Print full scene graph with transforms.
        println!("=== Fox.glb scene graph ===");
        fn print_node(node: &gltf::Node, depth: usize) {
            let indent = "  ".repeat(depth);
            let (t, r, s) = node.transform().decomposed();
            let has_mesh = node.mesh().is_some();
            let has_skin = node.skin().is_some();
            println!(
                "{}{}: '{}' mesh={} skin={} T=[{:.3},{:.3},{:.3}] R=[{:.3},{:.3},{:.3},{:.3}] S=[{:.3},{:.3},{:.3}]",
                indent,
                node.index(),
                node.name().unwrap_or("(unnamed)"),
                has_mesh,
                has_skin,
                t[0], t[1], t[2],
                r[0], r[1], r[2], r[3],
                s[0], s[1], s[2],
            );
            for child in node.children() {
                print_node(&child, depth + 1);
            }
        }
        for scene in document.scenes() {
            for root in scene.nodes() {
                print_node(&root, 0);
            }
        }

        // Load skinned data and check bone matrices.
        let data = load_gltf_skinned(path).expect("load_gltf_skinned");

        println!("\n=== bind_space_correction ===");
        println!("{:?}", data.skeleton.bind_space_correction);
        let det = data.skeleton.bind_space_correction.determinant();
        println!("determinant: {:.6}", det);

        // Check bind pose.
        let bind_pose = data.skeleton.bind_pose();
        println!("\n=== bind pose bone matrices (first 5) ===");
        for (j, m) in bind_pose.matrices.iter().enumerate().take(5) {
            let d = m.determinant();
            let near_identity = (*m - glam::Mat4::IDENTITY).abs_diff_eq(glam::Mat4::ZERO, 0.01);
            println!(
                "  joint {} '{}': det={:.6} near_identity={}",
                j, data.skeleton.joint_names[j], d, near_identity
            );
            if !near_identity {
                println!("    {:?}", m);
            }
        }

        // Check animated pose (first clip, t=0).
        if let Some(clip) = data.clips.first() {
            let pose = data.skeleton.compute_pose(clip, 0.0);
            println!("\n=== animated pose t=0 bone matrices (first 5) ===");
            for (j, m) in pose.matrices.iter().enumerate().take(5) {
                let d = m.determinant();
                println!(
                    "  joint {} '{}': det={:.6}",
                    j, data.skeleton.joint_names[j], d
                );
            }

            // Check if any bone has negative determinant.
            let any_neg = pose.matrices.iter().any(|m| m.determinant() < 0.0);
            println!("\nAny negative determinant: {}", any_neg);
        }

        // Check first triangle winding.
        if data.mesh.indices.len() >= 3 {
            let i0 = data.mesh.indices[0] as usize;
            let i1 = data.mesh.indices[1] as usize;
            let i2 = data.mesh.indices[2] as usize;
            let p0 = glam::Vec3::from(data.mesh.vertices[i0].position);
            let p1 = glam::Vec3::from(data.mesh.vertices[i1].position);
            let p2 = glam::Vec3::from(data.mesh.vertices[i2].position);
            let n0 = glam::Vec3::from(data.mesh.vertices[i0].normal);
            let edge1 = p1 - p0;
            let edge2 = p2 - p0;
            let face_normal = edge1.cross(edge2);
            let dot_with_vertex_normal = face_normal.dot(n0);
            println!(
                "\n=== first triangle winding check ===\nface_normal: {:?}\nvertex_normal: {:?}\ndot: {:.4} (positive = same direction = CW after flip is correct)",
                face_normal, n0, dot_with_vertex_normal,
            );
        }
    }
}
