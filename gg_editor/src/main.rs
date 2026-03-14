mod build;
mod camera_controller;
mod editor_settings;
mod file_ops;
mod gizmo;
mod hub;
mod icons;
mod panels;
mod physics_player;
mod playback;
mod selection;
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
// Post-processing & GPU timing state (synced with Renderer each frame)
// ---------------------------------------------------------------------------

/// Editor-side mirror of post-processing settings.
/// Synced to/from `PostProcessPipeline` in `on_render_viewport`.
pub(crate) struct PostProcessSettings {
    pub enabled: bool,
    pub bloom_enabled: bool,
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,
    pub bloom_filter_radius: f32,
    pub tonemapping: TonemappingMode,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub contact_shadows_enabled: bool,
    pub contact_shadows_max_distance: f32,
    pub contact_shadows_thickness: f32,
    pub contact_shadows_intensity: f32,
    pub contact_shadows_step_count: i32,
    pub contact_shadows_debug: i32,
    pub shadow_debug_mode: i32,
    pub shadow_quality: i32,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            bloom_enabled: true,
            bloom_threshold: 0.8,
            bloom_intensity: 0.3,
            bloom_filter_radius: 1.0,
            tonemapping: TonemappingMode::ACES,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            contact_shadows_enabled: false,
            contact_shadows_max_distance: 0.15,
            contact_shadows_thickness: 0.02,
            contact_shadows_intensity: 0.6,
            contact_shadows_step_count: 64,
            contact_shadows_debug: 0,
            shadow_debug_mode: 0,
            shadow_quality: 3, // Ultra (PCSS) by default
        }
    }
}

/// Snapshot of GPU timestamp profiling results for UI display.
pub(crate) struct GpuTimingSnapshot {
    pub enabled: bool,
    pub total_frame_ms: f32,
    pub results: Vec<(String, f32)>,
}

