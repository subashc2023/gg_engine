# Editor

The editor (`gg_editor/`) is an egui-based application using `egui_dock` 0.18 for dockable tab panels.

**Main file:** `gg_editor/src/main.rs`

## Editor State Decomposition

The `GGEditor` struct has been decomposed into focused sub-state structs to reduce god-object coupling. Each struct owns a distinct area of responsibility:

| Struct | Purpose |
|--------|---------|
| **ViewportInfo** | Framebuffer, viewport dimensions, focus/hover state, mouse position, hovered entity ID |
| **GizmoState** | Transform gizmo instance, current operation, drag state, local/world toggle |
| **FontState** | Pending font load requests and loaded font cache (`HashMap<PathBuf, Ref<Font>>`) |
| **SceneContext** | Scene file path, dirty flag, auto-save timer, warnings, deferred scene drops |
| **PlaybackState** | Scene state (Edit/Play/Simulate), stored editor scene, paused flag, step counter |
| **ProjectState** | Loaded project, assets root, current content browser directory, asset manager |
| **UiState** | Dock layout, egui texture map, keyboard focus flag, window title, modals, clipboard, pending scene open |

Top-level `GGEditor` fields:
- `editor_mode` (Hub vs Editor)
- `editor_settings` (persisted settings)
- `scene`, `selection_context`, `editor_camera`
- `tilemap_paint` (TilemapPaintState)
- `undo_system` (UndoSystem)
- `frame_time_ms`, `render_stats`
- File watcher fields (feature-gated on `lua-scripting`)

## Editor Modes

`EditorMode` enum: `Hub`, `Editor`

- **Hub** mode is shown when no project is loaded (no CLI arg, or first launch). Shows the Project Hub UI.
- **Editor** mode is the full editing environment with dock panels, viewport, and scene editing.

## Project Hub / Creation Wizard

**File:** `gg_editor/src/hub.rs`

Shown when the editor starts without a loaded project.

### Main Hub View

- Title: "GGEngine" / "Game Engine" centered at top
- **Open Project...** button: opens a native file dialog for `.ggproject` files
- **New Project...** button: opens the inline creation wizard
- **Recent Projects** list (up to 10 entries):
  - Click to open a project
  - Missing projects shown with strikethrough and dimmed text
  - "x" button to remove from the list
  - Path shown to the right of each entry
  - Existence check cached per frame (syscall-efficient)

### New Project Wizard

Inline form replacing the main hub view:

- **Project Name** text field with validation (alphanumeric, spaces, hyphens, underscores only)
- **Location** text field with **Browse...** folder picker
- **Path preview**: shows the resolved `.ggproject` file path (`location/ProjectName/ProjectName.ggproject`)
- **Create** button (enabled when name is valid and location is set)
- **Cancel** button returns to the main hub view

Creates the project directory structure and `.ggproject` file, then opens the project.

## Panels

### Scene Hierarchy

**File:** `gg_editor/src/panels/scene_hierarchy.rs`

- **Search box** at the top: filters entities by name (case-insensitive, recursive through children)
  - "X" button to clear the filter
- Displays entities in a tree structure with parent-child relationships
  - Leaf entities rendered as selectable labels
  - Parent entities rendered as collapsing headers with recursively drawn children
- Click to select an entity
- Click blank space to deselect
- Right-click blank space: "Create Empty Entity"
- Right-click entity:
  - "Create Child Entity" (adds child under the clicked entity)
  - "Detach from Parent" (only shown for non-root entities)
  - "Delete Entity"

#### Drag-and-Drop Reparenting

- Entities can be dragged onto other entities to reparent them
- **Drop zone detection** based on cursor position relative to the target item:
  - Top/bottom edges (30% of item height): reorder among siblings (insertion line indicator)
  - Center: reparent (highlight border)
- Dropping on blank space detaches the entity to root level
- Visual feedback: blue accent border for reparent, horizontal line for reorder
- All hierarchy operations record undo snapshots

