# GGEngine

A 2D game engine written in Rust, built on Vulkan. Create scenes in the editor, script gameplay in Lua, and ship standalone builds with the player runtime.

~22,600 lines of Rust across 5 crates. Early stage (v0.1.0).

## What You Get

- **Visual Scene Editor** — Dockable panels (viewport, hierarchy, properties, content browser), transform gizmos, mouse picking, undo/redo, auto-save & recovery
- **Entity Component System** — Built on hecs with parent-child hierarchies, sprites, circles (SDF), text (MSDF), tilemaps, cameras, audio sources
- **2D Physics** — rapier2d integration with rigid bodies, box/circle colliders, collision events, fixed-timestep simulation with interpolation
- **Lua Scripting** — LuaJIT via mlua with per-entity isolated environments, hot reload, configurable script fields exposed in the editor, collision callbacks
- **Batch Renderer** — Vulkan backend with bindless textures (4096 max), batched quads/circles/lines/text, dual-pass rendering (offscreen scene + egui overlay)
- **MSDF Text** — Pure-Rust multi-channel signed distance field text rendering, no C dependencies
- **Sprite Animation** — Sprite sheet animator with named clips, configurable FPS, looping
- **Tilemap System** — Grid-based tile maps with visual palette painting in the editor, flip flags, Lua API
- **Audio** — kira-based playback (WAV, OGG, MP3, FLAC) with per-entity sources, play-on-start, volume/pitch/looping control from Lua
- **Asset System** — UUID-based handles, persistent YAML registry, async background loading, LRU texture cache
- **Project System** — `.ggproject` files organize assets/scenes/scripts; project hub with recent projects and creation wizard
- **Standalone Player** — Minimal runtime that loads a project and runs the game without editor overhead
- **Profiling** — Chrome Tracing JSON output, analyzable with the included CLI tool or `chrome://tracing`

## Prerequisites

- **Rust** — Install via [rustup](https://www.rust-lang.org/tools/install)
- **Vulkan SDK** — Install from [LunarG](https://vulkan.lunarg.com/). `glslc` must be on your `PATH` (shaders compile at build time)

## Quick Start

```sh
# Clone and build
git clone <repo-url> && cd GGEngine
cargo build

# Run the editor
cargo run -p gg_editor

# Open an existing project
cargo run -p gg_editor -- path/to/MyGame.ggproject
```

On first launch, the editor opens a **Project Hub** where you can create a new project or open an existing one. Projects organize your assets, scenes, and scripts into a self-contained directory.

## Editor at a Glance

```
+----------+--------------+------------------+
| Project  |              | Scene Hierarchy  |
+----------+   Viewport   +------------------+
| Settings |              |    Properties    |
+----------+--------------+                  |
|     Content Browser     |                  |
+-------------------------+------------------+
```

- **Viewport** — Scene view with transform gizmos (Q/W/E/R), grid, mouse picking
- **Scene Hierarchy** — Entity tree with drag-and-drop reparenting, search
- **Properties** — Component inspector with add/remove, color pickers, asset drag-and-drop
- **Content Browser** — File and asset browser with import, rename, delete, drag-and-drop
- **Settings** — Renderer stats, VSync, physics collider viz, grid settings
- **Project** — Scene list, project info

**Key shortcuts:** Ctrl+N (new scene), Ctrl+O (open), Ctrl+S (save), Ctrl+Z/Y (undo/redo), Ctrl+D (duplicate), Del (delete), Play/Stop/Simulate via toolbar

## Lua Scripting

Attach `.lua` scripts to entities. Scripts run in isolated per-entity environments with access to the `Engine` API:

```lua
fields = {
    speed = 5.0,
    jump_force = 10.0,
}

function on_create()
    print("Player spawned!")
end

function on_update(dt)
    local vx = 0
    if Engine.is_key_down("D") then vx = fields.speed end
    if Engine.is_key_down("A") then vx = -fields.speed end
    Engine.set_linear_velocity(entity_id, vx, 0)
end

function on_fixed_update(dt)
    if Engine.is_key_pressed("Space") then
        Engine.apply_impulse(entity_id, 0, fields.jump_force)
    end
end

function on_collision_enter(other_uuid)
    local name = Engine.get_entity_name(other_uuid)
    print("Hit: " .. name)
end
```

The `fields` table is editable per-entity in the Properties panel — override values without changing code. Scripts hot-reload on save during play mode.

**Engine API covers:** transforms, input (keyboard + mouse), physics (impulse/force/velocity), entity queries, hierarchy, audio, animation, tilemaps, math utilities, cross-entity field access.

## Building & Running

```sh
# Development (debug + Vulkan validation layers)
cargo build
cargo run -p gg_editor

# Release (optimized, profiling still on)
cargo build --release

# Distribution (optimized, profiling stripped)
cargo build --profile dist --no-default-features --features lua-scripting

# Run the standalone player
cargo run -p gg_player -- MyGame.ggproject

# Run tests
cargo test                          # all
cargo test -p gg_engine             # engine only
cargo test -p gg_engine -- test_fn  # single test

# Analyze a runtime profile
cargo run -p gg_tools               # auto-detects profile JSON
```

## Shipping a Game

Build the player with the `dist` profile and bundle it alongside your project:

```sh
cargo build --profile dist -p gg_player --no-default-features --features lua-scripting
```

```
dist/
├── gg_player.exe
├── MyGame.ggproject
└── assets/
    ├── AssetRegistry.ggregistry
    ├── scenes/
    ├── textures/
    ├── scripts/
    └── audio/
```

The player auto-detects `.ggproject` files next to the executable, or accepts a path as a CLI argument. Press `V` at runtime to toggle VSync.

## Workspace

| Crate | Type | Description |
|-------|------|-------------|
| `gg_engine` | lib | Core engine — Vulkan renderer, ECS, physics, scripting, audio, assets, text, UI |
| `gg_editor` | bin | Scene editor with dockable panels, gizmos, content browser |
| `gg_player` | bin | Standalone game runtime (loads `.ggproject`, runs start scene) |
| `gg_sandbox` | bin | Sandbox for testing engine features directly |
| `gg_tools` | bin | CLI for analyzing Chrome Tracing JSON profiles |

## Debugging (VS Code)

1. Install the [C/C++ extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode.cpptools)
2. Select **Debug GGEditor** or **Debug GGSandbox** from the launch dropdown
3. Press **F5**

## Platform

Primary target is **Windows 11** with an RTX GPU / Vulkan 1.3+. macOS is conditionally supported.

## License

MIT
