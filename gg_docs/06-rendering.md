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

### Compilation Pipeline

`gg_engine/build.rs` auto-compiles `.glsl` to SPIR-V via `glslc` at build time:
1. Reads `.glsl` files from `renderer/shaders/`
2. Splits on `#type vertex` / `#type fragment` markers
3. Compiles each stage to SPIR-V
4. Exposes as `pub const` byte slices in `gg_engine::shaders`

```rust
// Example: accessing compiled shaders
use gg_engine::shaders::{FLAT_COLOR_VERT_SPV, FLAT_COLOR_FRAG_SPV};
```

### Available Shaders

| File | Purpose |
|------|---------|
| `batch.glsl` | Batch 2D quad rendering (offscreen, 2 outputs) |
| `batch_swapchain.glsl` | Batch 2D quad rendering (swapchain, 1 output) |
| `circle.glsl` | SDF circle rendering (offscreen, 2 outputs) |
| `circle_swapchain.glsl` | SDF circle rendering (swapchain, 1 output) |
| `line.glsl` | Line rendering (offscreen, 2 outputs) |
| `line_swapchain.glsl` | Line rendering (swapchain, 1 output) |
| `flat_color.glsl` | Flat color rendering |
| `texture.glsl` | Textured rendering |
| `triangle.glsl` | Basic triangle (legacy) |

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
// CameraData: #[repr(C)], 64 bytes (Mat4), std140 compatible
struct CameraData {
    view_projection: Mat4,
}
```

Written to the UBO once per frame in `begin_scene` and whenever `set_view_projection` is called.

## Textures

**File:** `renderer/texture.rs`

`Texture2D` — Vulkan image + image view + sampler + descriptor set.

### Creation

```rust
let tex = renderer.create_texture_from_file("path/to/image.png");
let tex = renderer.create_texture_from_rgba8(width, height, &pixels);
```

### Internals

- Device-local memory with staging buffer upload
- One-shot command buffer for layout transitions: UNDEFINED → TRANSFER_DST → SHADER_READ_ONLY
- Sampler: NEAREST filtering, REPEAT addressing, 16x anisotropic filtering
- Each texture gets a `bindless_index: u32` on creation
- Destroyed on drop (sampler, image view, image, memory)

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
| `draw_particle(pos, size, rotation, color)` | Optimized for particles |

Position sets the **center**, size scales the unit quad (-0.5..0.5). Z-ordering is respected via depth testing.

### ECS Draw Methods

| Method | Description |
|--------|-------------|
| `draw_quad_transform(transform: &Mat4, color, entity_id)` | Quad from raw transform |
| `draw_sprite(transform: &Mat4, sprite: &SpriteRendererComponent, entity_id)` | Sprite component |
| `draw_circle(transform, color, thickness, fade, entity_id)` | SDF circle |
| `draw_circle_component(transform, circle: &CircleRendererComponent, entity_id)` | Circle component |

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
| `draw_rect(position, size, color)` | Wireframe XY rectangle (4 lines) |
| `draw_rect_transform(transform, color, entity_id)` | Wireframe rect via transform |

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
