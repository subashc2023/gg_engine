# Player & Project System

The project system provides a structured way to organize game assets, scenes, and scripts into a self-contained directory. The standalone player (`gg_player`) loads a project and runs the game without any editor overhead.

## Project System

**File:** `gg_engine/src/project.rs`

### ProjectConfig

`ProjectConfig` is the runtime representation of a project's settings. It is loaded from and saved to a `.ggproject` YAML file.

| Field | Type | Description |
|-------|------|-------------|
| `schema_version` | `u32` | Project file format version (current: 1) |
| `name` | `String` | Human-readable project name |
| `asset_directory` | `String` | Relative path to the assets root (typically `assets`) |
| `script_module_path` | `String` | Relative path to the Lua scripts directory |
| `start_scene` | `String` | Path to the default scene, relative to the asset directory |

### .ggproject YAML Format

The project file uses capitalized field names via serde rename attributes. A real-world example (`Sandbox.ggproject`):

```yaml
Project:
  Name: Sandbox
  AssetDirectory: assets
  ScriptModulePath: assets/scripts
  StartScene: scenes/text.ggscene
```

The `SchemaVersion` field is optional in the YAML and defaults to `CURRENT_SCHEMA_VERSION` (1) when absent. This means projects created before schema versioning was added are treated as version 1. When present:

```yaml
Project:
  SchemaVersion: 1
  Name: MyGame
  AssetDirectory: assets
  ScriptModulePath: assets/scripts
  StartScene: scenes/main.ggscene
```

### Schema Versioning

The `CURRENT_SCHEMA_VERSION` constant (currently `1`) tracks the project file format. Infrastructure is in place for future migrations:

- On load, if the file's schema version is **greater** than the current engine version, a warning is logged (forward compatibility).
- The stored version is clamped to `CURRENT_SCHEMA_VERSION` via `.min()`.
- Migration stubs are marked in the code: `if schema_version < 2 { migrate_v1_to_v2(...); }`.

### Project API

| Method | Signature | Description |
|--------|-----------|-------------|
| `load` | `(file_path: &str) -> Option<Project>` | Load an existing `.ggproject` YAML file |
| `new` | `(file_path: &str, name: &str) -> Option<Project>` | Create a new project with defaults and save it |
| `save` | `(&self) -> bool` | Serialize to the project's YAML file (atomic write) |
| `name` | `(&self) -> &str` | Project name |
| `config` | `(&self) -> &ProjectConfig` | Full config reference |
| `project_directory` | `(&self) -> &Path` | Directory containing the `.ggproject` file |
| `project_file_path` | `(&self) -> &str` | Path to the `.ggproject` file itself |

### Path Helpers

All path helpers return absolute paths by joining relative config values against the project directory.

| Method | Returns | Example |
|--------|---------|---------|
| `asset_directory_path()` | `project_dir / asset_directory` | `/game/assets` |
| `script_module_path()` | `project_dir / script_module_path` | `/game/assets/scripts` |
| `get_asset_path(relative)` | `asset_dir / relative` | `/game/assets/textures/hero.png` |
| `start_scene_path()` | `asset_dir / start_scene` | `/game/assets/scenes/main.ggscene` |

### Default Values for New Projects

When creating a project via `Project::new()`:

| Field | Default |
|-------|---------|
| `asset_directory` | `assets` |
| `script_module_path` | `assets/scripts` |
| `start_scene` | `scenes/new.ggscene` |

### Project Directory Structure

A typical project on disk:

```
MyGame/
├── MyGame.ggproject              # Project config
├── assets/
│   ├── AssetRegistry.ggregistry  # UUID-to-path asset mapping
│   ├── scenes/
│   │   └── main.ggscene          # YAML scene file
│   ├── textures/
│   │   └── hero.png
│   ├── scripts/
│   │   ├── player.lua
│   │   └── camera.lua
│   └── audio/
│       └── music.ogg
└── (auto-saves, if enabled)
```

When a project is loaded, the engine sets the current working directory (`CWD`) to the project directory. This means all relative paths in the project (asset paths, scene paths, script paths) resolve correctly from the project root.

### Editor Integration

- **Open**: `File > Open Project` shows a native file dialog filtered to `.ggproject` files. Also accepts a CLI argument: `cargo run -p gg_editor -- path/to/game.ggproject`.
- **Create**: The project creation wizard (`File > New Project`) calls `Project::new()` to scaffold a project.
- **Start scene**: Auto-loaded when a project is opened. The editor deserializes the scene specified by `start_scene`.
- **Script watcher**: When Lua scripting is enabled, the `notify` file watcher is (re)started for the new project's `script_module_path` directory, enabling hot reload of `.lua` files.
- **CWD**: Set to `project_directory()` on project load, so all relative asset paths resolve correctly.

## Standalone Player (gg_player)

**Files:** `gg_player/src/main.rs`, `gg_player/src/player.rs`

### Purpose

`gg_player` is a minimal `Application` implementation that loads a `.ggproject` file and runs the game loop. It is the shipping runtime -- no editor UI, no entity picking, no gizmos. The entire crate is roughly 186 lines of Rust plus a 7-line `main.rs`.

### Architecture

