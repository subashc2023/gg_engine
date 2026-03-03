# GGEngine Documentation

GGEngine is a Rust game engine organized as a Cargo workspace (edition 2021, resolver v2). Currently at version 0.1.0 (early stage).

## Workspace Structure

```
GGEngine/
├── gg_engine/    Core engine library crate (foundation for both apps)
├── gg_editor/    Editor binary (depends on gg_engine)
├── gg_sandbox/   Sandbox/test binary (depends on gg_engine)
├── gg_tools/     Offline CLI for analyzing Chrome Tracing JSON profiles (standalone)
└── gg_docs/      This documentation
```

### Dependency Graph

```
gg_editor ──┐
            ├──► gg_engine
gg_sandbox ─┘

gg_tools (standalone, no engine dependency)
```

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (via `rustup`)
- [Vulkan SDK](https://vulkan.lunarg.com/) — `glslc` must be on PATH for shader compilation at build time

## Quick Start

```sh
# Build the entire workspace
cargo build

# Run the editor
cargo run -p gg_editor

# Run the sandbox
cargo run -p gg_sandbox

# Run all tests
cargo test
```

## Build Profiles

| Profile   | Opt | Debug Info | LTO | Strip   | Profiling | Validation Layers |
|-----------|-----|------------|-----|---------|-----------|-------------------|
| `dev`     | 0   | full       | no  | no      | yes       | yes               |
| `release` | 3   | none       | fat | symbols | yes       | no                |
| `dist`    | 3   | none       | fat | all     | **no**    | no                |

```sh
cargo build                                                    # dev
cargo build --release                                          # release
cargo build --profile dist --no-default-features               # dist (shipping)

cargo run -p gg_sandbox                                        # dev
cargo run -p gg_sandbox --release                              # release
cargo run -p gg_sandbox --profile dist --no-default-features   # dist
```

- **dev** — Fast iteration. Full debug info, Vulkan validation layers, profiling enabled.
- **release** — Optimized with profiling still available. Validation layers off.
- **dist** — Shipping build. Profiling compiled out via `--no-default-features`. Zero runtime overhead.

## Documentation Index

| Document | Description |
|----------|-------------|
| [Engine Core](engine-core.md) | Application trait, layer system, input, events, timestep, logging |
| [Rendering](rendering.md) | Vulkan backend, shaders, buffers, textures, cameras, 2D batch renderer |
| [ECS & Scene](ecs.md) | Entity Component System, scene management, serialization, native scripting |
| [Physics](physics.md) | 2D rigid body physics with rapier2d |
| [Editor](editor.md) | Editor application, panels, gizmos, viewport, play/stop |
| [Build & Tools](build-and-tools.md) | Profiling, build profiles, logging, gg_tools CLI |

## Code Style

- Rust-analyzer with Clippy is the configured linter (see `.vscode/settings.json`)
- Format on save is enabled via rust-analyzer
- Run `cargo fmt` to format code manually
- Run `cargo clippy --all-targets` to lint

## Debugging (VS Code)

Two launch configurations using GDB with Intel disassembly syntax:
- **Debug GGEditor** — builds and runs `gg_editor`
- **Debug GGSandbox** — builds and runs `gg_sandbox`

## External Dependencies (gg_engine)

| Crate | Version | Purpose |
|-------|---------|---------|
| `log` | 0.4 | Logging (with `release_max_level_info`) |
| `winit` | 0.30 | Windowing and event loop |
| `ash` | 0.38 | Vulkan bindings |
| `ash-window` | 0.13 | Vulkan surface creation |
| `raw-window-handle` | 0.6 | Window handle abstraction |
| `egui` | 0.33 | Immediate-mode UI |
| `egui-winit` | 0.33 | egui winit integration (no default features) |
| `egui-ash-renderer` | 0.11 | egui Vulkan rendering |
| `glam` | 0.29 | Math types (Vec2/Vec3/Vec4/Mat4/Quat) |
| `image` | 0.25 | Texture loading (PNG + JPEG only) |
| `hecs` | 0.11 | ECS (archetypal storage) |
| `rapier2d` | 0.22 | 2D physics |
| `serde` | 1 | Serialization (with derive) |
| `serde_yaml` | 0.9 | YAML scene files |
| `rand` | 0.9 | UUID generation |
| `rfd` | 0.15 | Native file dialogs |

### Editor-only Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui_dock` | 0.18 | Dockable tab panels |
| `transform-gizmo-egui` | 0.8 | 3D manipulation gizmos (uses glam 0.30 internally) |
