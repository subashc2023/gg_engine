# 3D Rendering

The 3D rendering pipeline builds on the core Vulkan renderer (see `06-rendering.md`) with dedicated systems for mesh rendering, PBR materials, lighting, environment maps, shadow mapping, skeletal animation, and post-processing.

**Key files:** `scene/components.rs`, `scene/rendering.rs`, `renderer/renderer.rs`, `renderer/mesh.rs`, `renderer/material.rs`, `renderer/lighting.rs`, `renderer/environment_map.rs`, `renderer/shadow_map.rs`, `renderer/skeleton.rs`, `renderer/bone_palette.rs`, `renderer/postprocess.rs`, `renderer/cubemap.rs`

## 1. Mesh Rendering

**Files:** `scene/components.rs`, `renderer/mesh.rs`, `renderer/renderer.rs`, `renderer/shaders/mesh3d.glsl`

### MeshRendererComponent

Attached to an entity alongside `TransformComponent` to render a 3D mesh.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `mesh_source` | `MeshSource` | `Primitive(Cube)` | Geometry source (primitive or glTF asset) |
| `color` | `Vec4` | `ONE` | Vertex color / albedo tint |
| `metallic` | `f32` | `0.0` | 0.0 = dielectric, 1.0 = metal |
| `roughness` | `f32` | `0.5` | 0.0 = mirror-smooth, 1.0 = fully rough |
| `emissive_color` | `Vec3` | `ZERO` | Emissive color (HDR, black = no emission) |
| `emissive_strength` | `f32` | `1.0` | Emissive intensity multiplier for bloom |
| `texture_handle` | `Uuid` | `0` | Albedo texture asset handle |
| `normal_texture_handle` | `Uuid` | `0` | Normal map asset handle |
| `cast_alpha_shadow` | `bool` | `false` | Use alpha-tested shadow pipeline |
| `texture` | `Option<Ref<Texture2D>>` | `None` | Runtime-only loaded albedo texture |
| `normal_texture` | `Option<Ref<Texture2D>>` | `None` | Runtime-only loaded normal map |
| `loaded_mesh` | `Option<Ref<Mesh>>` | `None` | Runtime-only CPU mesh from glTF |
| `local_bounds` | `Option<(Vec3, Vec3)>` | `None` | Runtime-only AABB from mesh vertices |
| `vertex_array` | `Option<VertexArray>` | `None` | Runtime-only uploaded GPU vertex data |

Runtime-only fields (`texture`, `normal_texture`, `loaded_mesh`, `local_bounds`, `vertex_array`) are not serialized and are reset to `None` on clone.

### MeshSource

```rust
enum MeshSource {
    Primitive(MeshPrimitive),
    Asset(Uuid),  // glTF/GLB asset handle
}
```

### MeshPrimitive

| Variant | Description |
|---------|-------------|
| `Cube` | Unit cube (default) |
| `Sphere` | UV sphere |
| `Plane` | Flat XZ plane |
| `Cylinder` | Cylinder |
| `Cone` | Cone |
| `Torus` | Torus |
| `Capsule` | Capsule |

Each primitive provides `local_bounds()` returning an axis-aligned `(min, max)` bounding box.

### Descriptor Set Layout (3D Pipeline)

| Set | Binding | Type | Content |
|-----|---------|------|---------|
| 0 | 0 | `UNIFORM_BUFFER` | Camera UBO (VP matrix + time) |
| 1 | 0 | `COMBINED_IMAGE_SAMPLER[4096]` | Bindless texture array |
| 2 | 0 | `UNIFORM_BUFFER` | Material UBO (64-byte `MaterialGpuData`) |
| 3 | 0 | `UNIFORM_BUFFER` | Lighting UBO (896-byte `LightGpuData`) |
| 3 | 1-4 | `COMBINED_IMAGE_SAMPLER` | IBL textures (see section 4) |
| 4 | 0 | `COMBINED_IMAGE_SAMPLER` | Shadow map (`sampler2DShadow`) |

### mesh3d.glsl Push Constants (164 bytes)