### Viewport

**File:** `gg_editor/src/panels/viewport.rs`

- Displays the offscreen scene framebuffer as an egui image
- Tracks focus/hover state for input blocking
- Gizmo overlay renders here
- Mouse picking for entity selection
- Gizmo mode toolbar overlay (Q/W/E/R buttons) in the top-left corner
- Drag-and-drop from content browser: `.ggscene` files trigger scene load
- Tilemap paint cursor overlay (green for paint, red for eraser)
- Tilemap painting: mouse clicks apply brush to tilemap grid cells

### Properties

**File:** `gg_editor/src/panels/properties/mod.rs`

The Properties panel has been split into sub-modules for better organization:

| Module | Components |
|--------|-----------|
| `mod.rs` | Main dispatch, Vec3 control, Add Component button |
| `camera.rs` | CameraComponent |
| `sprite.rs` | SpriteRendererComponent, SpriteAnimatorComponent, CircleRendererComponent |
| `text.rs` | TextComponent |
| `physics.rs` | RigidBody2DComponent, BoxCollider2DComponent, CircleCollider2DComponent |
| `audio.rs` | AudioSourceComponent |
| `tilemap.rs` | TilemapComponent |
| `scripting.rs` | NativeScriptComponent, LuaScriptComponent |

#### Component Inspector

| Component | Controls |
|-----------|----------|
| **Tag** | Editable name field (inline with Add Component button) |
| **Transform** | Vec3 controls with colored XYZ reset buttons for translation/rotation (degrees)/scale |
| **Camera** | Projection type combo, perspective/orthographic params, primary toggle, fixed aspect ratio |
| **Sprite Renderer** | RGBA color picker, texture asset (file picker, drag-drop, clear button), tiling factor |
| **Sprite Animator** | Cell size (W/H), columns, clip list with name/start frame/end frame/FPS/looping, Add Clip button |
| **Circle Renderer** | Color, thickness, fade |
| **Text** | Text content, font path, font size, color, line spacing, kerning |
| **RigidBody2D** | Body type, fixed rotation |
| **BoxCollider2D** | Offset, size, density, friction, restitution, threshold |
| **CircleCollider2D** | Radius, offset, density, friction, restitution, threshold |
| **Audio Source** | Audio file (file picker + drag-drop from content browser), volume slider (0-1), pitch drag (0.1-4.0), looping checkbox, play on start checkbox, streaming checkbox, spatial audio checkbox with min/max distance controls |
| **Tilemap** | Grid width/height, tile size (X/Y), tileset columns, cell size, spacing, margin, tileset texture (drag-drop), tile palette with visual preview grid, eraser, flip H/V toggles |
| **Lua Script** | Script path (file picker + drag-and-drop), field overrides with live editing in play mode |
| **Native Script** | Display-only (runtime-only component) |

- Removable components have a right-click context menu on the header for "Remove Component"
- **Add Component** button (blue, inline with tag field) offers:
  - Camera, Sprite Renderer, Circle Renderer, Sprite Animator, Text, Rigidbody 2D, Box Collider 2D, Circle Collider 2D, Tilemap, Audio Source, Lua Script
- All property changes that start from drag interactions use coalesced undo (begin_edit/end_edit)
- Texture and audio file buttons accept drag-and-drop from the Content Browser with visual highlight feedback

#### Tilemap Tile Palette

The tilemap component includes an integrated tile palette for painting:

- **Eraser (X)** toggle and **Clear Brush (Esc)** button
- **Flip H / Flip V** checkboxes (shown when a tile is selected)
- **Tile coordinate picker**: Col/Row drag values for precise tile selection
- **Visual palette grid**: scrollable grid of tile previews
  - With tileset texture loaded: renders actual tile UV regions from the tileset image
  - Without texture: colored placeholder cells with tile ID numbers
  - Click to select, click again to deselect
  - Blue border on selected tile
  - Hover tooltip shows tile ID and grid position

