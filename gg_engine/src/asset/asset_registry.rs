use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AssetHandle, AssetMetadata, AssetType};
use crate::uuid::Uuid;

/// Maps asset handles (UUIDs) to asset metadata (file path + type).
///
/// Persisted to/from a `.ggregistry` YAML file in the project's assets directory.
pub struct AssetRegistry {
    assets: HashMap<AssetHandle, AssetMetadata>,
    /// Reverse index: normalized file path → asset handle for O(1) lookup.
    path_index: HashMap<String, AssetHandle>,
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
        }
    }

    pub fn insert(&mut self, handle: AssetHandle, metadata: AssetMetadata) {
        let normalized_path = metadata.file_path.replace('\\', "/");
        self.path_index.insert(normalized_path, handle);
        self.assets.insert(handle, metadata);
    }

    pub fn get(&self, handle: &AssetHandle) -> Option<&AssetMetadata> {
        self.assets.get(handle)
    }

    pub fn remove(&mut self, handle: &AssetHandle) -> Option<AssetMetadata> {
        if let Some(metadata) = self.assets.remove(handle) {
            let normalized_path = metadata.file_path.replace('\\', "/");
            self.path_index.remove(&normalized_path);
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

    /// Load the registry from a `.ggregistry` YAML file.
    pub fn load(file_path: &Path) -> Option<Self> {
        let contents = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to read registry file '{}': {}", file_path.display(), e);
                return None;
            }
        };

        let file_data: RegistryFileData = match serde_yml::from_str(&contents) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to parse registry file '{}': {}", file_path.display(), e);
                return None;
            }
        };

        let mut registry = Self::new();
        for entry in &file_data.assets {
            let handle = Uuid::from_raw(entry.handle);
            let asset_type = AssetType::from_str(&entry.asset_type);
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
        Some(registry)
    }

    /// Save the registry to a `.ggregistry` YAML file.
    pub fn save(&self, file_path: &Path) -> bool {
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

        match serde_yml::to_string(&file_data) {
            Ok(yaml) => {
                if let Err(e) = crate::platform_utils::atomic_write(file_path, &yaml) {
                    log::error!("Failed to write registry file '{}': {}", file_path.display(), e);
                    false
                } else {
                    log::info!("Asset registry saved to '{}'", file_path.display());
                    true
                }
            }
            Err(e) => {
                log::error!("Failed to serialize registry: {}", e);
                false
            }
        }
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
        reg.insert(handle, make_metadata("scenes/main.ggscene", AssetType::Scene));

        assert_eq!(reg.find_by_path("scenes/main.ggscene"), Some(handle));
        assert_eq!(reg.find_by_path("scenes/nonexistent.ggscene"), None);
    }

    #[test]
    fn find_by_path_normalizes_backslashes() {
        let mut reg = AssetRegistry::new();
        let handle = Uuid::new();
        reg.insert(handle, make_metadata("textures\\sprites\\player.png", AssetType::Texture2D));

        // Lookup with forward slashes should work.
        assert_eq!(reg.find_by_path("textures/sprites/player.png"), Some(handle));
        // Lookup with backslashes should also work.
        assert_eq!(reg.find_by_path("textures\\sprites\\player.png"), Some(handle));
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

        assert!(reg.save(&file_path));

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
    fn load_nonexistent_file_returns_none() {
        let result = AssetRegistry::load(Path::new("nonexistent_dir/fake.ggregistry"));
        assert!(result.is_none());
    }

    #[test]
    fn load_malformed_yaml_returns_none() {
        let dir = std::env::temp_dir().join("gg_asset_test_malformed");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("Bad.ggregistry");

        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"this is not valid yaml: [[[{{{").unwrap();

        let result = AssetRegistry::load(&file_path);
        assert!(result.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_produces_stable_sorted_output() {
        let mut reg = AssetRegistry::new();
        reg.insert(Uuid::from_raw(999), make_metadata("z_last.png", AssetType::Texture2D));
        reg.insert(Uuid::from_raw(111), make_metadata("a_first.png", AssetType::Texture2D));

        let dir = std::env::temp_dir().join("gg_asset_test_sorted");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("Sorted.ggregistry");

        assert!(reg.save(&file_path));
        let contents = std::fs::read_to_string(&file_path).unwrap();

        // a_first.png should appear before z_last.png in the output.
        let pos_a = contents.find("a_first.png").unwrap();
        let pos_z = contents.find("z_last.png").unwrap();
        assert!(pos_a < pos_z, "Registry output should be sorted by file path");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