| Offset | Size | Type | Field |
|--------|------|------|-------|
| 0 | 64 | `mat4` | Model matrix |
| 64 | 36 | `mat3` | Normal matrix (CPU-precomputed) |
| 100 | 4 | `int` | Entity ID (for mouse picking) |
| 104 | 4 | `float` | Metallic |
| 108 | 4 | `float` | Roughness |
| 112 | 4 | `float` | Emissive strength |
| 116 | 16 | `vec4` | Albedo color |
| 132 | 12 | `vec3` | Emissive color |
| 144 | 4 | `int` | Albedo texture index (-1 = none) |
| 148 | 4 | `int` | Normal texture index (-1 = none) |
| 152 | 12 | padding | Alignment |

### Renderer 3D API

| Method | Description |
|--------|-------------|
| `create_3d_pipeline(shader, layout, cull, depth, blend, color_attachment_count)` | Create a pipeline with sets 0-4 auto-wired |
| `bind_3d_shared_sets(pipeline)` | Bind pipeline + sets 0, 1, 3, 4 once before a batch of draws |
| `submit_3d(pipeline, vertex_array, model, mesh_component)` | Push per-draw constants and issue indexed draw |

Call `bind_3d_shared_sets` once, then `submit_3d` for each mesh. Set 2 (material) is bound per-draw inside `submit_3d`.

### MeshVertex

Standard 3D vertex: `position[3]`, `normal[3]`, `uv[2]`, `color[4]`, `tangent[4]` (xyz = direction, w = bitangent sign). Mesh data is loaded lazily and uploaded to device-local GPU memory.

---

## 2. Material System

**File:** `renderer/material.rs`

### Material

PBR metallic-roughness surface description.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | `"Default"` | Human-readable name |
| `albedo_color` | `Vec4` | `ONE` | Base color tint |
| `albedo_texture` | `Option<Ref<Texture2D>>` | `None` | Albedo texture |
| `albedo_texture_handle` | `Uuid` | `0` | Asset handle for serialization |
| `metallic` | `f32` | `0.0` | Metallic factor |
| `roughness` | `f32` | `0.5` | Roughness factor |
| `normal_texture` | `Option<Ref<Texture2D>>` | `None` | Normal map |
| `normal_texture_handle` | `Uuid` | `0` | Normal map asset handle |
| `emissive_color` | `Vec3` | `ZERO` | Emissive color (HDR) |
| `emissive_strength` | `f32` | `1.0` | Emissive intensity multiplier |
| `blend_mode` | `BlendMode` | `Opaque` | Fragment blending mode |
| `double_sided` | `bool` | `false` | Disable backface culling |
| `alpha_cutoff` | `f32` | `0.5` | Alpha-test threshold |
| `depth_test` | `bool` | `true` | Enable depth testing |
| `depth_write` | `bool` | `true` | Enable depth writing |

### BlendMode

| Variant | Description |
|---------|-------------|
| `Opaque` | No alpha blending, writes depth (default) |
| `AlphaBlend` | Standard `src_alpha / one_minus_src_alpha` |
| `Additive` | Additive `src_alpha / one` (particles, glow) |

### MaterialGpuData (64 bytes, std140)

GPU-side UBO struct written to descriptor set 2, binding 0.

| Offset | Size | Field |
|--------|------|-------|
| 0 | 16 | `albedo_color: [f32; 4]` |
| 16 | 12 | `emissive_color: [f32; 3]` |
| 28 | 4 | `metallic: f32` |
| 32 | 4 | `roughness: f32` |
| 36 | 4 | `emissive_strength: f32` |
| 40 | 4 | `alpha_cutoff: f32` |
| 44 | 4 | `albedo_tex_index: i32` (-1 = none) |
| 48 | 4 | `normal_tex_index: i32` (-1 = none) |
| 52 | 12 | Padding (align to 64) |

### MaterialLibrary

Central registry of materials with GPU UBO infrastructure.

- Owns the material descriptor set layout (set 2) and per-slot UBO buffers
- `MaterialHandle = Uuid` identifies materials
- Default material: white, opaque, roughness 0.5
- `.ggmaterial` asset extension for serialized materials
- Renderer exposes `material_library()` / `material_library_mut()` / `write_material()`

