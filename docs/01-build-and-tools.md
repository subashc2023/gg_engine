# Build & Tools

## Build Profiles

Three Cargo profiles are configured in the workspace `Cargo.toml`:

| Profile | Optimization | Debug Info | LTO | Strip | Profiling | Validation Layers |
|---------|-------------|-----------|-----|-------|-----------|-------------------|
| `dev` | 0 | full | no | no | yes | yes |
| `release` | 3 | none | fat | symbols | yes | no |
| `dist` | 3 | none | fat | all | **no** | no |

### Usage

```sh
# Build
cargo build                                                    # dev
cargo build --release                                          # release
cargo build --profile dist --no-default-features --features lua-scripting  # dist

# Run
cargo run -p gg_editor                                         # editor (primary dev target)
cargo run -p gg_sandbox                                        # sandbox
cargo run -p gg_player -- Sandbox.ggproject                    # standalone player

# Dist build for shipping (player)
cargo build --profile dist -p gg_player --no-default-features --features lua-scripting

# Build specific crates
cargo build -p gg_engine
cargo build -p gg_editor
cargo build -p gg_player
cargo build -p gg_sandbox
```

### Feature Flag Chain

Two default features control instrumentation and scripting:

```
gg_editor / gg_sandbox / gg_player
    │  profiling = ["gg_engine/profiling"]           (default on)
    │  lua-scripting = ["gg_engine/lua-scripting"]    (default on)
    │  depends on gg_engine with default-features = false
    ▼
gg_engine
    default = ["profiling", "lua-scripting"]
    profiling   — Chrome Tracing instrumentation
    lua-scripting = ["mlua"]  — LuaJIT scripting via mlua
```

`gg_editor` additionally pulls in `notify 7` as an optional dependency tied
to the `lua-scripting` feature (`lua-scripting = ["gg_engine/lua-scripting", "dep:notify"]`),
enabling file-system watching for Lua script hot-reload during play sessions.

Dist builds pass `--no-default-features --features lua-scripting` to strip
profiling while keeping Lua scripting enabled.

### Workspace Dependencies

All versions are pinned in `[workspace.dependencies]` in the root `Cargo.toml`:

| Category | Crate | Version | Notes |
|----------|-------|---------|-------|
| Logging | `log` | 0.4 | `release_max_level_info` |
| Windowing | `winit` | 0.30 | |
| | `raw-window-handle` | 0.6 | |
| Vulkan | `ash` | 0.38 | |
| | `ash-window` | 0.13 | |
| | `gpu-allocator` | 0.28 | Vulkan backend, no defaults |
| UI | `egui` | 0.33 | |
| | `egui-winit` | 0.33 | |
| | `egui-ash-renderer` | 0.11 | |
| | `egui_dock` | 0.18 | Editor only, serde feature |
| | `transform-gizmo-egui` | 0.8 | Editor only |
| Math & ECS | `glam` | 0.29 | |
| | `hecs` | 0.11 | |
| Serialization | `serde` | 1 | derive feature |
| | `serde_yaml_ng` | 0.10 | |
| | `serde_json` | 1 | |
| Assets | `image` | 0.25 | png + jpeg |
| | `ttf-parser` | 0.25 | MSDF text |
| Physics | `rapier2d` | 0.22 | simd-stable |
| Scripting | `mlua` | 0.10 | LuaJIT, vendored, optional |
| Audio | `kira` | 0.12 | |
| Misc | `rfd` | 0.15 | File dialogs |
| | `rand` | 0.9 | |
| | `notify` | 7 | File watching (editor, optional) |

## Profiling / Instrumentation

**File:** `gg_engine/src/profiling.rs`

All profiling is gated behind the `profiling` cargo feature (default on).

### Chrome Tracing JSON

The profiling system writes Chrome Tracing JSON files that can be viewed in `chrome://tracing` or `edge://tracing`, or analyzed offline with `gg_tools`.

**Automatic sessions** (always run):

| Session | File | Covers |
|---------|------|--------|
| Startup | `gg_profile_startup.json` | `T::new()` through `on_attach()` (Vulkan init, resource creation) |
| Shutdown | `gg_profile_shutdown.json` | `EngineRunner` drop (resource teardown, `device_wait_idle`) |

