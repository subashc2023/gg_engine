use std::path::PathBuf;

use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

struct PlayerConfig {
    project_path: Option<String>,
    width: u32,
    height: u32,
    vsync: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            project_path: None,
            width: 1280,
            height: 720,
            vsync: false, // Mailbox (no vsync) by default
        }
    }
}

/// Parse CLI arguments manually (no external crate).
///
/// Recognised flags:
///   --width N       Override window width  (default 1280)
///   --height N      Override window height (default 720)
///   --vsync         Enable VSync (Fifo present mode)
///   --no-vsync      Disable VSync (Mailbox present mode, default)
///   --help / -h     Print usage and exit
///
/// Any positional argument ending in `.ggproject` is treated as the project
/// path. At most one project path is accepted.
fn parse_args() -> PlayerConfig {
    let mut config = PlayerConfig::default();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1; // skip executable name

    while i < args.len() {
        match args[i].as_str() {
            "--width" => {
                i += 1;
                if i < args.len() {
                    config.width = args[i].parse::<u32>().unwrap_or_else(|_| {
                        eprintln!("Invalid value for --width: '{}'", args[i]);
                        std::process::exit(1);
                    });
                } else {
                    eprintln!("--width requires a value");
                    std::process::exit(1);
                }
            }
            "--height" => {
                i += 1;
                if i < args.len() {
                    config.height = args[i].parse::<u32>().unwrap_or_else(|_| {
                        eprintln!("Invalid value for --height: '{}'", args[i]);
                        std::process::exit(1);
                    });
                } else {
                    eprintln!("--height requires a value");
                    std::process::exit(1);
                }
            }
            "--vsync" => {
                config.vsync = true;
            }
            "--no-vsync" => {
                config.vsync = false;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                if other.starts_with('-') {
                    eprintln!("Unknown flag: {}", other);
                    print_usage();
                    std::process::exit(1);
                }
                // Treat as a positional argument — expect .ggproject path.
                if other.ends_with(".ggproject") {
                    config.project_path = Some(other.to_string());
                } else {
                    eprintln!("Unexpected argument: {}", other);
                    print_usage();
                    std::process::exit(1);
                }
            }
        }
        i += 1;
    }

    config
}

fn print_usage() {
    eprintln!(
        "Usage: gg_player [OPTIONS] [path/to/project.ggproject]\n\
         \n\
         Options:\n\
         \x20 --width N       Window width  (default: 1280)\n\
         \x20 --height N      Window height (default: 720)\n\
         \x20 --vsync         Enable VSync  (Fifo present mode)\n\
         \x20 --no-vsync      Disable VSync (Mailbox present mode, default)\n\
         \x20 --help, -h      Show this help message"
    );
}

/// Embedded splash screen PNG (displayed while assets load).
static SPLASH_PNG: &[u8] = include_bytes!("../splash.png");

pub struct GGPlayer {
    project_name: String,
    scene: Scene,
    asset_manager: Option<EditorAssetManager>,
    window_width: u32,
    window_height: u32,
    /// Set after the first frame kicks off async loading.
    loading_started: bool,
    runtime_started: bool,
    present_mode: PresentMode,
    /// Quit requested by Lua scripts.
    quit_requested: bool,
    /// Shadow quality to apply next frame (needs `&mut Renderer`).
    pending_shadow_quality: Option<i32>,
    /// Scenes awaiting GPU-safe deferred destruction after scene transitions.
    pending_drop_scenes: Vec<Scene>,
    /// Splash screen texture, created on attach, destroyed when runtime starts.
    splash_texture: Option<Texture2D>,
}

