use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AssetHandle, AssetMetadata, AssetType};
use crate::error::EngineResult;
use crate::uuid::Uuid;

/// Maps asset handles (UUIDs) to asset metadata (file path + type).
///
/// Persisted to/from a `.ggregistry` YAML file in the project's assets directory.
pub struct AssetRegistry {
    assets: HashMap<AssetHandle, AssetMetadata>,
    /// Reverse index: normalized file path → asset handle for O(1) lookup.
    path_index: HashMap<String, AssetHandle>,
    /// Forward dependency map: scene/prefab handle → set of asset handles it references.
    dependencies: HashMap<AssetHandle, HashSet<AssetHandle>>,
    /// Reverse dependency map: asset handle → set of scene/prefab handles that reference it.
    dependents: HashMap<AssetHandle, HashSet<AssetHandle>>,
}

// -- Serde intermediate types ------------------------------------------------

#[derive(Serialize, Deserialize)]
struct RegistryFileData {
    #[serde(rename = "Assets")]
    assets: Vec<RegistryEntryData>,
}

#[derive(Serialize, Deserialize)]
struct RegistryEntryData {
    #[serde(rename = "Handle")]
    handle: u64,
    #[serde(rename = "FilePath")]
    file_path: String,
    #[serde(rename = "Type")]
    asset_type: String,
}

// -- Implementation ----------------------------------------------------------

impl AssetRegistry {
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
            path_index: HashMap::new(),
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    pub fn insert(&mut self, handle: AssetHandle, metadata: AssetMetadata) {
        let normalized_path = metadata.file_path.replace('\\', "/");
        // Remove stale path_index entry if this handle already exists with a different path.
        if let Some(old_meta) = self.assets.get(&handle) {
            if old_meta.file_path != normalized_path {
                self.path_index.remove(&old_meta.file_path);
            }
        }
        self.path_index.insert(normalized_path.clone(), handle);
        self.assets.insert(
            handle,
            AssetMetadata {
                file_path: normalized_path,
                asset_type: metadata.asset_type,
            },
        );
    }

    pub fn get(&self, handle: &AssetHandle) -> Option<&AssetMetadata> {
        self.assets.get(handle)
    }

    pub fn remove(&mut self, handle: &AssetHandle) -> Option<AssetMetadata> {
        if let Some(metadata) = self.assets.remove(handle) {
            let normalized_path = metadata.file_path.replace('\\', "/");
            self.path_index.remove(&normalized_path);

            // Clean up forward dependencies (this asset as a source).
            if let Some(deps) = self.dependencies.remove(handle) {
                for dep in &deps {
                    if let Some(rev) = self.dependents.get_mut(dep) {
                        rev.remove(handle);
                        if rev.is_empty() {
                            self.dependents.remove(dep);
                        }
                    }
                }
            }

            // Clean up reverse dependencies (this asset as a dependency).
            if let Some(sources) = self.dependents.remove(handle) {
                for src in &sources {
                    if let Some(fwd) = self.dependencies.get_mut(src) {
                        fwd.remove(handle);
                        if fwd.is_empty() {
                            self.dependencies.remove(src);
                        }
                    }
                }
            }

            Some(metadata)
        } else {
            None
        }
    }

    pub fn contains(&self, handle: &AssetHandle) -> bool {
        self.assets.contains_key(handle)
    }