`GGPlayer` implements the `Application` trait with the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `project_name` | `String` | Displayed in the window title |
| `scene` | `Scene` | The active game scene |
| `asset_manager` | `Option<EditorAssetManager>` | Resolves texture and audio handles |
| `window_width` / `window_height` | `u32` | Current window dimensions |
| `textures_loaded` | `bool` | Whether first-frame asset init has run |
| `runtime_started` | `bool` | Whether `on_runtime_start` has been called |
| `present_mode` | `PresentMode` | Current VSync mode (toggleable at runtime) |

### Initialization Sequence

1. Parse CLI arguments (`parse_args()`).
2. Resolve the project path (CLI arg or auto-detect next to executable).
3. `Project::load()` the `.ggproject` file.
4. Set CWD to the project directory.
5. Deserialize the start scene via `SceneSerializer::deserialize()`.
6. Create `EditorAssetManager` and load the asset registry.
7. Determine present mode from `--vsync` / `--no-vsync` flags.

### Lazy First-Frame Initialization

Textures and the runtime are not initialized in `new()` because the Vulkan renderer is not yet available at that point. Instead, the first call to `on_render()` performs:

1. Set viewport size on the scene.
2. Resolve texture handles (sync load from asset manager to GPU).
3. Resolve audio handles (map UUID handles to file paths).
4. Load fonts (MSDF atlas generation and GPU upload).
5. Start the scene runtime (`on_runtime_start()` -- initializes physics, loads Lua scripts).

### Game Loop

Each frame after initialization:

- **`on_update(dt, input)`**: Runs physics (`on_update_physics`), Lua scripts (`on_update_lua_scripts` when the `lua-scripting` feature is enabled), and animations (`on_update_animations`).
- **`on_render(renderer)`**: Renders the scene through the ECS camera via `scene.on_update_runtime(renderer)`.
- **`on_event(event, input)`**: Handles window resize and the `V` key for VSync toggling.

### CLI Usage

```sh
# Run with a project file
cargo run -p gg_player -- path/to/game.ggproject
cargo run -p gg_player -- Sandbox.ggproject

# Override window size
cargo run -p gg_player -- --width 1920 --height 1080 Sandbox.ggproject

# Enable VSync (Fifo present mode)
cargo run -p gg_player -- --vsync Sandbox.ggproject
```

Full CLI options:

| Flag | Description | Default |
|------|-------------|---------|
| `<path>.ggproject` | Project file path (positional) | Auto-detect next to executable |
| `--width N` | Window width | 1280 |
| `--height N` | Window height | 720 |
| `--vsync` | Enable VSync (Fifo present mode) | off |
| `--no-vsync` | Disable VSync (Mailbox present mode) | default |
| `--help`, `-h` | Print usage and exit | |

### Project Auto-Detection

When no project path is given on the command line, `find_project_path_auto()` searches the directory containing the player executable for a `.ggproject` file. This enables a distribution workflow where you place the player binary next to the project file and double-click to launch.

### Building for Distribution

```sh
# Debug build
cargo build -p gg_player

# Release build (optimized, profiling still on)
cargo build --release -p gg_player

# Shipping build (profiling stripped, Lua scripting kept)
cargo build --profile dist -p gg_player --no-default-features --features lua-scripting
```

The `dist` profile strips debug info, enables LTO, and compiles out profiling macros. The `--no-default-features --features lua-scripting` flag combination removes the `profiling` feature while keeping Lua scripting.

A minimal distribution package:

```
dist/
├── gg_player.exe          # Built with --profile dist
├── MyGame.ggproject
└── assets/
    ├── AssetRegistry.ggregistry
    ├── scenes/
    ├── textures/
    ├── scripts/
    └── audio/
```

### Runtime Behavior

- **Window title**: Set to the project name (e.g., "Sandbox").
- **Default window size**: 1280x720.
- **Default present mode**: Mailbox (no VSync) -- same as the `Application` trait default.
- **VSync toggle**: Press `V` at runtime to switch between Mailbox (no VSync) and Fifo (VSync). The current mode is logged to the console.
- **Window decorations**: Enabled (standard OS title bar). The player does not use the custom title bar that the editor uses.
- **Viewport resize**: Handled via `on_event`, updating both the stored dimensions and the scene's viewport.

### Differences from Editor

| Feature | Editor | Player |
|---------|--------|--------|
| Render passes | Dual (offscreen + swapchain) | Single (swapchain only) |
| Entity picking | Yes (RedInteger attachment readback) | No |
| Gizmos | Yes (transform-gizmo-egui overlay) | No |
| egui UI | Yes (dockable panels, menus, properties) | No |
| Custom title bar | Yes (play controls, project/scene name) | No (OS decorations) |
| Native scripts | Yes (code-defined via NativeScriptComponent) | No |
| Lua scripts | Yes (with hot reload via notify watcher) | Yes (no hot reload) |
| Physics | Yes | Yes |
| Audio | Yes | Yes |
| Animations | Yes | Yes |
| Scene editing | Yes (create, modify, save) | No (read-only load) |
| Asset importing | Yes (Content Browser) | No (uses pre-built registry) |
| Play/Stop modes | Yes (Scene::copy snapshots) | No (always running) |
| VSync toggle | Via Settings panel | `V` key or `--vsync` flag |
| Window config | Persistent size/position/maximized state | CLI flags only |
