mod camera_controller;
mod editor_settings;
mod file_ops;
mod gizmo;
mod hub;
mod panels;
mod physics_player;
mod playback;
#[cfg(not(target_os = "macos"))]
mod title_bar;
#[cfg(target_os = "macos")]
mod toolbar;
mod undo;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::Gizmo;

use editor_settings::EditorSettings;
use gizmo::GizmoOperation;
use panels::content_browser::{render_dnd_ghost, ASSETS_DIR};
use panels::{
    EditorTabViewer, GameViewportState, ProjectContext, Tab, TilesetPreviewInfo, ViewportState,
};

// ---------------------------------------------------------------------------
// Scene state (edit vs play mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum SceneState {
    Edit,
    Play,
    Simulate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EditorMode {
    Hub,
    Editor,
}

// ---------------------------------------------------------------------------
// Tilemap paint brush state
// ---------------------------------------------------------------------------

pub(crate) struct TilemapPaintState {
    /// -2 = no brush, -1 = eraser, 0+ = tile ID
    pub brush_tile_id: i32,
    pub brush_flip_h: bool,
    pub brush_flip_v: bool,
    pub painting_in_progress: bool,
    pub painted_this_stroke: HashSet<(u32, u32)>,
}

impl TilemapPaintState {
    fn new() -> Self {
        Self {
            brush_tile_id: -2,
            brush_flip_h: false,
            brush_flip_v: false,
            painting_in_progress: false,
            painted_this_stroke: HashSet::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.brush_tile_id >= -1
    }

    pub fn clear_brush(&mut self) {
        self.brush_tile_id = -2;
        self.brush_flip_h = false;
        self.brush_flip_v = false;
    }

    /// Compose the tile value from brush_tile_id + flip flags.
    pub fn composed_value(&self) -> i32 {
        if self.brush_tile_id < 0 {
            return -1; // eraser
        }
        let mut v = self.brush_tile_id;
        if self.brush_flip_h {
            v |= TILE_FLIP_H;
        }
        if self.brush_flip_v {
            v |= TILE_FLIP_V;
        }
        v
    }
}

// ---------------------------------------------------------------------------
// Sub-state structs (decomposed from GGEditor to reduce god-object coupling)
// ---------------------------------------------------------------------------

/// Viewport rendering surface state: framebuffer, dimensions, focus, picking.
struct ViewportInfo {
    scene_fb: Option<Framebuffer>,
    size: (u32, u32),
    focused: bool,
    hovered: bool,
    mouse_pos: Option<(f32, f32)>,
    hovered_entity: i32,
    // Game camera viewport (lazy — created on first enable)
    game_fb: Option<Framebuffer>,
    game_size: (u32, u32),
    game_hovered: bool,
    game_viewport_enabled: bool,
}

/// Transform gizmo tool configuration and drag state.
struct GizmoState {
    gizmo: Gizmo,
    operation: GizmoOperation,
    editing: bool,
    local: bool,
}

/// Scene lifecycle: path, dirty flag, auto-save, warnings, deferred drops.
struct SceneContext {
    editor_scene_path: Option<String>,
    dirty: bool,
    autosave_timer: f32,
    warnings: Vec<String>,
    /// Old scenes awaiting GPU-safe destruction (deferred from on_egui to on_render).
    pending_drop_scenes: Vec<Scene>,
}

/// Play / Simulate / Edit mode and associated transient state.
struct PlaybackState {
    scene_state: SceneState,
    editor_scene: Option<Scene>,
    paused: bool,
    step_frames: i32,
}

/// Loaded project, asset root, content browser directory, asset manager.
struct ProjectState {
    project: Option<Project>,
    assets_root: PathBuf,
    current_directory: PathBuf,
    asset_manager: Option<EditorAssetManager>,
}

/// Transient UI state: dock layout, dialogs, texture mappings, input flags.
struct UiState {
    dock_state: egui_dock::DockState<Tab>,
    /// Mapping from opaque texture handle → egui TextureId for UI rendering.
    egui_texture_map: HashMap<u64, egui::TextureId>,
    /// Set from on_egui each frame; checked in on_event next frame to suppress
    /// editor shortcuts (Q/W/E/R/Delete/Escape/X) while typing in text fields.
    egui_wants_keyboard: bool,
    /// Previous window title; only call `window.set_title()` when it changes.
    prev_window_title: String,
    /// When `Some`, the "New Scene" modal is open with the current name text.
    new_scene_modal: Option<String>,
    /// Whether the keyboard shortcuts help dialog is open.
    show_shortcuts_dialog: bool,
    /// Deferred scene open (set by content browser / project panel, consumed in on_egui).
    pending_open_path: Option<PathBuf>,
    hierarchy_filter: String,
    reload_shaders_requested: bool,
    /// UUID of the entity last copied via Ctrl+C. Used by Ctrl+V to duplicate.
    clipboard_entity_uuid: Option<u64>,
    /// Deferred game viewport framebuffer creation (set in on_egui, consumed in on_render).
    create_game_fb: bool,
}

// ---------------------------------------------------------------------------
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    editor_mode: EditorMode,
    editor_settings: EditorSettings,
    project_state: ProjectState,
    scene_ctx: SceneContext,
    ui: UiState,
    viewport: ViewportInfo,
    playback: PlaybackState,
    gizmo_state: GizmoState,
    frame_time_ms: f32,
    render_stats: Renderer2DStats,
    scene: Scene,
    selection_context: Option<Entity>,
    editor_camera: EditorCamera,
    tilemap_paint: TilemapPaintState,
    undo_system: undo::UndoSystem,
    should_exit: bool,
    /// File watcher that monitors `assets/scripts/` for `.lua` changes.
    /// Kept alive so the OS keeps notifying us; the actual data flows
    /// through `script_reload_pending`.
    #[cfg(feature = "lua-scripting")]
    _script_watcher: Option<notify::RecommendedWatcher>,
    /// Atomic flag set by the file-watcher thread when a `.lua` file is
    /// modified on disk.  Checked each frame in `on_update` to trigger
    /// an automatic [`Scene::reload_lua_scripts`] call.
    #[cfg(feature = "lua-scripting")]
    script_reload_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // -- Project loading (CLI arg) --
        let project = std::env::args().nth(1).and_then(|arg| {
            if arg.ends_with(".ggproject") {
                // Canonicalize the path so project_directory is absolute.
                let abs_path = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
                Project::load(&abs_path.to_string_lossy())
            } else {
                None
            }
        });

