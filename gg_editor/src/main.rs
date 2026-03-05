mod camera_controller;
mod editor_settings;
mod gizmo;
mod hub;
mod panels;
mod physics_player;
#[cfg(not(target_os = "macos"))]
mod title_bar;
mod undo;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::Gizmo;

use camera_controller::NativeCameraFollow;
use editor_settings::EditorSettings;
use gizmo::GizmoOperation;
use physics_player::PhysicsPlayer;
use panels::content_browser::{render_dnd_ghost, ASSETS_DIR};
use panels::{EditorTabViewer, ProjectContext, Tab, TilesetPreviewInfo, ViewportState};

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
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    editor_mode: EditorMode,
    editor_settings: EditorSettings,
    project: Option<Project>,
    scene_state: SceneState,
    editor_scene: Option<Scene>,
    editor_scene_path: Option<String>,
    dock_state: egui_dock::DockState<Tab>,
    scene_fb: Option<Framebuffer>,
    viewport_size: (u32, u32),
    viewport_focused: bool,
    viewport_hovered: bool,
    vsync: bool,
    frame_time_ms: f32,
    render_stats: Renderer2DStats,
    scene: Scene,
    selection_context: Option<Entity>,
    gizmo: Gizmo,
    gizmo_operation: GizmoOperation,
    editor_camera: EditorCamera,
    hovered_entity: i32,
    assets_root: PathBuf,
    current_directory: PathBuf,
    pending_open_path: Option<PathBuf>,
    asset_manager: Option<EditorAssetManager>,
    pending_font_loads: Vec<(Entity, PathBuf)>,
    font_cache: HashMap<PathBuf, Ref<Font>>,
    /// Old scenes awaiting GPU-safe destruction (deferred from on_egui to on_render).
    pending_drop_scenes: Vec<Scene>,
    show_physics_colliders: bool,
    show_grid: bool,
    snap_to_grid: bool,
    grid_size: f32,
    hierarchy_filter: String,
    scene_warnings: Vec<String>,
    tilemap_paint: TilemapPaintState,
    viewport_mouse_pos: Option<(f32, f32)>,
    /// Mapping from opaque texture handle → egui TextureId for UI rendering.
    egui_texture_map: HashMap<u64, egui::TextureId>,
    scene_dirty: bool,
    /// Countdown timer for auto-save (seconds). Resets on manual save.
    autosave_timer: f32,
    undo_system: undo::UndoSystem,
    gizmo_editing: bool,
    gizmo_local: bool,
    is_paused: bool,
    step_frames: i32,
    should_exit: bool,
    /// Set from on_egui each frame; checked in on_event next frame to suppress
    /// editor shortcuts (Q/W/E/R/Delete/Escape/X) while typing in text fields.
    egui_wants_keyboard: bool,
    /// Previous window title; only call `window.set_title()` when it changes.
    prev_window_title: String,
    /// Cached window geometry for persistence on exit.
    cached_window_state: editor_settings::WindowState,
    /// When `Some`, the "New Scene" modal is open with the current name text.
    new_scene_modal: Option<String>,
    /// Whether the keyboard shortcuts help dialog is open.
    show_shortcuts_dialog: bool,
    /// UUID of the entity last copied via Ctrl+C. Used by Ctrl+V to duplicate.
    clipboard_entity_uuid: Option<u64>,
    /// Current editor color theme.
    theme: gg_engine::ui_theme::EditorTheme,
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
        let project = std::env::args()
            .nth(1)
            .and_then(|arg| {
                if arg.ends_with(".ggproject") {
                    // Canonicalize the path so project_directory is absolute.
                    let abs_path = std::fs::canonicalize(&arg)
                        .unwrap_or_else(|_| PathBuf::from(&arg));
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
                info!("CWD set to project directory: {}", proj.project_directory().display());
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
        let (scene, editor_scene_path) = if let Some(ref proj) = project {
            let start_path = proj.start_scene_path();
            if start_path.exists() {
                let mut scene = Scene::new();
                let path_str = start_path.to_string_lossy().to_string();
                if SceneSerializer::deserialize(&mut scene, &path_str) {
                    info!("Loaded project start scene: {}", path_str);
                    (scene, Some(path_str))
                } else {
                    warn!("Failed to load start scene, creating empty scene");
                    (Scene::new(), None)
                }
            } else {
                info!("Start scene '{}' not found, creating empty scene", start_path.display());
                (Scene::new(), None)
            }
        } else {
            (Scene::new(), None)
        };

        // Record CLI-loaded project in recent projects.
        if let Some(ref proj) = project {
            if let Some(arg) = std::env::args().nth(1) {
                let abs_path = std::fs::canonicalize(&arg)
                    .unwrap_or_else(|_| PathBuf::from(&arg));
                editor_settings.add_recent_project(proj.name(), &abs_path.to_string_lossy());
            }
        }

        // --- File watcher for automatic Lua script reloading ---------------
        #[cfg(feature = "lua-scripting")]
        let script_reload_pending =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Only create the file watcher in Editor mode (not Hub mode) to
        // avoid an OS-level filesystem monitor thread sitting idle.
        #[cfg(feature = "lua-scripting")]
        let _script_watcher = if editor_mode == EditorMode::Editor {
            create_script_watcher(
                &assets_root.join("scripts"),
                &script_reload_pending,
            )
        } else {
            None
        };

        let initial_vsync = editor_settings.vsync;
        let initial_show_colliders = editor_settings.show_physics_colliders;
        let initial_gizmo_op = editor_settings.gizmo_operation;
        let initial_cam_state = editor_settings.camera_state.clone();
        let initial_show_grid = editor_settings.show_grid;
        let initial_snap_to_grid = editor_settings.snap_to_grid;
        let initial_grid_size = editor_settings.grid_size;
        let initial_window_state = editor_settings.window_state.clone();
        let initial_theme = editor_settings.theme;

        // Restore dock layout from settings, or build default layout.
        let dock_state = if let Some(saved) = editor_settings.dock_layout.take() {
            saved
        } else {
            Self::default_dock_layout()
        };

        GGEditor {
            editor_mode,
            editor_settings,
            project,
            scene_state: SceneState::Edit,
            editor_scene: None,
            editor_scene_path,
            dock_state,
            scene_fb: None,
            viewport_size: (0, 0),
            viewport_focused: false,
            viewport_hovered: false,
            vsync: initial_vsync,
            frame_time_ms: 0.0,
            render_stats: Renderer2DStats::default(),
            scene,
            selection_context: None,
            gizmo: Gizmo::default(),
            gizmo_operation: initial_gizmo_op,
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
            hovered_entity: -1,
            current_directory: assets_root.clone(),
            assets_root,
            pending_open_path: None,
            asset_manager,
            pending_font_loads: Vec::new(),
            font_cache: HashMap::new(),
            pending_drop_scenes: Vec::new(),
            show_physics_colliders: initial_show_colliders,
            show_grid: initial_show_grid,
            snap_to_grid: initial_snap_to_grid,
            grid_size: initial_grid_size,
            hierarchy_filter: String::new(),
            scene_warnings: Vec::new(),
            tilemap_paint: TilemapPaintState::new(),
            viewport_mouse_pos: None,
            egui_texture_map: HashMap::new(),
            scene_dirty: false,
            autosave_timer: Self::AUTOSAVE_INTERVAL_SECS,
            undo_system: undo::UndoSystem::new(),
            gizmo_editing: false,
            gizmo_local: true,
            is_paused: false,
            step_frames: 0,
            should_exit: false,
            egui_wants_keyboard: false,
            prev_window_title: String::new(),
            cached_window_state: initial_window_state,
            new_scene_modal: None,
            show_shortcuts_dialog: false,
            clipboard_entity_uuid: None,
            theme: initial_theme,
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

    fn on_attach(&mut self, renderer: &Renderer) {
        let fb = renderer.create_framebuffer(FramebufferSpec {
            width: 800,
            height: 600,
            attachments: vec![
                FramebufferTextureFormat::RGBA8.into(),
                FramebufferTextureFormat::RedInteger.into(),
                FramebufferTextureFormat::Depth.into(),
            ],
        });
        self.scene_fb = Some(fb);
    }

    fn scene_framebuffer(&self) -> Option<&Framebuffer> {
        self.scene_fb.as_ref()
    }

    fn scene_framebuffer_mut(&mut self) -> Option<&mut Framebuffer> {
        self.scene_fb.as_mut()
    }

    fn desired_viewport_size(&self) -> Option<(u32, u32)> {
        if self.viewport_size.0 > 0 && self.viewport_size.1 > 0 {
            Some(self.viewport_size)
        } else {
            None
        }
    }

    fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Immediate
        }
    }

    fn block_events(&self) -> bool {
        !self.viewport_hovered
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
        if self.scene_state != SceneState::Play {
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
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.new_scene();
                }
                KeyCode::O if ctrl => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.open_scene();
                }
                KeyCode::S if ctrl && shift => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene_as();
                }
                KeyCode::S if ctrl && !shift => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene();
                }

                // Undo/Redo — edit mode only.
                KeyCode::Z if ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.perform_undo();
                }
                KeyCode::Z if ctrl && shift && self.scene_state == SceneState::Edit => {
                    self.perform_redo();
                }
                KeyCode::Y if ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.perform_redo();
                }

                // Copy entity — edit mode only.
                KeyCode::C if ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.on_copy_entity();
                }
                // Paste entity — edit mode only.
                KeyCode::V if ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.on_paste_entity();
                }

                // Entity duplication — edit mode only.
                KeyCode::D if ctrl && self.scene_state == SceneState::Edit => {
                    self.on_duplicate_entity();
                }

                // Script reload — available in any scene state.
                #[cfg(feature = "lua-scripting")]
                KeyCode::R if ctrl && !shift => {
                    self.scene.reload_lua_scripts();
                    panels::properties::clear_field_cache();
                }

                // Delete selected entity — edit mode only, not while typing.
                KeyCode::Delete if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    if let Some(entity) = self.selection_context.take() {
                        self.undo_system.record(&self.scene);
                        if self.scene.destroy_entity(entity).is_ok() {
                            self.scene_dirty = true;
                        }
                    }
                }

                // Escape — clear brush first, then clear selection (edit mode only).
                KeyCode::Escape if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    if self.tilemap_paint.is_active() {
                        self.tilemap_paint.clear_brush();
                    } else {
                        self.selection_context = None;
                    }
                }

                // X — toggle eraser mode (edit mode only, not while typing).
                KeyCode::X if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    if self.tilemap_paint.brush_tile_id == -1 {
                        self.tilemap_paint.clear_brush();
                    } else {
                        self.tilemap_paint.brush_tile_id = -1;
                    }
                }

                // Gizmo shortcuts (Q/W/E/R) — edit mode only, not while typing.
                KeyCode::Q if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    self.gizmo_operation = GizmoOperation::None;
                }
                KeyCode::W if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    self.gizmo_operation = GizmoOperation::Translate;
                }
                KeyCode::E if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    self.gizmo_operation = GizmoOperation::Rotate;
                }
                KeyCode::R if !ctrl && !shift && !self.egui_wants_keyboard
                    && self.scene_state == SceneState::Edit =>
                {
                    self.gizmo_operation = GizmoOperation::Scale;
                }
                _ => {}
            }
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        // Exponential moving average for stable frame time display.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Auto-save: periodically save a backup when there are unsaved changes.
        if self.scene_dirty && self.scene_state == SceneState::Edit {
            self.autosave_timer -= dt.seconds();
            if self.autosave_timer <= 0.0 {
                self.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
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
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
            self.editor_camera.set_viewport_size(w as f32, h as f32);
        }

        match self.scene_state {
            SceneState::Edit => {
                // Update editor camera (orbit/pan/zoom via Alt+mouse).
                self.editor_camera.on_update(dt, input);
            }
            SceneState::Simulate => {
                // Update editor camera — simulation renders from the editor
                // camera, not the scene camera.
                self.editor_camera.on_update(dt, input);
                // Step physics (no scripts) — skip when paused unless stepping.
                if !self.is_paused || self.step_frames > 0 {
                    // When manually stepping, use a fixed dt so each click
                    // advances exactly one physics step regardless of frame rate.
                    let physics_dt = if self.step_frames > 0 {
                        Timestep::from_seconds(1.0 / 60.0)
                    } else {
                        dt
                    };
                    self.scene.on_update_physics(physics_dt, None);
                    self.scene.on_update_animations(physics_dt.seconds());
                    if self.step_frames > 0 {
                        self.step_frames -= 1;
                    }
                }
            }
            SceneState::Play => {
                // Skip updates when paused unless stepping.
                if !self.is_paused || self.step_frames > 0 {
                    // When manually stepping, use a fixed dt so each click
                    // advances exactly one physics step regardless of frame rate.
                    let step_dt = if self.step_frames > 0 {
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
                    if self.step_frames > 0 {
                        self.step_frames -= 1;
                    }
                }
            }
        }

        // Read latest pixel readback result.
        self.hovered_entity = self
            .scene_fb
            .as_ref()
            .map(|fb| fb.hovered_entity())
            .unwrap_or(-1);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        // Drop old scenes that may hold GPU resources (textures). We must
        // wait for all in-flight GPU work to finish before destroying them,
        // since previous frames' command buffers may still reference them.
        if !self.pending_drop_scenes.is_empty() {
            renderer.wait_gpu_idle();
            self.pending_drop_scenes.clear();
        }

        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Step 1: Poll completed async loads.
        if let Some(ref mut am) = self.asset_manager {
            let font_results = am.poll_loaded(renderer);
            for result in font_results {
                if let gg_engine::asset::LoadResult::Font { font_key, data } = result {
                    match data {
                        Ok(cpu_data) => {
                            let font = Ref::new(renderer.upload_font(cpu_data));
                            self.font_cache.insert(font_key, font);
                        }
                        Err(e) => {
                            warn!("Async font load failed: {e}");
                        }
                    }
                }
            }
        }

        // Step 2: Resolve texture handles (async — requests loads, assigns ready textures).
        if let Some(ref mut am) = self.asset_manager {
            self.scene.resolve_texture_handles_async(am);
            self.scene.resolve_audio_handles(am);
        }

        // Step 3: Process deferred font loads for TextComponents.
        for (entity, path) in self.pending_font_loads.drain(..) {
            if self.scene.is_alive(entity) {
                if let Some(font) = self.font_cache.get(&path) {
                    if let Some(mut tc) = self.scene.get_component_mut::<TextComponent>(entity) {
                        tc.font = Some(font.clone());
                    }
                } else if let Some(ref mut am) = self.asset_manager {
                    am.loader().request_font(path);
                }
            }
        }

        // Step 4: Check for TextComponents that need fonts loaded (e.g. after font_path change).
        {
            let needs_load: Vec<(Entity, std::path::PathBuf)> = self
                .scene
                .each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    let tc = self.scene.get_component::<TextComponent>(*entity)?;
                    if tc.font.is_none() && !tc.font_path.is_empty() {
                        let path = std::path::PathBuf::from(&tc.font_path);
                        if path.exists() {
                            Some((*entity, path))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            for (entity, path) in needs_load {
                if let Some(font) = self.font_cache.get(&path) {
                    if let Some(mut tc) = self.scene.get_component_mut::<TextComponent>(entity) {
                        tc.font = Some(font.clone());
                    }
                } else if let Some(ref mut am) = self.asset_manager {
                    am.loader().request_font(path);
                }
            }
        }

        match self.scene_state {
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

    fn on_egui(&mut self, ctx: &egui::Context, window: &Window) {
        // Apply saved theme on first frame (engine defaults to Dark).
        if self.prev_window_title.is_empty() && self.theme != gg_engine::ui_theme::EditorTheme::Dark {
            gg_engine::ui_theme::apply_theme(ctx, self.theme);
        }

        // Track whether egui wants keyboard input (text editing, etc.)
        // so on_event can suppress editor shortcuts next frame.
        self.egui_wants_keyboard = ctx.wants_keyboard_input();

        // Cache window geometry for persistence on exit.
        if !window.is_minimized().unwrap_or(false) {
            self.cached_window_state.maximized = window.is_maximized();
            if !window.is_maximized() {
                if let Ok(pos) = window.outer_position() {
                    self.cached_window_state.x = pos.x;
                    self.cached_window_state.y = pos.y;
                }
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    self.cached_window_state.width = size.width;
                    self.cached_window_state.height = size.height;
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
        let dirty_marker = if self.scene_dirty { " *" } else { "" };
        let title = {
            let project_prefix = match &self.project {
                Some(proj) => format!("GGEditor - {}", proj.name()),
                None => "GGEditor".into(),
            };
            match &self.editor_scene_path {
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
        if title != self.prev_window_title {
            window.set_title(&title);
            self.prev_window_title = title;
        }

        // -- Title bar / Menu bar --
        #[cfg(not(target_os = "macos"))]
        {
            let play_state = match self.scene_state {
                SceneState::Edit => title_bar::PlayState::Edit,
                SceneState::Play => title_bar::PlayState::Play,
                SceneState::Simulate => title_bar::PlayState::Simulate,
            };
            let project_title = match &self.project {
                Some(proj) => {
                    match &self.editor_scene_path {
                        Some(path) => {
                            let scene_name = std::path::Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            format!("GGEngine - {} - {}", proj.name(), scene_name)
                        }
                        None => format!("GGEngine - {}", proj.name()),
                    }
                }
                None => {
                    match &self.editor_scene_path {
                        Some(path) => {
                            let scene_name = std::path::Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            format!("GGEngine - {}", scene_name)
                        }
                        None => "GGEngine".into(),
                    }
                }
            };
            let response = title_bar::title_bar_ui(ctx, window, play_state, self.is_paused, &project_title, |ui| {
                self.menu_bar_contents(ui);
            });
            if response.close_requested {
                self.request_exit();
            }
            if response.play_toggled {
                match self.scene_state {
                    SceneState::Edit => self.on_scene_play(),
                    SceneState::Simulate => {
                        self.on_scene_stop();
                        self.on_scene_play();
                    }
                    SceneState::Play => self.on_scene_stop(),
                }
            }
            if response.simulate_toggled {
                match self.scene_state {
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

        let fb_tex_id = self.scene_fb.as_ref().and_then(|fb| fb.egui_texture_id());

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
            let egui_tex = self.egui_texture_map.get(&tex.egui_handle()).copied()?;
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

        // Scope the viewer so its borrows are released before we handle
        // pending actions and paint the DnD ghost overlay.
        {
            let current_snap_to_grid = self.snap_to_grid;
            let current_grid_size = self.grid_size;
            let mut viewer = EditorTabViewer {
                scene: &mut self.scene,
                selection_context: &mut self.selection_context,
                pending_open_path: &mut self.pending_open_path,
                is_playing: self.scene_state == SceneState::Play,  // Simulate still uses editor camera + gizmos
                scene_dirty: &mut self.scene_dirty,
                undo_system: &mut self.undo_system,
                hierarchy_filter: &mut self.hierarchy_filter,
                scene_warnings: &self.scene_warnings,
                tilemap_paint: &mut self.tilemap_paint,
                vsync: &mut self.vsync,
                frame_time_ms: self.frame_time_ms,
                render_stats: self.render_stats,
                show_physics_colliders: &mut self.show_physics_colliders,
                show_grid: &mut self.show_grid,
                snap_to_grid: &mut self.snap_to_grid,
                grid_size: &mut self.grid_size,
                theme: &mut self.theme,
                viewport: ViewportState {
                    size: &mut self.viewport_size,
                    focused: &mut self.viewport_focused,
                    hovered: &mut self.viewport_hovered,
                    fb_tex_id,
                    gizmo: &mut self.gizmo,
                    gizmo_operation: &mut self.gizmo_operation,
                    gizmo_editing: &mut self.gizmo_editing,
                    editor_camera: &self.editor_camera,
                    scene_fb: &mut self.scene_fb,
                    hovered_entity: self.hovered_entity,
                    mouse_pos: &mut self.viewport_mouse_pos,
                    tileset_preview,
                    snap_to_grid: current_snap_to_grid,
                    grid_size: current_grid_size,
                    gizmo_local: &mut self.gizmo_local,
                },
                project: ProjectContext {
                    assets_root: &self.assets_root,
                    current_directory: &mut self.current_directory,
                    asset_manager: &mut self.asset_manager,
                    project_name: self.project.as_ref().map(|p| p.name()),
                    editor_scene_path: self.editor_scene_path.as_deref(),
                    egui_texture_map: &self.egui_texture_map,
                },
            };

            egui_dock::DockArea::new(&mut self.dock_state)
                .style(dock_style)
                .show(ctx, &mut viewer);
        }

        // Sync editor settings to disk when they change.
        if self.vsync != self.editor_settings.vsync
            || self.show_physics_colliders != self.editor_settings.show_physics_colliders
            || self.gizmo_operation != self.editor_settings.gizmo_operation
            || self.show_grid != self.editor_settings.show_grid
            || self.snap_to_grid != self.editor_settings.snap_to_grid
            || self.grid_size != self.editor_settings.grid_size
            || self.theme != self.editor_settings.theme
        {
            self.editor_settings.vsync = self.vsync;
            self.editor_settings.show_physics_colliders = self.show_physics_colliders;
            self.editor_settings.gizmo_operation = self.gizmo_operation;
            self.editor_settings.show_grid = self.show_grid;
            self.editor_settings.snap_to_grid = self.snap_to_grid;
            self.editor_settings.grid_size = self.grid_size;
            self.editor_settings.theme = self.theme;
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
        if let Some(path) = self.pending_open_path.take() {
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
        handles
    }

    fn receive_egui_user_textures(&mut self, map: &HashMap<u64, egui::TextureId>) {
        self.egui_texture_map = map.clone();
    }
}

// ---------------------------------------------------------------------------
// Overlay rendering (collider visualization, debug shapes)
// ---------------------------------------------------------------------------

impl GGEditor {
    /// Auto-save interval in seconds (5 minutes).
    const AUTOSAVE_INTERVAL_SECS: f32 = 300.0;

    fn render_grid(&self, renderer: &mut Renderer) {
        let grid_size = self.grid_size;
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
            renderer.draw_line(Vec3::new(x, lo_y, -0.01), Vec3::new(x, hi_y, -0.01), color, -1);
        }

        // Horizontal lines (constant Y).
        for j in y_min..=y_max {
            let y = j as f32 * grid_size;
            let color = if j == 0 { axis_color_x } else { grid_color };
            renderer.draw_line(Vec3::new(lo_x, y, -0.01), Vec3::new(hi_x, y, -0.01), color, -1);
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
        self.editor_settings.show_grid = self.show_grid;
        self.editor_settings.snap_to_grid = self.snap_to_grid;
        self.editor_settings.grid_size = self.grid_size;
        // Persist window geometry.
        self.editor_settings.window_state = self.cached_window_state.clone();
        // Persist dock layout.
        self.editor_settings.dock_layout = Some(self.dock_state.clone());
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
        let [top_left, _bottom_left] = surface.split_below(left, 0.7, vec![Tab::ContentBrowser]);
        let [left_sidebar, _viewport] = surface.split_right(top_left, 0.20, vec![Tab::Viewport]);
        surface.split_below(left_sidebar, 0.5, vec![Tab::Settings]);
        dock_state
    }

    fn on_overlay_render(&self, renderer: &mut Renderer) {
        // Set the appropriate camera for the overlay pass.
        match self.scene_state {
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
        if self.show_grid && self.scene_state != SceneState::Play {
            self.render_grid(renderer);
        }

        // Physics collider visualization (uses world transforms for hierarchy support).
        if self.show_physics_colliders {
            let collider_color = Vec4::new(0.0, 1.0, 0.0, 1.0);

            // Collect entities with colliders (need owned data to avoid borrow conflicts).
            let circle_entities: Vec<_> = self.scene.each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    self.scene.get_component::<CircleCollider2DComponent>(*entity)
                        .map(|cc| (*entity, cc.offset, cc.radius))
                })
                .collect();
            for (entity, offset, radius) in circle_entities {
                let world = self.scene.get_world_transform(entity);
                let (world_scale, world_rot, world_trans) =
                    world.to_scale_rotation_translation();
                let rotated_offset = world_rot * Vec3::new(
                    offset.x * world_scale.x,
                    offset.y * world_scale.y,
                    0.0,
                );
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

            let box_entities: Vec<_> = self.scene.each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    self.scene.get_component::<BoxCollider2DComponent>(*entity)
                        .map(|bc| (*entity, bc.offset, bc.size))
                })
                .collect();
            for (entity, offset, size) in box_entities {
                let world = self.scene.get_world_transform(entity);
                let (world_scale, world_rot, world_trans) =
                    world.to_scale_rotation_translation();
                let rotated_offset = world_rot * Vec3::new(
                    offset.x * world_scale.x,
                    offset.y * world_scale.y,
                    0.0,
                );
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
                let collider_transform = Mat4::from_scale_rotation_translation(
                    scale,
                    world_rot,
                    translation,
                );
                renderer.draw_rect_transform(&collider_transform, collider_color, -1);
            }
        }

        // Selected entity outline.
        if let Some(selected) = self.selection_context {
            if let Some(transform) = self.scene.get_component::<TransformComponent>(selected) {
                let outline_color = Vec4::new(1.0, 0.5, 0.0, 1.0);
                renderer.draw_rect_transform(&transform.get_transform(), outline_color, -1);
            }
        }

        // Tilemap paint cursor highlight.
        if self.tilemap_paint.is_active() && self.scene_state == SceneState::Edit {
            if let Some(entity) = self.selection_context {
                if self.scene.has_component::<TilemapComponent>(entity) {
                    if let Some((px, py)) = self.viewport_mouse_pos {
                        let vp = self.editor_camera.view_projection();
                        let world_transform = self.scene.get_world_transform(entity);
                        let tilemap_z = self.scene.get_component::<TransformComponent>(entity)
                            .map(|tc| tc.translation.z)
                            .unwrap_or(0.0);
                        let (tile_size, grid_w, grid_h) = {
                            let tm = self.scene.get_component::<TilemapComponent>(entity).unwrap();
                            (tm.tile_size, tm.width, tm.height)
                        };

                        if let Some((col, row)) = panels::viewport::screen_to_tile_grid(
                            px, py, self.viewport_size, &vp, &world_transform,
                            tilemap_z, tile_size, grid_w, grid_h,
                        ) {
                            // Compute world position of this tile cell.
                            let local_x = col as f32 * tile_size.x;
                            let local_y = row as f32 * tile_size.y;
                            let tile_world = world_transform * Vec4::new(local_x, local_y, 0.0, 1.0);
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

// ---------------------------------------------------------------------------
// Play / Stop
// ---------------------------------------------------------------------------

impl GGEditor {
    #[cfg(target_os = "macos")]
    fn toolbar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(34.0)
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0x25, 0x25, 0x26))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                // 1px bottom border line.
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.min.x, rect.max.y),
                        egui::pos2(rect.max.x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3C, 0x3C, 0x3C)),
                );

                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center)
                        .with_main_justify(true),
                    |ui| {
                        ui.add_space(3.0);

                        let is_edit = self.scene_state == SceneState::Edit;
                        let has_play_button = is_edit;
                        let has_simulate_button = is_edit;
                        let has_stop_button = !is_edit;
                        let has_pause_button = !is_edit;
                        let has_step_button = !is_edit && self.is_paused;

                        let btn_size = egui::vec2(28.0, 28.0);
                        let spacing = 4.0;
                        let button_count = [has_play_button, has_simulate_button, has_stop_button, has_pause_button, has_step_button]
                            .iter()
                            .filter(|&&b| b)
                            .count() as f32;
                        let total_width = btn_size.x * button_count + spacing * (button_count - 1.0).max(0.0);
                        let avail = ui.available_width();
                        ui.add_space((avail - total_width) / 2.0);

                        let hover_bg = egui::Color32::from_rgb(0x40, 0x40, 0x40);
                        let pause_active_bg = egui::Color32::from_rgb(0x2A, 0x50, 0x70);

                        // Allocate buttons in order.
                        let play_alloc = has_play_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            ui.add_space(spacing);
                            a
                        });
                        let sim_alloc = has_simulate_button.then(|| {
                            ui.allocate_exact_size(btn_size, egui::Sense::click())
                        });
                        let stop_alloc = has_stop_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            ui.add_space(spacing);
                            a
                        });
                        let pause_alloc = has_pause_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            if has_step_button { ui.add_space(spacing); }
                            a
                        });
                        let step_alloc = has_step_button.then(|| {
                            ui.allocate_exact_size(btn_size, egui::Sense::click())
                        });

                        // Paint icons.
                        if let Some((rect, ref resp)) = play_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), hover_bg);
                            }
                            paint_play_triangle(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = sim_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), hover_bg);
                            }
                            paint_gear_icon(ui.painter(), rect.center(), 8.0);
                        }
                        if let Some((rect, ref resp)) = stop_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), hover_bg);
                            }
                            paint_stop_square(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = pause_alloc {
                            if self.is_paused {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), pause_active_bg);
                            }
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), hover_bg);
                            }
                            paint_pause_icon(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = step_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, egui::CornerRadius::same(3), hover_bg);
                            }
                            paint_step_icon(ui.painter(), rect.center());
                        }

                        // Handle clicks.
                        if let Some((_, ref resp)) = play_alloc {
                            if resp.clicked() { self.on_scene_play(); }
                        }
                        if let Some((_, ref resp)) = sim_alloc {
                            if resp.clicked() { self.on_scene_simulate(); }
                        }
                        if let Some((_, ref resp)) = stop_alloc {
                            if resp.clicked() { self.on_scene_stop(); }
                        }
                        if let Some((_, ref resp)) = pause_alloc {
                            if resp.clicked() { self.on_scene_pause(); }
                        }
                        if let Some((_, ref resp)) = step_alloc {
                            if resp.clicked() { self.on_scene_step(); }
                        }
                    },
                );
            });
    }

    fn on_scene_play(&mut self) {
        self.validate_scene();
        self.scene_state = SceneState::Play;
        let runtime_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, runtime_scene);
        self.editor_scene = Some(editor_scene);

        // Attach native scripts to known entities by tag name.
        // NativeScriptComponent is runtime-only (not serialized), so we bind
        // them here on the runtime copy before starting.
        self.attach_native_scripts();

        self.scene.on_runtime_start();
    }

    fn on_scene_simulate(&mut self) {
        self.validate_scene();
        self.scene_state = SceneState::Simulate;
        let sim_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, sim_scene);
        self.editor_scene = Some(editor_scene);
        self.scene.on_simulation_start();
    }

    /// Validate the current scene and populate `scene_warnings`.
    fn validate_scene(&mut self) {
        let mut warnings = Vec::new();

        // 1. Check for primary camera.
        if self.scene.get_primary_camera_entity().is_none() {
            warnings.push("No primary camera found. The scene will not render correctly at runtime.".to_string());
        }

        // Iterate all entities once, checking component-based validations.
        let entities = self.scene.each_entity_with_tag();
        for (entity, tag) in &entities {
            let entity = *entity;

            // 2. Orphaned colliders (collider without a RigidBody2D).
            if self.scene.has_component::<BoxCollider2DComponent>(entity)
                && !self.scene.has_component::<RigidBody2DComponent>(entity)
            {
                warnings.push(format!("Entity '{}' has BoxCollider2D but no RigidBody2D.", tag));
            }
            if self.scene.has_component::<CircleCollider2DComponent>(entity)
                && !self.scene.has_component::<RigidBody2DComponent>(entity)
            {
                warnings.push(format!("Entity '{}' has CircleCollider2D but no RigidBody2D.", tag));
            }

            // 3. Missing texture assets.
            if let Some(sr) = self.scene.get_component::<SpriteRendererComponent>(entity) {
                let raw = sr.texture_handle.raw();
                if raw != 0 {
                    if let Some(ref am) = self.asset_manager {
                        let handle = Uuid::from_raw(raw);
                        if am.get_metadata(&handle).is_none() {
                            warnings.push(format!("Entity '{}' references a missing texture asset.", tag));
                        }
                    }
                }
            }

            // 4. Missing audio assets.
            if let Some(ac) = self.scene.get_component::<AudioSourceComponent>(entity) {
                let raw = ac.audio_handle.raw();
                if raw != 0 {
                    if let Some(ref am) = self.asset_manager {
                        let handle = Uuid::from_raw(raw);
                        if am.get_metadata(&handle).is_none() {
                            warnings.push(format!("Entity '{}' references a missing audio asset.", tag));
                        }
                    }
                }
            }
        }

        // Log warnings.
        for w in &warnings {
            warn!("[Scene Validation] {}", w);
        }

        self.scene_warnings = warnings;
    }

    fn on_scene_stop(&mut self) {
        match self.scene_state {
            SceneState::Play => self.scene.on_runtime_stop(),
            SceneState::Simulate => self.scene.on_simulation_stop(),
            SceneState::Edit => return,
        }

        self.scene_state = SceneState::Edit;
        self.is_paused = false;
        self.step_frames = 0;

        // Finalize any in-progress gizmo drag so the undo system is clean.
        if self.gizmo_editing {
            self.undo_system.end_edit();
            self.gizmo_editing = false;
        }
        self.tilemap_paint.painting_in_progress = false;
        self.tilemap_paint.painted_this_stroke.clear();

        if let Some(editor_scene) = self.editor_scene.take() {
            let old = std::mem::replace(&mut self.scene, editor_scene);
            self.pending_drop_scenes.push(old);
            self.selection_context = None;

            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
        }
    }

    fn on_scene_pause(&mut self) {
        if self.scene_state == SceneState::Edit {
            return;
        }
        self.is_paused = !self.is_paused;
        if !self.is_paused {
            self.step_frames = 0;
        }
    }

    fn on_scene_step(&mut self) {
        if !self.is_paused {
            return;
        }
        self.step_frames = 1;
    }

    /// Attach known native scripts to entities by tag name.
    ///
    /// Since `NativeScriptComponent` is runtime-only (not serialized to
    /// `.ggscene` files), we bind them here on the runtime scene copy.
    /// This lets `.ggscene` files work with native scripts — the editor
    /// recognizes entity names and attaches the correct script.
    fn attach_native_scripts(&mut self) {
        // Bind PhysicsPlayer (WASD+Space) to "Player" or "Native Player"
        // if they don't already have a script.
        for name in &["Player", "Native Player"] {
            if let Some((entity, _)) = self.scene.find_entity_by_name(name) {
                let has_lua = {
                    #[cfg(feature = "lua-scripting")]
                    { self.scene.has_component::<LuaScriptComponent>(entity) }
                    #[cfg(not(feature = "lua-scripting"))]
                    { false }
                };
                if !has_lua && !self.scene.has_component::<NativeScriptComponent>(entity) {
                    self.scene.add_component(entity, NativeScriptComponent::bind::<PhysicsPlayer>());
                }
            }
        }

        // Bind NativeCameraFollow to "Camera" if it doesn't have a Lua script.
        if let Some((camera, _)) = self.scene.find_entity_by_name("Camera") {
            let has_lua = {
                #[cfg(feature = "lua-scripting")]
                { self.scene.has_component::<LuaScriptComponent>(camera) }
                #[cfg(not(feature = "lua-scripting"))]
                { false }
            };
            if !has_lua && !self.scene.has_component::<NativeScriptComponent>(camera) {
                self.scene.add_component(camera, NativeScriptComponent::bind::<NativeCameraFollow>());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File commands (New / Open / Save As)
// ---------------------------------------------------------------------------

impl GGEditor {
    /// Returns true if it's safe to discard the current scene (either not dirty,
    /// or the user confirmed). Shows a native dialog when the scene has unsaved changes.
    fn confirm_discard_changes(&self) -> bool {
        if !self.scene_dirty {
            return true;
        }
        gg_engine::platform_utils::confirm_dialog(
            "Unsaved Changes",
            "The current scene has unsaved changes. Discard them?",
        )
    }

    /// Shared menu bar contents used by both the custom title bar (Windows/Linux)
    /// and the native menu bar (macOS).
    fn menu_bar_contents(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui
                .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                .clicked()
            {
                if self.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.new_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                .clicked()
            {
                if self.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.open_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                .clicked()
            {
                if self.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.save_scene();
                ui.close();
            }
            if ui
                .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                .clicked()
            {
                if self.scene_state != SceneState::Edit {
                    self.on_scene_stop();
                }
                self.save_scene_as();
                ui.close();
            }
            ui.separator();
            if ui
                .add(egui::Button::new("Open Project..."))
                .clicked()
            {
                self.open_project();
                ui.close();
            }
        });
        let in_edit_mode = self.scene_state == SceneState::Edit;
        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(
                    in_edit_mode && self.undo_system.can_undo(),
                    egui::Button::new("Undo").shortcut_text("Ctrl+Z"),
                )
                .clicked()
            {
                self.perform_undo();
                ui.close();
            }
            if ui
                .add_enabled(
                    in_edit_mode && self.undo_system.can_redo(),
                    egui::Button::new("Redo").shortcut_text("Ctrl+Y"),
                )
                .clicked()
            {
                self.perform_redo();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    in_edit_mode && self.selection_context.is_some(),
                    egui::Button::new("Copy").shortcut_text("Ctrl+C"),
                )
                .clicked()
            {
                self.on_copy_entity();
                ui.close();
            }
            if ui
                .add_enabled(
                    in_edit_mode && self.clipboard_entity_uuid.is_some(),
                    egui::Button::new("Paste").shortcut_text("Ctrl+V"),
                )
                .clicked()
            {
                self.on_paste_entity();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    in_edit_mode && self.selection_context.is_some(),
                    egui::Button::new("Duplicate").shortcut_text("Ctrl+D"),
                )
                .clicked()
            {
                self.on_duplicate_entity();
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            if ui
                .checkbox(&mut self.show_physics_colliders, "Show Physics Colliders")
                .clicked()
            {
                ui.close();
            }
            ui.separator();
            if ui.button("Reset Layout").clicked() {
                self.dock_state = Self::default_dock_layout();
                ui.close();
            }
        });
        #[cfg(feature = "lua-scripting")]
        ui.menu_button("Script", |ui| {
            if ui
                .add(egui::Button::new("Reload Scripts").shortcut_text("Ctrl+R"))
                .clicked()
            {
                self.scene.reload_lua_scripts();
                panels::properties::clear_field_cache();
                ui.close();
            }
        });
        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard Shortcuts").clicked() {
                self.show_shortcuts_dialog = true;
                ui.close();
            }
        });
    }

    fn new_scene(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if self.project.is_some() {
            // Show naming modal — scene will be created on confirm.
            self.new_scene_modal = Some("New Scene".into());
        } else {
            // No project — just create an empty unnamed scene.
            self.create_empty_scene();
        }
    }

    fn create_empty_scene(&mut self) {
        let old = std::mem::replace(&mut self.scene, Scene::new());
        self.pending_drop_scenes.push(old);
        self.selection_context = None;
        self.editor_scene_path = None;
        self.scene_dirty = false;
        self.undo_system.clear();

        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }
    }

    fn new_scene_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(ref mut scene_name) = self.new_scene_modal else {
            return;
        };

        let mut confirmed = false;
        let mut cancelled = false;

        // Dim background.
        let screen_rect = ctx.input(|i| i.viewport_rect());
        egui::Area::new(egui::Id::new("new_scene_modal_bg"))
            .fixed_pos(screen_rect.left_top())
            .show(ctx, |ui| {
                ui.allocate_response(screen_rect.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(128),
                );
            });

        egui::Window::new("New Scene")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(300.0, 0.0))
            .show(ctx, |ui| {
                ui.label("Scene name:");
                let text_edit = ui.text_edit_singleline(scene_name);

                // Auto-focus the text field on first frame.
                if text_edit.gained_focus() || !text_edit.has_focus() {
                    text_edit.request_focus();
                }

                // Enter confirms.
                if text_edit.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    confirmed = true;
                }

                // Escape cancels.
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancelled = true;
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    let name_valid = !scene_name.trim().is_empty();
                    if ui
                        .add_enabled(name_valid, egui::Button::new("Create"))
                        .clicked()
                    {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            let name = self.new_scene_modal.take().unwrap_or_default();
            let name = name.trim().to_string();
            if !name.is_empty() {
                self.create_named_scene(&name);
            }
        } else if cancelled {
            self.new_scene_modal = None;
        }
    }

    fn shortcuts_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts_dialog {
            return;
        }

        egui::Window::new("Keyboard Shortcuts")
            .collapsible(false)
            .resizable(false)
            .open(&mut self.show_shortcuts_dialog)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("General").strong());
                egui::Grid::new("shortcuts_general")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Ctrl+N");
                        ui.label("New Scene");
                        ui.end_row();
                        ui.label("Ctrl+O");
                        ui.label("Open Scene");
                        ui.end_row();
                        ui.label("Ctrl+S");
                        ui.label("Save Scene");
                        ui.end_row();
                        ui.label("Ctrl+Shift+S");
                        ui.label("Save As");
                        ui.end_row();
                        ui.label("Ctrl+Z");
                        ui.label("Undo");
                        ui.end_row();
                        ui.label("Ctrl+Y");
                        ui.label("Redo");
                        ui.end_row();
                        ui.label("Ctrl+D");
                        ui.label("Duplicate Entity");
                        ui.end_row();
                        ui.label("Delete");
                        ui.label("Delete Entity");
                        ui.end_row();
                        ui.label("Ctrl+R");
                        ui.label("Reload Scripts");
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Gizmo").strong());
                egui::Grid::new("shortcuts_gizmo")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Q");
                        ui.label("Select (No Gizmo)");
                        ui.end_row();
                        ui.label("W");
                        ui.label("Translate");
                        ui.end_row();
                        ui.label("E");
                        ui.label("Rotate");
                        ui.end_row();
                        ui.label("R");
                        ui.label("Scale");
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("Viewport").strong());
                egui::Grid::new("shortcuts_viewport")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Middle Mouse");
                        ui.label("Pan");
                        ui.end_row();
                        ui.label("Alt + Left Mouse");
                        ui.label("Orbit");
                        ui.end_row();
                        ui.label("Scroll");
                        ui.label("Zoom");
                        ui.end_row();
                        ui.label("F");
                        ui.label("Focus Selected");
                        ui.end_row();
                    });
            });
    }

    fn create_named_scene(&mut self, name: &str) {
        let scene = Scene::new();

        // Build path: assets_root/scenes/<name>.ggscene
        let scenes_dir = self.assets_root.join("scenes");
        let _ = std::fs::create_dir_all(&scenes_dir);
        let file_name = format!("{}.ggscene", name);
        let scene_path = scenes_dir.join(&file_name);
        let path_str = scene_path.to_string_lossy().to_string();

        // Serialize the empty scene to disk immediately.
        SceneSerializer::serialize(&scene, &path_str, Some(name));

        // Swap in the new scene.
        let old = std::mem::replace(&mut self.scene, scene);
        self.pending_drop_scenes.push(old);
        self.selection_context = None;
        self.editor_scene_path = Some(path_str);
        self.scene_dirty = false;
        self.undo_system.clear();

        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }

        panels::project::invalidate_scene_cache();
    }

    fn open_scene(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if let Some(path) = FileDialogs::open_file("GGScene files", &["ggscene"]) {
            let mut new_scene = Scene::new();
            if SceneSerializer::deserialize(&mut new_scene, &path) {
                let old = std::mem::replace(&mut self.scene, new_scene);
                self.pending_drop_scenes.push(old);
                self.selection_context = None;
                self.editor_scene_path = Some(path);
                self.scene_dirty = false;
                self.undo_system.clear();

                let (w, h) = self.viewport_size;
                if w > 0 && h > 0 {
                    self.scene.on_viewport_resize(w, h);
                }
                self.queue_font_loads_from_scene();
            }
        }
    }

    fn scene_name_from_path(path: &str) -> Option<&str> {
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
    }

    fn save_scene(&mut self) {
        if let Some(ref path) = self.editor_scene_path {
            if SceneSerializer::serialize(&self.scene, path, Self::scene_name_from_path(path)) {
                self.scene_dirty = false;
                self.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
                Self::remove_autosave_file(path);
            } else {
                warn!("Failed to save scene to '{}'", path);
            }
        } else {
            self.save_scene_as();
        }
    }

    fn save_scene_as(&mut self) {
        if let Some(path) = FileDialogs::save_file("GGScene files", &["ggscene"]) {
            if SceneSerializer::serialize(&self.scene, &path, Self::scene_name_from_path(&path)) {
                self.editor_scene_path = Some(path);
                self.scene_dirty = false;
                self.autosave_timer = Self::AUTOSAVE_INTERVAL_SECS;
                panels::project::invalidate_scene_cache();
            } else {
                warn!("Failed to save scene to '{}'", path);
            }
        }
    }

    /// Auto-save the current scene to a `.autosave.ggscene` sidecar file.
    fn perform_autosave(&self) {
        if let Some(ref path) = self.editor_scene_path {
            let autosave_path = Self::autosave_path_for(path);
            if SceneSerializer::serialize(&self.scene, &autosave_path, Self::scene_name_from_path(path)) {
                info!("Auto-saved to '{}'", autosave_path);
            } else {
                warn!("Auto-save failed for '{}'", autosave_path);
            }
        }
    }

    /// Build the auto-save sidecar path: `foo.ggscene` -> `foo.autosave.ggscene`.
    fn autosave_path_for(scene_path: &str) -> String {
        let p = std::path::Path::new(scene_path);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("scene");
        if let Some(parent) = p.parent() {
            parent
                .join(format!("{}.autosave.ggscene", stem))
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{}.autosave.ggscene", stem)
        }
    }

    /// Remove the auto-save sidecar file after a successful manual save.
    fn remove_autosave_file(scene_path: &str) {
        let autosave = Self::autosave_path_for(scene_path);
        if std::path::Path::new(&autosave).exists() {
            if let Err(e) = std::fs::remove_file(&autosave) {
                warn!("Failed to remove auto-save file '{}': {}", autosave, e);
            }
        }
    }

    fn perform_undo(&mut self) {
        // Capture the selected entity's UUID before replacing the scene,
        // since hecs entity IDs change after deserialization.
        let selected_uuid = self.selection_context.and_then(|sel| {
            self.scene
                .get_component::<IdComponent>(sel)
                .map(|id| id.id.raw())
        });
        if let Some(restored) = self.undo_system.undo(&self.scene) {
            let old = std::mem::replace(&mut self.scene, restored);
            self.pending_drop_scenes.push(old);
            // Restore selection by IdComponent UUID (stable across serialization).
            self.selection_context =
                selected_uuid.and_then(|uuid| self.scene.find_entity_by_uuid(uuid));
            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.queue_font_loads_from_scene();
            self.scene_dirty = true;
        }
    }

    fn perform_redo(&mut self) {
        let selected_uuid = self.selection_context.and_then(|sel| {
            self.scene
                .get_component::<IdComponent>(sel)
                .map(|id| id.id.raw())
        });
        if let Some(restored) = self.undo_system.redo(&self.scene) {
            let old = std::mem::replace(&mut self.scene, restored);
            self.pending_drop_scenes.push(old);
            self.selection_context =
                selected_uuid.and_then(|uuid| self.scene.find_entity_by_uuid(uuid));
            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.queue_font_loads_from_scene();
            self.scene_dirty = true;
        }
    }

    fn on_copy_entity(&mut self) {
        if let Some(selected) = self.selection_context {
            if self.scene.is_alive(selected) {
                self.clipboard_entity_uuid = self
                    .scene
                    .get_component::<IdComponent>(selected)
                    .map(|id| id.id.raw());
            }
        }
    }

    fn on_paste_entity(&mut self) {
        if let Some(uuid) = self.clipboard_entity_uuid {
            if let Some(source) = self.scene.find_entity_by_uuid(uuid) {
                self.undo_system.record(&self.scene);
                let duplicate = self.scene.duplicate_entity(source);
                self.selection_context = Some(duplicate);
                self.scene_dirty = true;
            }
        }
    }

    fn on_duplicate_entity(&mut self) {
        if let Some(selected) = self.selection_context {
            if self.scene.is_alive(selected) {
                self.undo_system.record(&self.scene);
                let duplicate = self.scene.duplicate_entity(selected);
                self.selection_context = Some(duplicate);
                self.scene_dirty = true;
            }
        }
    }

    fn open_scene_from_path(&mut self, path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let mut new_scene = Scene::new();
        if SceneSerializer::deserialize(&mut new_scene, &path_str) {
            // Only clear state after confirming the load succeeded.
            self.scene_dirty = false;
            self.undo_system.clear();
            let old = std::mem::replace(&mut self.scene, new_scene);
            self.pending_drop_scenes.push(old);
            self.selection_context = None;
            self.editor_scene_path = Some(path_str);
            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.queue_font_loads_from_scene();
        }
    }

    /// Queue font loads for the current scene.
    ///
    /// Textures are now resolved via the asset manager in `on_render`
    /// (using `resolve_texture_handles`). Only fonts still need queuing.
    fn queue_font_loads_from_scene(&mut self) {
        let entities = self.scene.each_entity_with_tag();
        for (entity, _tag) in &entities {
            if let Some(tc) = self.scene.get_component::<TextComponent>(*entity) {
                if !tc.font_path.is_empty() && tc.font.is_none() {
                    let path = std::path::PathBuf::from(&tc.font_path);
                    if path.exists() {
                        self.pending_font_loads.push((*entity, path));
                    } else {
                        warn!("Font not found: {}", tc.font_path);
                    }
                }
            }
        }
    }

    fn load_project_from_path(&mut self, project_path: &std::path::Path) {
        let abs_path = std::fs::canonicalize(project_path)
            .unwrap_or_else(|_| project_path.to_path_buf());
        let Some(project) = Project::load(&abs_path.to_string_lossy()) else {
            warn!("Failed to load project: {}", abs_path.display());
            return;
        };

        // Stop playback if active.
        if self.scene_state != SceneState::Edit {
            self.on_scene_stop();
        }

        // Set CWD to the project directory.
        if let Err(e) = std::env::set_current_dir(project.project_directory()) {
            warn!(
                "Failed to set CWD to project directory '{}': {}",
                project.project_directory().display(),
                e
            );
        }

        // Update assets root.
        self.assets_root = project.asset_directory_path();
        self.current_directory = self.assets_root.clone();

        // Create and load asset manager for the new project.
        let mut am = EditorAssetManager::new(&self.assets_root);
        am.load_registry();
        self.asset_manager = Some(am);

        // Load start scene.
        let start_path = project.start_scene_path();
        if start_path.exists() {
            let path_str = start_path.to_string_lossy().to_string();
            let mut new_scene = Scene::new();
            if SceneSerializer::deserialize(&mut new_scene, &path_str) {
                let old = std::mem::replace(&mut self.scene, new_scene);
                self.pending_drop_scenes.push(old);
                self.editor_scene_path = Some(path_str);
                self.queue_font_loads_from_scene();
            }
        } else {
            let old = std::mem::replace(&mut self.scene, Scene::new());
            self.pending_drop_scenes.push(old);
            self.editor_scene_path = None;
        }

        // Restart script watcher for the new project's scripts directory.
        #[cfg(feature = "lua-scripting")]
        {
            self._script_watcher = create_script_watcher(
                &self.assets_root.join("scripts"),
                &self.script_reload_pending,
            );
        }

        // Resize viewport for the new scene.
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }

        // Update recent projects and editor state.
        self.editor_settings.add_recent_project(project.name(), &abs_path.to_string_lossy());
        self.project = Some(project);
        self.selection_context = None;
        self.scene_dirty = false;
        self.undo_system.clear();
        self.editor_mode = EditorMode::Editor;

        // Reset all thread-local panel caches/dialogs for the new project.
        panels::reset_all_panel_state();
    }

    fn open_project(&mut self) {
        if !self.confirm_discard_changes() {
            return;
        }
        if let Some(path) = FileDialogs::open_file("GGProject files", &["ggproject"]) {
            self.load_project_from_path(&PathBuf::from(&path));
        }
    }

    fn handle_new_project_from_hub(&mut self, path: &PathBuf) {
        // Ensure the parent directory exists (project_name/ subfolder).
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    error!("Failed to create project directory {}: {}", parent.display(), e);
                    return;
                }
            }
        }

        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());

        if Project::new(&path.to_string_lossy(), &name).is_some() {
            self.load_project_from_path(path);
        }
    }
}