impl Default for GpuTimingSnapshot {
    fn default() -> Self {
        Self {
            enabled: false,
            total_frame_ms: 0.0,
            results: Vec::new(),
        }
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
    /// Old framebuffers kept alive until the GPU is done with them.
    /// Drained on the next frame after `device_wait_idle`.
    pending_drop_fbs: Vec<Framebuffer>,
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
    /// UUIDs of entities last copied via Ctrl+C. Used by Ctrl+V to duplicate.
    clipboard_entity_uuids: Vec<u64>,
    /// Deferred game viewport framebuffer creation (set in on_egui, consumed in on_render).
    create_game_fb: bool,
    /// MSAA sample count changed in settings — triggers framebuffer + pipeline recreation.
    msaa_changed: bool,
    /// Current wireframe rendering mode (Off / WireOnly / Overlay).
    wireframe_mode: WireframeMode,
    /// Post-processing output texture registered with egui.
    pp_output_egui_tex_id: Option<egui::TextureId>,
    /// Draw MSAA test pattern (diagonal lines) to verify anti-aliasing.
    show_msaa_test: bool,
    /// When `Some`, the build project modal is open.
    build_modal: Option<build::BuildModal>,
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
    selection: selection::Selection,
    editor_camera: EditorCamera,
    tilemap_paint: TilemapPaintState,
    undo_system: undo::UndoSystem,
    should_exit: bool,
    /// Maximum MSAA supported by the GPU (stored as highest MsaaSamples variant).
    max_msaa_samples: MsaaSamples,
    /// Post-processing settings (synced with Renderer each frame).
    postprocess_settings: PostProcessSettings,
    /// GPU timestamp profiling snapshot for UI display.
    gpu_timing: GpuTimingSnapshot,
    /// Post-processing output descriptor set handle (for egui texture registration).
    pp_output_handle: u64,
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
    /// File watcher that monitors the assets directory for texture changes.
    _asset_watcher: Option<notify::RecommendedWatcher>,
    /// Receiver for texture file paths changed on disk (relative to assets root).
    asset_reload_rx: std::sync::mpsc::Receiver<String>,
    /// Input action map loaded from the project config.
    input_actions: InputActionMap,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // -- Project loading (CLI arg) --
        let project = std::env::args().nth(1).and_then(|arg| {
            if arg.ends_with(".ggproject") {
                // Canonicalize the path so project_directory is absolute.
                let abs_path = std::fs::canonicalize(&arg).unwrap_or_else(|_| PathBuf::from(&arg));
                match Project::load(&abs_path.to_string_lossy()) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        warn!("Failed to load project '{}': {}", abs_path.display(), e);
                        None
                    }
                }
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
                if SceneSerializer::deserialize(&mut scene, &path_str).is_ok() {
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

        // --- File watcher for automatic texture hot-reload -----------------
        let (asset_reload_tx, asset_reload_rx) = std::sync::mpsc::channel::<String>();
        let _asset_watcher = if editor_mode == EditorMode::Editor {
            create_asset_watcher(&assets_root, asset_reload_tx)
        } else {
            None
        };

        let input_actions = project
            .as_ref()
            .map(|p| p.input_actions().clone())
            .unwrap_or_default();

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
                clipboard_entity_uuids: Vec::new(),
                create_game_fb: false,
                msaa_changed: false,
                wireframe_mode: WireframeMode::Off,
                pp_output_egui_tex_id: None,
                show_msaa_test: false,
                build_modal: None,
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
                pending_drop_fbs: Vec::new(),
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
            selection: selection::Selection::default(),
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
            max_msaa_samples: MsaaSamples::S1, // Updated in on_attach
            postprocess_settings: PostProcessSettings::default(),
            gpu_timing: GpuTimingSnapshot::default(),
            pp_output_handle: 0,
            #[cfg(feature = "lua-scripting")]
            _script_watcher,
            #[cfg(feature = "lua-scripting")]
            script_reload_pending,
            _asset_watcher,
            asset_reload_rx,
            input_actions,
        }
    }

    fn window_config(&self) -> WindowConfig {
        let ws = &self.editor_settings.window_state;
        WindowConfig {
            title: "GGEditor".into(),
            width: ws.width,
            height: ws.height,
            decorations: cfg!(target_os = "macos"),
            position: ws.position,
            maximized: ws.maximized,
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        self.max_msaa_samples = MsaaSamples::from_vk(renderer.max_msaa_samples());
        let msaa_samples = self
            .editor_settings
            .msaa_samples
            .clamp_to_device(self.max_msaa_samples.to_vk());
        match renderer.create_framebuffer(FramebufferSpec {
            width: 800,
            height: 600,
            attachments: vec![
                FramebufferTextureFormat::RGBA16F.into(),
                FramebufferTextureFormat::RedInteger.into(),
                FramebufferTextureFormat::NormalMap.into(),
                FramebufferTextureFormat::Depth.into(),
            ],
            samples: msaa_samples.to_vk(),
        }) {
            Ok(fb) => {
                // Initialize post-processing pipeline with the scene framebuffer.
                if let Err(e) = renderer.init_postprocess(
                    fb.color_image_view(),
                    fb.depth_image_view(),
                    fb.msaa_depth_image_view(),
                    fb.normal_image_view(),
                    fb.width(),
                    fb.height(),
                ) {
                    warn!("Failed to create post-processing pipeline: {e}");
                } else if let Some(pp) = renderer.postprocess() {
                    self.pp_output_handle = pp.output_egui_handle();
                }
                self.viewport.scene_fb = Some(fb);
            }
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

    fn on_render_shadows(
        &mut self,
        renderer: &mut Renderer,
        cmd_buf: gg_engine::ash::vk::CommandBuffer,
        current_frame: usize,
    ) {
        // Flush deferred-destroy vertex arrays that are now safe to drop
        // (the fence for this frame slot has been waited on by the caller).
        self.scene.rotate_va_graveyard();

        // Ensure meshes invalidated during on_egui are re-uploaded before
        // the shadow pass, preventing one-frame shadow gaps.
        self.scene.resolve_meshes(renderer);

        // Run the shadow depth pass (shared shadow map for all viewports).
        // Pass the editor camera's frustum info for per-cascade fitting.
        // Read shadow distance from the scene's directional light (if any).
        let shadow_distance = self.scene.find_first_shadow_distance().unwrap_or(100.0);
        let camera_info = gg_engine::renderer::ShadowCameraInfo {
            view_projection: self.editor_camera.view_projection(),
            near: self.editor_camera.near_clip(),
            far: self.editor_camera.far_clip(),
            camera_position: self.editor_camera.position(),
            shadow_distance,
        };
        self.scene
            .render_shadow_pass(renderer, cmd_buf, current_frame, 0, Some(&camera_info));
    }

    fn on_render_viewport(&mut self, renderer: &mut Renderer, viewport_index: usize) {
        // Sync post-processing settings from editor UI to the renderer pipeline.
        if viewport_index == 0 {
            if let Some(pp) = renderer.postprocess_mut() {
                let s = &self.postprocess_settings;
                pp.enabled = s.enabled;
                pp.bloom_enabled = s.bloom_enabled;
                pp.bloom_threshold = s.bloom_threshold;
                pp.bloom_intensity = s.bloom_intensity;
                pp.bloom_filter_radius = s.bloom_filter_radius;
                pp.tonemapping = s.tonemapping;
                pp.exposure = s.exposure;
                pp.contrast = s.contrast;
                pp.saturation = s.saturation;
                pp.contact_shadows_enabled = s.contact_shadows_enabled;
                pp.contact_shadows_max_distance = s.contact_shadows_max_distance;
                pp.contact_shadows_thickness = s.contact_shadows_thickness;
                pp.contact_shadows_intensity = s.contact_shadows_intensity;
                pp.contact_shadows_step_count = s.contact_shadows_step_count;
                pp.contact_shadows_debug = s.contact_shadows_debug;
            }

            // Sync shadow cascade debug mode and quality tier to renderer.
            renderer.set_shadow_debug_mode(self.postprocess_settings.shadow_debug_mode);
            renderer.set_shadow_quality(self.postprocess_settings.shadow_quality);

            // Update post-process output handle (may change on resize).
            if let Some(pp) = renderer.postprocess() {
                self.pp_output_handle = pp.output_egui_handle();
            }

            // Sync GPU profiler enabled state.
            if let Some(profiler) = renderer.gpu_profiler_mut() {
                profiler.set_enabled(self.gpu_timing.enabled);
            }

            // Read back GPU timing results for UI display.
            if let Some(profiler) = renderer.gpu_profiler() {
                self.gpu_timing.total_frame_ms = profiler.total_frame_ms();
                self.gpu_timing.results.clear();
                for result in profiler.results() {
                    self.gpu_timing
                        .results
                        .push((result.name.to_string(), result.time_ms));
                }
            }
        }

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

    fn input_action_map(&self) -> Option<InputActionMap> {
        if self.input_actions.actions.is_empty() {
            None
        } else {
            Some(self.input_actions.clone())
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
                KeyCode::B if ctrl && shift => {
                    self.open_build_modal();
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

                // Delete selected entities — edit mode only, not while typing.
                KeyCode::Delete
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    if !self.selection.is_empty() {
                        let entities: Vec<Entity> = self.selection.iter().collect();
                        self.selection.clear();
                        self.undo_system.record(&self.scene, "Delete entity");
                        for entity in entities {
                            if self.scene.destroy_entity(entity).is_ok() {
                                self.scene_ctx.dirty = true;
                            }
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
                        self.selection.clear();
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

                // Gizmo shortcuts (Q/W/E/R) — edit mode only, not while typing or flying.
                KeyCode::Q
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && !self.editor_camera.is_flying()
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::None;
                }
                KeyCode::W
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && !self.editor_camera.is_flying()
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Translate;
                }
                KeyCode::E
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && !self.editor_camera.is_flying()
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Rotate;
                }
                KeyCode::R
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && !self.editor_camera.is_flying()
                        && self.playback.scene_state == SceneState::Edit =>
                {
                    self.gizmo_state.operation = GizmoOperation::Scale;
                }

                // Focus on selected entity (F).
                KeyCode::F
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state != SceneState::Play =>
                {
                    if let Some(entity) = self.selection.single() {
                        let world = self.scene.get_world_transform(entity);
                        let pos = world.col(3).truncate();
                        self.editor_camera.focus_on(pos);
                    } else {
                        // No selection — focus on origin.
                        self.editor_camera.focus_on(Vec3::ZERO);
                    }
                }

                // Play/Stop toggle (F5) — mirrors toolbar play button.
                KeyCode::F5 if !ctrl && !shift && !self.ui.egui_wants_keyboard => {
                    match self.playback.scene_state {
                        SceneState::Edit => self.on_scene_play(),
                        SceneState::Simulate => {
                            self.on_scene_stop();
                            self.on_scene_play();
                        }
                        SceneState::Play => self.on_scene_stop(),
                    }
                }
                // Simulate toggle (F6) — mirrors toolbar simulate button.
                KeyCode::F6 if !ctrl && !shift && !self.ui.egui_wants_keyboard => {
                    match self.playback.scene_state {
                        SceneState::Edit => self.on_scene_simulate(),
                        SceneState::Play => {
                            self.on_scene_stop();
                            self.on_scene_simulate();
                        }
                        SceneState::Simulate => self.on_scene_stop(),
                    }
                }
                // Pause toggle (F7) — mirrors toolbar pause button.
                KeyCode::F7
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state != SceneState::Edit =>
                {
                    self.on_scene_pause();
                }
                // Step frame (F8) — mirrors toolbar step button.
                KeyCode::F8
                    if !ctrl
                        && !shift
                        && !self.ui.egui_wants_keyboard
                        && self.playback.scene_state != SceneState::Edit =>
                {
                    self.on_scene_step();
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
                // Editor camera moved to on_late_update for minimal input-to-display latency.
                // Tick animation previews (editor inspector play button).
                self.scene.on_update_animation_previews(dt.seconds());
            }
            SceneState::Simulate => {
                // Step physics (no scripts) — skip when paused unless stepping.
                if !self.playback.paused || self.playback.step_frames > 0 {
                    // When manually stepping, use a fixed dt so each click
                    // advances exactly one physics step regardless of frame rate.
                    let physics_dt = if self.playback.step_frames > 0 {
                        Timestep::from_seconds(1.0 / 60.0)
                    } else {
                        dt
                    };
                    self.scene.on_update_all_physics(physics_dt, None);
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
                    self.scene.on_update_all_physics(step_dt, Some(input));
                    // Run native scripts (e.g. CameraController) with up-to-date transforms.
                    self.scene.on_update_scripts(step_dt, input);
                    // Run Lua scripts.
                    #[cfg(feature = "lua-scripting")]
                    self.scene.on_update_lua_scripts(step_dt, input);
                    // Advance sprite animations.
                    self.scene.on_update_animations(step_dt.seconds());
                    // Update spatial audio panning/attenuation.
                    self.scene.update_spatial_audio();

                    // UI interaction: hit test + dispatch Lua callbacks.
                    if let Some((px, py)) = self.viewport.mouse_pos {
                        let mouse_world = self.scene.screen_to_world_2d(px, py);
                        let mouse_down = input.is_mouse_button_pressed(MouseButton::Left);
                        let just_pressed = input.is_mouse_button_just_pressed(MouseButton::Left);
                        let just_released = input.is_mouse_button_just_released(MouseButton::Left);
                        let events = self.scene.update_ui_with_input(
                            mouse_world,
                            mouse_down,
                            just_pressed,
                            just_released,
                        );
                        if !events.is_empty() {
                            self.scene.dispatch_ui_events(&events);
                        }
                    }

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

    fn on_late_update(&mut self, dt: Timestep, input: &Input) {
        // Update editor camera as late as possible to minimize input-to-display
        // latency. This runs after on_update + on_egui, right before the VP
        // matrix is captured for GPU command recording.
        if self.editor_mode != EditorMode::Hub && self.playback.scene_state != SceneState::Play {
            self.editor_camera.on_update(dt, input);
        }
    }

    fn on_pre_render(&mut self, renderer: &mut Renderer) {
        // Drain deferred framebuffers from a previous MSAA change.
        // The old FBs were kept alive so this frame's egui primitives (which
        // may reference the old descriptor sets) could render safely. Now that
        // the GPU is idle, we can destroy them.
        if !self.viewport.pending_drop_fbs.is_empty() {
            renderer.wait_gpu_idle();
            self.viewport.pending_drop_fbs.clear();
        }

        // Handle MSAA sample count change — recreate framebuffers + pipelines.
        // This runs BEFORE viewport_infos extraction so the new framebuffer
        // handles are captured for the current frame's command buffer recording.
        if self.ui.msaa_changed {
            self.ui.msaa_changed = false;
            renderer.wait_gpu_idle();

            let msaa = self
                .editor_settings
                .msaa_samples
                .clamp_to_device(self.max_msaa_samples.to_vk());
            let samples = msaa.to_vk();
            let attachments = vec![
                FramebufferTextureFormat::RGBA16F.into(),
                FramebufferTextureFormat::RedInteger.into(),
                FramebufferTextureFormat::NormalMap.into(),
                FramebufferTextureFormat::Depth.into(),
            ];

            // Move old framebuffers to deferred-destroy list (kept alive for
            // this frame because egui primitives may still reference the old
            // descriptor sets / egui texture IDs).
            if let Some(old_fb) = self.viewport.scene_fb.take() {
                self.viewport.pending_drop_fbs.push(old_fb);
            }
            match renderer.create_framebuffer(FramebufferSpec {
                width: self.viewport.size.0.max(1),
                height: self.viewport.size.1.max(1),
                attachments: attachments.clone(),
                samples,
            }) {
                Ok(fb) => {
                    if let Err(e) = renderer.resize_postprocess(
                        fb.color_image_view(),
                        fb.depth_image_view(),
                        fb.msaa_depth_image_view(),
                        fb.normal_image_view(),
                        fb.width(),
                        fb.height(),
                    ) {
                        error!("Failed to resize postprocess for MSAA: {e}");
                    } else if let Some(pp) = renderer.postprocess() {
                        self.pp_output_handle = pp.output_egui_handle();
                    }
                    self.viewport.scene_fb = Some(fb);
                }
                Err(e) => error!("Failed to recreate scene FB for MSAA: {e}"),
            }

            // Recreate game framebuffer if it exists.
            if let Some(old_game_fb) = self.viewport.game_fb.take() {
                self.viewport.pending_drop_fbs.push(old_game_fb);
                match renderer.create_framebuffer(FramebufferSpec {
                    width: self.viewport.game_size.0.max(1),
                    height: self.viewport.game_size.1.max(1),
                    attachments,
                    samples,
                }) {
                    Ok(fb) => self.viewport.game_fb = Some(fb),
                    Err(e) => error!("Failed to recreate game FB for MSAA: {e}"),
                }
            }

            // Offscreen pipeline recreation is handled automatically by
            // application.rs after on_pre_render returns.
            info!("MSAA changed to {msaa}");
        }

        // Deferred game viewport framebuffer creation (toggled from View menu).
        if self.ui.create_game_fb && self.viewport.game_fb.is_none() {
            self.ui.create_game_fb = false;
            let msaa_samples = self
                .editor_settings
                .msaa_samples
                .clamp_to_device(self.max_msaa_samples.to_vk());
            match renderer.create_framebuffer(FramebufferSpec {
                width: 800,
                height: 600,
                attachments: vec![
                    FramebufferTextureFormat::RGBA8.into(),
                    FramebufferTextureFormat::RedInteger.into(),
                    FramebufferTextureFormat::Depth.into(),
                ],
                samples: msaa_samples.to_vk(),
            }) {
                Ok(fb) => self.viewport.game_fb = Some(fb),
                Err(e) => warn!("Failed to create game framebuffer: {e}"),
            }
        }
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("GGEditor::on_render");

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
                    let msg = e.to_string();
                    error!("Shader hot-reload failed: {}", msg);
                    gg_engine::platform_utils::error_dialog("Shader Reload Error", &msg);
                }
            }
        }

        if self.editor_mode == EditorMode::Hub {
            return;
        }

        // Hot-reload textures modified on disk.
        {
            let mut reloaded_any = false;
            while let Ok(relative_path) = self.asset_reload_rx.try_recv() {
                if let Some(ref mut am) = self.project_state.asset_manager {
                    if let Some(handle) = am.registry().find_by_path(&relative_path) {
                        if am.is_loaded(&handle) {
                            am.reload_texture(&handle, renderer);
                            self.scene.clear_texture_refs_for_handle(handle);
                            reloaded_any = true;
                        }
                    }
                }
            }
            if reloaded_any {
                self.scene.invalidate_texture_cache();
            }
        }

        // Step 1: Poll completed async loads (textures + fonts).
        if let Some(ref mut am) = self.project_state.asset_manager {
            am.poll_loaded(renderer);
        }

        // Submit any batched texture/font uploads before rendering.
        renderer.flush_transfers();

        // Step 2: Resolve texture, audio, font, and mesh handles (async — non-blocking).
        // If the scene was modified this frame, invalidate the texture resolution
        // cache so new or changed texture handles get picked up.
        if self.scene_ctx.dirty {
            self.scene.invalidate_texture_cache();
        }
        if let Some(ref mut am) = self.project_state.asset_manager {
            self.scene.resolve_texture_handles_async(am);
            self.scene.resolve_audio_handles(am);
            self.scene.load_fonts_async(am);
            self.scene.resolve_mesh_assets(am);
            self.scene.resolve_skinned_mesh_assets(am);
            self.scene.resolve_environment_map(renderer, am);
        }
        self.scene.resolve_meshes(renderer);
        self.scene.resolve_skinned_meshes(renderer);

        // Apply wireframe mode to the renderer.
        let wf_mode = self.ui.wireframe_mode;
        renderer.set_wireframe_mode(wf_mode);

        match self.playback.scene_state {
            SceneState::Edit => {
                renderer.set_camera_position(self.editor_camera.position());
                renderer.set_camera_clip_planes(
                    self.editor_camera.near_clip(),
                    self.editor_camera.far_clip(),
                );
                renderer.set_camera_matrices(
                    *self.editor_camera.view_matrix(),
                    *self.editor_camera.projection(),
                );
                // Apply UI anchors so anchored entities appear at their
                // runtime positions in the editor viewport.
                self.scene.apply_ui_anchors();
                self.scene
                    .on_update_editor(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Simulate => {
                renderer.set_camera_position(self.editor_camera.position());
                renderer.set_camera_clip_planes(
                    self.editor_camera.near_clip(),
                    self.editor_camera.far_clip(),
                );
                renderer.set_camera_matrices(
                    *self.editor_camera.view_matrix(),
                    *self.editor_camera.projection(),
                );
                self.scene.apply_ui_anchors();
                self.scene
                    .on_update_simulation(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Play => {
                self.scene.on_update_runtime(renderer);
            }
        }

        // MSAA test pattern: diagonal lines at various angles to verify anti-aliasing.
        if self.ui.show_msaa_test {
            self.draw_msaa_test_pattern(renderer);
        }

        // Wireframe overlay: draw dark outlines on top of shaded geometry.
        if wf_mode == WireframeMode::Overlay {
            self.draw_wireframe_overlay(renderer);
        }

        // Reset wireframe mode so overlays/grid/gizmos render filled.
        renderer.set_wireframe_mode(WireframeMode::Off);

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
                    self.editor_settings.window_state.position = Some((pos.x, pos.y));
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

        // If post-processing is active, use its output texture; otherwise use the raw scene FB.
        let fb_tex_id = if self.postprocess_settings.enabled {
            self.ui.pp_output_egui_tex_id
        } else {
            self.viewport
                .scene_fb
                .as_ref()
                .and_then(|fb| fb.egui_texture_id())
        };

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
        let tileset_preview = self.selection.single().and_then(|entity| {
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
            self.editor_settings.show_xz_grid,
            self.editor_settings.snap_to_grid,
            self.editor_settings.grid_size,
            self.editor_settings.theme,
            self.editor_settings.gizmo_operation,
            self.editor_settings.msaa_samples,
        );

        // Scope the viewer so its borrows are released before we handle
        // pending actions and paint the DnD ghost overlay.
        {
            let current_snap_to_grid = self.editor_settings.snap_to_grid;
            let current_grid_size = self.editor_settings.grid_size;
            let mut hierarchy_action = None;
            let project_name_owned = self
                .project_state
                .project
                .as_ref()
                .map(|p| p.name().to_string());
            let mut viewer = EditorTabViewer {
                scene: &mut self.scene,
                selection: &mut self.selection,
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
                theme: &mut self.editor_settings.theme,
                reload_shaders_requested: &mut self.ui.reload_shaders_requested,
                msaa_samples: &mut self.editor_settings.msaa_samples,
                max_msaa_samples: self.max_msaa_samples,
                msaa_changed: &mut self.ui.msaa_changed,
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
                    project_name: project_name_owned.as_deref(),
                    editor_scene_path: self.scene_ctx.editor_scene_path.as_deref(),
                    egui_texture_map: &self.ui.egui_texture_map,
                    input_actions: &mut self.input_actions,
                    project: &mut self.project_state.project,
                },
                hierarchy_action: &mut hierarchy_action,
                postprocess_settings: &mut self.postprocess_settings,
                gpu_timing: &mut self.gpu_timing,
                show_msaa_test: &mut self.ui.show_msaa_test,
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
            self.editor_settings.show_xz_grid,
            self.editor_settings.snap_to_grid,
            self.editor_settings.grid_size,
            self.editor_settings.theme,
            self.editor_settings.gizmo_operation,
            self.editor_settings.msaa_samples,
        );
        if settings_snapshot != settings_current {
            self.editor_settings.save();
        }

        // Auto-clear tilemap brush when selection changes to a non-tilemap
        // entity or deselects entirely.
        if self.tilemap_paint.is_active() {
            let has_tilemap = self
                .selection
                .single()
                .map(|e| self.scene.has_component::<TilemapComponent>(e))
                .unwrap_or(false);
            if !has_tilemap {
                self.tilemap_paint.clear_brush();
            }
        }

        // "New Scene" naming modal.
        self.new_scene_modal_ui(ctx);

        // "Build Project" modal.
        self.build_modal_ui(ctx);

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

        // Register sprite sheet textures for selected entities (animation timeline panel).
        for entity in self.selection.iter() {
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

        // Register loaded asset textures for content browser thumbnails.
        if let Some(ref am) = self.project_state.asset_manager {
            handles.extend(am.loaded_texture_egui_handles());
        }

        // Register post-processing output for viewport display.
        if self.pp_output_handle != 0 {
            handles.push(self.pp_output_handle);
        }

        handles
    }

    fn receive_egui_user_textures(&mut self, map: &HashMap<u64, egui::TextureId>) {
        self.ui.egui_texture_map = map.clone();
        // Capture the post-processing output's egui texture ID for viewport display.
        self.ui.pp_output_egui_tex_id = if self.pp_output_handle != 0 {
            map.get(&self.pp_output_handle).copied()
        } else {
            None
        };
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

    fn render_xz_grid(&self, renderer: &mut Renderer) {
        profile_scope!("GGEditor::render_xz_grid");
        let grid_size = self.editor_settings.grid_size;
        if grid_size <= 0.0 {
            return;
        }

        let grid_color = Vec4::new(0.35, 0.35, 0.35, 0.5);
        let axis_color_x = Vec4::new(0.8, 0.2, 0.2, 0.6);
        let axis_color_z = Vec4::new(0.2, 0.4, 0.8, 0.6);

        // Determine visible range from camera.
        let focal = self.editor_camera.focal_point();
        let dist = self.editor_camera.distance();
        let half_extent = dist * 1.5;

        // Snap grid bounds to grid lines.
        let x_min = ((focal.x - half_extent) / grid_size).floor() as i32;
        let x_max = ((focal.x + half_extent) / grid_size).ceil() as i32;
        let z_min = ((focal.z - half_extent) / grid_size).floor() as i32;
        let z_max = ((focal.z + half_extent) / grid_size).ceil() as i32;

        let lo_z = z_min as f32 * grid_size;
        let hi_z = z_max as f32 * grid_size;
        let lo_x = x_min as f32 * grid_size;
        let hi_x = x_max as f32 * grid_size;

        // Lines along Z (constant X).
        for i in x_min..=x_max {
            let x = i as f32 * grid_size;
            let color = if i == 0 { axis_color_z } else { grid_color };
            renderer.draw_line(Vec3::new(x, 0.0, lo_z), Vec3::new(x, 0.0, hi_z), color, -1);
        }

        // Lines along X (constant Z).
        for j in z_min..=z_max {
            let z = j as f32 * grid_size;
            let color = if j == 0 { axis_color_x } else { grid_color };
            renderer.draw_line(Vec3::new(lo_x, 0.0, z), Vec3::new(hi_x, 0.0, z), color, -1);
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

    /// Draw dark wireframe outlines on top of shaded geometry (Shaded Wireframe mode).
    /// Draw a test pattern of diagonal lines to visually verify MSAA.
    /// Without MSAA these lines show obvious stairstepping; with MSAA
    /// they should appear smooth.
    fn draw_msaa_test_pattern(&self, renderer: &Renderer) {
        let white = Vec4::new(1.0, 1.0, 1.0, 1.0);
        let red = Vec4::new(1.0, 0.2, 0.2, 1.0);
        let green = Vec4::new(0.2, 1.0, 0.2, 1.0);
        let blue = Vec4::new(0.3, 0.5, 1.0, 1.0);
        let yellow = Vec4::new(1.0, 1.0, 0.2, 1.0);
        let z = 0.0;

        // Fan of lines at different angles from origin — each angle
        // produces a different aliasing pattern.
        let center = Vec3::new(0.0, 0.0, z);
        let radius = 3.0;
        let line_count = 16;
        for i in 0..line_count {
            let angle = std::f32::consts::PI * i as f32 / line_count as f32;
            let dx = radius * angle.cos();
            let dy = radius * angle.sin();
            let color = match i % 4 {
                0 => white,
                1 => red,
                2 => green,
                _ => blue,
            };
            renderer.draw_line(center, Vec3::new(dx, dy, z), color, -1);
        }

        // A few rotated rectangles (thin quads drawn as line loops) to
        // show edge aliasing clearly.
        for i in 0..6 {
            let angle = std::f32::consts::PI * i as f32 / 6.0;
            let c = angle.cos();
            let s = angle.sin();
            let hw = 2.0;
            let hh = 0.01; // Very thin — almost a line
            let corners = [
                Vec3::new(hw * c - hh * s, hw * s + hh * c, z),
                Vec3::new(-hw * c - hh * s, -hw * s + hh * c, z),
                Vec3::new(-hw * c + hh * s, -hw * s - hh * c, z),
                Vec3::new(hw * c + hh * s, hw * s - hh * c, z),
            ];
            for j in 0..4 {
                renderer.draw_line(corners[j], corners[(j + 1) % 4], yellow, -1);
            }
        }
    }

    fn draw_wireframe_overlay(&self, renderer: &mut Renderer) {
        let wire_color = Vec4::new(0.0, 0.0, 0.0, 0.6);
        let prev_line_width = renderer.line_width();
        renderer.set_line_width(1.0);

        // Collect all entity handles + relevant data (avoids borrow conflicts with Scene).
        let entities = self.scene.each_entity_with_tag();

        for (entity, _tag) in &entities {
            // 2D sprites — quad outlines.
            if self
                .scene
                .get_component::<SpriteRendererComponent>(*entity)
                .is_some()
            {
                let world = self.scene.get_world_transform(*entity);
                renderer.draw_rect_transform(&world, wire_color, -1);
                continue;
            }

            // 2D circles — underlying quad outlines.
            if self
                .scene
                .get_component::<CircleRendererComponent>(*entity)
                .is_some()
            {
                let world = self.scene.get_world_transform(*entity);
                renderer.draw_rect_transform(&world, wire_color, -1);
                continue;
            }

            // 3D meshes — bounding box outlines.
            if let Some(mesh) = self.scene.get_component::<MeshRendererComponent>(*entity) {
                let bounds = if let Some(b) = mesh.local_bounds {
                    b
                } else if let MeshSource::Primitive(p) = &mesh.mesh_source {
                    p.local_bounds()
                } else {
                    continue;
                };
                let world = self.scene.get_world_transform(*entity);
                renderer.draw_box_outline(&world, bounds.0, bounds.1, wire_color, -1);
            }
        }

        renderer.set_line_width(prev_line_width);
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
        if (self.editor_settings.show_grid || self.editor_settings.show_xz_grid)
            && self.playback.scene_state != SceneState::Play
        {
            let prev_line_width = renderer.line_width();
            renderer.set_line_width(1.0);
            if self.editor_settings.show_grid {
                self.render_grid(renderer);
            }
            if self.editor_settings.show_xz_grid {
                self.render_xz_grid(renderer);
            }
            renderer.set_line_width(prev_line_width);
        }

        // Viewport bounds rectangle — shows the primary camera's visible area
        // so the user can see where UI-anchored elements will appear at runtime.
        if self.editor_settings.show_camera_bounds && self.playback.scene_state != SceneState::Play
        {
            if let Some((center, half_w, half_h)) = self.scene.primary_camera_bounds() {
                let prev_line_width = renderer.line_width();
                renderer.set_line_width(1.0);
                let bounds_color = Vec4::new(0.35, 0.65, 1.0, 0.5);
                let z = -0.01; // slightly in front of grid
                let tl = Vec3::new(center.x - half_w, center.y + half_h, z);
                let tr = Vec3::new(center.x + half_w, center.y + half_h, z);
                let br = Vec3::new(center.x + half_w, center.y - half_h, z);
                let bl = Vec3::new(center.x - half_w, center.y - half_h, z);
                renderer.draw_line(tl, tr, bounds_color, -1);
                renderer.draw_line(tr, br, bounds_color, -1);
                renderer.draw_line(br, bl, bounds_color, -1);
                renderer.draw_line(bl, tl, bounds_color, -1);
                renderer.set_line_width(prev_line_width);
            }
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

        // Light gizmos (directional rays + point light radius).
        if self.playback.scene_state != SceneState::Play {
            let prev_lw = renderer.line_width();

            // -- Directional lights: sun circle + direction rays (selected only) --
            let dir_lights: Vec<_> = self
                .scene
                .each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    if !self.selection.contains(*entity) {
                        return None;
                    }
                    let dl = self
                        .scene
                        .get_component::<DirectionalLightComponent>(*entity)?;
                    let world = self.scene.get_world_transform(*entity);
                    let (_, world_rot, pos) = world.to_scale_rotation_translation();
                    let direction = DirectionalLightComponent::direction(world_rot);
                    Some((pos, direction, dl.color))
                })
                .collect();

            for (pos, direction, color) in &dir_lights {
                let dir = direction.normalize();
                // Build perpendicular frame for the sun circle.
                let up_ref = if dir.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
                let right = dir.cross(up_ref).normalize();
                let up = right.cross(dir).normalize();

                let gizmo_color = Vec4::new(
                    color.x.clamp(0.3, 1.0),
                    color.y.clamp(0.3, 1.0),
                    color.z.clamp(0.1, 0.8),
                    1.0,
                );

                let radius = 0.4;
                let ray_len = 1.5;
                let segments = 8;

                renderer.set_line_width(2.0);

                // Circle in the plane perpendicular to the light direction.
                for i in 0..segments {
                    let a0 = (i as f32 / segments as f32) * std::f32::consts::TAU;
                    let a1 = ((i + 1) as f32 / segments as f32) * std::f32::consts::TAU;
                    let p0 = *pos + right * a0.cos() * radius + up * a0.sin() * radius;
                    let p1 = *pos + right * a1.cos() * radius + up * a1.sin() * radius;
                    renderer.draw_line(p0, p1, gizmo_color, -1);
                }

                // Rays from each circle vertex in the light direction.
                for i in 0..segments {
                    let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
                    let edge = *pos + right * angle.cos() * radius + up * angle.sin() * radius;
                    let tip = edge + dir * ray_len;
                    renderer.draw_line(edge, tip, gizmo_color, -1);
                }

                // Central ray (slightly longer).
                renderer.draw_line(*pos, *pos + dir * (ray_len + radius), gizmo_color, -1);
            }

            // -- Point lights: 3-axis wireframe sphere showing radius (selected only) --
            let point_lights: Vec<_> = self
                .scene
                .each_entity_with_tag()
                .iter()
                .filter_map(|(entity, _)| {
                    if !self.selection.contains(*entity) {
                        return None;
                    }
                    let pl = self.scene.get_component::<PointLightComponent>(*entity)?;
                    let world = self.scene.get_world_transform(*entity);
                    let (_, _, pos) = world.to_scale_rotation_translation();
                    Some((pos, pl.color, pl.radius))
                })
                .collect();

            for (pos, color, radius) in &point_lights {
                let gizmo_color = Vec4::new(
                    color.x.clamp(0.2, 1.0),
                    color.y.clamp(0.2, 1.0),
                    color.z.clamp(0.2, 1.0),
                    1.0,
                );

                renderer.set_line_width(2.0);

                let segs = 32;
                // XY plane circle.
                for i in 0..segs {
                    let a0 = (i as f32 / segs as f32) * std::f32::consts::TAU;
                    let a1 = ((i + 1) as f32 / segs as f32) * std::f32::consts::TAU;
                    let p0 = *pos + Vec3::new(a0.cos() * radius, a0.sin() * radius, 0.0);
                    let p1 = *pos + Vec3::new(a1.cos() * radius, a1.sin() * radius, 0.0);
                    renderer.draw_line(p0, p1, gizmo_color, -1);
                }
                // XZ plane circle.
                for i in 0..segs {
                    let a0 = (i as f32 / segs as f32) * std::f32::consts::TAU;
                    let a1 = ((i + 1) as f32 / segs as f32) * std::f32::consts::TAU;
                    let p0 = *pos + Vec3::new(a0.cos() * radius, 0.0, a0.sin() * radius);
                    let p1 = *pos + Vec3::new(a1.cos() * radius, 0.0, a1.sin() * radius);
                    renderer.draw_line(p0, p1, gizmo_color, -1);
                }
                // YZ plane circle.
                for i in 0..segs {
                    let a0 = (i as f32 / segs as f32) * std::f32::consts::TAU;
                    let a1 = ((i + 1) as f32 / segs as f32) * std::f32::consts::TAU;
                    let p0 = *pos + Vec3::new(0.0, a0.cos() * radius, a0.sin() * radius);
                    let p1 = *pos + Vec3::new(0.0, a1.cos() * radius, a1.sin() * radius);
                    renderer.draw_line(p0, p1, gizmo_color, -1);
                }

                // Small cross at center.
                let cs = 0.15;
                renderer.draw_line(*pos - Vec3::X * cs, *pos + Vec3::X * cs, gizmo_color, -1);
                renderer.draw_line(*pos - Vec3::Y * cs, *pos + Vec3::Y * cs, gizmo_color, -1);
                renderer.draw_line(*pos - Vec3::Z * cs, *pos + Vec3::Z * cs, gizmo_color, -1);
            }

            renderer.set_line_width(prev_lw);
        }

        // Selected entity outlines.
        for selected in self.selection.iter() {
            // Light entities have their own gizmo visuals — skip the generic outline.
            let is_light = self
                .scene
                .has_component::<DirectionalLightComponent>(selected)
                || self.scene.has_component::<PointLightComponent>(selected)
                || self.scene.has_component::<AmbientLightComponent>(selected);
            if is_light {
                continue;
            }
            if let Some(transform) = self.scene.get_component::<TransformComponent>(selected) {
                let outline_color = Vec4::new(1.0, 0.5, 0.0, 1.0);
                if let Some(mesh) = self.scene.get_component::<MeshRendererComponent>(selected) {
                    // 3D mesh: wireframe box outline matching mesh bounds.
                    let (bmin, bmax) = if let Some(b) = mesh.local_bounds {
                        b
                    } else if let MeshSource::Primitive(p) = &mesh.mesh_source {
                        p.local_bounds()
                    } else {
                        (Vec3::splat(-0.5), Vec3::splat(0.5))
                    };
                    let world = self.scene.get_world_transform(selected);
                    renderer.draw_box_outline(&world, bmin, bmax, outline_color, -1);
                } else {
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
        }

        // Tilemap paint cursor highlight.
        if self.tilemap_paint.is_active() && self.playback.scene_state == SceneState::Edit {
            if let Some(entity) = self.selection.single() {
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

/// Texture file extensions eligible for hot-reload.
const TEXTURE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];

/// Create a file watcher that monitors the assets directory for texture
/// changes (create/modify) and sends the relative path through `tx`.
fn create_asset_watcher(
    assets_dir: &std::path::Path,
    tx: std::sync::mpsc::Sender<String>,
) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    if !assets_dir.is_dir() {
        info!(
            "Assets directory '{}' not found — texture hot-reload disabled",
            assets_dir.display()
        );
        return None;
    }

    let assets_dir_owned = assets_dir.to_path_buf();
    let assets_dir_for_closure = assets_dir_owned.clone();
    match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            if !matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            ) {
                return;
            }
            for path in &event.paths {
                let is_texture = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| {
                        let lower = ext.to_lowercase();
                        TEXTURE_EXTENSIONS.iter().any(|t| *t == lower)
                    });
                if !is_texture {
                    continue;
                }
                // Convert absolute path to relative (forward slashes).
                if let Ok(rel) = path.strip_prefix(&assets_dir_for_closure) {
                    let relative = rel.to_string_lossy().replace('\\', "/");
                    let _ = tx.send(relative);
                }
            }
        }
    }) {
        Ok(mut watcher) => {
            if let Err(e) = watcher.watch(&assets_dir_owned, RecursiveMode::Recursive) {
                warn!("Failed to watch assets directory for textures: {e}");
                None
            } else {
                info!(
                    "Watching {} for texture changes (hot-reload)",
                    assets_dir_owned.display()
                );
                Some(watcher)
            }
        }
        Err(e) => {
            warn!("Failed to create asset file watcher: {e}");
            None
        }
    }
}

fn main() {
    run::<GGEditor>();
}