        // If a project is loaded, set CWD to the project directory.
        if let Some(ref proj) = project {
            if let Err(e) = std::env::set_current_dir(proj.project_directory()) {
                warn!(
                    "Failed to set CWD to project directory '{}': {}",
                    proj.project_directory().display(),
                    e
                );
            } else {
                info!(
                    "CWD set to project directory: {}",
                    proj.project_directory().display()
                );
            }
        }

        // Determine asset root directory.
        let assets_root = match &project {
            Some(proj) => proj.asset_directory_path(),
            None => PathBuf::from(ASSETS_DIR),
        };

        // Create and load asset manager if a project is loaded.
        let asset_manager = if project.is_some() {
            let mut am = EditorAssetManager::new(&assets_root);
            am.load_registry();
            Some(am)
        } else {
            None
        };

        // Load editor settings (recent projects, etc.).
        let mut editor_settings = EditorSettings::load();

        let editor_mode = if project.is_some() {
            EditorMode::Editor
        } else {
            EditorMode::Hub
        };

        // Load scene: from project start scene, or empty scene for hub mode.
        let (scene, editor_scene_path, recovered_autosave) = if let Some(ref proj) = project {
            let start_path = proj.start_scene_path();
            if start_path.exists() {
                let mut scene = Scene::new();
                let path_str = start_path.to_string_lossy().to_string();
                if SceneSerializer::deserialize(&mut scene, &path_str) {
                    info!("Loaded project start scene: {}", path_str);
                    // Check for auto-save recovery.
                    if let Some(recovered) = Self::check_autosave_recovery(&path_str) {
                        info!("Using recovered auto-save for start scene");
                        (recovered, Some(path_str), true)
                    } else {
                        (scene, Some(path_str), false)
                    }
                } else {
                    warn!("Failed to load start scene, creating empty scene");
                    (Scene::new(), None, false)
                }
            } else {
                info!(
                    "Start scene '{}' not found, creating empty scene",
                    start_path.display()
                );
                (Scene::new(), None, false)
            }
        } else {
            (Scene::new(), None, false)
        };

        // Record CLI-loaded project in recent projects.
        if let Some(ref proj) = project {
            if let Some(arg) = std::env::args().nth(1) {
                let abs_path = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
                editor_settings.add_recent_project(proj.name(), &abs_path.to_string_lossy());
            }
        }

        // --- File watcher for automatic Lua script reloading ---------------
        #[cfg(feature = "lua-scripting")]
        let script_reload_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Only create the file watcher in Editor mode (not Hub mode) to
        // avoid an OS-level filesystem monitor thread sitting idle.
        #[cfg(feature = "lua-scripting")]
        let _script_watcher = if editor_mode == EditorMode::Editor {
            create_script_watcher(&assets_root.join("scripts"), &script_reload_pending)
        } else {
            None
        };

        let initial_gizmo_op = editor_settings.gizmo_operation;
        let initial_cam_state = editor_settings.camera_state.clone();

        // Restore dock layout from settings, or build default layout.
        let mut dock_state = if let Some(saved) = editor_settings.dock_layout.take() {
            saved
        } else {
            Self::default_dock_layout()
        };
        // If saved layout contains GameViewport from a previous session, remove it.
        // The user re-enables it via View > Game Viewport.
        if let Some((s, n, t)) = dock_state.find_tab(&Tab::GameViewport) {
            dock_state[s][n].remove_tab(t);
        }

