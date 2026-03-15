# Rendering

The rendering system lives in `gg_engine/src/renderer/` and provides a layered abstraction over Vulkan.

## Architecture Overview

The rendering stack is layered:

```
Renderer          High-level public API (factory methods, 2D draw calls)
    │
RenderCommand     Static forwarding functions
    │
RendererAPI       Enum dispatch (wrapping VulkanRendererAPI)
    │
DrawContext       Per-frame command buffer + extent
    │
Vulkan            Raw Vulkan (ash 0.38)
```

`RendererAPI` uses enum dispatch (single Vulkan variant), not trait objects.

## VulkanContext

**File:** `renderer/vulkan_context.rs`

Owns all core Vulkan state:
- Entry, Instance, Debug messenger
- Surface
- Physical device / Logical device
- Graphics queue

Configuration:
- Targets **Vulkan 1.3**
- Validation layers and debug messenger enabled in debug builds (`#[cfg(debug_assertions)]`)
- Physical device selection prefers discrete GPUs
- `wideLines` device feature enabled for line widths > 1.0

## Swapchain

**File:** `renderer/swapchain.rs`

Manages swapchain, image views, depth buffer (D32_SFLOAT), render pass, framebuffers, command pool/buffers, and sync primitives.

- `MAX_FRAMES_IN_FLIGHT = 2`
- Render pass: 2 attachments (color + depth/stencil)
- `PresentMode` variants: `Fifo` (default/VSync), `Mailbox`, `Immediate` (each with fallback chains)

### Sync Primitives (Critical Correctness Detail)

| Primitive | Indexing | Key |
|-----------|----------|-----|
| `image_available_semaphores` | Per-frame-in-flight | `current_frame` |
| `in_flight_fences` | Per-frame-in-flight | `current_frame` |
| `render_finished_semaphores` | Per-swapchain-image | `image_index` |

**Fence reset timing:** Fence is only reset after `acquire_next_image` succeeds. If acquire returns `OUT_OF_DATE`, the fence stays signaled to prevent deadlock on the next `wait_for_fences`.

## Shaders

**File:** `renderer/shader.rs`

### GLSL Source Format

Single-file `.glsl` format with stage markers:

```glsl
#type vertex
#version 450
// vertex shader code...

#type fragment
#version 450
// fragment shader code...
```

Shader sources live in `gg_engine/src/renderer/shaders/`.

### Build-Time Compilation

`gg_engine/build.rs` auto-compiles `.glsl` to SPIR-V via `glslc` at build time:
1. Reads `.glsl` files from `renderer/shaders/`
2. Splits on `#type vertex` / `#type fragment` markers
3. Compiles each stage to SPIR-V
4. Exposes as `pub const` byte slices in `gg_engine::shaders`

```rust
// Example: accessing compiled shaders
use gg_engine::shaders::{BATCH_VERT_SPV, BATCH_FRAG_SPV};
```

### Runtime Hot-Reload

**Files:** `renderer/shader_compiler.rs`, `renderer/renderer_2d.rs`, `renderer/renderer.rs`

Shaders can be recompiled and pipelines rebuilt at runtime without restarting the application. This enables rapid iteration on shader code during development.

**API:**

```rust
renderer.reload_shaders(shader_dir: &Path) -> Result<u32, String>
```

**How it works:**

1. `shader_compiler::compile_glsl(path)` reads each `.glsl` source, splits by `#type` markers, and invokes `glslc` to produce SPIR-V (same logic as `build.rs`, but returns `Result` instead of panicking)
2. All shaders are compiled first — if any fail, the operation aborts and old pipelines remain intact (atomic update)
3. `device_wait_idle()` ensures no in-flight command buffers reference the old pipelines
4. New `Shader` objects are created from the compiled SPIR-V
5. All pipelines (swapchain and offscreen variants) are rebuilt with the new shaders
6. Old pipelines are dropped via `Arc` reference counting

**Requirements:** `glslc` must be on PATH at runtime (standard Vulkan SDK install).

**Editor integration:** The Settings panel has a "Reload Shaders" button that triggers hot-reload. Errors are shown via a native error dialog. The shader source directory is resolved from `CARGO_MANIFEST_DIR` at compile time.

**Stored pipeline creation params:** `Renderer2DData` stores all parameters needed to recreate pipelines (`swapchain_render_pass`, `camera_ubo_ds_layout`, `pipeline_cache`, `offscreen_render_pass`, `offscreen_color_attachment_count`), enabling full pipeline reconstruction from a single `reload_shaders()` call.

### Available Shaders