---

## 3. Lighting System

**File:** `renderer/lighting.rs`

### Light Components

#### DirectionalLightComponent

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `color` | `Vec3` | `ONE` | Light color (linear RGB) |
| `intensity` | `f32` | `1.0` | Brightness multiplier |
| `cast_shadows` | `bool` | `true` | Enable shadow map for this light |
| `shadow_cull_front_faces` | `bool` | `true` | Front-face culling in shadow pass (eliminates acne) |

Direction is derived from the entity's rotation: `rotation * Vec3::NEG_Y`. With identity rotation the light points straight down (noon sun). Rotate the entity to aim the light.

#### PointLightComponent

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `color` | `Vec3` | `ONE` | Light color (linear RGB) |
| `intensity` | `f32` | `1.0` | Brightness multiplier |
| `radius` | `f32` | `10.0` | Maximum influence radius |

Position is taken from the entity's `TransformComponent`. Uses smooth quadratic attenuation: `max(0, 1 - (d/radius)^2)^2`.

#### AmbientLightComponent

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `color` | `Vec3` | `(0.03, 0.03, 0.03)` | Ambient light color |
| `intensity` | `f32` | `1.0` | Intensity multiplier |

Only the first entity with this component is used (scene-wide setting). If no entity has it, the default `(0.03, 0.03, 0.03)` ambient is applied.

### LightingSystem

Manages per-frame per-viewport lighting UBO at descriptor set 3.

Descriptor set 3 layout:

| Binding | Type | Content |
|---------|------|---------|
| 0 | `UNIFORM_BUFFER` | `LightGpuData` (lighting UBO) |
| 1 | `COMBINED_IMAGE_SAMPLER` | Irradiance cubemap (IBL diffuse) |
| 2 | `COMBINED_IMAGE_SAMPLER` | Pre-filtered specular cubemap (IBL specular) |
| 3 | `COMBINED_IMAGE_SAMPLER` | BRDF integration LUT (IBL split-sum) |
| 4 | `COMBINED_IMAGE_SAMPLER` | Source environment cubemap (skybox) |

### LightGpuData (896 bytes, std140)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 16 | `dir_direction` | xyz = direction, w = unused |
| 16 | 16 | `dir_color` | xyz = color, w = intensity |
| 32 | 256 | `point_positions[16]` | xyz = position, w = radius |
| 288 | 256 | `point_colors[16]` | xyz = color, w = intensity |
| 544 | 16 | `ambient_color` | xyz = color, w = intensity |
| 560 | 16 | `camera_position` | xyz = eye position, w = unused |
| 576 | 16 | `counts` | x = num_point_lights, y = has_directional, z = has_shadow, w = csm_debug |
| 592 | 256 | `shadow_light_vp[4]` | 4 cascade light-space VP matrices |
| 848 | 16 | `cascade_split_depth` | xyz = 3 split depths (NDC), w = shadow_distance |
| 864 | 16 | `cascade_texel_size` | World-units-per-texel per cascade |
| 880 | 16 | `shadow_settings` | x = quality (0-3), yzw = reserved |

**Constants:** `MAX_POINT_LIGHTS = 16`, `NUM_SHADOW_CASCADES = 4`.

### LightEnvironment

CPU-side collector built by `Scene::collect_lights()` each frame before 3D rendering. Fields:

- `directional: Option<(direction, color, intensity)>`
- `point_lights: Vec<(position, color, intensity, radius)>` (clamped to 16)
- `ambient_color`, `ambient_intensity`, `camera_position`
- `shadow_cascade_vps`, `cascade_split_depths`, `shadow_distance`

Converted to GPU data via `to_gpu_data()` and uploaded via `Renderer::upload_lights()`.

### Shading Model

`mesh3d.glsl` implements Blinn-Phong shading: diffuse + metallic-scaled specular + point light quadratic attenuation + IBL contribution when environment maps are loaded (see section 4).

---

## 4. IBL / Environment Maps

**Files:** `renderer/environment_map.rs`, `renderer/cubemap.rs`, `scene/components.rs`