### Content Browser

**File:** `gg_editor/src/panels/content_browser.rs`

#### Mode Toggle

Two modes selected via toggle buttons at the top:

- **File** mode: filesystem browser rooted at the project assets directory
- **Asset** mode: shows imported assets from the asset registry

#### Search

Search field below the mode toggle filters entries by name (case-insensitive substring match). Clear button ("x") when filter is active.

#### File Browser

- Grid layout with folder/file icons (custom-painted folder and file icons)
- Back button arrow when deeper than the assets root
- Double-click folders to navigate into them
- Drag-and-drop support for files and folders:
  - `.ggscene` files can be dragged to the viewport to load them
  - `.lua` files can be dragged onto Lua Script component fields
  - Image files (`.png`, `.jpg`, `.jpeg`) can be dragged onto texture/tileset fields
  - Audio files (`.wav`, `.ogg`, `.mp3`, `.flac`) can be dragged onto audio source fields
  - Ghost overlay follows the cursor during drag
- Right-click file context menu:
  - **Import** (if asset manager available and not already imported)
  - **Rename** (inline text field, Enter to confirm, Escape to cancel)
  - **Delete** (confirmation dialog)
- Right-click folder context menu:
  - **Open**, **Rename**, **Delete**
- Right-click blank space context menu:
  - **New Folder**, **New Lua Script** (with template), **New Scene**
- Directory listing is cached and invalidated on navigation or file operations

#### Asset Browser

- Lists all imported assets from the asset registry, sorted by path
- Format: `[AssetType] file_path`
- Search filter applies to file paths
- Drag-and-drop from asset entries (same as file browser)
- Right-click context menu: **Remove from registry**
  - If the asset is referenced by entities in the scene, a confirmation dialog lists all referencing entities with a "Remove Anyway" / "Cancel" choice

### Settings

**File:** `gg_editor/src/panels/settings.rs`

- **Renderer** section:
  - Frame time display (ms)
  - FPS counter
  - Draw calls, quads, vertices, indices counts
  - VSync toggle
  - Reload Shaders button (recompiles `.glsl` sources and rebuilds all pipelines at runtime; requires `glslc` on PATH)
  - Theme selector (combo box with `EditorTheme` variants)
- **Debug** section:
  - Show Physics Colliders toggle
- **Grid** section:
  - Show Grid toggle (default: on)
  - Snap to Grid toggle (default: off)
  - Grid Size combo box: 0.1, 0.25, 0.5, 1.0 (default), 2.0, 5.0, 10.0
- **Scene** section:
  - Entity count
- **Mouse Picking** section:
  - Hovered entity name (resolved from entity ID)
- **Warnings** section (shown when scene warnings exist):
  - Warning messages with yellow "!" indicator

### Project

**File:** `gg_editor/src/panels/project.rs`

- Displays project name and active scene path
- Lists all `.ggscene` files in the project assets directory
- Click a scene to load it

### Game Viewport

**File:** `gg_editor/src/panels/game_viewport.rs`

- Simplified viewport showing the game camera's framebuffer (no editor tools)
- DPI-aware sizing
- Shows "No camera available" if no primary camera or framebuffer is None
- Hover state tracking for input routing
- Enabled/disabled via View menu → Game Viewport toggle
- Framebuffer created lazily on first enable (`create_game_fb` flag)

### Console

**File:** `gg_editor/src/panels/console.rs`

- Runtime log viewer displaying engine and application log messages
- **Level filtering**: buttons for Error, Warn, Info, Debug, Trace (toggle independently)
- **Auto-scroll** toggle (scrolls to bottom on new messages)
- **Clear** button to flush the log buffer
- Color-coded entries: red (error), yellow (warn), green (info), blue (debug), gray (trace)
- Efficient virtual scrolling via `show_rows()` for large log buffers

### Animation Timeline

**File:** `gg_editor/src/panels/animation_timeline.rs`

