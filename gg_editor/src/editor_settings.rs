use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use gg_engine::ui_theme::EditorTheme;
use gg_engine::MsaaSamples;

use crate::gizmo::GizmoOperation;
use crate::panels::Tab;

const SETTINGS_FILE_NAME: &str = "editor_settings.yaml";
const MAX_RECENT_PROJECTS: usize = 10;

fn default_true() -> bool {
    true
}

fn default_grid_size() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct RecentProject {
    pub name: String,
    pub path: String,
}

/// Persisted editor camera state (focal point, distance, orientation).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct CameraState {
    pub focal_point: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            focal_point: [0.0, 0.0, 0.0],
            distance: 10.0,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

/// Maximum number of camera bookmark slots (0–9).
pub(crate) const MAX_CAMERA_BOOKMARKS: usize = 10;

/// Maximum number of saved layout presets.
pub(crate) const MAX_SAVED_LAYOUTS: usize = 20;

/// A named dock layout preset.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct NamedLayout {
    pub name: String,
    pub dock_state: egui_dock::DockState<Tab>,
}

/// Persisted window size and position.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct WindowState {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub position: Option<(i32, i32)>,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 900,
            position: None,
            maximized: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct EditorSettings {
    pub recent_projects: Vec<RecentProject>,
    #[serde(default = "default_true")]
    pub vsync: bool,
    #[serde(default)]
    pub show_physics_colliders: bool,
    #[serde(default)]
    pub gizmo_operation: GizmoOperation,
    #[serde(default)]
    pub camera_state: CameraState,
    #[serde(default = "default_true")]
    pub show_grid: bool,
    #[serde(default)]
    pub show_xz_grid: bool,
    #[serde(default = "default_grid_size")]
    pub grid_size: f32,
    #[serde(default)]
    pub snap_to_grid: bool,
    #[serde(default)]
    pub window_state: WindowState,
    #[serde(default)]
    pub dock_layout: Option<egui_dock::DockState<Tab>>,
    #[serde(default)]
    pub theme: EditorTheme,
    #[serde(default)]
    pub msaa_samples: MsaaSamples,
    #[serde(default = "default_true")]
    pub show_camera_bounds: bool,
    /// Camera bookmark slots (Ctrl+0–9 to save, 0–9 to recall).
    #[serde(default)]
    pub camera_bookmarks: [Option<CameraState>; MAX_CAMERA_BOOKMARKS],
    /// Saved dock layout presets.
    #[serde(default)]
    pub saved_layouts: Vec<NamedLayout>,
}

impl EditorSettings {
    fn settings_dir() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .ok()
                .map(|s| PathBuf::from(s).join("GGEngine"))
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .ok()
                .map(|s| PathBuf::from(s).join("Library/Application Support/GGEngine"))
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .or_else(|| std::env::var("HOME").ok().map(|h| format!("{}/.config", h)))
                .map(|s| PathBuf::from(s).join("GGEngine"))
        }
    }

    fn settings_path() -> Option<PathBuf> {
        Self::settings_dir().map(|d| d.join(SETTINGS_FILE_NAME))
    }

    pub fn load() -> Self {
        let Some(path) = Self::settings_path() else {
            return Self::default();
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_yaml_ng::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(dir) = Self::settings_dir() else {
            return;
        };
        let Some(path) = Self::settings_path() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(yaml) = serde_yaml_ng::to_string(self) {
            let _ = gg_engine::platform_utils::atomic_write(&path, &yaml);
        }
    }

    pub fn add_recent_project(&mut self, name: &str, path: &str) {
        self.recent_projects.retain(|r| r.path != path);
        self.recent_projects.insert(
            0,
            RecentProject {
                name: name.to_string(),
                path: path.to_string(),
            },
        );
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
        self.save();
    }

    pub fn remove_recent_project(&mut self, path: &str) {
        self.recent_projects.retain(|r| r.path != path);
        self.save();
    }

    pub fn save_layout(&mut self, name: &str, dock_state: &egui_dock::DockState<Tab>) {
        // Overwrite if a layout with the same name already exists.
        self.saved_layouts.retain(|l| l.name != name);
        self.saved_layouts.push(NamedLayout {
            name: name.to_string(),
            dock_state: dock_state.clone(),
        });
        self.saved_layouts.truncate(MAX_SAVED_LAYOUTS);
        self.save();
    }

    pub fn delete_layout(&mut self, name: &str) {
        self.saved_layouts.retain(|l| l.name != name);
        self.save();
    }
}