| File | Purpose |
|------|---------|
| `batch.glsl` | Batch 2D quad rendering (offscreen, 2 outputs) |
| `batch_swapchain.glsl` | Batch 2D quad rendering (swapchain, 1 output) |
| `circle.glsl` | SDF circle rendering (offscreen, 2 outputs) |
| `circle_swapchain.glsl` | SDF circle rendering (swapchain, 1 output) |
| `line.glsl` | Line rendering (offscreen, 2 outputs) |
| `line_swapchain.glsl` | Line rendering (swapchain, 1 output) |
| `text.glsl` | MSDF text rendering (offscreen, 2 outputs) |
| `text_swapchain.glsl` | MSDF text rendering (swapchain, 1 output) |
| `instance.glsl` | Instanced sprite rendering with GPU animation (offscreen, 2 outputs) |
| `instance_swapchain.glsl` | Instanced sprite rendering with GPU animation (swapchain, 1 output) |
| `particle_sim.glsl` | GPU particle simulation (compute shader) |
| `mesh3d.glsl` | 3D mesh rendering with Blinn-Phong lighting, shadows, quality tiers |
| `shadow.glsl` | Shadow depth pass (push constant model matrix + light VP) |
| `shadow_alpha.glsl` | Shadow depth pass with alpha testing for transparent geometry |
| `bloom_downsample.glsl` | Bloom downsample (4-tap bilinear + brightness threshold) |
| `bloom_upsample.glsl` | Bloom upsample (3x3 tent filter, additive blend) |
| `postprocess_composite.glsl` | Final composite: tone mapping (None/ACES/Reinhard) + color grading |
| `contact_shadows.glsl` | Screen-space contact shadows via ray marching from depth buffer |
| `bilateral_blur.glsl` | Edge-preserving bilateral blur |
| `depth_resolve.glsl` | MSAA depth resolve |

Shaders now use `#ifdef OFFSCREEN` for dual compilation instead of separate `_swapchain` files.

Legacy shaders exist in `shaders/legacy/` but are unused: `flat_color.glsl`, `texture.glsl`, `triangle.glsl`.

### Shader / ShaderLibrary

- `Shader` wraps a named vertex + fragment SPIR-V module pair. Created via `Renderer::create_shader()`.
- `ShaderLibrary` — `HashMap<String, Arc<Shader>>` keyed by `shader.name()`. Standalone struct (not on `Renderer`) to avoid `&mut` conflicts.

## Buffers

**File:** `renderer/buffer.rs`

### Vertex Attribute Description

```
ShaderDataType    Cross-API type (HLSL naming: Float, Float2, Float3, Float4, Int, etc.)
     │
BufferElement     Named attribute with type
     │
BufferLayout      Auto-computes offsets and stride; generates Vulkan descriptions
```

`as_bytes()` utility converts typed vertex slices to raw bytes for buffer upload.

### Buffer Types

| Type | Memory | Purpose |
|------|--------|---------|
| `VertexBuffer` | HOST_VISIBLE, HOST_COHERENT | Static vertex data |
| `IndexBuffer` | HOST_VISIBLE, HOST_COHERENT | Index data |
| `DynamicVertexBuffer` | HOST_VISIBLE, HOST_COHERENT, persistently mapped | Per-frame streaming (2 internal buffers for frame-in-flight) |

- `DynamicVertexBuffer` maintains two buffers (one per frame-in-flight) to avoid GPU/CPU stalls
- Data uploaded via `write_at(offset, data)` each frame
- Used by the batch 2D renderer for dynamic quad geometry

## Vertex Array

**File:** `renderer/vertex_array.rs`

CPU-side abstraction grouping vertex buffers + index buffer (analogous to OpenGL VAO). Validates that each vertex buffer has a layout before accepting it. Generates combined Vulkan binding/attribute descriptions. Created via `Renderer::create_vertex_array()`.

## Pipeline

**File:** `renderer/pipeline.rs`

Wraps `vk::Pipeline` + `vk::PipelineLayout`, destroyed on drop.

### Creation Methods

| Method | Use Case |
|--------|----------|
| `create_pipeline(shader, va, has_material_color, blend_enable)` | General purpose |
| `create_texture_pipeline(shader, va)` | Includes texture descriptor set layout, enables blending |
| `create_batch_pipeline(...)` | Batch 2D: camera UBO set 0, bindless textures set 1, no push constants |
| `create_line_batch_pipeline(...)` | Line rendering: LINE_LIST topology |

### Defaults

- Triangle list topology (except line pipelines)
- No culling
- Dynamic viewport/scissor
- Depth testing (LESS_OR_EQUAL) with depth writes

### Descriptor Set Layout Convention

- **Set 0** — Camera UBO (binding 0, UNIFORM_BUFFER, VERTEX stage) — shared by all pipelines
- **Set 1** — Textures (bindless array or per-texture samplers)

### Push Constants

- Vertex stage (offset 0, 64 bytes): transform matrix (mat4)
- Fragment stage (offset 64, 20 bytes, optional): material color (vec4) + tiling factor (float)
- Batch pipeline: **no push constants** (VP in UBO, transform baked into vertices)

## Uniform Buffer Objects (UBOs)

**File:** `renderer/uniform_buffer.rs`

`UniformBuffer` — per-frame-in-flight double-buffered UBO with persistent mapping (`HOST_VISIBLE | HOST_COHERENT`).

```rust
// CameraData: #[repr(C)], 80 bytes, std140 compatible
struct CameraData {
    view_projection: Mat4,  // 64 bytes
    time: f32,              // 4 bytes — monotonic scene time for GPU animation (u_time in shaders)
    _padding: [f32; 3],     // 12 bytes — std140 alignment
}
```

Written to the UBO once per frame in `begin_scene`. The `time` field is used by instance shaders for GPU-computed animation. Multi-viewport: `MAX_FRAMES_IN_FLIGHT × MAX_VIEWPORTS` UBO slots, indexed by `DrawContext.viewport_index`.

## Textures

**File:** `renderer/texture.rs`

`Texture2D` — Vulkan image + image view + sampler + descriptor set.

### Creation

```rust
let tex = renderer.create_texture_from_file("path/to/image.png");
let tex = renderer.create_texture_from_rgba8(width, height, &pixels);
```

