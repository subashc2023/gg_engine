# GGEngine

A Rust game engine built on Vulkan. Early stage (v0.1.0).

## Workspace

| Crate | Type | Description |
|-------|------|-------------|
| `gg_engine` | Library | Core engine — windowing, Vulkan renderer, egui UI, input, cameras, profiling |
| `gg_editor` | Binary | Editor application (egui-based UI shell) |
| `gg_sandbox` | Binary | Sandbox for testing engine features |
| `gg_tools` | Binary | Offline CLI for analyzing Chrome Tracing JSON profiles |

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (install via `rustup`)
- [Vulkan SDK](https://vulkan.lunarg.com/) — `glslc` must be on `PATH` for shader compilation at build time

## Building

```sh
cargo build                                       # dev (debug, Vulkan validation layers)
cargo build --release                             # release (optimized, profiling on)
cargo build --profile dist --no-default-features --features lua-scripting  # dist (optimized, profiling stripped)
```

## Running

```sh
cargo run -p gg_editor      # Editor
cargo run -p gg_sandbox     # Sandbox
```

## Testing

```sh
cargo test                          # all tests
cargo test -p gg_engine             # engine tests only
cargo test -p gg_engine -- test_fn  # single test by name
```

## Profiling

The engine automatically writes Chrome Tracing JSON profiles next to the executable when the `profiling` feature is enabled (on by default, stripped in `dist`). Open the `.json` files in `chrome://tracing` or `edge://tracing`.

```sh
# Analyze a runtime profile
cargo run -p gg_tools                         # auto-detects gg_profile_runtime.json next to exe
cargo run -p gg_tools -- path/to/profile.json # explicit path
```

## Debugging (VSCode)

1. Install the [C/C++ extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode.cpptools)
2. Select **Debug GGEditor** or **Debug GGSandbox** from the launch dropdown
3. Press **F5**

## License

MIT