    /// Find the handle for an asset by its relative file path.
    /// Normalizes backslashes to forward slashes for cross-platform consistency.
    pub fn find_by_path(&self, path: &str) -> Option<AssetHandle> {
        let normalized = path.replace('\\', "/");
        self.path_index.get(&normalized).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&AssetHandle, &AssetMetadata)> {
        self.assets.iter()
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    // -- Dependency tracking --------------------------------------------------

    /// Set the dependencies for a scene/prefab asset.
    ///
    /// Replaces any previous dependency set and updates the reverse index.
    pub fn set_dependencies(&mut self, source: AssetHandle, deps: HashSet<AssetHandle>) {
        // Remove old reverse entries.
        if let Some(old_deps) = self.dependencies.remove(&source) {
            for dep in &old_deps {
                if let Some(rev) = self.dependents.get_mut(dep) {
                    rev.remove(&source);
                    if rev.is_empty() {
                        self.dependents.remove(dep);
                    }
                }
            }
        }

        // Insert new reverse entries.
        for dep in &deps {
            self.dependents.entry(*dep).or_default().insert(source);
        }

        if !deps.is_empty() {
            self.dependencies.insert(source, deps);
        }
    }

    /// Get assets that the given scene/prefab depends on.
    pub fn get_dependencies(&self, source: &AssetHandle) -> Option<&HashSet<AssetHandle>> {
        self.dependencies.get(source)
    }

    /// Get scenes/prefabs that reference the given asset.
    pub fn get_dependents(&self, asset: &AssetHandle) -> Option<&HashSet<AssetHandle>> {
        self.dependents.get(asset)
    }

    /// Scan a scene/prefab YAML string to extract referenced asset handles.
    ///
    /// Looks for `TextureHandle`, `AudioHandle`, `AlbedoTexture`, and `MeshAsset`
    /// fields with non-zero u64 values.
    pub fn scan_scene_dependencies(yaml: &str) -> HashSet<AssetHandle> {
        let mut deps = HashSet::new();
        for line in yaml.lines() {
            let trimmed = line.trim();
            let handle_raw = if let Some(rest) = trimmed.strip_prefix("TextureHandle:") {
                rest.trim().parse::<u64>().ok()
            } else if let Some(rest) = trimmed.strip_prefix("AudioHandle:") {
                rest.trim().parse::<u64>().ok()
            } else if let Some(rest) = trimmed.strip_prefix("AlbedoTexture:") {
                rest.trim().parse::<u64>().ok()
            } else if let Some(rest) = trimmed.strip_prefix("MeshAsset:") {
                rest.trim().parse::<u64>().ok()
            } else {
                None
            };
            if let Some(raw) = handle_raw {
                if raw != 0 {
                    deps.insert(Uuid::from_raw(raw));
                }
            }
        }
        deps
    }

    /// Rebuild all dependency information by scanning scene/prefab files.
    ///
    /// Reads each registered scene and prefab from the given asset directory,
    /// parses for asset references, and populates the dependency maps.
    pub fn rebuild_dependencies(&mut self, asset_directory: &Path) {
        self.dependencies.clear();
        self.dependents.clear();

        let scene_handles: Vec<(AssetHandle, String)> = self
            .assets
            .iter()
            .filter(|(_, meta)| matches!(meta.asset_type, AssetType::Scene | AssetType::Prefab))
            .map(|(h, meta)| (*h, meta.file_path.clone()))
            .collect();

        for (handle, file_path) in scene_handles {
            let abs_path = asset_directory.join(&file_path);
            if let Ok(yaml) = fs::read_to_string(&abs_path) {
                let deps = Self::scan_scene_dependencies(&yaml);
                if !deps.is_empty() {
                    log::debug!("Asset '{}' depends on {} assets", file_path, deps.len());
                    self.set_dependencies(handle, deps);
                }
            }
        }

        log::info!(
            "Rebuilt asset dependencies: {} sources, {} referenced assets",
            self.dependencies.len(),
            self.dependents.len()
        );
    }

    /// Load the registry from a `.ggregistry` YAML file.
    pub fn load(file_path: &Path) -> EngineResult<Self> {
        let contents = fs::read_to_string(file_path)?;
        let file_data: RegistryFileData = serde_yaml_ng::from_str(&contents)?;

        let mut registry = Self::new();
        for entry in &file_data.assets {
            let handle = Uuid::from_raw(entry.handle);
            let asset_type = AssetType::parse_str(&entry.asset_type);
            registry.insert(
                handle,
                AssetMetadata {
                    file_path: entry.file_path.clone(),
                    asset_type,
                },
            );
        }

        log::info!(
            "Loaded asset registry from '{}' ({} assets)",
            file_path.display(),
            registry.len()
        );
        Ok(registry)
    }

    /// Save the registry to a `.ggregistry` YAML file.
    pub fn save(&self, file_path: &Path) -> EngineResult<()> {
        let mut entries: Vec<RegistryEntryData> = self
            .assets
            .iter()
            .map(|(handle, meta)| RegistryEntryData {
                handle: handle.raw(),
                file_path: meta.file_path.clone(),
                asset_type: meta.asset_type.as_str().to_string(),
            })
            .collect();

        // Sort by file path for stable output.
        entries.sort_by(|a, b| a.file_path.cmp(&b.file_path));

        let file_data = RegistryFileData { assets: entries };

        let yaml = serde_yaml_ng::to_string(&file_data)?;
        crate::platform_utils::atomic_write(file_path, &yaml)?;

        log::info!("Asset registry saved to '{}'", file_path.display());
        Ok(())
    }
}

impl Default for AssetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{AssetMetadata, AssetType};
    use crate::uuid::Uuid;
    use std::io::Write;

    fn make_metadata(path: &str, asset_type: AssetType) -> AssetMetadata {
        AssetMetadata {
            file_path: path.to_string(),
            asset_type,
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = AssetRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn insert_and_get() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        let meta = make_metadata("textures/player.png", AssetType::Texture2D);
        reg.insert(handle, meta);

        assert_eq!(reg.len(), 1);
        assert!(!reg.is_empty());

        let got = reg.get(&handle).unwrap();
        assert_eq!(got.file_path, "textures/player.png");
        assert_eq!(got.asset_type, AssetType::Texture2D);
    }

    #[test]
    fn contains_returns_true_for_inserted() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(handle, make_metadata("test.png", AssetType::Texture2D));
        assert!(reg.contains(&handle));
    }