/// Procedural play triangle icon (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_play_triangle(painter: &egui::Painter, center: egui::Pos2) {
    let half = 7.0;
    let points = vec![
        egui::pos2(center.x - half * 0.7, center.y - half),
        egui::pos2(center.x + half, center.y),
        egui::pos2(center.x - half * 0.7, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::from_rgb(0x4E, 0xC9, 0x4E),
        egui::Stroke::NONE,
    ));
}

/// Procedural stop square icon (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_stop_square(painter: &egui::Painter, center: egui::Pos2) {
    let half = 6.0;
    let stop_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(
        stop_rect,
        egui::CornerRadius::same(2),
        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
    );
}

/// Procedural pause icon — two vertical bars (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_pause_icon(painter: &egui::Painter, center: egui::Pos2) {
    let bar_w = 3.0;
    let bar_h = 12.0;
    let gap = 2.5;
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x - gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x + gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
}

/// Procedural step-forward icon — play triangle + vertical bar (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_step_icon(painter: &egui::Painter, center: egui::Pos2) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let half = 5.0;
    let offset_x = -2.0;
    let points = vec![
        egui::pos2(center.x + offset_x - half * 0.6, center.y - half),
        egui::pos2(center.x + offset_x + half * 0.7, center.y),
        egui::pos2(center.x + offset_x - half * 0.6, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    let bar_x = center.x + half * 0.7;
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(bar_x, center.y),
            egui::vec2(2.5, half * 2.0),
        ),
        0.0,
        color,
    );
}

/// Procedural gear icon for the simulate button (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_gear_icon(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let bg = egui::Color32::from_rgb(0x25, 0x25, 0x26);
    let teeth = 6;
    let inner_r = radius * 0.55;
    let outer_r = radius;
    let tooth_width = std::f32::consts::PI / (teeth as f32 * 2.0);

    let mut points = Vec::new();
    for i in 0..teeth {
        let angle = (i as f32 / teeth as f32) * std::f32::consts::TAU;
        let a1 = angle - tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a1.cos(),
            center.y + inner_r * a1.sin(),
        ));
        let a2 = angle - tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a2.cos(),
            center.y + outer_r * a2.sin(),
        ));
        let a3 = angle + tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a3.cos(),
            center.y + outer_r * a3.sin(),
        ));
        let a4 = angle + tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a4.cos(),
            center.y + inner_r * a4.sin(),
        ));
    }

    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    painter.circle_filled(center, radius * 0.25, bg);
}

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