**Runtime session** (on-demand): The runtime session is **not** started automatically — it must be triggered explicitly to avoid per-frame overhead (mutex lock + disk I/O per scope). In the editor, use the **"Capture Trace"** button in the Settings panel. Programmatically:

```rust
// Start recording
gg_engine::profiling::begin_session("Runtime", "gg_profile_runtime.json");
// ... run for a while ...
// Stop and flush
gg_engine::profiling::end_session();
```

An atomic fast-path (`SESSION_ACTIVE`) skips the mutex entirely when no session is active, so `profile_scope!` / `ProfileTimer` have near-zero cost during normal operation.

Session management is in `application.rs`:
1. Startup session opens in `run()`
2. Startup session closes after `on_attach()` inside `resumed()`
3. Runtime session is on-demand (editor: Settings → Capture Trace)
4. `EngineRunner::Drop` calls `end_session()` (no-op if no session active)
5. Shutdown session wraps the `drop(runner)` call

Profile JSONs are written next to the executable (resolved via `std::env::current_exe()`), landing in `target/debug/`, `target/release/`, etc.

### profile_scope! Macro

RAII scope timer for instrumenting functions or blocks:

```rust
fn on_update(&mut self, dt: Timestep, input: &Input) {
    profile_scope!("MyApp::on_update");
    // ... timed work ...
}
```

When the `profiling` feature is disabled, the macro expands to nothing (zero cost).

`ProfileTimer` can also be used directly (the engine uses it for "Run loop", "Application::on_egui", etc.).

**Instrumented scopes** (built-in):
- **Engine loop:** `Run loop`, `LayerStack::on_update`
- **egui:** `Application::on_egui`, `egui::tessellate`, `egui::set_textures`
- **Render frame:** `render_frame`, `render_frame::wait_fence`, `render_frame::acquire_image`, `render_frame::record_commands`, `render_frame::queue_submit`, `render_frame::queue_present`
- **Renderer:** `Renderer::begin_scene`, `Renderer::end_scene`, `Renderer2D::flush_quads`, `Renderer2D::flush_circles`, `Renderer2D::flush_lines`, `Renderer2D::flush_text`
- **Scene:** `Scene::render_scene`, `Scene::build_world_transform_cache`, `Scene::render_sprites`, `Scene::render_circles`, `Scene::render_text`, `Scene::render_tilemaps`, `Scene::on_update_editor`, `Scene::on_update_runtime`, `Scene::on_update_physics`, `Scene::on_update_scripts`, `Scene::on_update_lua_scripts`, `Scene::resolve_texture_handles_async`
- **Editor:** `GGEditor::on_update`, `GGEditor::on_render`, `GGEditor::on_overlay_render`, `GGEditor::render_grid`

**Important:** Do NOT add `ProfileTimer` to per-draw-call functions (e.g. `draw_sprite`, `draw_quad`). These are called thousands of times per frame and the timer overhead (Instant::now × 2 + mutex lock when recording) dominates the function cost. Profile at batch/scene level instead.

### ProfileResult / drain_profile_results()

Thread-local per-frame results always available (independent of Chrome Tracing sessions). Apps can drain and display them in `on_egui` if desired. This is cheap — no mutex, no I/O.

### Typical profiling workflow

1. Run the editor (debug or release)
2. Open the **Settings** panel → click **"Capture Trace"**
3. Interact with the scene for a few seconds
4. Click **"Stop Capture"**
5. Analyze: `cargo run -p gg_tools -- target/debug/gg_profile_runtime.json`
6. Optionally generate a flame graph: add `--flamegraph` flag

## Logging

**File:** `gg_engine/src/logging.rs`

Custom `Log` implementation with:
- **Output:** colorized, timestamped to stderr (not stdout)
- **Tags:** messages from `gg_*` targets tagged `GGEngine`; all others tagged `APP`
- **Timestamps:** relative `MM:SS.mmm` from engine start
- **Initialization:** automatic via `run()`
- **Release behavior:** `release_max_level_info` (debug/trace compiled out in release)

## gg_tools (Profile Analyzer)

**Location:** `gg_tools/`

Offline CLI for analyzing Chrome Tracing JSON profiles. Depends on `serde` + `serde_json` only (no engine dependency).