### TextureSpecification

```rust
struct TextureSpecification {
    pub format: ImageFormat,    // Rgba8Srgb (color) or Rgba8Unorm (data/MSDF)
    pub filter: Filter,         // NEAREST (default) or LINEAR
    pub address_mode: SamplerAddressMode,  // REPEAT default
    pub anisotropy: bool,       // default true
    pub max_anisotropy: f32,    // default 16.0
}
```

Factory method `TextureSpecification::font_atlas()` creates LINEAR + UNORM spec for MSDF atlas textures.

### Internals

- Device-local memory with staging buffer upload
- One-shot command buffer for layout transitions: UNDEFINED → TRANSFER_DST → SHADER_READ_ONLY
- Default sampler: NEAREST filtering, REPEAT addressing, 16x anisotropic filtering
- `ImageFormat::Rgba8Srgb` for color textures, `ImageFormat::Rgba8Unorm` for data textures (MSDF atlases)
- Each texture gets a `bindless_index: u32` on creation
- Destroyed on drop (sampler, image view, image, memory)

### Async Texture Loading

**Files:** `renderer/texture.rs`, `asset/asset_loader.rs`

The engine supports non-blocking background texture loading via a two-phase CPU/GPU split.

**`TextureCpuData`** is a thread-safe intermediate struct containing decoded pixel data with no Vulkan types:

```rust
pub struct TextureCpuData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub spec: TextureSpecification,
}
```

**Loading pipeline:**

1. **`Texture2D::load_cpu_data(path, spec) -> Result<TextureCpuData>`** — decodes an image file (PNG, JPEG, etc.) into raw RGBA8 pixels. Contains no GPU calls, safe to call on any background thread.
2. **`Texture2D::from_cpu_data(resources, allocator, data)`** — performs GPU upload from pre-loaded `TextureCpuData` (staging buffer, layout transitions, sampler/descriptor set creation). Must be called on the main/render thread.

**`AssetLoader`** (`asset/asset_loader.rs`) drives this pipeline with a pool of 2 worker threads (`WORKER_COUNT = 2`). Workers are spawned lazily on first request. The main thread sends `LoadRequest::Texture` messages, workers call `load_cpu_data` on a background thread, and completed `TextureCpuData` results are polled non-blocking via `AssetLoader::poll_results()`. The main thread then finalizes GPU upload with `from_cpu_data`. The same pattern is used for font atlas generation.

### SubTexture2D (Sprite Sheets)

**File:** `renderer/sub_texture.rs`

Lightweight value type representing a sub-region of a `Texture2D`. Stores pre-computed UV coordinates and the parent texture's `bindless_index`. Does **not** own GPU resources.

```rust
// Explicit UV bounds
let sub = SubTexture2D::new(&texture, [0.0, 0.0], [0.5, 0.5]);

// Grid-based sprite sheet access
let sub = SubTexture2D::from_coords(
    &texture,
    Vec2::new(7.0, 6.0),   // grid position (column, row)
    Vec2::new(128.0, 128.0), // cell size in pixels
    Vec2::ONE,               // sprite size in cells
);
```

### Bindless Textures

The renderer uses a bindless texture descriptor array (`MAX_BINDLESS_TEXTURES = 4096`).

- Bound at **set 1** (after camera UBO at set 0)
- `UPDATE_AFTER_BIND | PARTIALLY_BOUND` flags
- Two descriptor sets maintained (one per frame-in-flight)
- Textures auto-registered on creation via `register_texture()`
- **Slot recycling:** `Renderer::unregister_texture(texture)` returns the texture's bindless slot to a free-list (`bindless_free_list: Vec<u32>`). The next `register_texture` call reuses freed slots before allocating new indices. Slot 0 (white texture) is never freed. This prevents exhausting the 4096 slot limit when textures are created and destroyed over time.
- Batch shader: `texture(u_textures[nonuniformEXT(tex_index)], uv)` using `GL_EXT_nonuniform_qualifier`

## Cameras

### OrthographicCamera

**File:** `renderer/orthographic_camera.rs`

Stores projection, view, and cached view-projection matrices, plus position (`Vec3`) and Z-axis rotation (`f32`, radians).

```rust
let camera = OrthographicCamera::new(left, right, bottom, top);
camera.set_position(Vec3::new(1.0, 0.0, 0.0));
camera.set_rotation(0.5); // radians
```

- Uses `Mat4::orthographic_lh` with near/far = -1/1
- Vulkan NDC Y-flip applied (`y_axis.y *= -1.0`) so world Y+ is up
- Engine creates a default camera from window aspect ratio

### OrthographicCameraController

**File:** `gg_engine/src/orthographic_camera_controller.rs`

Wraps `OrthographicCamera` with built-in input handling.

- **WASD** — movement
- **Q/E** — rotation (optional)
- **Scroll wheel** — zoom
- Translation speed scales with zoom level

```rust
let controller = OrthographicCameraController::new(aspect_ratio, rotation_enabled);
controller.on_update(dt, &input);
controller.on_event(&event);

// Utility
controller.bounds()        // -> (left, right, bottom, top)
controller.bounds_size()   // -> (width, height) in world units
controller.screen_to_world(screen_x, screen_y, window_w, window_h)
```

### EditorCamera

**File:** `renderer/editor_camera.rs`

Maya-style 3D perspective camera for the editor viewport.