Full animation clip editor for `SpriteAnimatorComponent` entities:

- **Split layout**: left = sprite sheet grid preview, right = timeline
- **Sprite sheet grid**: visual grid of frames from the sprite sheet texture; click to select frame for pick mode
- **Timeline**: frame ruler with grid lines, zoom (8x–64x pixels per frame), horizontal scroll
- **Playhead**: red vertical line, draggable for scrubbing
- **Clip bars**: blue rectangles on the timeline, draggable (body, start edge, end edge)
- **Toolbar**: Play/Pause/Stop buttons, FPS control, looping toggle
- **Pick mode**: click "Set Start" or "Set End", then click a frame in the sprite sheet grid to set clip boundaries
- **State tracking**: per-entity state (selected clip, zoom, scroll, playhead position) reset when switching entities

Thread-local state: `SELECTED_CLIP`, `ZOOM`, `SCROLL_X`, `TRACKED_ENTITY`, `ACTIVE_DRAG`, `HOVERED_FRAME`, `PICK_MODE`

## File Operations

### Menu Bar

Built with `egui::TopBottomPanel::top` + `egui::MenuBar::new().ui()`.

#### File Menu

| Action | Shortcut | Description |
|--------|----------|-------------|
| New | Ctrl+N | New scene (shows naming modal if project loaded, otherwise empty scene) |
| Open... | Ctrl+O | Open `.ggscene` file via native dialog |
| Save | Ctrl+S | Save to current path (or Save As if no path set) |
| Save As... | Ctrl+Shift+S | Save to new path via native dialog |
| Open Project... | -- | Open `.ggproject` file via native dialog |

#### Edit Menu

| Action | Shortcut | Description |
|--------|----------|-------------|
| Undo | Ctrl+Z | Undo last edit (edit mode only, grayed when stack empty) |
| Redo | Ctrl+Y | Redo last undone edit (edit mode only, grayed when stack empty) |
| Copy | Ctrl+C | Copy selected entity UUID to clipboard (edit mode only) |
| Paste | Ctrl+V | Duplicate entity from clipboard UUID (edit mode only) |
| Duplicate | Ctrl+D | Duplicate selected entity (edit mode only) |

#### View Menu

- **Show Physics Colliders** -- toggle collider visualization overlay
- **Game Viewport** -- toggle the game camera preview panel
- **Reset Layout** -- restore the default dock panel layout

#### Script Menu

- **Reload Scripts** (Ctrl+R) -- manually reload all Lua scripts

#### Help Menu

- **Keyboard Shortcuts** -- opens a dialog listing all keyboard shortcuts

### Scene State

- `editor_scene_path` tracks the current file (set on open/save-as, cleared on new scene)
- All file shortcuts stop playback first if in play mode
- Unsaved changes shown as `*` in the title bar

### New Scene Modal

When a project is loaded, Ctrl+N opens a modal dialog:
- Scene name text field (auto-focused)
- Create / Cancel buttons
- Enter to confirm, Escape to cancel
- Creates the scene file in the project's assets directory

### Entity Operations

| Action | Shortcut | Description |
|--------|----------|-------------|
| Copy | Ctrl+C | Copy selected entity (edit mode only) |
| Paste | Ctrl+V | Paste copied entity as duplicate (edit mode only) |
| Duplicate | Ctrl+D | Duplicate selected entity (edit mode only) |
| Delete | Del | Delete selected entity (edit mode only) |
| Deselect | Escape | Clear tilemap brush first, then clear entity selection (edit mode only) |

### Tilemap Shortcuts

| Action | Shortcut | Description |
|--------|----------|-------------|
| Toggle Eraser | X | Toggle eraser mode for tilemap painting (edit mode only) |
| Clear Brush | Escape | Clear active tilemap brush (before clearing selection) |

## Undo/Redo System

**File:** `gg_editor/src/undo.rs`

Snapshot-based undo/redo using YAML scene serialization.

### Design