### EnvironmentComponent

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `environment_handle` | `u64` | `0` | Asset UUID for `.hdr` environment map |
| `loaded` | `bool` | `false` | Whether the HDR has been preprocessed (runtime-only) |
| `skybox_exposure` | `f32` | `1.0` | Skybox brightness multiplier |
| `ibl_intensity` | `f32` | `1.0` | IBL ambient intensity multiplier |
| `skybox_rotation` | `f32` | `0.0` | Y-axis rotation in degrees |
| `show_skybox` | `bool` | `true` | Whether to render the skybox background |

Attached to an entity (typically the same one holding `AmbientLightComponent`) to enable HDR skybox and physically-based ambient lighting. Only the first entity with this component is used.

### EnvironmentMapSystem

Full IBL preprocessing pipeline, created lazily via `Renderer::ensure_environment_map()`.

#### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `ENV_CUBEMAP_SIZE` | 1024 | Environment cubemap face resolution |
| `IRRADIANCE_SIZE` | 64 | Irradiance map face resolution (low-res diffuse) |
| `PREFILTER_SIZE` | 1024 | Pre-filtered specular map face resolution |
| `PREFILTER_MIP_LEVELS` | 9 | Roughness mip levels (0-8: roughness 0.0-1.0) |
| `BRDF_LUT_SIZE` | 256 | BRDF integration LUT resolution |

#### IBL Pipeline

```
Equirectangular HDR (.hdr)
        │
        ▼   equirect_to_cube.glsl (compute, 16x16 workgroup)
  Cubemap (1024x1024 per face, R16G16B16A16_SFLOAT)
        │
        ├──▶ irradiance_convolve.glsl (compute, 8x8, ~2500 samples, MAX_RADIANCE=64.0 clamp)
        │         └──▶ Irradiance cubemap (64x64 per face)
        │
        ├──▶ prefilter_specular.glsl (compute, 16x16, 1024 GGX importance samples)
        │         └──▶ Pre-filtered specular cubemap (1024x1024, 9 mip levels)
        │
        └──▶ brdf_lut.glsl (compute, 16x16, CPU fallback available)
                  └──▶ BRDF LUT (256x256, RG16F)
```

All intermediate textures are `R16G16B16A16_SFLOAT` (HDR linear space).

#### Cubemap Struct

**File:** `renderer/cubemap.rs`

Wraps a Vulkan image with 6 array layers. Provides:
- `image_view` (CUBE type) for sampling in shaders
- `face_mip_views` (2D type per face per mip) for compute shader `imageStore` writes
- Sampler with `LINEAR` filtering and `CLAMP_TO_EDGE` addressing

### Skybox Rendering

**File:** `renderer/shaders/skybox.glsl`

The skybox is rendered as a unit cube with `SkyboxPushConstants`:

| Field | Size | Description |
|-------|------|-------------|
| `vp_rotation` | 64 bytes (`mat4`) | View-projection with translation removed |
| `exposure` | 4 bytes (`f32`) | Skybox brightness multiplier |
| `rotation_y` | 4 bytes (`f32`) | Y-axis rotation in radians |

Depth is forced to `0.0` in the vertex shader (reverse-Z far plane, rendered behind all geometry). ACES tonemapping is applied in non-offscreen mode.

The skybox pipeline binds set 0 (camera) and set 1 (lighting descriptor set, which contains the environment cubemap at binding 4).

### IBL Integration in mesh3d.glsl

When environment maps are loaded, `mesh3d.glsl` adds IBL contribution:

```
ambient = kD * irradiance_sample * albedo
        + prefiltered_sample(roughness_LOD) * brdf_lut_sample
```

- `irradiance` sampled from binding 1 (diffuse IBL)
- `prefiltered` sampled from binding 2 with roughness-based LOD selection
- `brdf_lut` sampled from binding 3 for energy compensation

### Lazy Initialization

`Renderer::ensure_environment_map()` creates the system only when a 3D scene actually uses an environment component. 2D-only scenes skip the GPU allocation entirely. When no environment is loaded, a 1x1 black fallback cubemap is used.

