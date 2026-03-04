use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::gizmo::GizmoOperation;

const SETTINGS_FILE_NAME: &str = "editor_settings.yaml";
const MAX_RECENT_PROJECTS: usize = 10;

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct RecentProject {
    pub name: String,
    pub path: String,
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
        serde_yaml::from_str(&contents).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(dir) = Self::settings_dir() else {
            return;
        };
        let Some(path) = Self::settings_path() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(yaml) = serde_yaml::to_string(self) {
            let _ = std::fs::write(&path, yaml);
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
}
