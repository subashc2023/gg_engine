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
        }
    }

    pub fn insert(&mut self, handle: AssetHandle, metadata: AssetMetadata) {
        self.assets.insert(handle, metadata);
    }

    pub fn get(&self, handle: &AssetHandle) -> Option<&AssetMetadata> {
        self.assets.get(handle)
    }

    pub fn remove(&mut self, handle: &AssetHandle) -> Option<AssetMetadata> {
        self.assets.remove(handle)
    }

    pub fn contains(&self, handle: &AssetHandle) -> bool {
        self.assets.contains_key(handle)
    }

    /// Find the handle for an asset by its relative file path.
    pub fn find_by_path(&self, path: &str) -> Option<AssetHandle> {
        self.assets
            .iter()
            .find(|(_, meta)| meta.file_path == path)
            .map(|(handle, _)| *handle)
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

        let file_data: RegistryFileData = match serde_yaml::from_str(&contents) {
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

        match serde_yaml::to_string(&file_data) {
            Ok(yaml) => {
                if let Err(e) = fs::write(file_path, &yaml) {
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