        GGEditor {
            editor_mode,
            editor_settings,
            project_state: ProjectState {
                current_directory: assets_root.clone(),
                assets_root,
                asset_manager,
                project,
            },
            scene_ctx: SceneContext {
                editor_scene_path,
                dirty: recovered_autosave,
                autosave_timer: Self::AUTOSAVE_INTERVAL_SECS,
                warnings: Vec::new(),
                pending_drop_scenes: Vec::new(),
            },
            ui: UiState {
                dock_state,
                egui_texture_map: HashMap::new(),
                egui_wants_keyboard: false,
                prev_window_title: String::new(),
                new_scene_modal: None,
                show_shortcuts_dialog: false,
                pending_open_path: None,
                hierarchy_filter: String::new(),
                reload_shaders_requested: false,
                clipboard_entity_uuid: None,
                create_game_fb: false,
            },
            viewport: ViewportInfo {
                scene_fb: None,
                size: (0, 0),
                focused: false,
                hovered: false,
                mouse_pos: None,
                hovered_entity: -1,
                game_fb: None,
                game_size: (0, 0),
                game_hovered: false,
                game_viewport_enabled: false,
            },
            playback: PlaybackState {
                scene_state: SceneState::Edit,
                editor_scene: None,
                paused: false,
                step_frames: 0,
            },
            gizmo_state: GizmoState {
                gizmo: Gizmo::default(),
                operation: initial_gizmo_op,
                editing: false,
                local: true,
            },
            frame_time_ms: 0.0,
            render_stats: Renderer2DStats::default(),
            scene,
            selection_context: None,
            editor_camera: {
                let mut cam = EditorCamera::new(45.0_f32.to_radians(), 0.1, 1000.0);
                cam.restore_state(
                    Vec3::from(initial_cam_state.focal_point),
                    initial_cam_state.distance,
                    initial_cam_state.yaw,
                    initial_cam_state.pitch,
                );
                cam
            },
            tilemap_paint: TilemapPaintState::new(),
            undo_system: undo::UndoSystem::new(),
            should_exit: false,
            #[cfg(feature = "lua-scripting")]
            _script_watcher,
            #[cfg(feature = "lua-scripting")]
            script_reload_pending,
        }
    }

    fn window_config(&self) -> WindowConfig {
        let ws = &self.editor_settings.window_state;
        WindowConfig {
            title: "GGEditor".into(),
            width: ws.width,
            height: ws.height,
            decorations: cfg!(target_os = "macos"),
            position: if ws.x >= 0 && ws.y >= 0 {
                Some((ws.x, ws.y))
            } else {
                None
            },
            maximized: ws.maximized,
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        match renderer.create_framebuffer(FramebufferSpec {
            width: 800,
            height: 600,
            attachments: vec![
                FramebufferTextureFormat::RGBA8.into(),
                FramebufferTextureFormat::RedInteger.into(),
                FramebufferTextureFormat::Depth.into(),
            ],
        }) {
            Ok(fb) => self.viewport.scene_fb = Some(fb),
            Err(e) => warn!("Failed to create scene framebuffer: {e}"),
        }
        // Game viewport FB created lazily when enabled via View menu.
    }

    fn scene_framebuffer(&self) -> Option<&Framebuffer> {
        self.viewport.scene_fb.as_ref()
    }

    fn scene_framebuffer_mut(&mut self) -> Option<&mut Framebuffer> {
        self.viewport.scene_fb.as_mut()
    }

    fn desired_viewport_size(&self) -> Option<(u32, u32)> {
        if self.viewport.size.0 > 0 && self.viewport.size.1 > 0 {
            Some(self.viewport.size)
        } else {
            None
        }
    }

    // --- Multi-viewport ---

    fn viewport_count(&self) -> usize {
        let mut count = 0;
        if self.viewport.scene_fb.is_some() {
            count += 1;
        }
        if self.viewport.game_viewport_enabled && self.viewport.game_fb.is_some() {
            count += 1;
        }
        count
    }

    fn viewport_framebuffer(&self, index: usize) -> Option<&Framebuffer> {
        match index {
            0 => self.viewport.scene_fb.as_ref(),
            1 => self.viewport.game_fb.as_ref(),
            _ => None,
        }
    }

    fn viewport_framebuffer_mut(&mut self, index: usize) -> Option<&mut Framebuffer> {
        match index {
            0 => self.viewport.scene_fb.as_mut(),
            1 => self.viewport.game_fb.as_mut(),
            _ => None,
        }
    }

    fn viewport_desired_size(&self, index: usize) -> Option<(u32, u32)> {
        let (w, h) = match index {
            0 => self.viewport.size,
            1 => self.viewport.game_size,
            _ => return None,
        };
        if w > 0 && h > 0 {
            Some((w, h))
        } else {
            None
        }
    }

    fn on_render_viewport(&mut self, renderer: &mut Renderer, viewport_index: usize) {
        match viewport_index {
            0 => {
                // Editor viewport — full render + overlays (existing on_render logic)
                self.on_render(renderer);
            }
            1 => {
                // Game viewport — always render from game camera perspective
                self.render_game_viewport(renderer);
            }
            _ => {}
        }
    }

    fn present_mode(&self) -> PresentMode {
        if self.editor_settings.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Immediate
        }
    }

    fn block_events(&self) -> bool {
        !self.viewport.hovered
    }

    fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn on_close_requested(&mut self) -> bool {
        self.confirm_discard_changes()
    }

    fn on_event(&mut self, event: &Event, input: &Input) {
        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Editor camera responds in edit and simulate modes.
        if self.playback.scene_state != SceneState::Play {
            self.editor_camera.on_event(event);
        }

        if let Event::Key(KeyEvent::Pressed {
            key_code,
            repeat: false,
        }) = event
        {
            let ctrl =
                input.is_key_pressed(KeyCode::LeftCtrl) || input.is_key_pressed(KeyCode::RightCtrl);
            let shift = input.is_key_pressed(KeyCode::LeftShift)
                || input.is_key_pressed(KeyCode::RightShift);

            match key_code {
                // File commands — always available; stop playback/simulation first.
                KeyCode::N if ctrl => {
                    if self.playback.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.new_scene();
                }
                KeyCode::O if ctrl => {
                    if self.playback.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.open_scene();
                }
                KeyCode::S if ctrl && shift => {
                    if self.playback.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene_as();
                }
                KeyCode::S if ctrl && !shift => {
                    if self.playback.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene();
                }

                // Undo/Redo — edit mode only.
                KeyCode::Z if ctrl && !shift && self.playback.scene_state == SceneState::Edit => {
                    self.perform_undo();
                }
                KeyCode::Z if ctrl && shift && self.playback.scene_state == SceneState::Edit => {
                    self.perform_redo();
                }
                KeyCode::Y if ctrl && !shift && self.playback.scene_state == SceneState::Edit => {
                    self.perform_redo();
                }

                // Copy entity — edit mode only.
                KeyCode::C if ctrl && !shift && self.playback.scene_state == SceneState::Edit => {
                    self.on_copy_entity();
                }
                // Paste entity — edit mode only.
                KeyCode::V if ctrl && !shift && self.playback.scene_state == SceneState::Edit => {
                    self.on_paste_entity();
                }

                // Entity duplication — edit mode only.
                KeyCode::D if ctrl && self.playback.scene_state == SceneState::Edit => {
                    self.on_duplicate_entity();
                }

                // Script reload — available in any scene state.
                #[cfg(feature = "lua-scripting")]
                KeyCode::R if ctrl && !shift => {
                    self.scene.reload_lua_scripts();
                    panels::properties::clear_field_cache();
                }

                // Delete selected entity — edit mode only, not while typing.
                KeyCode::Delete
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    if let Some(entity) = self.selection_context.take() {
                        self.undo_system.record(&self.scene);
                        if self.scene.destroy_entity(entity).is_ok() {
                            self.scene_ctx.dirty = true;
                        }
                    }
                }

                // Escape — clear brush first, then clear selection (edit mode only).
                KeyCode::Escape
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    if self.tilemap_paint.is_active() {
                        self.tilemap_paint.clear_brush();
                    } else {
                        self.selection_context = None;
                    }
                }

                // X — toggle eraser mode (edit mode only, not while typing).
                KeyCode::X
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    if self.tilemap_paint.brush_tile_id == -1 {
                        self.tilemap_paint.clear_brush();
                    } else {
                        self.tilemap_paint.brush_tile_id = -1;
                    }
                }

                // Gizmo shortcuts (Q/W/E/R) — edit mode only, not while typing.
                KeyCode::Q
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::None;
                }
                KeyCode::W
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Translate;
                }
                KeyCode::E
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Rotate;
                }
                KeyCode::R
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Scale;
                }
                _ => {}
            }
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        profile_scope!("GGEditor::on_update");
        // Exponential moving average for stable frame time display.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Auto-save: periodically save a backup when there are unsaved changes.
        if self.scene_ctx.dirty && self.playback.scene_state == SceneState::Edit {
            self.scene_ctx.autosave_timer -= dt.seconds();
            if self.scene_ctx.autosave_timer <= 0.0 {
                self.scene_ctx.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
                self.perform_autosave();
            }
        }

        // Auto-reload Lua scripts when the file watcher detects changes.
        #[cfg(feature = "lua-scripting")]
        if self
            .script_reload_pending
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.scene.reload_lua_scripts();
            panels::properties::clear_field_cache();
        }

        // Notify scene cameras of viewport resize.
        let (w, h) = self.viewport.size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
            self.editor_camera.set_viewport_size(w as f32, h as f32);
        }

        match self.playback.scene_state {
            SceneState::Edit => {
                // Update editor camera (orbit/pan/zoom via Alt+mouse).
                self.editor_camera.on_update(dt, input);
                // Tick animation previews (editor inspector play button).
                self.scene.on_update_animation_previews(dt.seconds());
            }
            SceneState::Simulate => {
                // Update editor camera — simulation renders from the editor
                // camera, not the scene camera.
                self.editor_camera.on_update(dt, input);
                // Step physics (no scripts) — skip when paused unless stepping.
                if !self.playback.paused || self.playback.step_frames > 0 {
                    // When manually stepping, use a fixed dt so each click
                    // advances exactly one physics step regardless of frame rate.
                    let physics_dt = if self.playback.step_frames > 0 {
                        Timestep::from_seconds(1.0 / 60.0)
                    } else {
                        dt
                    };
                    self.scene.on_update_physics(physics_dt, None);
                    self.scene.on_update_animations(physics_dt.seconds());
                    self.scene.update_spatial_audio();
                    if self.playback.step_frames > 0 {
                        self.playback.step_frames -= 1;
                    }
                }
            }
            SceneState::Play => {
                // Skip updates when paused unless stepping.
                if !self.playback.paused || self.playback.step_frames > 0 {
                    // When manually stepping, use a fixed dt so each click
                    // advances exactly one physics step regardless of frame rate.
                    let step_dt = if self.playback.step_frames > 0 {
                        Timestep::from_seconds(1.0 / 60.0)
                    } else {
                        dt
                    };
                    // Step physics + Lua fixed-update interleaved.
                    self.scene.on_update_physics(step_dt, Some(input));
                    // Run native scripts (e.g. CameraController) with up-to-date transforms.
                    self.scene.on_update_scripts(step_dt, input);
                    // Run Lua scripts.
                    #[cfg(feature = "lua-scripting")]
                    self.scene.on_update_lua_scripts(step_dt, input);
                    // Advance sprite animations.
                    self.scene.on_update_animations(step_dt.seconds());
                    // Update spatial audio panning/attenuation.
                    self.scene.update_spatial_audio();
                    if self.playback.step_frames > 0 {
                        self.playback.step_frames -= 1;
                    }
                }
            }
        }

        // Read latest pixel readback result.
        self.viewport.hovered_entity = self
            .viewport
            .scene_fb
            .as_ref()
            .map(|fb| fb.hovered_entity())
            .unwrap_or(-1);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("GGEditor::on_render");

        // Deferred game viewport framebuffer creation (toggled from View menu).
        if self.ui.create_game_fb && self.viewport.game_fb.is_none() {
            self.ui.create_game_fb = false;
            match renderer.create_framebuffer(FramebufferSpec {
                width: 800,
                height: 600,
                attachments: vec![
                    FramebufferTextureFormat::RGBA8.into(),
                    FramebufferTextureFormat::RedInteger.into(),
                    FramebufferTextureFormat::Depth.into(),
                ],
            }) {
                Ok(fb) => self.viewport.game_fb = Some(fb),
                Err(e) => warn!("Failed to create game framebuffer: {e}"),
            }
        }

        // Drop old scenes that may hold GPU resources (textures). We must
        // wait for all in-flight GPU work to finish before destroying them,
        // since previous frames' command buffers may still reference them.
        if !self.scene_ctx.pending_drop_scenes.is_empty() {
            renderer.wait_gpu_idle();
            self.scene_ctx.pending_drop_scenes.clear();
        }

        // Poll completed transfer fences and free staging buffers from previous frames.
        renderer.poll_transfers();

        // Handle shader hot-reload request from settings panel.
        if self.ui.reload_shaders_requested {
            self.ui.reload_shaders_requested = false;
            let shader_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("gg_engine")
                .join("src")
                .join("renderer")
                .join("shaders");
            match renderer.reload_shaders(&shader_dir) {
                Ok(count) => {
                    info!(
                        "Shader hot-reload: {} shaders recompiled successfully",
                        count
                    );
                }
                Err(e) => {
                    error!("Shader hot-reload failed: {}", e);
                    gg_engine::platform_utils::error_dialog("Shader Reload Error", &e);
                }
            }
        }

        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Step 1: Poll completed async loads (textures + fonts).
        if let Some(ref mut am) = self.project_state.asset_manager {
            am.poll_loaded(renderer);
        }

        // Submit any batched texture/font uploads before rendering.
        renderer.flush_transfers();

        // Step 2: Resolve texture, audio, and font handles (async — non-blocking).
        if let Some(ref mut am) = self.project_state.asset_manager {
            self.scene.resolve_texture_handles_async(am);
            self.scene.resolve_audio_handles(am);
            self.scene.load_fonts_async(am);
        }

        match self.playback.scene_state {
            SceneState::Edit => {
                self.scene
                    .on_update_editor(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Simulate => {
                self.scene
                    .on_update_simulation(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Play => {
                self.scene.on_update_runtime(renderer);
            }
        }

        // -- Overlay rendering (collider visualization) --
        self.on_overlay_render(renderer);

        // Snapshot renderer stats for the settings panel.
        self.render_stats = renderer.stats_2d();
    }

    fn on_device_lost(&mut self) {
        // Emergency auto-save before exiting due to GPU device lost.
        error!("GPU device lost — performing emergency auto-save");
        self.perform_autosave();
    }

    fn on_egui(&mut self, ctx: &egui::Context, window: &Window) {
        // Apply saved theme on first frame (engine defaults to Dark).
        if self.ui.prev_window_title.is_empty()
            && self.editor_settings.theme != gg_engine::ui_theme::EditorTheme::Dark
        {
            gg_engine::ui_theme::apply_theme(ctx, self.editor_settings.theme);
        }

        // Track whether egui wants keyboard input (text editing, etc.)
        // so on_event can suppress editor shortcuts next frame.
        self.ui.egui_wants_keyboard = ctx.wants_keyboard_input();

        // Cache window geometry for persistence on exit.
        if !window.is_minimized().unwrap_or(false) {
            self.editor_settings.window_state.maximized = window.is_maximized();
            if !window.is_maximized() {
                if let Ok(pos) = window.outer_position() {
                    self.editor_settings.window_state.x = pos.x;
                    self.editor_settings.window_state.y = pos.y;
                }
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    self.editor_settings.window_state.width = size.width;
                    self.editor_settings.window_state.height = size.height;
                }
            }
        }

        // -- Hub mode --
        if self.editor_mode == EditorMode::Hub {
            window.set_title("GGEngine");

            #[cfg(not(target_os = "macos"))]
            {
                if title_bar::hub_title_bar_ui(ctx, window) {
                    self.request_exit();
                }
            }

            let hub_response = hub::hub_ui(ctx, &mut self.editor_settings);

            if let Some(path) = hub_response.open_project_path {
                self.load_project_from_path(&path);
            }
            if let Some(path) = hub_response.create_project_path {
                self.handle_new_project_from_hub(&path);
            }
            return;
        }

        // -- Editor mode --

        // Sync window title with active scene/project name (only when changed).
        let dirty_marker = if self.scene_ctx.dirty { " *" } else { "" };
        let title = {
            let project_prefix = match &self.project_state.project {
                Some(proj) => format!("GGEditor - {}", proj.name()),
                None => "GGEditor".into(),
            };
            match &self.scene_ctx.editor_scene_path {
                Some(path) => {
                    let scene_name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    format!("{} - {}{}", project_prefix, scene_name, dirty_marker)
                }
                None => format!("{}{}", project_prefix, dirty_marker),
            }
        };
        if title != self.ui.prev_window_title {
            window.set_title(&title);
            self.ui.prev_window_title = title;
        }

        // -- Title bar / Menu bar --
        #[cfg(not(target_os = "macos"))]
        {
            let play_state = match self.playback.scene_state {
                SceneState::Edit => title_bar::PlayState::Edit,
                SceneState::Play => title_bar::PlayState::Play,
                SceneState::Simulate => title_bar::PlayState::Simulate,
            };
            let project_title = match &self.project_state.project {
                Some(proj) => match &self.scene_ctx.editor_scene_path {
                    Some(path) => {
                        let scene_name = std::path::Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        format!("GGEngine - {} - {}", proj.name(), scene_name)
                    }
                    None => format!("GGEngine - {}", proj.name()),
                },
                None => match &self.scene_ctx.editor_scene_path {
                    Some(path) => {
                        let scene_name = std::path::Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        format!("GGEngine - {}", scene_name)
                    }
                    None => "GGEngine".into(),
                },
            };
            let response = title_bar::title_bar_ui(
                ctx,
                window,
                play_state,
                self.playback.paused,
                &project_title,
                |ui| {
                    self.menu_bar_contents(ui);
                },
            );
            if response.close_requested {
                self.request_exit();
            }
            if response.play_toggled {
                match self.playback.scene_state {
                    SceneState::Edit => self.on_scene_play(),
                    SceneState::Simulate => {
                        self.on_scene_stop();
                        self.on_scene_play();
                    }
                    SceneState::Play => self.on_scene_stop(),
                }
            }
            if response.simulate_toggled {
                match self.playback.scene_state {
                    SceneState::Edit => self.on_scene_simulate(),
                    SceneState::Play => {
                        self.on_scene_stop();
                        self.on_scene_simulate();
                    }
                    SceneState::Simulate => self.on_scene_stop(),
                }
            }
            if response.pause_toggled {
                self.on_scene_pause();
            }
            if response.step_pressed {
                self.on_scene_step();
            }
        }

        #[cfg(target_os = "macos")]
        {
            let _ = window;
            egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    self.menu_bar_contents(ui);
                });
            });
            // Toolbar (Play / Stop) — macOS only (Windows has it in the title bar).
            self.toolbar_ui(ctx);
        }

        let fb_tex_id = self
            .viewport
            .scene_fb
            .as_ref()
            .and_then(|fb| fb.egui_texture_id());

        let game_fb_tex_id = self
            .viewport
            .game_fb
            .as_ref()
            .and_then(|fb| fb.egui_texture_id());

        let mut dock_style = egui_dock::Style::from_egui(ctx.style().as_ref());

        // Tab bar background and separator line.
        dock_style.tab_bar.bg_fill = egui::Color32::from_rgb(0x18, 0x18, 0x18);
        dock_style.tab_bar.hline_color = egui::Color32::from_rgb(0x3C, 0x3C, 0x3C);

        // Active tab — matches panel background, white text.
        dock_style.tab.active.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
        dock_style.tab.active.text_color = egui::Color32::WHITE;

        // Inactive tab — dark, dimmed text.
        dock_style.tab.inactive.bg_fill = egui::Color32::from_rgb(0x18, 0x18, 0x18);
        dock_style.tab.inactive.text_color = egui::Color32::from_rgb(0x96, 0x96, 0x96);

        // Focused tab — same as active.
        dock_style.tab.focused.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
        dock_style.tab.focused.text_color = egui::Color32::WHITE;

        // Hovered tab.
        dock_style.tab.hovered.bg_fill = egui::Color32::from_rgb(0x25, 0x25, 0x26);
        dock_style.tab.hovered.text_color = egui::Color32::WHITE;

        // Blue underline on active tab.
        dock_style.tab.hline_below_active_tab_name = true;

        // Separator colors.
        dock_style.separator.color_idle = egui::Color32::from_rgb(0x28, 0x28, 0x28);
        dock_style.separator.color_hovered = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);
        dock_style.separator.color_dragged = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);

        // Tab body matches panel.
        dock_style.tab.tab_body.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);

        // Compute tileset preview info for viewport overlay.
        let tileset_preview = self.selection_context.and_then(|entity| {
            let tm = self.scene.get_component::<TilemapComponent>(entity)?;
            let tex = tm.texture.as_ref()?;
            let egui_tex = self.ui.egui_texture_map.get(&tex.egui_handle()).copied()?;
            Some(TilesetPreviewInfo {
                egui_tex,
                tex_w: tex.width() as f32,
                tex_h: tex.height() as f32,
                tileset_columns: tm.tileset_columns.max(1),
                cell_size: tm.cell_size,
                spacing: tm.spacing,
                margin: tm.margin,
            })
        });

        // Snapshot settings values before panels run, so we can detect
        // changes and persist to disk afterwards.
        let settings_snapshot = (
            self.editor_settings.vsync,
            self.editor_settings.show_physics_colliders,
            self.editor_settings.show_grid,
            self.editor_settings.snap_to_grid,
            self.editor_settings.grid_size,
            self.editor_settings.theme,
            self.editor_settings.gizmo_operation,
        );

        // Scope the viewer so its borrows are released before we handle
        // pending actions and paint the DnD ghost overlay.
        {
            let current_snap_to_grid = self.editor_settings.snap_to_grid;
            let current_grid_size = self.editor_settings.grid_size;
            let mut hierarchy_action = None;
            let mut viewer = EditorTabViewer {
                scene: &mut self.scene,
                selection_context: &mut self.selection_context,
                pending_open_path: &mut self.ui.pending_open_path,
                is_playing: self.playback.scene_state == SceneState::Play, // Simulate still uses editor camera + gizmos
                scene_dirty: &mut self.scene_ctx.dirty,
                undo_system: &mut self.undo_system,
                hierarchy_filter: &mut self.ui.hierarchy_filter,
                scene_warnings: &self.scene_ctx.warnings,
                tilemap_paint: &mut self.tilemap_paint,
                vsync: &mut self.editor_settings.vsync,
                frame_time_ms: self.frame_time_ms,
                render_stats: self.render_stats,
                show_physics_colliders: &mut self.editor_settings.show_physics_colliders,
                show_grid: &mut self.editor_settings.show_grid,
                snap_to_grid: &mut self.editor_settings.snap_to_grid,
                grid_size: &mut self.editor_settings.grid_size,
                theme: &mut self.editor_settings.theme,
                reload_shaders_requested: &mut self.ui.reload_shaders_requested,
                viewport: ViewportState {
                    size: &mut self.viewport.size,
                    focused: &mut self.viewport.focused,
                    hovered: &mut self.viewport.hovered,
                    fb_tex_id,
                    gizmo: &mut self.gizmo_state.gizmo,
                    gizmo_operation: &mut self.gizmo_state.operation,
                    gizmo_editing: &mut self.gizmo_state.editing,
                    editor_camera: &self.editor_camera,
                    scene_fb: &mut self.viewport.scene_fb,
                    hovered_entity: self.viewport.hovered_entity,
                    mouse_pos: &mut self.viewport.mouse_pos,
                    tileset_preview,
                    snap_to_grid: current_snap_to_grid,
                    grid_size: current_grid_size,
                    gizmo_local: &mut self.gizmo_state.local,
                },
                game_viewport: GameViewportState {
                    size: &mut self.viewport.game_size,
                    hovered: &mut self.viewport.game_hovered,
                    fb_tex_id: game_fb_tex_id,
                },
                project: ProjectContext {
                    assets_root: &self.project_state.assets_root,
                    current_directory: &mut self.project_state.current_directory,
                    asset_manager: &mut self.project_state.asset_manager,
                    project_name: self.project_state.project.as_ref().map(|p| p.name()),
                    editor_scene_path: self.scene_ctx.editor_scene_path.as_deref(),
                    egui_texture_map: &self.ui.egui_texture_map,
                },
                hierarchy_action: &mut hierarchy_action,
            };

            egui_dock::DockArea::new(&mut self.ui.dock_state)
                .style(dock_style)
                .show(ctx, &mut viewer);

            // Handle hierarchy external actions (prefab save/instantiate).
            // Drop viewer to release mutable borrows before handle_hierarchy_action.
            #[allow(clippy::drop_non_drop)]
            drop(viewer);
            self.handle_hierarchy_action(hierarchy_action);
        }

        // Sync gizmo operation back to settings (lives in gizmo_state, not settings).
        self.editor_settings.gizmo_operation = self.gizmo_state.operation;

        // Persist settings to disk when any value changed this frame.
        let settings_current = (
            self.editor_settings.vsync,
            self.editor_settings.show_physics_colliders,
            self.editor_settings.show_grid,
            self.editor_settings.snap_to_grid,
            self.editor_settings.grid_size,
            self.editor_settings.theme,
            self.editor_settings.gizmo_operation,
        );
        if settings_snapshot != settings_current {
            self.editor_settings.save();
        }

        // Auto-clear tilemap brush when selection changes to a non-tilemap
        // entity or deselects entirely.
        if self.tilemap_paint.is_active() {
            let has_tilemap = self
                .selection_context
                .map(|e| self.scene.has_component::<TilemapComponent>(e))
                .unwrap_or(false);
            if !has_tilemap {
                self.tilemap_paint.clear_brush();
            }
        }

        // "New Scene" naming modal.
        self.new_scene_modal_ui(ctx);

        // Keyboard shortcuts help dialog.
        self.shortcuts_dialog_ui(ctx);

        // DnD ghost overlay — painted on a tooltip layer so it floats above
        // all panels and follows the cursor.
        render_dnd_ghost(ctx);

        // Handle pending scene open from content browser drag-drop.
        if let Some(path) = self.ui.pending_open_path.take() {
            self.open_scene_from_path(&path);
        }
    }

    fn egui_user_textures(&self) -> Vec<u64> {
        // Register tileset textures from all TilemapComponents so we can
        // render tile previews in the properties panel.
        let mut handles = Vec::new();
        for tm in self.scene.world().query::<&TilemapComponent>().iter() {
            if let Some(ref tex) = tm.texture {
                handles.push(tex.egui_handle());
            }
        }

        // Register sprite sheet textures for the selected entity (animation timeline panel).
        if let Some(entity) = self.selection_context {
            if let Some(sprite) = self.scene.get_component::<SpriteRendererComponent>(entity) {
                if let Some(ref tex) = sprite.texture {
                    handles.push(tex.egui_handle());
                }
            }
            if let Some(animator) = self.scene.get_component::<SpriteAnimatorComponent>(entity) {
                for clip in &animator.clips {
                    if let Some(ref tex) = clip.texture {
                        handles.push(tex.egui_handle());
                    }
                }
            }
        }

        handles
    }

    fn receive_egui_user_textures(&mut self, map: &HashMap<u64, egui::TextureId>) {
        self.ui.egui_texture_map = map.clone();
    }
}