    #[test]
    fn contains_returns_false_for_unknown() {
        let reg = AssetRegistry::new();
        let handle = Uuid::new();
        assert!(!reg.contains(&handle));
    }

    #[test]
    fn find_by_path() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(
            handle,
            make_metadata("scenes/main.ggscene", AssetType::Scene),
        );

        assert_eq!(reg.find_by_path("scenes/main.ggscene"), Some(handle));
        assert_eq!(reg.find_by_path("scenes/nonexistent.ggscene"), None);
    }

    #[test]
    fn find_by_path_normalizes_backslashes() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(
            handle,
            make_metadata("textures\\sprites\\player.png", AssetType::Texture2D),
        );

        // Stored file_path should be normalized to forward slashes.
        assert_eq!(
            reg.get(&handle).unwrap().file_path,
            "textures/sprites/player.png"
        );

        // Lookup with forward slashes should work.
        assert_eq!(
            reg.find_by_path("textures/sprites/player.png"),
            Some(handle)
        );
        // Lookup with backslashes should also work.
        assert_eq!(
            reg.find_by_path("textures\\sprites\\player.png"),
            Some(handle)
        );
    }

    #[test]
    fn remove_asset() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(handle, make_metadata("audio/bgm.ogg", AssetType::Audio));
        assert_eq!(reg.len(), 1);

        let removed = reg.remove(&handle);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().file_path, "audio/bgm.ogg");
        assert_eq!(reg.len(), 0);
        assert!(!reg.contains(&handle));
        assert_eq!(reg.find_by_path("audio/bgm.ogg"), None);
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        assert!(reg.remove(&handle).is_none());
    }

    #[test]
    fn insert_overwrites_existing() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(handle, make_metadata("old.png", AssetType::Texture2D));
        reg.insert(handle, make_metadata("new.png", AssetType::Texture2D));

        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(&handle).unwrap().file_path, "new.png");
        // Old path should no longer resolve to this handle.
        assert_eq!(reg.find_by_path("old.png"), None);
        // New path should resolve correctly.
        assert_eq!(reg.find_by_path("new.png"), Some(handle));
    }

    #[test]
    fn iter_yields_all_assets() {
        let mut reg = AssetRegistry::new();
        let h1 = Uuid::new();
        let h2 = Uuid::new();
        reg.insert(h1, make_metadata("a.png", AssetType::Texture2D));
        reg.insert(h2, make_metadata("b.ogg", AssetType::Audio));

        let collected: HashMap<_, _> = reg.iter().map(|(h, m)| (*h, m.file_path.clone())).collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[&h1], "a.png");
        assert_eq!(collected[&h2], "b.ogg");
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut reg = AssetRegistry::new();
        let h1 = Uuid::from_raw(1001);
        let h2 = Uuid::from_raw(2002);
        reg.insert(h1, make_metadata("textures/hero.png", AssetType::Texture2D));
        reg.insert(h2, make_metadata("scenes/level1.ggscene", AssetType::Scene));

        let dir = std::env::temp_dir().join("gg_asset_test_round_trip");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("TestRegistry.ggregistry");

        reg.save(&file_path).expect("save failed");

        let loaded = AssetRegistry::load(&file_path).expect("Failed to load registry");
        assert_eq!(loaded.len(), 2);

        let m1 = loaded.get(&h1).expect("h1 missing");
        assert_eq!(m1.file_path, "textures/hero.png");
        assert_eq!(m1.asset_type, AssetType::Texture2D);

        let m2 = loaded.get(&h2).expect("h2 missing");
        assert_eq!(m2.file_path, "scenes/level1.ggscene");
        assert_eq!(m2.asset_type, AssetType::Scene);

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_nonexistent_file_returns_err() {
        let result = AssetRegistry::load(Path::new("nonexistent_dir/fake.ggregistry"));
        assert!(result.is_err());
    }

    #[test]
    fn load_malformed_yaml_returns_err() {
        let dir = std::env::temp_dir().join("gg_asset_test_malformed");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("Bad.ggregistry");

        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"this is not valid yaml: [[[{{{").unwrap();

        let result = AssetRegistry::load(&file_path);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_produces_stable_sorted_output() {
        let mut reg = AssetRegistry::new();
        reg.insert(
            Uuid::from_raw(999),
            make_metadata("z_last.png", AssetType::Texture2D),
        );
        reg.insert(
            Uuid::from_raw(111),
            make_metadata("a_first.png", AssetType::Texture2D),
        );

        let dir = std::env::temp_dir().join("gg_asset_test_sorted");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("Sorted.ggregistry");

        reg.save(&file_path).unwrap();
        let contents = std::fs::read_to_string(&file_path).unwrap();

        // a_first.png should appear before z_last.png in the output.
        let pos_a = contents.find("a_first.png").unwrap();
        let pos_z = contents.find("z_last.png").unwrap();
        assert!(
            pos_a < pos_z,
            "Registry output should be sorted by file path"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_scene_dependencies_extracts_handles() {
        let yaml = r#"
Version: 2
Scene: test
Entities:
- Entity: 100
  SpriteRendererComponent:
    TextureHandle: 5001
  AudioSourceComponent:
    AudioHandle: 6001
  MeshRendererComponent:
    AlbedoTexture: 7001
    MeshAsset: 8001
"#;
        let deps = AssetRegistry::scan_scene_dependencies(yaml);
        assert_eq!(deps.len(), 4);
        assert!(deps.contains(&Uuid::from_raw(5001)));
        assert!(deps.contains(&Uuid::from_raw(6001)));
        assert!(deps.contains(&Uuid::from_raw(7001)));
        assert!(deps.contains(&Uuid::from_raw(8001)));
    }

    #[test]
    fn scan_scene_dependencies_skips_zero_handles() {
        let yaml = r#"
SpriteRendererComponent:
  TextureHandle: 0
AudioSourceComponent:
  AudioHandle: 0
"#;
        let deps = AssetRegistry::scan_scene_dependencies(yaml);
        assert!(deps.is_empty());
    }

    #[test]
    fn dependency_tracking_set_and_query() {
        let mut reg = AssetRegistry::new();
        let scene_h = Uuid::from_raw(100);
        let tex_h = Uuid::from_raw(200);
        let audio_h = Uuid::from_raw(300);

        reg.insert(
            scene_h,
            make_metadata("scenes/test.ggscene", AssetType::Scene),
        );
        reg.insert(tex_h, make_metadata("textures/a.png", AssetType::Texture2D));
        reg.insert(audio_h, make_metadata("audio/b.ogg", AssetType::Audio));

        let mut deps = std::collections::HashSet::new();
        deps.insert(tex_h);
        deps.insert(audio_h);
        reg.set_dependencies(scene_h, deps);

        // Forward lookup.
        let fwd = reg.get_dependencies(&scene_h).unwrap();
        assert_eq!(fwd.len(), 2);
        assert!(fwd.contains(&tex_h));
        assert!(fwd.contains(&audio_h));

        // Reverse lookup.
        let rev_tex = reg.get_dependents(&tex_h).unwrap();
        assert!(rev_tex.contains(&scene_h));
        let rev_audio = reg.get_dependents(&audio_h).unwrap();
        assert!(rev_audio.contains(&scene_h));
    }

    #[test]
    fn dependency_cleanup_on_remove() {
        let mut reg = AssetRegistry::new();
        let scene_h = Uuid::from_raw(100);
        let tex_h = Uuid::from_raw(200);

        reg.insert(
            scene_h,
            make_metadata("scenes/test.ggscene", AssetType::Scene),
        );
        reg.insert(tex_h, make_metadata("textures/a.png", AssetType::Texture2D));

        let mut deps = std::collections::HashSet::new();
        deps.insert(tex_h);
        reg.set_dependencies(scene_h, deps);

        // Remove the scene — dependency entries should be cleaned up.
        reg.remove(&scene_h);
        assert!(reg.get_dependencies(&scene_h).is_none());
        assert!(reg.get_dependents(&tex_h).is_none());
    }

    #[test]
    fn dependency_replace_updates_reverse_index() {
        let mut reg = AssetRegistry::new();
        let scene_h = Uuid::from_raw(100);
        let tex_a = Uuid::from_raw(200);
        let tex_b = Uuid::from_raw(300);

        reg.insert(
            scene_h,
            make_metadata("scenes/test.ggscene", AssetType::Scene),
        );
        reg.insert(tex_a, make_metadata("a.png", AssetType::Texture2D));
        reg.insert(tex_b, make_metadata("b.png", AssetType::Texture2D));

        // Initially depends on tex_a.
        let mut deps1 = std::collections::HashSet::new();
        deps1.insert(tex_a);
        reg.set_dependencies(scene_h, deps1);
        assert!(reg.get_dependents(&tex_a).is_some());

        // Replace with tex_b only.
        let mut deps2 = std::collections::HashSet::new();
        deps2.insert(tex_b);
        reg.set_dependencies(scene_h, deps2);

        // tex_a should no longer have scene_h as dependent.
        assert!(reg.get_dependents(&tex_a).is_none());
        // tex_b should now have scene_h as dependent.
        assert!(reg.get_dependents(&tex_b).unwrap().contains(&scene_h));
    }
}