- Uses `Mat4::perspective_lh` with Vulkan Y-flip
- **Alt+LMB** = orbit, **Alt+MMB** = pan, **Alt+RMB** or **scroll** = zoom
- Constructor: `new(fov, near, far)` (fov in radians)
- Key methods: `on_update(dt, input)`, `on_event(event) -> bool`, `set_viewport_size(w, h)`

### SceneCamera (ECS)

**File:** `renderer/scene_camera.rs`

Projection-only camera for ECS use — no position/rotation/view matrix (those come from `TransformComponent`).

- Two projection types: `Perspective` (0) and `Orthographic` (1, default)
- Both use `Mat4::*_lh` with Vulkan Y-flip

## Built-in 2D Renderer (Batch-Based)

**File:** `renderer/renderer_2d.rs`

Initialized automatically by the engine before `Application::on_attach`.

### Architecture

- **Batching**: up to `MAX_QUADS = 10,000` per batch (40,000 vertices), `MAX_BATCHES_PER_FRAME = 16`
- **Vertex format**: `BatchQuadVertex` — position (Float3), color (Float4), tex_coord (Float2), tex_index (float), entity_id (Int)
- **Pipeline**: `batch.glsl` shader with camera UBO (set 0) and bindless texture array (set 1)
- **White texture**: 1x1 white default at bindless index 0 for flat-colored quads

### High-Level 2D Draw API

All methods are on `Renderer` — no pipeline/vertex array management needed.

| Method | Description |
|--------|-------------|
| `draw_quad(pos: &Vec3, size: &Vec2, color)` | Flat-colored quad |
| `draw_quad_2d(pos: &Vec2, size: &Vec2, color)` | Same at z=0 |
| `draw_textured_quad(pos, size, texture, tiling, tint)` | Textured quad |
| `draw_textured_quad_2d(...)` | Same at z=0 |
| `draw_rotated_quad(pos, size, rotation, color)` | Z-axis rotated quad |
| `draw_rotated_quad_2d(...)` | Same at z=0 |
| `draw_rotated_textured_quad(...)` | Rotated + textured |
| `draw_sub_textured_quad(pos, size, sub_texture, tint)` | Sprite sheet region |
| `draw_rotated_sub_textured_quad(...)` | Rotated sprite sheet region |
| `draw_sub_textured_quad_transformed(transform, sub_texture, tint, entity_id)` | Sub-texture quad from raw Mat4 transform |

Position sets the **center**, size scales the unit quad (-0.5..0.5). Z-ordering is respected via depth testing.

### ECS Draw Methods

| Method | Description |
|--------|-------------|
| `draw_quad_transform(transform: &Mat4, color, entity_id)` | Quad from raw transform |
| `draw_sprite(transform: &Mat4, sprite: &SpriteRendererComponent, entity_id)` | Sprite component |
| `draw_circle(transform, color, thickness, fade, entity_id)` | SDF circle |
| `draw_circle_component(transform, circle: &CircleRendererComponent, entity_id)` | Circle component |

## Scene Rendering

**File:** `scene/mod.rs` — `Scene::render_scene()`

`render_scene()` is the central method that draws all renderable entities in the scene. It is called by `on_update_runtime`, `on_update_editor`, and `on_update_simulation` after setting the view-projection matrix.

The method pre-computes world transforms for all entities once (`build_world_transform_cache()`), then iterates each renderable component type in order:

