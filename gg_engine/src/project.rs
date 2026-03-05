use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Serialization data types (intermediate representation)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ProjectData {
    #[serde(rename = "Project")]
    config: ProjectConfigData,
}

#[derive(Serialize, Deserialize)]
struct ProjectConfigData {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "AssetDirectory")]
    asset_directory: String,
    #[serde(rename = "ScriptModulePath")]
    script_module_path: String,
    #[serde(rename = "StartScene")]
    start_scene: String,
}

// ---------------------------------------------------------------------------
// ProjectConfig
// ---------------------------------------------------------------------------

pub struct ProjectConfig {
    pub name: String,
    pub asset_directory: String,
    pub script_module_path: String,
    pub start_scene: String,
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

pub struct Project {
    config: ProjectConfig,
    project_directory: PathBuf,
    project_file_path: String,
}

impl Project {
    /// Load an existing project from a `.ggproject` YAML file.
    pub fn load(file_path: &str) -> Option<Project> {
        let contents = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to read project file '{}': {}", file_path, e);
                return None;
            }
        };

        let data: ProjectData = match serde_yml::from_str(&contents) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to parse project file '{}': {}", file_path, e);
                return None;
            }
        };

        let project_directory = Path::new(file_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        log::info!(
            "Loaded project '{}' from '{}'",
            data.config.name,
            file_path
        );

        Some(Project {
            config: ProjectConfig {
                name: data.config.name,
                asset_directory: data.config.asset_directory,
                script_module_path: data.config.script_module_path,
                start_scene: data.config.start_scene,
            },
            project_directory,
            project_file_path: file_path.to_string(),
        })
    }

    /// Create a new project with default settings and save it.
    pub fn new(file_path: &str, name: &str) -> Option<Project> {
        let project_directory = Path::new(file_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let project = Project {
            config: ProjectConfig {
                name: name.to_string(),
                asset_directory: "assets".to_string(),
                script_module_path: "assets/scripts".to_string(),
                start_scene: "scenes/new.ggscene".to_string(),
            },
            project_directory,
            project_file_path: file_path.to_string(),
        };

        if project.save() {
            Some(project)
        } else {
            None
        }
    }

    /// Serialize the project to its YAML file.
    pub fn save(&self) -> bool {
        let data = ProjectData {
            config: ProjectConfigData {
                name: self.config.name.clone(),
                asset_directory: self.config.asset_directory.clone(),
                script_module_path: self.config.script_module_path.clone(),
                start_scene: self.config.start_scene.clone(),
            },
        };

        match serde_yml::to_string(&data) {
            Ok(yaml) => {
                if let Err(e) = crate::platform_utils::atomic_write(&self.project_file_path, &yaml) {
                    log::error!(
                        "Failed to write project file '{}': {}",
                        self.project_file_path,
                        e
                    );
                    false
                } else {
                    log::info!("Project saved to '{}'", self.project_file_path);
                    true
                }
            }
            Err(e) => {
                log::error!("Failed to serialize project: {}", e);
                false
            }
        }
    }

    // -- Getters --------------------------------------------------------------

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn project_directory(&self) -> &Path {
        &self.project_directory
    }

    pub fn config(&self) -> &ProjectConfig {
        &self.config
    }

    pub fn project_file_path(&self) -> &str {
        &self.project_file_path
    }

    // -- Path helpers ---------------------------------------------------------

    /// Absolute path to the asset directory (`project_dir / asset_directory`).
    pub fn asset_directory_path(&self) -> PathBuf {
        self.project_directory.join(&self.config.asset_directory)
    }

    /// Absolute path to the script module directory (`project_dir / script_module_path`).
    pub fn script_module_path(&self) -> PathBuf {
        self.project_directory.join(&self.config.script_module_path)
    }

    /// Resolve a relative asset path to an absolute path.
    pub fn get_asset_path(&self, relative: &str) -> PathBuf {
        self.asset_directory_path().join(relative)
    }

    /// Absolute path to the start scene.
    pub fn start_scene_path(&self) -> PathBuf {
        self.get_asset_path(&self.config.start_scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_project_serialize() {
        let dir = std::env::temp_dir();
        let file_path = dir.join("test_project.ggproject");
        let file_str = file_path.to_string_lossy().to_string();

        // Create and save.
        let project = Project::new(&file_str, "TestProject").expect("Failed to create project");
        assert_eq!(project.name(), "TestProject");
        assert_eq!(project.config().asset_directory, "assets");
        assert_eq!(project.config().start_scene, "scenes/new.ggscene");

        // Load it back.
        let loaded = Project::load(&file_str).expect("Failed to load project");
        assert_eq!(loaded.name(), "TestProject");
        assert_eq!(loaded.config().asset_directory, "assets");
        assert_eq!(loaded.config().script_module_path, "assets/scripts");
        assert_eq!(loaded.config().start_scene, "scenes/new.ggscene");
        assert_eq!(loaded.project_directory(), dir.as_path());

        // Path helpers.
        assert_eq!(loaded.asset_directory_path(), dir.join("assets"));
        assert_eq!(loaded.script_module_path(), dir.join("assets/scripts"));
        assert_eq!(
            loaded.start_scene_path(),
            dir.join("assets").join("scenes/new.ggscene")
        );
        assert_eq!(
            loaded.get_asset_path("textures/test.png"),
            dir.join("assets").join("textures/test.png")
        );

        // Clean up.
        let _ = std::fs::remove_file(&file_path);
    }
}