- Each undo entry is a complete YAML string snapshot of the scene state
- Uses `VecDeque<String>` for the undo stack and `Vec<String>` for the redo stack
- **100-entry limit** per stack (oldest entries dropped from the front)
- Redo stack is **cleared on any new edit**

### API

| Method | Description |
|--------|-------------|
| `begin_edit(scene)` | Capture a "before" snapshot for a continuous gesture (drag, gizmo, text edit). No-op if already inside a gesture. |
| `end_edit()` | End the gesture, push the "before" snapshot to the undo stack |
| `record(scene)` | Capture and push an immediate snapshot (discrete edits like add/remove component) |
| `undo(current_scene)` | Pop undo stack, push current state to redo, return restored scene |
| `redo(current_scene)` | Pop redo stack, push current state to undo, return restored scene |
| `clear()` | Clear both stacks and any pending gesture |

### Gesture Coalescing

Continuous edits (drag values, gizmo transforms) are bracketed with `begin_edit()` / `end_edit()` so they produce a single undo step. The Properties panel detects drag interactions each frame: if `dragged_id` is `Some` and no edit is in progress, it calls `begin_edit`; when the drag ends, it calls `end_edit`.

### Keyboard Shortcuts

- **Ctrl+Z** -- Undo (edit mode only)
- **Ctrl+Shift+Z** or **Ctrl+Y** -- Redo (edit mode only)

## Auto-Save and Recovery

### Auto-Save

- **Interval**: `AUTOSAVE_INTERVAL_SECS = 300` (5 minutes)
- Runs only when the scene has unsaved changes (`dirty` flag) and in Edit mode
- Timer decrements each frame; resets to interval after each auto-save
- Writes to a sidecar file: `SceneName.autosave.ggscene` (alongside the original `.ggscene`)
- Manual save (Ctrl+S) resets the timer and **removes** the auto-save file

### Recovery

- On scene open (`open_scene`, `open_scene_from_path`, project start scene), the editor calls `check_autosave_recovery()`
- If a `.autosave.ggscene` sidecar exists and is **newer** than the original scene file:
  - Shows a native confirmation dialog: "An auto-save was found... Recover unsaved changes?"
  - If accepted: deserializes the auto-save, removes the sidecar file, marks scene as dirty
  - If declined: removes the stale auto-save file
- If the auto-save is older than the original, it is silently cleaned up

### Emergency Auto-Save

- On GPU device loss (`on_device_lost`), the editor performs an emergency auto-save before the application exits

## Grid Settings

### Configuration

Persisted in `EditorSettings`:

| Setting | Default | Description |
|---------|---------|-------------|
| `show_grid` | `true` | Toggle grid visibility |
| `snap_to_grid` | `false` | Snap entity transforms to grid |
| `grid_size` | `1.0` | Grid cell size |

Predefined grid sizes available in the Settings panel combo box: 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0

### Grid Rendering

- Rendered in the overlay pass (behind scene entities), at Z = -0.01
- Visible in Edit and Simulate modes, hidden in Play mode
- Grid lines: semi-transparent gray (`0.35, 0.35, 0.35, 0.5`)
- X axis (Y=0 horizontal line): red (`0.8, 0.2, 0.2, 0.6`)
- Y axis (X=0 vertical line): green (`0.2, 0.8, 0.2, 0.6`)
- Grid extent is determined dynamically from the editor camera focal point and distance

### Grid Snapping

When snap-to-grid is enabled, gizmo translation snaps to the configured grid size.

## Tilemap Painting

### TilemapPaintState

**File:** `gg_editor/src/main.rs`

```
brush_tile_id: i32   // -2 = no brush, -1 = eraser, 0+ = tile ID
brush_flip_h: bool
brush_flip_v: bool
painting_in_progress: bool
painted_this_stroke: HashSet<(u32, u32)>
```

### Paint Mode

Tilemap painting is active when:
- An entity with a `TilemapComponent` is selected
- A brush is active (`brush_tile_id >= -1`)
- The editor is in Edit mode