---

## 5. Shadow Mapping

**File:** `renderer/shadow_map.rs`

Cascaded shadow maps (CSM) with 4 cascades for directional lights.

### Architecture

- **D32_SFLOAT** depth images (4096x4096 per cascade), 2-layer array texture
- **Comparison sampler** (`sampler2DShadow`) with `CLAMP_TO_BORDER` + `FLOAT_OPAQUE_WHITE`
- **Depth-only render pass** with front-face culling + depth bias
- Per-cascade framebuffers, rendered sequentially
- Push constant model matrix for each mesh
- Camera-frustum-fitted cascade splits for optimal shadow resolution near the viewer

### Shadow Quality Tiers

| Tier | Name | Method | Description |
|------|------|--------|-------------|
| 0 | Low | 4-tap PCF | Fastest, blocky shadows |
| 1 | Medium | 9-tap PCF | Balanced quality |
| 2 | High | 16-tap PCF | Smooth shadows |
| 3 | Ultra | PCSS | Percentage-Closer Soft Shadows with variable penumbra |

Shadow quality stored on `Renderer` (`shadow_quality: i32`, default 3). Settable at runtime via `Renderer::set_shadow_quality()` or Lua `Engine.set_shadow_quality(0-3)`.

### Shadow Shaders

| Shader | Purpose |
|--------|---------|
| `shadow.glsl` | Vertex-only depth pass with push constant model matrix + light VP |
| `shadow_alpha.glsl` | Depth pass with alpha testing for transparent geometry (foliage, fences) |
| `skinned_shadow.glsl` | Depth pass with bone skinning for skeletal meshes |

### Scene Integration

```
Application::on_render_shadows(renderer, cmd_buf, current_frame)
    └── Scene::render_shadow_pass(renderer, cmd_buf, current_frame, camera)
            ├── For each cascade:
            │   ├── Compute cascade-fitted light VP matrix
            │   ├── Begin shadow render pass (per-cascade framebuffer)
            │   ├── For each MeshRendererComponent:
            │   │   └── Push model matrix + light VP, draw indexed
            │   └── End render pass
            └── Stash cascade VPs + split depths in shadow_cascade_cache
```

`render_shadow_pass` stashes cascade VP matrices and split depths in `shadow_cascade_cache` (on `SceneCore`), consumed later by `render_meshes` to populate the `LightEnvironment`.

### Deferred Allocation

`ShadowMapSystem` is created lazily via `Renderer::ensure_shadow_map()` on the first 3D pipeline creation or shadow pass. 2D-only scenes skip the GPU allocation entirely.

### Per-Light Control

`DirectionalLightComponent::cast_shadows` (default `true`) enables shadow casting per light. `shadow_cull_front_faces` (default `true`) eliminates self-shadowing acne at the cost of slight light leaking on thin geometry.

---

## 6. Skeletal Animation

**Files:** `renderer/skeleton.rs`, `renderer/bone_palette.rs`, `renderer/mesh.rs`, `scene/components.rs`, `renderer/shaders/skinned_mesh3d.glsl`, `renderer/shaders/skinned_shadow.glsl`

### SkeletalAnimationComponent

Attached to an entity that also has a `MeshRendererComponent`. Holds a shared skeleton and clips loaded from glTF.

| Field | Type | Default | Serialized | Description |
|-------|------|---------|------------|-------------|
| `mesh_asset` | `Uuid` | `0` | Yes | glTF/GLB asset handle |
| `skeleton` | `Arc<Skeleton>` | — | No | Shared joint hierarchy |
| `clips` | `Vec<SkeletalAnimationClip>` | `[]` | No | Animation clips from glTF |
| `current_clip` | `Option<usize>` | `None` | Yes | Currently playing clip index |
| `playback_time` | `f32` | `0.0` | Yes | Time within current clip (seconds) |
| `speed` | `f32` | `1.0` | Yes | Playback speed multiplier |
| `looping` | `bool` | `true` | Yes | Whether the current clip loops |
| `playing` | `bool` | `true` | Yes | Whether animation is actively playing |
| `skinned_vertex_array` | `Option<VertexArray>` | `None` | No | Runtime-only uploaded GPU data |
| `loaded_skinned_mesh` | `Option<Arc<SkinnedMesh>>` | `None` | No | Runtime-only mesh data |

