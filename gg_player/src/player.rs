use std::path::PathBuf;

use gg_engine::prelude::*;

pub struct GGPlayer {
    project_name: String,
    scene: Scene,
    window_width: u32,
    window_height: u32,
    textures_loaded: bool,
    runtime_started: bool,
    present_mode: PresentMode,
}

impl Application for GGPlayer {
    fn new(_layers: &mut LayerStack) -> Self {
        let project_path = find_project_path().unwrap_or_else(|| {
            panic!(
                "No .ggproject file found. Pass a path as a CLI argument \
                 or place the player executable next to a .ggproject file."
            );
        });

        let project = Project::load(&project_path)
            .unwrap_or_else(|| panic!("Failed to load project: {}", project_path));

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
            panic!("Start scene not found: {}", path_str);
        }

        let mut scene = Scene::new();
        if !SceneSerializer::deserialize(&mut scene, &path_str) {
            panic!("Failed to deserialize scene: {}", path_str);
        }

        info!("GGPlayer: loaded project '{}', scene '{}'", project_name, path_str);

        GGPlayer {
            project_name,
            scene,
            window_width: 1280,
            window_height: 720,
            textures_loaded: false,
            runtime_started: false,
            present_mode: PresentMode::Mailbox,
        }
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: self.project_name.clone(),
            width: 1280,
            height: 720,
            decorations: true,
        }
    }

    fn present_mode(&self) -> PresentMode {
        self.present_mode
    }

    fn block_events(&self) -> bool {
        false
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

        self.scene.on_update_physics(dt, Some(input));

        #[cfg(feature = "lua-scripting")]
        self.scene.on_update_lua_scripts(dt, input);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("GGPlayer::on_render");
        // First-frame initialization: load textures and start runtime.
        if !self.textures_loaded {
            self.scene
                .on_viewport_resize(self.window_width, self.window_height);
            self.scene.load_textures(renderer);
            self.scene.load_fonts(renderer);
            self.textures_loaded = true;
        }

        if !self.runtime_started && self.textures_loaded {
            self.scene.on_runtime_start();
            self.runtime_started = true;
        }

        // Render through the scene's primary ECS camera.
        self.scene.on_update_runtime(renderer);
    }
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

/// Find the path to a `.ggproject` file.
///
/// 1. CLI argument ending in `.ggproject`
/// 2. Auto-detect in the directory containing the executable
fn find_project_path() -> Option<String> {
    // 1. CLI argument.
    if let Some(arg) = std::env::args().nth(1) {
        if arg.ends_with(".ggproject") {
            let abs = std::fs::canonicalize(&arg)
                .unwrap_or_else(|_| PathBuf::from(&arg));
            return Some(abs.to_string_lossy().to_string());
        }
    }

    // 2. Scan directory next to the executable.
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
