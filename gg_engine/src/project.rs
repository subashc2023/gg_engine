use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EngineResult;
use crate::input_action::InputActionMap;

/// Current project schema version. Bump this when changing the project file
/// format so that older projects can be migrated automatically.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// Serialization data types (intermediate representation)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ProjectData {
    #[serde(rename = "Project")]
    config: ProjectConfigData,
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

#[derive(Serialize, Deserialize)]
struct ProjectConfigData {
    #[serde(rename = "SchemaVersion", default = "default_schema_version")]
    schema_version: u32,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "AssetDirectory")]
    asset_directory: String,
    #[serde(rename = "ScriptModulePath")]
    script_module_path: String,
    #[serde(rename = "StartScene")]
    start_scene: String,
    #[serde(rename = "InputActions", default)]
    input_actions: InputActionMap,
}

// ---------------------------------------------------------------------------
// ProjectConfig
// ---------------------------------------------------------------------------

pub struct ProjectConfig {
    pub schema_version: u32,
    pub name: String,
    pub asset_directory: String,
    pub script_module_path: String,
    pub start_scene: String,
    pub input_actions: InputActionMap,
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
    pub fn load(file_path: &str) -> EngineResult<Project> {
        let contents = fs::read_to_string(file_path)?;
        let data: ProjectData = serde_yaml_ng::from_str(&contents)?;

        let project_directory = Path::new(file_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let schema_version = data.config.schema_version;
        if schema_version > CURRENT_SCHEMA_VERSION {
            log::warn!(
                "Project '{}' was saved with schema version {} (current: {}). Some data may not load correctly.",
                data.config.name,
                schema_version,
                CURRENT_SCHEMA_VERSION
            );
        }

        // v1 → v2: no migration needed; InputActions defaults to empty via serde.

        let action_count = data.config.input_actions.actions.len();
        log::info!(
            "Loaded project '{}' (schema v{}, {} input actions) from '{}'",
            data.config.name,
            schema_version,
            action_count,
            file_path
        );

        Ok(Project {
            config: ProjectConfig {
                schema_version: schema_version.min(CURRENT_SCHEMA_VERSION),
                name: data.config.name,
                asset_directory: data.config.asset_directory,
                script_module_path: data.config.script_module_path,
                start_scene: data.config.start_scene,
                input_actions: data.config.input_actions,
            },
            project_directory,
            project_file_path: file_path.to_string(),
        })
    }

    /// Create a new project with default settings and save it.
    pub fn new(file_path: &str, name: &str) -> EngineResult<Project> {
        let project_directory = Path::new(file_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let project = Project {
            config: ProjectConfig {
                schema_version: CURRENT_SCHEMA_VERSION,
                name: name.to_string(),
                asset_directory: "assets".to_string(),
                script_module_path: "assets/scripts".to_string(),
                start_scene: "scenes/new.ggscene".to_string(),
                input_actions: InputActionMap::default(),
            },
            project_directory,
            project_file_path: file_path.to_string(),
        };

        project.save()?;
        Ok(project)
    }

    /// Serialize the project to its YAML file.
    pub fn save(&self) -> EngineResult<()> {
        let data = ProjectData {
            config: ProjectConfigData {
                schema_version: CURRENT_SCHEMA_VERSION,
                name: self.config.name.clone(),
                asset_directory: self.config.asset_directory.clone(),
                script_module_path: self.config.script_module_path.clone(),
                start_scene: self.config.start_scene.clone(),
                input_actions: self.config.input_actions.clone(),
            },
        };

        let yaml = serde_yaml_ng::to_string(&data)?;
        crate::platform_utils::atomic_write(&self.project_file_path, &yaml)?;
        log::info!("Project saved to '{}'", self.project_file_path);
        Ok(())
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

    pub fn config_mut(&mut self) -> &mut ProjectConfig {
        &mut self.config
    }

    pub fn project_file_path(&self) -> &str {
        &self.project_file_path
    }

    /// Convenience accessor for the project's input action map.
    pub fn input_actions(&self) -> &InputActionMap {
        &self.config.input_actions
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
        assert!(project.input_actions().actions.is_empty());

        // Load it back.
        let loaded = Project::load(&file_str).expect("Failed to load project");
        assert_eq!(loaded.name(), "TestProject");
        assert_eq!(loaded.config().asset_directory, "assets");
        assert_eq!(loaded.config().script_module_path, "assets/scripts");
        assert_eq!(loaded.config().start_scene, "scenes/new.ggscene");
        assert_eq!(loaded.project_directory(), dir.as_path());
        assert!(loaded.input_actions().actions.is_empty());

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

    #[test]
    fn round_trip_with_input_actions() {
        use crate::events::{KeyCode, MouseButton};
        use crate::input_action::{ActionType, InputAction, InputBinding};

        let dir = std::env::temp_dir();
        let file_path = dir.join("test_project_actions.ggproject");
        let file_str = file_path.to_string_lossy().to_string();

        let mut project = Project::new(&file_str, "ActionTest").expect("Failed to create project");
        project.config_mut().input_actions = InputActionMap {
            actions: vec![
                InputAction {
                    name: "jump".to_string(),
                    action_type: ActionType::Button,
                    bindings: vec![
                        InputBinding::Key(KeyCode::Space),
                        InputBinding::Mouse(MouseButton::Left),
                    ],
                },
                InputAction {
                    name: "move_h".to_string(),
                    action_type: ActionType::Axis,
                    bindings: vec![InputBinding::KeyComposite {
                        negative: KeyCode::A,
                        positive: KeyCode::D,
                    }],
                },
            ],
        };
        project.save().expect("Failed to save");

        let loaded = Project::load(&file_str).expect("Failed to load");
        assert_eq!(loaded.input_actions().actions.len(), 2);
        assert_eq!(loaded.input_actions().actions[0].name, "jump");
        assert_eq!(loaded.input_actions().actions[0].bindings.len(), 2);
        assert_eq!(loaded.input_actions().actions[1].name, "move_h");

        let _ = std::fs::remove_file(&file_path);
    }
}