Created from glTF data via `from_gltf_skin_data()` or as a stub via `from_asset(handle)` for lazy loading.

### Skeleton

| Field | Type | Description |
|-------|------|-------------|
| `joint_names` | `Vec<String>` | Human-readable joint names (indexed by joint index) |
| `parent_indices` | `Vec<i32>` | Parent index per joint (`-1` = root) |
| `inverse_bind_matrices` | `Vec<Mat4>` | Transform from mesh space to bone-local space |
| `rest_local_transforms` | `Vec<Mat4>` | Rest pose local transforms from glTF nodes |
| `bind_space_correction` | `Mat4` | `inverse(meshGlobal) * rootJointAncestorGlobal` |

**Pose evaluation:** `compute_pose(clip, time) -> BonePose` performs stateless evaluation:

1. Start from rest pose local transforms (joints without animation channels keep rest values)
2. Sample each channel's TRS keyframes with linear interpolation (lerp for position/scale, slerp for rotation)
3. Forward kinematics: propagate parent transforms to compute world-space joint matrices
4. Multiply by inverse bind matrices and bind-space correction per the glTF spec

### SkeletalAnimationClip

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Clip name (e.g. "Walk", "Run", "Idle") |
| `duration` | `f32` | Clip duration in seconds |
| `channels` | `Vec<JointChannel>` | Per-joint animation channels |

### JointChannel

| Field | Type | Description |
|-------|------|-------------|
| `joint_index` | `usize` | Index into the skeleton's joint arrays |
| `translations` | `Vec<Keyframe<Vec3>>` | Translation keyframes |
| `rotations` | `Vec<Keyframe<Quat>>` | Rotation keyframes (slerp interpolated) |
| `scales` | `Vec<Keyframe<Vec3>>` | Scale keyframes |

### BonePaletteSystem

**File:** `renderer/bone_palette.rs`

Per-frame SSBO for uploading bone matrices to the GPU. Bound at descriptor set 5.

| Constant | Value | Description |
|----------|-------|-------------|
| `MAX_SKINNED_BONES_PER_FRAME` | 4096 | Maximum bone matrices per frame |
| SSBO size | 256 KB | `4096 * 64 bytes` per frame-in-flight |

**API:**

| Method | Description |
|--------|-------------|
| `begin_frame(frame_index)` | Reset write offset for a new frame |
| `write_bones(matrices) -> Option<usize>` | Write matrices, return offset into SSBO |
| `ds_layout()` | Descriptor set layout for binding |
| `descriptor_set(frame_index)` | Per-frame descriptor set |

Each skinned draw call uses a `bone_offset` push constant to index into the shared SSBO. Lazily initialized via `Renderer::ensure_bone_palette()`.

### SkinnedMeshVertex

Extended vertex format for skeletal meshes:

| Field | Type | Description |
|-------|------|-------------|
| `position` | `[f32; 3]` | Vertex position |
| `normal` | `[f32; 3]` | Vertex normal |
| `uv` | `[f32; 2]` | Texture coordinates |
| `color` | `[f32; 4]` | Vertex color |
| `tangent` | `[f32; 4]` | Tangent (xyz = direction, w = bitangent sign) |
| `bone_indices` | `[i32; 4]` | Indices into the bone palette (up to 4 influences) |
| `bone_weights` | `[f32; 4]` | Blend weights (should sum to 1.0) |

### Skinned Mesh Shaders

- **`skinned_mesh3d.glsl`:** Vertex shader computes `skin_matrix` from 4 bone weights, transforms position/normal/tangent. Fragment shader identical to `mesh3d.glsl`.
- **`skinned_shadow.glsl`:** Depth-only pass with bone skinning for correct shadow silhouettes.

### GltfSkinData

Result of `load_gltf_skinned()`: groups `SkinnedMesh` + `Skeleton` + `Vec<SkeletalAnimationClip>` from the glTF loader.