impl GGPlayer {
    /// Render the embedded splash image as a fullscreen quad.
    fn render_splash(&self, renderer: &mut Renderer) {
        if let Some(ref tex) = self.splash_texture {
            // Orthographic projection: -1..1 on both axes, Vulkan Y-flip.
            let mut proj = Mat4::orthographic_lh(-1.0, 1.0, -1.0, 1.0, -1.0, 1.0);
            proj.y_axis.y *= -1.0;
            renderer.set_view_projection(proj);

            // Fullscreen quad: 2×2 world units fills the -1..1 viewport.
            let transform = Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
            renderer.draw_textured_quad_transform(&transform, tex, 1.0, Vec4::ONE);
            renderer.flush_all_batches();
        }
    }

    /// Load a new scene, stopping the current runtime and starting the asset
    /// loading pipeline for the replacement.
    fn load_new_scene(&mut self, path: &str) {
        info!("GGPlayer: loading scene '{}'", path);

        // Stop current runtime (physics, scripts, audio).
        self.scene.on_runtime_stop();

        // Deserialize the new scene.
        let mut new_scene = Scene::new();
        if let Err(e) = SceneSerializer::deserialize(&mut new_scene, path) {
            error!("Failed to load scene '{}': {}", path, e);
            // Rollback — restart the current scene.
            self.scene.on_runtime_start();
            return;
        }

        // Carry current settings to the new scene.
        new_scene.set_vsync_enabled(self.present_mode == PresentMode::Fifo);
        new_scene.set_fullscreen_mode(self.scene.fullscreen_mode());
        new_scene.set_shadow_quality_state(self.scene.shadow_quality());
        new_scene.set_gui_scale(self.scene.gui_scale());
        new_scene.set_cursor_mode(self.scene.cursor_mode());
        // Preserve script module search path for the new scene.
        if let Some(search_path) = self.scene.script_module_search_path() {
            new_scene.set_script_module_search_path(search_path.to_path_buf());
        }

        // Swap scenes — old scene goes to deferred destroy.
        let old = std::mem::replace(&mut self.scene, new_scene);
        self.pending_drop_scenes.push(old);

        // Reset loading pipeline.
        self.scene
            .on_viewport_resize(self.window_width, self.window_height);
        self.loading_started = false;
        self.runtime_started = false;

        info!("GGPlayer: scene '{}' queued for loading", path);
    }
}

impl Application for GGPlayer {
    fn new(_layers: &mut LayerStack) -> Self {
        let config = parse_args();

        let project_path = config
            .project_path
            .map(|p| {
                // Resolve to an absolute path when provided via CLI.
                std::fs::canonicalize(&p)
                    .unwrap_or_else(|_| PathBuf::from(&p))
                    .to_string_lossy()
                    .to_string()
            })
            .or_else(find_project_path_auto)
            .unwrap_or_else(|| {
                error_dialog(
                    "GGPlayer — No Project Found",
                    "No .ggproject file found.\n\n\
                     Pass a path as a CLI argument or place the player \
                     executable next to a .ggproject file.",
                );
                std::process::exit(1);
            });

        let project = match Project::load(&project_path) {
            Ok(p) => p,
            Err(e) => {
                error_dialog(
                    "GGPlayer — Project Load Failed",
                    &format!("Failed to load project:\n{}\n{}", project_path, e),
                );
                std::process::exit(1);
            }
        };

        let project_name = project.name().to_string();

        // Set CWD to the project directory so relative asset paths resolve.
        if let Err(e) = std::env::set_current_dir(project.project_directory()) {
            error!(
                "Failed to set CWD to '{}': {}",
                project.project_directory().display(),
                e
            );
        }

        // Deserialize the start scene.
        let start_scene_path = project.start_scene_path();
        let path_str = start_scene_path.to_string_lossy().to_string();

        if !start_scene_path.exists() {
            error_dialog(
                "GGPlayer — Scene Not Found",
                &format!("Start scene not found:\n{}", path_str),
            );
            std::process::exit(1);
        }

        let mut scene = Scene::new();
        if let Err(e) = SceneSerializer::deserialize(&mut scene, &path_str) {
            error_dialog(
                "GGPlayer — Scene Load Failed",
                &format!("Failed to deserialize scene:\n{}\n{}", path_str, e),
            );
            std::process::exit(1);
        }

        // Create asset manager and load registry.
        let assets_root = project.asset_directory_path();
        let mut asset_manager = EditorAssetManager::new(&assets_root);
        asset_manager.load_registry();

        let present_mode = if config.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Mailbox
        };