// ---------------------------------------------------------------------------
// Overlay rendering (collider visualization, debug shapes)
// ---------------------------------------------------------------------------

impl GGEditor {
    /// Auto-save interval in seconds (5 minutes).
    const AUTOSAVE_INTERVAL_SECS: f32 = 300.0;

    fn render_grid(&self, renderer: &mut Renderer) {
        profile_scope!("GGEditor::render_grid");
        let grid_size = self.editor_settings.grid_size;
        if grid_size <= 0.0 {
            return;
        }

        let grid_color = Vec4::new(0.35, 0.35, 0.35, 0.5);
        let axis_color_x = Vec4::new(0.8, 0.2, 0.2, 0.6);
        let axis_color_y = Vec4::new(0.2, 0.8, 0.2, 0.6);

        // Determine visible range from camera.
        let focal = self.editor_camera.focal_point();
        let dist = self.editor_camera.distance();
        let half_extent = dist * 1.5;

        // Snap grid bounds to grid lines.
        let x_min = ((focal.x - half_extent) / grid_size).floor() as i32;
        let x_max = ((focal.x + half_extent) / grid_size).ceil() as i32;
        let y_min = ((focal.y - half_extent) / grid_size).floor() as i32;
        let y_max = ((focal.y + half_extent) / grid_size).ceil() as i32;

        let lo_y = y_min as f32 * grid_size;
        let hi_y = y_max as f32 * grid_size;
        let lo_x = x_min as f32 * grid_size;
        let hi_x = x_max as f32 * grid_size;

        // Vertical lines (constant X).
        for i in x_min..=x_max {
            let x = i as f32 * grid_size;
            let color = if i == 0 { axis_color_y } else { grid_color };
            renderer.draw_line(
                Vec3::new(x, lo_y, -0.01),
                Vec3::new(x, hi_y, -0.01),
                color,
                -1,
            );
        }

        // Horizontal lines (constant Y).
        for j in y_min..=y_max {
            let y = j as f32 * grid_size;
            let color = if j == 0 { axis_color_x } else { grid_color };
            renderer.draw_line(
                Vec3::new(lo_x, y, -0.01),
                Vec3::new(hi_x, y, -0.01),
                color,
                -1,
            );
        }
    }