### Viewport Painting

- Mouse position is converted from screen coordinates to tilemap grid coordinates via `screen_to_tile_grid()`
- Cursor highlight: green rectangle for paint brush, red rectangle for eraser
- The composed tile value includes flip flags (`TILE_FLIP_H`, `TILE_FLIP_V`) OR'd into the tile ID
- **Stroke deduplication**: `painted_this_stroke` tracks which `(col, row)` cells have been painted during the current mouse-down stroke to avoid redundant writes

### Brush Controls

- Select a tile from the palette in the Tilemap component inspector
- Click tile again to deselect
- **X** key toggles eraser mode
- **Escape** clears the brush (before clearing entity selection)
- Brush auto-clears when selection changes to a non-tilemap entity or deselects

## Play / Stop / Simulate

**File:** `gg_editor/src/main.rs`

`SceneState` enum: `Edit`, `Play`, `Simulate`

### Play Flow

```
Edit Mode
    | Play button (green triangle)
    v
1. Scene::copy() snapshots the current scene
2. Original stored as editor_scene
3. Native scripts attached by tag name (PhysicsPlayer, CameraController)
4. on_runtime_start() called on the copy (initializes physics + Lua scripts)
5. Enter Play mode (scripts + physics run each frame)
```

### Simulate Flow

```
Edit Mode
    | Simulate button (gear icon)
    v
1. Scene::copy() snapshots the current scene
2. Original stored as editor_scene
3. on_simulation_start() called (initializes physics only, no scripts)
4. Enter Simulate mode (physics runs, editor camera still active)
```

Simulate mode differs from Play:
- Uses editor camera (orbit/pan/zoom), not the scene's primary camera
- Runs physics only -- no NativeScripts or Lua scripts
- Mouse picking and gizmos still work (like Edit mode)

### Pause / Step

Available in both Play and Simulate modes:
- **Pause button** (two vertical bars) -- toggles pause state. Background highlights when paused.
- **Step button** (play triangle + vertical bar) -- appears only when paused. Advances exactly one frame using a fixed 1/60s timestep.

### Stop Flow

```
Play/Simulate Mode
    | Stop button (blue square)
    v
1. on_runtime_stop() or on_simulation_stop() (tear down physics/scripts)
2. Swap editor_scene back as active scene
3. Push old runtime scene to pending_drop_scenes (GPU-safe deferred destruction)
4. Return to Edit mode, reset pause/step state
5. Clear tilemap painting state
```

Key implementation details:
- `editor_scene: Option<Scene>` holds the original while playing/simulating (`None` during edit mode)
- `pending_drop_scenes: Vec<Scene>` for GPU-safe deferred destruction in `on_render`
- Textures shared via `Arc` between editor and runtime scenes
- Play <-> Simulate transitions: stop current state first, then start the other

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
- Translation: 0.5 units (or grid size if snap-to-grid is enabled)
- Scale: 0.5 units

### Implementation Notes

- Projection matrix needs Vulkan Y-flip undone before passing to gizmo: `proj.y_axis.y *= -1.0`
- Transform conversion requires glam f32 -> gizmo's mint f64 types (`DVec3`, `DQuat`)
- Rotation uses delta approach: `delta = new_euler - orig_euler`, then `final = original_stored + delta` (avoids gimbal lock snapping)
- mint `Quaternion` layout: `s` = scalar (w), `v` = vector (x, y, z) -- different from glam's xyzw order
- `mat4_to_f64()` helper converts glam `Mat4` to row-major f64 array
- Supports local/world coordinate toggle

## Camera Controller Script

**File:** `gg_editor/src/camera_controller.rs`

A `NativeScript` implementation providing WASD camera movement for scene cameras during play mode.

## Editor Camera

The editor uses `EditorCamera` for viewport navigation:
- **Alt+LMB** -- Orbit
- **Alt+MMB** -- Pan
- **Alt+RMB** or **Scroll** -- Zoom