        // Seed initial settings state on the scene.
        scene.set_vsync_enabled(config.vsync);
        scene.set_cursor_mode(gg_engine::cursor::CursorMode::Confined);
        scene.set_script_module_search_path(project.script_module_path());

        info!(
            "GGPlayer: loaded project '{}', scene '{}', {}x{}, vsync={}",
            project_name, path_str, config.width, config.height, config.vsync
        );

        GGPlayer {
            project_name,
            scene,
            asset_manager: Some(asset_manager),
            window_width: config.width,
            window_height: config.height,
            loading_started: false,
            runtime_started: false,
            present_mode,
            quit_requested: false,
            pending_shadow_quality: None,
            pending_drop_scenes: Vec::new(),
            splash_texture: None,
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        self.splash_texture = renderer.create_texture_from_memory(SPLASH_PNG);
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: self.project_name.clone(),
            width: self.window_width,
            height: self.window_height,
            decorations: true,
            ..Default::default()
        }
    }

    fn present_mode(&self) -> PresentMode {
        self.present_mode
    }

    fn block_events(&self) -> bool {
        false
    }

    fn should_exit(&self) -> bool {
        self.quit_requested
    }

    fn cursor_mode(&self) -> CursorMode {
        self.scene.cursor_mode()
    }

    fn requested_window_size(&self) -> Option<(u32, u32)> {
        self.scene.take_requested_window_size()
    }

    fn requested_fullscreen(&self) -> Option<FullscreenMode> {
        self.scene.take_requested_fullscreen()
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        match event {
            Event::Window(WindowEvent::Resize { width, height }) => {
                if *width > 0 && *height > 0 {
                    self.window_width = *width;
                    self.window_height = *height;
                    self.scene.on_viewport_resize(*width, *height);
                }
            }
            Event::Key(KeyEvent::Pressed {
                key_code: KeyCode::V,
                repeat: false,
            }) => {
                self.present_mode = match self.present_mode {
                    PresentMode::Mailbox => PresentMode::Fifo,
                    _ => PresentMode::Mailbox,
                };
                self.scene
                    .set_vsync_enabled(self.present_mode == PresentMode::Fifo);
                info!("VSync toggled → {}", self.present_mode);
            }
            _ => {}
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        profile_scope!("GGPlayer::on_update");
        if !self.runtime_started {
            return;
        }

        self.scene.on_update_all_physics(dt, Some(input));
        self.scene.on_update_scripts(dt, input);

        #[cfg(feature = "lua-scripting")]
        self.scene.on_update_lua_scripts(dt, input);

        self.scene.on_update_animations(dt.seconds());
        self.scene.update_spatial_audio();

        // --- UI interaction (hit testing + Lua callbacks) ---
        {
            let (mx, my) = input.mouse_position();
            let mouse_world = self.scene.screen_to_world_2d(mx as f32, my as f32);
            let mouse_down = input.is_mouse_button_pressed(MouseButton::Left);
            let just_pressed = input.is_mouse_button_just_pressed(MouseButton::Left);
            let just_released = input.is_mouse_button_just_released(MouseButton::Left);
            let events = self
                .scene
                .update_ui_with_input(mouse_world, mouse_down, just_pressed, just_released);
            if !events.is_empty() {
                self.scene.dispatch_ui_events(&events);
            }
        }

        // --- Poll runtime setting requests from Lua scripts ---

        // Quit.
        if self.scene.take_requested_quit() {
            self.quit_requested = true;
        }

        // VSync.
        if let Some(vsync) = self.scene.take_requested_vsync() {
            self.present_mode = if vsync {
                PresentMode::Fifo
            } else {
                PresentMode::Mailbox
            };
            self.scene.set_vsync_enabled(vsync);
            info!("VSync set via Lua → {}", self.present_mode);
        }

        // Shadow quality.
        if let Some(quality) = self.scene.take_requested_shadow_quality() {
            self.pending_shadow_quality = Some(quality);
            self.scene.set_shadow_quality_state(quality);
            info!("Shadow quality set via Lua → {}", quality);
        }

        // Scene loading (deferred — must be last so current frame finishes).
        if let Some(path) = self.scene.take_requested_load_scene() {
            self.load_new_scene(&path);
        }
    }

    fn on_render_shadows(
        &mut self,
        renderer: &mut Renderer,
        cmd_buf: gg_engine::ash::vk::CommandBuffer,
        current_frame: usize,
    ) {
        if self.runtime_started {
            // Find the primary camera for per-cascade frustum fitting.
            let camera_info = self.scene.primary_camera_info();
            self.scene.render_shadow_pass(
                renderer,
                cmd_buf,
                current_frame,
                0,
                camera_info.as_ref(),
            );
        }
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("GGPlayer::on_render");

        // GPU-safe deferred scene destruction after scene transitions.
        if !self.pending_drop_scenes.is_empty() {
            renderer.wait_gpu_idle();
            self.pending_drop_scenes.clear();
        }

        // Apply pending shadow quality change (needs &mut Renderer).
        if let Some(quality) = self.pending_shadow_quality.take() {
            renderer.set_shadow_quality(quality);
        }

        // First frame: set viewport and kick off async loading.
        if !self.loading_started {
            self.scene
                .on_viewport_resize(self.window_width, self.window_height);
            if let Some(ref mut am) = self.asset_manager {
                self.scene.resolve_audio_handles(am);
                self.scene.resolve_texture_handles_async(am);
                self.scene.load_fonts_async(am);
                self.scene.resolve_mesh_assets(am);
                self.scene.resolve_skinned_mesh_assets(am);
            }
            self.loading_started = true;
        }

        // Per-frame: poll async completions and GPU-upload.
        if let Some(ref mut am) = self.asset_manager {
            am.poll_loaded(renderer);
        }
        renderer.flush_transfers();
        renderer.poll_transfers();

        // Assign newly loaded textures, fonts, and meshes to entities.
        if let Some(ref mut am) = self.asset_manager {
            self.scene.resolve_texture_handles_async(am);
            self.scene.load_fonts_async(am);
            self.scene.resolve_mesh_assets(am);
            self.scene.resolve_skinned_mesh_assets(am);
            self.scene.resolve_environment_map(renderer, am);
        }
        self.scene.resolve_meshes(renderer);
        self.scene.resolve_skinned_meshes(renderer);

        // Start runtime once all pending loads are complete.
        if !self.runtime_started {
            let pending = self
                .asset_manager
                .as_ref()
                .map(|am| am.pending_load_count())
                .unwrap_or(0);
            if pending == 0 {
                self.scene.on_runtime_start();
                self.runtime_started = true;

                // Release splash texture — no longer needed.
                if let Some(tex) = self.splash_texture.take() {
                    renderer.unregister_texture(&tex);
                }
            }
        }

        if self.runtime_started {
            // Render through the scene's primary ECS camera.
            self.scene.on_update_runtime(renderer);
        } else {
            // Show splash screen while assets are loading.
            self.render_splash(renderer);
        }
    }
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

/// Auto-detect a `.ggproject` file in the directory containing the executable.
///
/// This is the fallback when no project path was supplied via CLI arguments.
fn find_project_path_auto() -> Option<String> {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if let Ok(entries) = std::fs::read_dir(exe_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("ggproject") {
                        return Some(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    None
}