    fn request_exit(&mut self) {
        // Persist camera state on exit.
        self.editor_settings.camera_state = editor_settings::CameraState {
            focal_point: self.editor_camera.focal_point().into(),
            distance: self.editor_camera.distance(),
            yaw: self.editor_camera.yaw(),
            pitch: self.editor_camera.pitch(),
        };
        // Persist dock layout.
        self.editor_settings.dock_layout = Some(self.ui.dock_state.clone());
        self.editor_settings.save();
        self.should_exit = true;
    }

    fn default_dock_layout() -> egui_dock::DockState<Tab> {
        //  ┌──────────┬──────────────┬─────────────────┐
        //  │ Project  │              │ Scene Hierarchy  │
        //  ├──────────┤   Viewport   ├─────────────────┤
        //  │ Settings │              │   Properties    │
        //  ├──────────┴──────────────┤                  │
        //  │     Content Browser     │                  │
        //  └─────────────────────────┴─────────────────┘
        let mut dock_state = egui_dock::DockState::new(vec![Tab::Project]);
        let surface = dock_state.main_surface_mut();
        let root = egui_dock::NodeIndex::root();
        let [left, right] = surface.split_right(root, 0.77, vec![Tab::SceneHierarchy]);
        surface.split_below(right, 0.5, vec![Tab::Properties]);
        let [top_left, _bottom_left] = surface.split_below(
            left,
            0.7,
            vec![Tab::ContentBrowser, Tab::Console, Tab::AnimationTimeline],
        );
        let [left_sidebar, _viewport] = surface.split_right(top_left, 0.20, vec![Tab::Viewport]);
        surface.split_below(left_sidebar, 0.5, vec![Tab::Settings]);
        dock_state
    }