### Usage

```sh
# Auto-detect gg_profile_runtime.json next to the executable
cargo run -p gg_tools

# Analyze a specific profile
cargo run -p gg_tools -- path/to/profile.json

# Generate a flame graph SVG alongside analysis
cargo run -p gg_tools -- --flamegraph path/to/profile.json
```

### Output

- **Frame summary:** average/min/max frame time, FPS (detected via "Run loop" events)
- **Top functions by total time** (with avg, max, call count)
- **Percentile analysis** (P50, P95, P99, max per function)
- **Top functions by average time**
- **Top functions by call count**
- **Flame graph SVG** (optional, via `--flamegraph` / `--svg` flag)

Handles truncated JSON gracefully (e.g. from force-killed sessions).

## Shader Compilation

Shader compilation is automatic via `gg_engine/build.rs` (requires `glslc` from Vulkan SDK on PATH).

### Build-Time Pipeline

1. `build.rs` reads `.glsl` files from `gg_engine/src/renderer/shaders/`
2. Splits on `#type vertex` / `#type fragment` markers
3. Compiles each stage to SPIR-V via `glslc` (target: `vulkan1.2`, `-O` in release/dist)
4. Optionally validates with `spirv-val` (skipped silently if not installed)
5. SPIR-V bytes exposed as `pub const` in `gg_engine::shaders`

### Runtime Hot-Reload

Shaders can be recompiled at runtime without restarting the application. In the editor, use Settings → **Reload Shaders**. Programmatically:

```rust
renderer.reload_shaders(shader_dir)?;
```

The hot-reload system (`renderer/shader_compiler.rs`) replicates the `build.rs` logic but returns `Result` for graceful error handling. All shaders are compiled first — if any fail, old pipelines remain intact. See `docs/06-rendering.md` for implementation details.

### Shader Files

| File | Generated Constants | Purpose |
|------|-------------------|---------|
| `batch.glsl` | `BATCH_VERT_SPV`, `BATCH_FRAG_SPV` | Textured quad batching (offscreen) |
| `batch_swapchain.glsl` | `BATCH_SWAPCHAIN_VERT_SPV`, `BATCH_SWAPCHAIN_FRAG_SPV` | Textured quad batching (swapchain) |
| `circle.glsl` | `CIRCLE_VERT_SPV`, `CIRCLE_FRAG_SPV` | SDF circle rendering (offscreen) |
| `circle_swapchain.glsl` | `CIRCLE_SWAPCHAIN_VERT_SPV`, `CIRCLE_SWAPCHAIN_FRAG_SPV` | SDF circle rendering (swapchain) |
| `line.glsl` | `LINE_VERT_SPV`, `LINE_FRAG_SPV` | Line rendering (offscreen) |
| `line_swapchain.glsl` | `LINE_SWAPCHAIN_VERT_SPV`, `LINE_SWAPCHAIN_FRAG_SPV` | Line rendering (swapchain) |
| `text.glsl` | `TEXT_VERT_SPV`, `TEXT_FRAG_SPV` | MSDF text rendering (offscreen) |
| `text_swapchain.glsl` | `TEXT_SWAPCHAIN_VERT_SPV`, `TEXT_SWAPCHAIN_FRAG_SPV` | MSDF text rendering (swapchain) |
| `instance.glsl` | `INSTANCE_VERT_SPV`, `INSTANCE_FRAG_SPV` | Instanced sprite rendering with GPU animation (offscreen) |
| `instance_swapchain.glsl` | `INSTANCE_SWAPCHAIN_VERT_SPV`, `INSTANCE_SWAPCHAIN_FRAG_SPV` | Instanced sprite rendering with GPU animation (swapchain) |
| `particle_sim.glsl` | `PARTICLE_SIM_COMP_SPV` | GPU particle simulation (compute shader) |

```rust
use gg_engine::shaders::*;
```

`*.spv` files are gitignored (generated at build time, not committed).

## Testing

```sh
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p gg_engine

# Run a single test by name
cargo test -p gg_engine -- test_name
```

## Code Quality

```sh
# Format code
cargo fmt

# Lint
cargo clippy --all-targets
```

Rust-analyzer with Clippy is configured as the default checker in `.vscode/settings.json`. Format on save is enabled.
