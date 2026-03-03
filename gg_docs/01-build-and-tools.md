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
cargo build --profile dist --no-default-features               # dist

# Run
cargo run -p gg_sandbox                                        # dev
cargo run -p gg_sandbox --release                              # release
cargo run -p gg_sandbox --profile dist --no-default-features   # dist

# Build specific crates
cargo build -p gg_engine
cargo build -p gg_editor
cargo build -p gg_sandbox
```

### Feature Flag Chain

The `profiling` feature controls instrumentation:
2
```
gg_sandbox / gg_editor
    │  profiling = ["gg_engine/profiling"]  (default on)
    │  depends on gg_engine with default-features = false
    ▼
gg_engine
    profiling feature (default on)
```

Dist builds pass `--no-default-features` to disable the entire chain.

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
```

### Output

- **Frame summary:** average/min/max time, FPS
- **Top functions by total time**
- **Top functions by average time**
- **Top functions by call count**

Handles truncated JSON gracefully (e.g. from force-killed sessions).

## Shader Compilation

Shader compilation is automatic via `gg_engine/build.rs` (requires `glslc` from Vulkan SDK on PATH).

### Pipeline

1. `build.rs` reads `.glsl` files from `gg_engine/src/renderer/shaders/`
2. Splits on `#type vertex` / `#type fragment` markers
3. Compiles each stage to SPIR-V via `glslc`
4. SPIR-V bytes exposed as `pub const` in `gg_engine::shaders`

```rust
// Example: FLAT_COLOR_VERT_SPV, FLAT_COLOR_FRAG_SPV
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