### Lazy Loading

Components created with `from_asset(handle)` start as stubs (empty skeleton, no clips). The asset resolver fills `skeleton`, `clips`, and `loaded_skinned_mesh` when loading completes. The `skinned_vertex_array` is uploaded on first render.

---

## 7. Post-Processing Pipeline

**File:** `renderer/postprocess.rs`

### Architecture

`PostProcessPipeline` operates on the scene's offscreen framebuffer color output and produces a final composited image suitable for viewport display.

All intermediate images use `R16G16B16A16_SFLOAT` (HDR linear space). Rendering uses fullscreen triangle (no vertex buffer).

### Bloom

4-level downsample + upsample chain:

```
Scene color (full res)
    │
    ▼  bloom_downsample.glsl (4-tap bilinear + brightness threshold)
  Mip 0 (1/2 res) → Mip 1 (1/4) → Mip 2 (1/8) → Mip 3 (1/16)
    │
    ▼  bloom_upsample.glsl (3x3 tent filter, additive blend)
  Mip 2 ← Mip 3
  Mip 1 ← Mip 2
  Mip 0 ← Mip 1
    │
    ▼  postprocess_composite.glsl (adds bloom to scene color)
  Final output
```

Settings: `bloom_enabled`, `bloom_threshold`, `bloom_intensity`, `bloom_filter_radius`.

### Tone Mapping

| Mode | Description |
|------|-------------|
| `None` | Pass-through (no HDR-to-LDR conversion) |
| `ACES` | ACES filmic tone curve (industry standard) |
| `Reinhard` | Simple, preserves color ratios |

### Color Grading

Applied in the composite pass after tonemapping:
- `exposure` — overall brightness multiplier
- `contrast` — mid-tone contrast adjustment
- `saturation` — color saturation multiplier

### Contact Shadows

**File:** `renderer/shaders/contact_shadows.glsl`

Screen-space ray march from the depth buffer toward the directional light direction. Produces soft, detail-preserving contact shadows that complement the main shadow map.

| Setting | Description |
|---------|-------------|
| `enabled` | Toggle contact shadows |
| `max_distance` | Maximum ray march distance |
| `thickness` | Shadow thickness |
| `intensity` | Shadow darkening strength |
| `step_count` | Number of ray march steps |

### Lifecycle

- **Lazy init:** Created from the scene framebuffer when post-processing is first enabled
- **Auto-resize:** Rebuilds all intermediate images when the viewport resizes
- **Output:** Final composited image registered as an egui user texture for viewport display

---

## 3D Shader Summary

| Shader | Type | Purpose |
|--------|------|---------|
| `mesh3d.glsl` | Vert + Frag | 3D mesh rendering: Blinn-Phong + PBR + IBL + shadows |
| `skinned_mesh3d.glsl` | Vert + Frag | Skinned mesh rendering with bone palette |
| `shadow.glsl` | Vert only | Depth pass for opaque shadow casters |
| `shadow_alpha.glsl` | Vert + Frag | Depth pass with alpha testing |
| `skinned_shadow.glsl` | Vert only | Depth pass with bone skinning |
| `skybox.glsl` | Vert + Frag | Environment cubemap skybox |
| `equirect_to_cube.glsl` | Compute | Equirectangular HDR to cubemap conversion |
| `irradiance_convolve.glsl` | Compute | Diffuse irradiance convolution |
| `prefilter_specular.glsl` | Compute | GGX importance-sampled specular pre-filter |
| `brdf_lut.glsl` | Compute | Split-sum BRDF integration LUT |
| `bloom_downsample.glsl` | Vert + Frag | Bloom downsample pass |
| `bloom_upsample.glsl` | Vert + Frag | Bloom upsample pass |
| `postprocess_composite.glsl` | Vert + Frag | Final composite (bloom + tone + grading) |
| `contact_shadows.glsl` | Vert + Frag | Screen-space contact shadows |
| `bilateral_blur.glsl` | Vert + Frag | Edge-preserving blur |
| `depth_resolve.glsl` | Vert + Frag | MSAA depth resolve |