Camera state (focal point, distance, yaw, pitch) is persisted in `EditorSettings` and restored on startup.

## Mouse Picking

Entity selection in the viewport uses GPU-based mouse picking:

1. Scene renders entity IDs to `RedInteger` (R32_SINT) framebuffer attachment
2. `schedule_pixel_readback()` reads the pixel under the cursor
3. `hovered_entity()` returns the entity ID
4. `Scene::find_entity_by_id()` resolves it to an `Entity`

## Dual-Pass Rendering

The editor uses two render passes per frame:

1. **Offscreen pass** -- Scene rendered to framebuffer (RGBA8 + RedInteger + Depth)
2. **Pipeline barrier** -- COLOR_ATTACHMENT_WRITE -> SHADER_READ
3. **Swapchain pass** -- Egui draws editor chrome, displays framebuffer as viewport texture

Clear color for the editor chrome: `[0.06, 0.06, 0.06, 1.0]`

## Overlay Rendering

The overlay pass renders on top of the scene but behind egui:

1. **Grid** (when enabled, Edit/Simulate modes only)
2. **Physics collider visualization** (when enabled):
   - Circle colliders: SDF circle outlines using world transforms
   - Box colliders: rect outlines using world transforms
   - Both respect entity hierarchy (use `get_world_transform`)
3. **Selected entity outline**: orange rectangle around the selected entity
4. **Tilemap paint cursor**: colored rectangle at the hovered tilemap grid cell

## Editor Settings

**File:** `gg_editor/src/editor_settings.rs`

Persisted as `editor_settings.yaml` in the platform-specific config directory:
- Windows: `%APPDATA%/GGEngine/`
- macOS: `~/Library/Application Support/GGEngine/`
- Linux: `$XDG_CONFIG_HOME/GGEngine/` (or `~/.config/GGEngine/`)

### Persisted Fields

| Field | Type | Description |
|-------|------|-------------|
| `recent_projects` | `Vec<RecentProject>` | Up to 10 recent projects (name + path) |
| `vsync` | `bool` | VSync toggle (default: true) |
| `show_physics_colliders` | `bool` | Collider visualization (default: false) |
| `gizmo_operation` | `GizmoOperation` | Last-used gizmo tool |
| `camera_state` | `CameraState` | Editor camera focal point, distance, yaw, pitch |
| `show_grid` | `bool` | Grid visibility (default: true) |
| `grid_size` | `f32` | Grid cell size (default: 1.0) |
| `snap_to_grid` | `bool` | Grid snapping (default: false) |
| `window_state` | `WindowState` | Window width, height, position (x, y), maximized flag |
| `dock_layout` | `Option<DockState<Tab>>` | Dock panel layout (restored on startup) |
| `theme` | `EditorTheme` | UI color theme |

### Window State Persistence

The editor saves and restores:
- Window size (width, height) -- default 1600x900
- Window position (x, y) -- default: system-placed (-1, -1)
- Maximized state

## Project System

The editor supports `.ggproject` files for managing game projects:

- **Hub mode**: shown on startup when no project is loaded, with recent projects and new project wizard
- **File > Open Project...** loads a `.ggproject` file
- CLI arg: `cargo run -p gg_editor -- path/to/game.ggproject`
- On load: sets CWD to project directory, updates asset root, loads start scene
- Script watcher restarted for the new project's scripts directory
- Project added to recent projects list on load

See `gg_engine/src/project.rs` for the `Project` and `ProjectConfig` types.

## Default Dock Layout

```
+----------+--------------+------------------+
| Project  |              | Scene Hierarchy  |
+----------+   Viewport   +------------------+
| Settings |              |    Properties    |
+----------+--------------+                  |
|     Content Browser     |                  |
+-------------------------+------------------+
```

Restored from `EditorSettings.dock_layout` if available; otherwise built from code. View > Reset Layout restores the default.
