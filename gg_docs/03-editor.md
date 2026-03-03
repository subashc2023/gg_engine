# Editor

The editor (`gg_editor/`) is an egui-based application using `egui_dock` 0.18 for dockable tab panels.

**Main file:** `gg_editor/src/main.rs`

## Panels

### Scene Hierarchy

**File:** `gg_editor/src/panels/scene_hierarchy.rs`

- Lists all entities sorted by ID
- Click to select an entity
- Right-click blank space → "Create Empty Entity"
- Right-click entity → "Delete Entity" (deferred deletion with selection clearing)

### Viewport

**File:** `gg_editor/src/panels/viewport.rs`

- Displays the offscreen scene framebuffer as an egui image
- Tracks focus/hover state for input blocking
- Gizmo overlay renders here
- Mouse picking for entity selection
- Gizmo mode toolbar overlay (Q/W/E/R buttons) in the top-left corner

### Properties

**File:** `gg_editor/src/panels/properties.rs`

Component inspector for the selected entity:

| Component | Controls |
|-----------|----------|
| **Tag** | Editable name field |
| **Transform** | Vec3 controls with colored XYZ reset buttons for translation/rotation (degrees)/scale |
| **Camera** | Projection type combo, perspective/orthographic params, primary toggle, fixed aspect ratio |
| **Sprite Renderer** | RGBA color picker, texture path, tiling factor |
| **Circle Renderer** | Color, thickness, fade |
| **RigidBody2D** | Body type, fixed rotation |
| **BoxCollider2D** | Offset, size, density, friction, restitution, threshold |
| **CircleCollider2D** | Radius, offset, density, friction, restitution, threshold |
| **Lua Script** | Script path (file picker + drag-and-drop), field overrides with live editing in play mode |

- Removable components have a "+" settings button on the header for removal
- "Add Component" button at the bottom offers: Camera, Sprite Renderer, Circle Renderer, RigidBody2D, BoxCollider2D, CircleCollider2D, Lua Script

### Content Browser

**File:** `gg_editor/src/panels/content_browser.rs`

File browser for project assets with drag-and-drop support. Supports dragging `.ggscene` files into the viewport to load them, and `.lua` files onto the Lua Script component.

### Settings

**File:** `gg_editor/src/panels/settings.rs`

- Frame time display
- FPS counter
- VSync toggle
- Mouse picking debug info (hovered entity ID)

### Project

**File:** `gg_editor/src/panels/project.rs`

- Displays project name and active scene path
- Lists all `.ggscene` files in the project assets directory
- Click a scene to load it

## File Operations

### Menu Bar

Built with `egui::TopBottomPanel::top` + `egui::MenuBar::new().ui()`.

| Action | Shortcut | Description |
|--------|----------|-------------|
| New | Ctrl+N | New empty scene (clears selection, triggers viewport resize) |
| Open... | Ctrl+O | Open `.ggscene` file via native dialog |
| Save | Ctrl+S | Save to current path (or Save As if no path set) |
| Save As... | Ctrl+Shift+S | Save to new path via native dialog |
| Open Project... | — | Open `.ggproject` file via native dialog |

View menu:
- Show Physics Colliders — toggle collider visualization overlay

Script menu:
- Reload Scripts (Ctrl+R) — manually reload all Lua scripts

- `editor_scene_path` tracks the current file (set on open/save-as, cleared on new scene)
- All file shortcuts stop playback first if in play mode
- Unsaved changes shown as `*` in the title bar

### Entity Operations

| Action | Shortcut | Description |
|--------|----------|-------------|
| Duplicate | Ctrl+D | Duplicate selected entity (edit mode only) |
| Delete | Del | Delete selected entity (edit mode only) |
| Deselect | Escape | Clear entity selection (edit mode only) |

## Play / Stop / Simulate

**File:** `gg_editor/src/main.rs`

`SceneState` enum: `Edit`, `Play`, `Simulate`

### Play Flow

```
Edit Mode
    │ Play button (green triangle)
    ▼
1. Scene::copy() snapshots the current scene
2. Original stored as editor_scene
3. Native scripts attached by tag name (PhysicsPlayer, CameraController)
4. on_runtime_start() called on the copy (initializes physics + Lua scripts)
5. Enter Play mode (scripts + physics run each frame)
```

### Simulate Flow

```
Edit Mode
    │ Simulate button (gear icon)
    ▼
1. Scene::copy() snapshots the current scene
2. Original stored as editor_scene
3. on_simulation_start() called (initializes physics only, no scripts)
4. Enter Simulate mode (physics runs, editor camera still active)
```

Simulate mode differs from Play:
- Uses editor camera (orbit/pan/zoom), not the scene's primary camera
- Runs physics only — no NativeScripts or Lua scripts
- Mouse picking and gizmos still work (like Edit mode)