    /// Render the game camera viewport (viewport index 1).
    /// Always uses the scene's primary camera, regardless of editor mode.
    fn render_game_viewport(&mut self, renderer: &mut Renderer) {
        if self.editor_mode == EditorMode::Hub {
            return;
        }
        // The game viewport always renders from the primary camera's perspective.
        self.scene.on_update_runtime(renderer);
    }

    fn on_overlay_render(&self, renderer: &mut Renderer) {
        profile_scope!("GGEditor::on_overlay_render");
        // Set the appropriate camera for the overlay pass.
        match self.playback.scene_state {
            SceneState::Play => {
                if let Some(cam_entity) = self.scene.get_primary_camera_entity() {
                    let cam = self.scene.get_component::<CameraComponent>(cam_entity);
                    let tc = self.scene.get_component::<TransformComponent>(cam_entity);
                    if let (Some(cam), Some(tc)) = (cam, tc) {
                        let vp = *cam.camera.projection() * tc.get_transform().inverse();
                        renderer.set_view_projection(vp);
                    }
                }
            }
            SceneState::Edit | SceneState::Simulate => {
                renderer.set_view_projection(self.editor_camera.view_projection());
            }
        }

        // Grid rendering (behind everything else in the overlay).
        if self.editor_settings.show_grid && self.playback.scene_state != SceneState::Play {
            let prev_line_width = renderer.line_width();
            renderer.set_line_width(1.0);
            self.render_grid(renderer);
            renderer.set_line_width(prev_line_width);
        }

        // Physics collider visualization (uses world transforms for hierarchy support).
        if self.editor_settings.show_physics_colliders {
            let collider_color = Vec4::new(0.0, 1.0, 0.0, 1.0);

            // Collect entities with colliders (need owned data to avoid borrow conflicts).
            let circle_entities: Vec<_> = self
                .scene
                .each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    self.scene
                        .get_component::<CircleCollider2DComponent>(*entity)
                        .map(|cc| (*entity, cc.offset, cc.radius))
                })
                .collect();
            for (entity, offset, radius) in circle_entities {
                let world = self.scene.get_world_transform(entity);
                let (world_scale, world_rot, world_trans) = world.to_scale_rotation_translation();
                let rotated_offset =
                    world_rot * Vec3::new(offset.x * world_scale.x, offset.y * world_scale.y, 0.0);
                let translation = Vec3::new(
                    world_trans.x + rotated_offset.x,
                    world_trans.y + rotated_offset.y,
                    world_trans.z - 0.001,
                );
                let scale = world_scale * radius * 2.0;
                let collider_transform = Mat4::from_scale_rotation_translation(
                    Vec3::new(scale.x, scale.y, 1.0),
                    Quat::IDENTITY,
                    translation,
                );
                renderer.draw_circle(&collider_transform, collider_color, 0.01, 0.005, -1);
            }