1. **Sprites** (with optional animation) — see [Animation Rendering](#animation-rendering) below
2. **Circles** — `CircleRendererComponent` entities
3. **Text** — `TextComponent` entities
4. **Tilemaps** — `TilemapComponent` entities — see [Tilemap Rendering](#tilemap-rendering) below

### Animation Rendering

During `render_scene()`, each `SpriteRendererComponent` entity is checked for an accompanying `SpriteAnimatorComponent`. If the animator has an active clip playing:

1. `current_grid_coords()` returns the current frame's `(column, row)` in the sprite sheet grid
2. A `SubTexture2D` is created from those grid coordinates using the animator's `cell_size`
3. `draw_sub_textured_quad_transformed()` renders the current frame's sub-region instead of the full texture

If no animator is present or no clip is playing, the sprite renders normally via `draw_sprite()`.

Animation timing is advanced separately by `Scene::on_update_animations(dt)`, which iterates all `SpriteAnimatorComponent` entities and calls `update(dt)` to advance their frame timers. The `SpriteAnimatorComponent` stores per-entity state: current clip index, frame timer, current frame number, and playing flag.

**`AnimationClip`** defines a contiguous range of frames in the sprite sheet (start_frame, end_frame inclusive), playback FPS, and a looping flag. Frame indices are 0-based, row-major: frame 0 is top-left, and frames wrap across rows based on the animator's `columns` count.

### Tilemap Rendering

`render_scene()` iterates all `TilemapComponent` entities and renders their tile grids. Each `TilemapComponent` describes a row-major grid of tile IDs referencing sub-regions of a tileset texture.

**Per-tile rendering steps:**

1. Skip empty tiles (tile ID = -1)
2. Extract flip flags from high bits: bit 30 = horizontal flip (`TILE_FLIP_H`), bit 29 = vertical flip (`TILE_FLIP_V`). The lower 29 bits (`TILE_ID_MASK`) hold the actual tile ID
3. Compute tileset grid coordinates: `col = tile_id % tileset_columns`, `row = tile_id / tileset_columns`
4. Calculate UV rectangle accounting for `cell_size`, `spacing`, and `margin` (all in pixels). Flip UVs are applied by swapping min/max on the appropriate axis
5. Create a `SubTexture2D` from the computed UVs
6. Build a per-tile transform by combining the entity's world transform with a translation offset (`col * tile_size.x`, `row * tile_size.y`) and scale (`tile_size`)
7. Draw via `draw_sub_textured_quad_transformed()`

**`TilemapComponent` fields:**

| Field | Type | Description |
|-------|------|-------------|
| `width`, `height` | `u32` | Grid dimensions |
| `tile_size` | `Vec2` | World-space size per tile |
| `texture_handle` | `Uuid` | Asset handle for tileset texture |
| `texture` | `Option<Ref<Texture2D>>` | Runtime-loaded texture (not serialized) |
| `tileset_columns` | `u32` | Columns in the tileset image |
| `cell_size` | `Vec2` | Pixel size per cell in tileset |
| `spacing` | `Vec2` | Pixel spacing between tileset cells |
| `margin` | `Vec2` | Pixel margin from tileset edge |
| `tiles` | `Vec<i32>` | Row-major tile IDs (-1 = empty, high bits = flip flags) |

## Instanced Sprite Rendering

**Files:** `renderer/renderer_2d.rs`, `instance.glsl`, `instance_swapchain.glsl`

For rendering many sprites with the same texture efficiently, the renderer provides instanced rendering via a static unit quad plus per-instance data buffers.

### SpriteInstanceData (148 bytes, repr(C))

| Field | Type | Description |
|-------|------|-------------|
| `transform` | `[f32; 16]` | 4x4 model matrix (column-major) |
| `color` | `[f32; 4]` | RGBA tint color |
| `uv_min` / `uv_max` | `[f32; 2]` each | Texture UV bounds |
| `tex_index` | `f32` | Bindless texture array index |
| `tiling_factor` | `f32` | Texture tiling |
| `entity_id` | `i32` | For mouse picking |
| `anim_start_time` | `f32` | GPU animation: clip start timestamp |
| `anim_fps` | `f32` | GPU animation: frames per second |
| `anim_start_frame` | `f32` | GPU animation: first frame index |
| `anim_frame_count` | `f32` | GPU animation: total frames (0 = not GPU-animated) |
| `anim_columns` | `f32` | GPU animation: sprite sheet columns |
| `anim_looping` | `f32` | GPU animation: 1.0 = looping, 0.0 = one-shot |
| `anim_cell_size` | `[f32; 2]` | GPU animation: pixel size per cell |
| `anim_tex_size` | `[f32; 2]` | GPU animation: texture dimensions |

### GPU Animation

When `anim_frame_count > 0`, the instance vertex shader computes UV coordinates from `u_time` (global monotonic time from camera UBO) plus the per-instance animation parameters. This means **zero CPU animation cost** for playing instanced sprites — the GPU computes the correct frame each render.

When `anim_frame_count == 0`, the shader uses the CPU-provided `uv_min`/`uv_max` values (backward compatible with non-animated sprites).

### Limits

- `MAX_INSTANCES = 10,000` per instanced draw call
- Instance buffers are per-frame (one per frame-in-flight)

## GPU Particle System

**Files:** `renderer/gpu_particle_system.rs`, `particle_sim.glsl`

Compute shader-driven particle simulation with indirect rendering.

### Architecture

1. **Compute shader** (`particle_sim.glsl`) advances particle positions, velocities, colors, lifetimes
2. **Indirect draw**: compute shader updates a `VkDrawIndexedIndirectCommand` buffer (instance_count = active particles)
3. **Instanced rendering**: particles rendered as unit quads with per-particle instance data
4. **No CPU readback**: the GPU manages particle counts entirely

### GpuParticle (80 bytes, std430)

Position, velocity, rotation, rotation_speed, size_begin/end, color_begin/end (RGBA), lifetime, life_remaining, is_active flag.

### Features

- Gravity, velocity damping, size/color interpolation over lifetime
- Non-blocking lifecycle: only active particles are processed
- Push constants: `dt` and `max_particles`

## Circle Rendering

SDF-based circle rendering on quads in the fragment shader (smoothstep for thickness/fade).

- Separate batch pipeline from quads: `BatchCircleVertex` (world_position, local_position, color, thickness, fade, entity_id)
- `local_position` = quad corner x 2 (range -1 to 1), used as UV for SDF computation
- Circles reuse the same index buffer as quads
- Circles don't use textures — no bindless descriptor set (only camera UBO at set 0)
- Fragments with alpha <= 0 are discarded (correct picking for entity selection)

## Line Rendering

Debug rendering primitive — **not** an ECS component, purely a renderer API.

- Vulkan `LINE_LIST` topology with non-indexed draw (`vkCmdDraw`)
- `BatchLineVertex`: position, color, entity_id
- `LINE_WIDTH` dynamic state via `vkCmdSetLineWidth`, default 2.0, configurable via `Renderer::set_line_width`
- `MAX_LINES = 10,000`

| Method | Description |
|--------|-------------|
| `draw_line(p0, p1, color, entity_id)` | Single line segment |
| `draw_rect(position, size, color, entity_id)` | Wireframe XY rectangle (4 lines) |
| `draw_rect_transform(transform, color, entity_id)` | Wireframe rect via transform |

## MSDF Text Rendering

**Files:** `renderer/font.rs`, `renderer/msdf.rs`

Pure-Rust MSDF (Multi-channel Signed Distance Field) text rendering — no C library dependency.

### Font

`Font` loads a TTF file via `ttf-parser 0.25` and generates an MSDF atlas texture.

- **Atlas generation**: 48px glyph cells, 2px padding, 4px SDF range
- **Glyph metrics**: advance width, bearing, bounding box per glyph
- **Kerning table**: extracted from TTF, applied during text layout
- **Atlas texture**: `Rgba8Unorm` format, LINEAR filtering (via `TextureSpecification::font_atlas()`)

### MSDF Generator

`msdf.rs` (866 lines) implements the full Chlumsky MSDF algorithm:

1. **Edge extraction** from TTF glyph outlines (linear, quadratic, cubic Bezier segments)
2. **Edge coloring** — assigns R/G/B channels to edges for multi-channel distance fields
3. **SDF evaluation** — per-pixel signed distance to each edge type
4. **Winding number** for sign correction (inside/outside determination)
5. **Autoframe** — automatically positions glyphs within atlas cells

### Text Pipeline

- Batch text vertices: position, color, tex_coord, entity_id (similar to quad batches)
- `text.glsl` / `text_swapchain.glsl` shaders
- Fragment shader: takes median of R/G/B channels from MSDF atlas, applies `smoothstep` with `fwidth` for screen-space antialiasing
- Font atlas bound at set 1 (bindless texture array)
- `Renderer::draw_text(text, transform, text_component, font, entity_id)` — high-level API

### Scene Integration

- `Scene::load_fonts(renderer)` loads all fonts referenced by `TextComponent` entities, caches on Scene
- Text rendered alongside sprites and circles in scene render loops

## Framebuffer (Offscreen Rendering)

**File:** `renderer/framebuffer.rs`

`Framebuffer` — offscreen render target supporting multiple color attachments + depth.

### Supported Formats

| Format | Vulkan Format | Purpose |
|--------|--------------|---------|
| `RGBA8` | R8G8B8A8 | Standard color |
| `RedInteger` | R32_SINT | Entity ID picking |
| `Depth` | D32_SFLOAT | Depth buffer |

The editor creates a 3-attachment framebuffer: RGBA8 + RedInteger + Depth.

`resize(w, h)` rebuilds images while preserving the descriptor set (egui `TextureId` remains valid). The first color attachment is sampled via a LINEAR sampler and exposed as an egui texture for viewport display.

### Mouse Picking

The `RedInteger` attachment enables GPU-based mouse picking:
1. Entity IDs written to R32_SINT during rendering (via `entity_id` parameter)
2. `schedule_pixel_readback(attachment_index, x, y)` initiates async GPU readback
3. `hovered_entity() -> i32` returns last result (`-1` = no entity)
4. `Scene::find_entity_by_id(id)` converts pixel value back to `Entity`

## Dual-Pass Rendering (Editor Mode)

When `Application::scene_framebuffer()` returns `Some`:

1. **Offscreen pass** — renders scene to framebuffer, pipeline barrier after render pass
2. **Swapchain pass** — draws egui (displays framebuffer texture in Viewport tab)

Clear color for swapchain pass: `[0.06, 0.06, 0.06, 1.0]` (editor chrome).

When `scene_framebuffer()` returns `None` (sandbox apps), single-pass rendering is used.

## Renderer Lifecycle

```
begin_scene(camera)
  ├── Copy camera VP to internal state
  ├── Write VP to camera UBO
  └── Set viewport/scissor
app draw calls...
end_scene()
  ├── flush_quads()
  ├── flush_circles()
  └── flush_lines()
```

## Shadow Mapping

**File:** `renderer/shadow_map.rs`

Cascaded shadow maps (CSM) with 4 cascades for directional lights.

### Architecture

- D32_SFLOAT depth images (4096x4096 per cascade), 2-layer array texture
- Comparison sampler (`sampler2DShadow`) with `CLAMP_TO_BORDER` + `FLOAT_OPAQUE_WHITE`
- Depth-only render pass with front-face culling + depth bias
- Per-cascade framebuffers, rendered sequentially
- Push constant model matrix for each mesh (replaced UBO for per-cascade correctness)
- Camera-frustum-fitted cascade splits via `compute_cascade_splits()`

### Shadow Quality Tiers

| Tier | Name | PCF Method | Description |
|------|------|-----------|-------------|
| 0 | Low | 4-tap PCF | Fastest, blocky shadows |
| 1 | Medium | 9-tap PCF | Balanced quality |
| 2 | High | 16-tap PCF | Smooth shadows |
| 3 | Ultra | PCSS | Percentage-Closer Soft Shadows with variable penumbra |

Shadow quality is stored on `Renderer` (`shadow_quality: i32`), default 3 (Ultra). Settable at runtime via `Renderer::set_shadow_quality()` or Lua `Engine.set_shadow_quality(0-3)`.

### Shaders

- `shadow.glsl` — vertex-only depth pass, push constant model matrix
- `shadow_alpha.glsl` — depth pass with alpha testing for transparent/masked geometry
- `mesh3d.glsl` — reads shadow map at descriptor set 4, applies quality-tiered PCF

## Post-Processing

**File:** `renderer/postprocess.rs`

`PostProcessPipeline` provides bloom, tone mapping, color grading, and contact shadows. Created lazily when the scene framebuffer is available. Operates on the offscreen framebuffer's color output and writes to an internal R16G16B16A16_SFLOAT output image registered as an egui user texture for viewport display.

### Bloom

4-level mip chain (each half the previous resolution):

1. **Downsample** — 4-tap bilinear sampling with brightness threshold (first pass only). `bloom_downsample.glsl`
2. **Upsample** — 3x3 tent filter with additive blending. `bloom_upsample.glsl`

Settings: `bloom_enabled`, `bloom_threshold`, `bloom_intensity`, `bloom_filter_radius`.

### Tone Mapping & Color Grading

Final composite pass (`postprocess_composite.glsl`) applies:

- **Tone mapping**: `TonemappingMode` enum — `None` (pass-through), `ACES` (filmic), `Reinhard`
- **Color grading**: `exposure`, `contrast`, `saturation`

All intermediate images use R16G16B16A16_SFLOAT to preserve HDR range through the chain.

### Contact Shadows

**Files:** `renderer/postprocess.rs`, `contact_shadows.glsl`, `bilateral_blur.glsl`

Screen-space ray march from the depth buffer toward the directional light, producing a per-pixel shadow factor. This catches small-scale contact occlusion that cascaded shadow maps miss at their resolution limit.

**Pipeline:**

1. **Contact shadow pass** — fullscreen triangle rendering via `contact_shadows.glsl`. Reconstructs world-space position from depth, marches a ray toward the light in clip space (perspective-correct), and tests for occlusion against the depth buffer using NDC-space comparisons with ULP-scaled epsilon and thickness thresholds
2. **Bilateral blur** — two-pass edge-preserving blur (horizontal then vertical) via `bilateral_blur.glsl`. Respects depth discontinuities to avoid shadow bleeding across edges. Skipped in debug modes
3. **Composite** — the blurred shadow factor is sampled in `postprocess_composite.glsl` and multiplied into the final color (`apply_shadow` push constant flag)

**Settings on `PostProcessPipeline`:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `contact_shadows_enabled` | `bool` | `false` | Master toggle |
| `contact_shadows_max_distance` | `f32` | `0.5` | World-space ray march length |
| `contact_shadows_thickness` | `f32` | `0.15` | World-space occluder thickness for depth comparison |
| `contact_shadows_intensity` | `f32` | `0.6` | Shadow strength (0 = no shadow, 1 = full black) |
| `contact_shadows_step_count` | `i32` | `24` | Number of ray march steps |
| `contact_shadows_debug` | `i32` | `0` | Debug visualization mode (0=normal, 1=linear depth, 2=raw shadow, 3=precision) |

**Push constants** (`ContactShadowPushConstants`, 176 bytes): inverse VP matrix, VP matrix, light direction, max_distance, thickness, intensity, step_count, near/far planes, debug_mode.

**Activation conditions** (`contact_shadows_active()`): enabled flag set, directional light present (`cs_has_light`), contact shadow pipeline and intermediate images allocated, depth and normal descriptor sets available.

**Shader details** (`contact_shadows.glsl`):
- Fullscreen triangle vertex shader (no vertex buffer)
- Fragment shader inputs: depth texture (set 0) and normal texture (set 1)
- Interleaved gradient noise (Jimenez 2014) for per-pixel jitter to break up stairstepping
- Linearizes reverse-Z depth for world-space reconstruction
- Normal-aware fade: attenuates shadows on surfaces facing away from the light
- Precision fade: smoothly disables shadows where float32 depth precision is insufficient (based on ULP headroom analysis)
- Sky pixels (NDC depth <= 0.0001) and 2D sprites (normal alpha < 0.5) are fully lit early-out

## Compressed Textures

**File:** `renderer/texture.rs`

### ImageFormat

`ImageFormat` enumerates all supported pixel formats, including block-compressed variants:

| Variant | Vulkan Format | Block | Bytes/Block | Description |
|---------|--------------|-------|-------------|-------------|
| `Rgba8Srgb` | R8G8B8A8_SRGB | 1x1 | 4 | Standard color textures (default) |
| `Rgba8Unorm` | R8G8B8A8_UNORM | 1x1 | 4 | Data textures, MSDF atlases |
| `Bc1Srgb` | BC1_RGBA_SRGB_BLOCK | 4x4 | 8 | RGB + 1-bit alpha (DXT1) |
| `Bc3Srgb` | BC3_SRGB_BLOCK | 4x4 | 16 | RGB + full alpha (DXT5) |
| `Bc5Unorm` | BC5_UNORM_BLOCK | 4x4 | 16 | Two-channel (normal maps) |
| `Bc7Srgb` | BC7_SRGB_BLOCK | 4x4 | 16 | High quality RGBA |
| `Astc4x4Srgb` | ASTC_4X4_SRGB_BLOCK | 4x4 | 16 | ASTC 4x4 |
| `Astc6x6Srgb` | ASTC_6X6_SRGB_BLOCK | 6x6 | 16 | ASTC 6x6 |
| `Astc8x8Srgb` | ASTC_8X8_SRGB_BLOCK | 8x8 | 16 | ASTC 8x8 |
| `Rgba16Float` | R16G16B16A16_SFLOAT | 1x1 | 8 | HDR cubemaps, IBL textures |
| `Rg16Float` | R16G16_SFLOAT | 1x1 | 4 | BRDF integration LUT |

### Helper Methods

| Method | Return | Description |
|--------|--------|-------------|
| `is_compressed()` | `bool` | `true` for BC and ASTC formats |
| `block_dimensions()` | `(u32, u32)` | Block width/height (1x1 for uncompressed) |
| `block_bytes()` | `u32` | Bytes per block (or per pixel for uncompressed) |
| `data_size(width, height)` | `u64` | Expected data size in bytes, accounting for block alignment via `div_ceil` |

### Texture2D::from_compressed()

```rust
pub(crate) fn from_compressed(
    res: &RendererResources<'_>,
    allocator: &Arc<Mutex<GpuAllocator>>,
    width: u32,
    height: u32,
    data: &[u8],
    format: ImageFormat,
    spec: &TextureSpecification,
) -> EngineResult<Self>
```

Uploads pre-compressed texture data to the GPU. Key behaviors:

- **Validates format**: returns an error if `format.is_compressed()` is `false`
- **Validates data size**: compares `data.len()` against `format.data_size(width, height)` and rejects mismatches
- **No mipmap generation**: compressed formats cannot be blitted, so images are created with `mip_levels = 1`. Pre-computed mipmaps must be included in the source data
- **No TRANSFER_SRC**: image usage is `TRANSFER_DST | SAMPLED` only (no blit source capability needed)
- **Device-local memory**: allocated via `GpuAllocator` with `GpuOnly` location, staging buffer upload

Currently `pub(crate)` (reserved for future compressed texture loading from asset pipeline).

### Runtime Format Support Check

```rust
// On VulkanContext:
pub fn is_format_supported(
    &self,
    instance: &ash::Instance,
    format: vk::Format,
    features: vk::FormatFeatureFlags,
) -> bool
```

Queries `vkGetPhysicalDeviceFormatProperties` for the physical device and checks whether the requested `features` are present in `optimal_tiling_features`. Use this to verify compressed format support before attempting to create textures (e.g., ASTC may not be supported on desktop GPUs).

## GPU Profiling

**File:** `renderer/gpu_profiling.rs`

`GpuProfiler` measures GPU-side timing using Vulkan timestamp queries. It records sequential timestamp markers at key points in the frame's command buffer and reports the time between consecutive markers with 1-frame latency.

### Architecture

- **Query pools**: one `vk::QueryPool` per frame-in-flight (`MAX_FRAMES_IN_FLIGHT = 2`), each with capacity for `MAX_TIMESTAMPS = 16` sequential markers
- **Pipeline stage**: all timestamps recorded at `BOTTOM_OF_PIPE` to capture full GPU work
- **Timestamp period**: `VulkanContext::timestamp_period_ns()` provides the device-specific nanosecond-per-tick conversion factor
- **1-frame latency**: results are read back from the previous use of each frame slot (after `wait_for_fences`), so displayed timing is always one frame behind

### GpuProfiler Struct

```rust
pub struct GpuProfiler {
    query_pools: [vk::QueryPool; MAX_FRAMES_IN_FLIGHT],
    timestamp_period_ns: f32,
    query_counts: [u32; MAX_FRAMES_IN_FLIGHT],
    query_names: [Vec<&'static str>; MAX_FRAMES_IN_FLIGHT],
    results: Vec<GpuTimingResult>,
    total_frame_ms: f32,
    enabled: bool,
}
```

### API

| Method | Description |
|--------|-------------|
| `new(device, timestamp_period_ns)` | Creates query pools (one per frame-in-flight) |
| `set_enabled(bool)` | Enable/disable profiling. Clears results when disabled |
| `is_enabled()` | Query enabled state |
| `begin_frame(cmd_buf, current_frame)` | Reads back results from previous frame slot (`get_query_pool_results` with `TYPE_64 \| WAIT`), resets query pool via `cmd_reset_query_pool` |
| `timestamp(cmd_buf, current_frame, name)` | Records a timestamp marker. The `name` labels the region between this marker and the next |
| `results()` | Returns `&[GpuTimingResult]` — timing between consecutive markers |
| `total_frame_ms()` | Total GPU frame time (first marker to last) |

### GpuTimingResult

```rust
pub struct GpuTimingResult {
    pub name: &'static str,
    pub time_ms: f32,
}
```

Time deltas are computed from consecutive 64-bit timestamps: `delta_ticks * timestamp_period_ns / 1,000,000`.

### Frame Markers

The engine records timestamps at these points in `application.rs` (after `begin_frame`):

```
Particles → Shadows → Scene → PostProcess → Egui → End
```

Each result measures the GPU time for the section whose name appears at the *start* of the region. For example, "Particles" measures the GPU time between the Particles marker and the Shadows marker.

### Renderer Integration

- `Renderer` holds an `Option<GpuProfiler>`, initialized via `init_gpu_profiler(timestamp_period_ns)`
- Accessed via `renderer.gpu_profiler()` / `renderer.gpu_profiler_mut()`
- Query pools are destroyed on drop (via `Drop` impl)
- Created lazily — not allocated if GPU profiling is never enabled

### Editor Integration

The editor Settings panel provides a "GPU Timestamps" checkbox (`GpuTimingSnapshot` struct). When enabled:

1. `gpu_timing.enabled` is synced to the profiler each frame via `profiler.set_enabled()`
2. Results are copied from the profiler into `GpuTimingSnapshot` for UI display
3. The panel shows total GPU frame time and per-section breakdown (e.g., "Shadows: 0.142 ms")