### Pause / Step

Available in both Play and Simulate modes:
- **Pause button** (two vertical bars) — toggles pause state. Background highlights when paused.
- **Step button** (play triangle + vertical bar) — appears only when paused. Advances exactly one frame using a fixed 1/60s timestep.

### Stop Flow

```
Play/Simulate Mode
    │ Stop button (blue square)
    ▼
1. on_runtime_stop() or on_simulation_stop() (tear down physics/scripts)
2. Swap editor_scene back as active scene
3. Push old runtime scene to pending_drop_scenes (GPU-safe deferred destruction)
4. Return to Edit mode, reset pause/step state
```

Key implementation details:
- `editor_scene: Option<Scene>` holds the original while playing/simulating (`None` during edit mode)
- `pending_drop_scenes: Vec<Scene>` for GPU-safe deferred destruction in `on_render`
- Textures shared via `Arc` between editor and runtime scenes
- Play↔Simulate transitions: stop current state first, then start the other

## Script Hot Reloading

The editor uses the `notify` crate to watch `assets/scripts/` for `.lua` file changes:

1. File watcher runs on a background thread, sets an atomic flag on Create/Modify events
2. Each frame, the editor checks the flag and calls `Scene::reload_lua_scripts()` if set
3. Reload calls `on_destroy()` for all entities, then reloads scripts from disk and calls `on_create()`
4. Field overrides are re-applied after reload

Manual reload: Ctrl+R or Script menu > Reload Scripts.

## Custom Title Bar

**File:** `gg_editor/src/title_bar.rs`

Windows-specific (`#[cfg(not(target_os = "macos"))]`) custom title bar with integrated play/stop/pause/step controls and menu bar.

Displays: `GGEngine - [ProjectName] - [SceneName] [*]` (asterisk when unsaved changes exist).

## Gizmos

**File:** `gg_editor/src/gizmo.rs`

Uses `transform-gizmo-egui` 0.8 for 3D manipulation gizmos in the viewport.

### Operations

| Operation | Key | Button | Description |
|-----------|-----|--------|-------------|
| None | Q | Q button in viewport toolbar | No gizmo |
| Translate | W | W button in viewport toolbar | Move entity |
| Rotate | E | E button in viewport toolbar | Rotate entity |
| Scale | R | R button in viewport toolbar | Scale entity |

Keyboard shortcuts only fire **without** Ctrl/Shift modifiers (avoids conflicting with file commands).

### Snapping

Hold **Ctrl** to enable snapping:
- Rotation: 45 degrees
- Translation: 0.5 units
- Scale: 0.5 units

### Implementation Notes

- Projection matrix needs Vulkan Y-flip undone before passing to gizmo: `proj.y_axis.y *= -1.0`
- Transform conversion requires glam f32 → gizmo's mint f64 types (`DVec3`, `DQuat`)
- Rotation uses delta approach: `delta = new_euler - orig_euler`, then `final = original_stored + delta` (avoids gimbal lock snapping)
- mint `Quaternion` layout: `s` = scalar (w), `v` = vector (x, y, z) — different from glam's xyzw order
- `mat4_to_f64()` helper converts glam `Mat4` to row-major f64 array

## Camera Controller Script

**File:** `gg_editor/src/camera_controller.rs`

A `NativeScript` implementation providing WASD camera movement for scene cameras during play mode.

## Editor Camera

The editor uses `EditorCamera` for viewport navigation:
- **Alt+LMB** — Orbit
- **Alt+MMB** — Pan
- **Alt+RMB** or **Scroll** — Zoom

## Mouse Picking

Entity selection in the viewport uses GPU-based mouse picking:

1. Scene renders entity IDs to `RedInteger` (R32_SINT) framebuffer attachment
2. `schedule_pixel_readback()` reads the pixel under the cursor
3. `hovered_entity()` returns the entity ID
4. `Scene::find_entity_by_id()` resolves it to an `Entity`

## Dual-Pass Rendering

The editor uses two render passes per frame:

1. **Offscreen pass** — Scene rendered to framebuffer (RGBA8 + RedInteger + Depth)
2. **Pipeline barrier** — COLOR_ATTACHMENT_WRITE → SHADER_READ
3. **Swapchain pass** — Egui draws editor chrome, displays framebuffer as viewport texture

Clear color for the editor chrome: `[0.06, 0.06, 0.06, 1.0]`

## Project System

The editor supports `.ggproject` files for managing game projects:

- **File > Open Project...** loads a `.ggproject` file
- CLI arg: `cargo run -p gg_editor -- path/to/game.ggproject`
- On load: sets CWD to project directory, updates asset root, loads start scene
- Script watcher restarted for the new project's scripts directory

See `gg_engine/src/project.rs` for the `Project` and `ProjectConfig` types.