            let box_entities: Vec<_> = self
                .scene
                .each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    self.scene
                        .get_component::<BoxCollider2DComponent>(*entity)
                        .map(|bc| (*entity, bc.offset, bc.size))
                })
                .collect();
            for (entity, offset, size) in box_entities {
                let world = self.scene.get_world_transform(entity);
                let (world_scale, world_rot, world_trans) = world.to_scale_rotation_translation();
                let rotated_offset =
                    world_rot * Vec3::new(offset.x * world_scale.x, offset.y * world_scale.y, 0.0);
                let translation = Vec3::new(
                    world_trans.x + rotated_offset.x,
                    world_trans.y + rotated_offset.y,
                    world_trans.z - 0.001,
                );
                let scale = Vec3::new(
                    world_scale.x * size.x * 2.0,
                    world_scale.y * size.y * 2.0,
                    1.0,
                );
                let collider_transform =
                    Mat4::from_scale_rotation_translation(scale, world_rot, translation);
                renderer.draw_rect_transform(&collider_transform, collider_color, -1);
            }

            // Velocity arrows (only during play/simulate when physics is active).
            if self.playback.scene_state != SceneState::Edit {
                let velocity_color = Vec4::new(1.0, 0.4, 0.1, 1.0);
                let rb_entities: Vec<_> = self
                    .scene
                    .each_entity_with_tag()
                    .iter()
                    .filter_map(|(entity, _)| {
                        self.scene.get_component::<RigidBody2DComponent>(*entity)?;
                        let vel = self.scene.get_linear_velocity(*entity)?;
                        if vel.length_squared() < 0.001 {
                            return None;
                        }
                        let tc = self.scene.get_component::<TransformComponent>(*entity)?;
                        Some((tc.translation, vel))
                    })
                    .collect();

                let prev_line_width = renderer.line_width();
                renderer.set_line_width(2.0);
                for (pos, vel) in rb_entities {
                    let end = Vec3::new(pos.x + vel.x * 0.2, pos.y + vel.y * 0.2, pos.z - 0.001);
                    let start = Vec3::new(pos.x, pos.y, pos.z - 0.001);
                    renderer.draw_line(start, end, velocity_color, -1);
                }
                renderer.set_line_width(prev_line_width);
            }
        }

        // Selected entity outline.
        if let Some(selected) = self.selection_context {
            if let Some(transform) = self.scene.get_component::<TransformComponent>(selected) {
                let outline_color = Vec4::new(1.0, 0.5, 0.0, 1.0);
                let outline_transform =
                    if let Some(tm) = self.scene.get_component::<TilemapComponent>(selected) {
                        // Expand outline to cover the full tilemap grid.
                        transform.get_transform()
                            * Mat4::from_scale_rotation_translation(
                                Vec3::new(
                                    tm.width as f32 * tm.tile_size.x,
                                    tm.height as f32 * tm.tile_size.y,
                                    1.0,
                                ),
                                Quat::IDENTITY,
                                Vec3::new(
                                    (tm.width as f32 - 1.0) * tm.tile_size.x * 0.5,
                                    (tm.height as f32 - 1.0) * tm.tile_size.y * 0.5,
                                    0.0,
                                ),
                            )
                    } else {
                        transform.get_transform()
                    };
                renderer.draw_rect_transform(&outline_transform, outline_color, -1);
            }
        }

        // Tilemap paint cursor highlight.
        if self.tilemap_paint.is_active() && self.playback.scene_state == SceneState::Edit {
            if let Some(entity) = self.selection_context {
                if self.scene.has_component::<TilemapComponent>(entity) {
                    if let Some((px, py)) = self.viewport.mouse_pos {
                        let vp = self.editor_camera.view_projection();
                        let world_transform = self.scene.get_world_transform(entity);
                        let tilemap_z = self
                            .scene
                            .get_component::<TransformComponent>(entity)
                            .map(|tc| tc.translation.z)
                            .unwrap_or(0.0);
                        let (tile_size, grid_w, grid_h) = {
                            let tm = self
                                .scene
                                .get_component::<TilemapComponent>(entity)
                                .unwrap();
                            (tm.tile_size, tm.width, tm.height)
                        };

                        if let Some((col, row)) = panels::viewport::screen_to_tile_grid(
                            px,
                            py,
                            self.viewport.size,
                            &vp,
                            &world_transform,
                            tilemap_z,
                            tile_size,
                            grid_w,
                            grid_h,
                        ) {
                            // Compute world position of this tile cell.
                            let local_x = col as f32 * tile_size.x;
                            let local_y = row as f32 * tile_size.y;
                            let tile_world =
                                world_transform * Vec4::new(local_x, local_y, 0.0, 1.0);
                            let tile_transform = Mat4::from_scale_rotation_translation(
                                Vec3::new(tile_size.x, tile_size.y, 1.0),
                                Quat::IDENTITY,
                                Vec3::new(tile_world.x, tile_world.y, tilemap_z - 0.001),
                            );

                            let cursor_color = if self.tilemap_paint.brush_tile_id == -1 {
                                Vec4::new(1.0, 0.2, 0.2, 1.0) // red for eraser
                            } else {
                                Vec4::new(0.2, 1.0, 0.2, 1.0) // green for paint
                            };
                            renderer.draw_rect_transform(&tile_transform, cursor_color, -1);
                        }
                    }
                }
            }
        }
    }
}

// File commands, playback, and toolbar are in file_ops.rs, playback.rs, and toolbar.rs.

// ---------------------------------------------------------------------------
// Script file watcher factory
// ---------------------------------------------------------------------------

#[cfg(feature = "lua-scripting")]
fn create_script_watcher(
    scripts_dir: &std::path::Path,
    reload_pending: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    if !scripts_dir.is_dir() {
        info!(
            "Scripts directory '{}' not found — file watcher disabled",
            scripts_dir.display()
        );
        return None;
    }

    let pending = reload_pending.clone();
    let scripts_dir_owned = scripts_dir.to_path_buf();
    match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            if matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            ) && event
                .paths
                .iter()
                .any(|p| p.extension().is_some_and(|e| e == "lua"))
            {
                pending.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }) {
        Ok(mut watcher) => {
            if let Err(e) = watcher.watch(&scripts_dir_owned, RecursiveMode::Recursive) {
                warn!("Failed to watch scripts directory: {e}");
                None
            } else {
                info!(
                    "Watching {} for script changes",
                    scripts_dir_owned.display()
                );
                Some(watcher)
            }
        }
        Err(e) => {
            warn!("Failed to create script file watcher: {e}");
            None
        }
    }
}

fn main() {
    run::<GGEditor>();
}
