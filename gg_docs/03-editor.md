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

### Properties

**File:** `gg_editor/src/panels/properties.rs`

Component inspector for the selected entity:

| Component | Controls |
|-----------|----------|
| **Tag** | Editable name field |
| **Transform** | Vec3 controls with colored XYZ reset buttons for translation/rotation (degrees)/scale |
| **Camera** | Projection type combo, perspective/orthographic params, primary toggle, fixed aspect ratio |
| **Sprite Renderer** | RGBA color picker |
| **Circle Renderer** | Color, thickness, fade |
| **RigidBody2D** | Body type, fixed rotation |
| **BoxCollider2D** | Offset, size, density, friction, restitution, threshold |

- Removable components have a "+" settings button on the header for removal
- "Add Component" button at the bottom offers: Camera, Sprite Renderer, Circle Renderer, RigidBody2D, BoxCollider2D

### Content Browser

**File:** `gg_editor/src/panels/content_browser.rs`

File browser for project assets with drag-and-drop support.

### Settings

**File:** `gg_editor/src/panels/settings.rs`

- Frame time display
- FPS counter
- VSync toggle

## File Operations

### Menu Bar

Built with `egui::TopBottomPanel::top` + `egui::MenuBar::new().ui()`.

| Action | Shortcut | Description |
|--------|----------|-------------|
| New | Ctrl+N | New empty scene (clears selection, triggers viewport resize) |
| Open... | Ctrl+O | Open `.ggscene` file via native dialog |
| Save | Ctrl+S | Save to current path (or Save As if no path set) |
| Save As... | Ctrl+Shift+S | Save to new path via native dialog |

- `editor_scene_path` tracks the current file (set on open/save-as, cleared on new scene)
- All file shortcuts stop playback first if in play mode

### Entity Operations

| Action | Shortcut | Description |
|--------|----------|-------------|
| Duplicate | Ctrl+D | Duplicate selected entity (edit mode only) |

## Play/Stop System

**File:** `gg_editor/src/main.rs`

`SceneState` enum: `Edit`, `Play`

### Play Flow

```
Edit Mode
    │ Play button
    ▼
1. Scene::copy() snapshots the current scene
2. Original stored as editor_scene
3. on_runtime_start() called on the copy (initializes physics)
4. Enter Play mode (scripts + physics run each frame)
```

### Stop Flow

```
Play Mode
    │ Stop button
    ▼
1. on_runtime_stop() (tear down physics)
2. Swap editor_scene back as active scene
3. Push old runtime scene to pending_drop_scenes (GPU-safe deferred destruction)
4. Return to Edit mode
```

Key implementation details:
- `editor_scene: Option<Scene>` holds the original while playing (`None` during edit mode)
- `pending_drop_scenes: Vec<Scene>` for GPU-safe deferred destruction in `on_render`
- Textures shared via `Arc` between editor and runtime scenes

## Custom Title Bar

**File:** `gg_editor/src/title_bar.rs`

Windows-specific (`#[cfg(not(target_os = "macos"))]`) custom title bar with integrated play/stop controls.

## Gizmos

**File:** `gg_editor/src/gizmo.rs`

Uses `transform-gizmo-egui` 0.8 for 3D manipulation gizmos in the viewport.

### Operations

| Operation | Key | Description |
|-----------|-----|-------------|
| None | Q | No gizmo |
| Translate | W | Move entity |
| Rotate | E | Rotate entity |
| Scale | R | Scale entity |

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

## Test Scene

The editor sets up a test scene with:
- Three colored sprite entities
- Two cameras (orthographic primary + perspective secondary)
- `CameraController` NativeScript for WASD movement
