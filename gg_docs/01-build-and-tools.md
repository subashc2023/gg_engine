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

The engine automatically runs three profiling sessions:

| Session | File | Covers |
|---------|------|--------|
| Startup | `gg_profile_startup.json` | `T::new()` through `on_attach()` (Vulkan init, resource creation) |
| Runtime | `gg_profile_runtime.json` | Main loop (all frames from first to exit) |
| Shutdown | `gg_profile_shutdown.json` | `EngineRunner` drop (resource teardown, `device_wait_idle`) |

Session management is in `application.rs`:
1. Startup session opens in `run()`
2. Transitions to runtime after `on_attach()` inside `resumed()`
3. Runtime ends in `EngineRunner::Drop`
4. Shutdown wraps the `drop(runner)` call

Profile JSONs are written next to the executable (resolved via `std::env::current_exe()`), landing in `target/debug/`, `target/release/`, etc.

**Viewing:** Open `.json` files in `chrome://tracing` or `edge://tracing`.

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

### ProfileResult / drain_profile_results()

Thread-local per-frame results always available (independent of feature flag). Apps can drain and display them in `on_egui` if desired.

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

### Pipeline

1. `build.rs` reads `.glsl` files from `gg_engine/src/renderer/shaders/`
2. Splits on `#type vertex` / `#type fragment` markers
3. Compiles each stage to SPIR-V via `glslc` (target: `vulkan1.2`, `-O` in release/dist)
4. Optionally validates with `spirv-val` (skipped silently if not installed)
5. SPIR-V bytes exposed as `pub const` in `gg_engine::shaders`

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
