use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{asset_type_from_extension, AssetHandle, AssetMetadata, AssetRegistry, AssetType};
use crate::renderer::{Renderer, Texture2D};
use crate::uuid::Uuid;
use crate::Ref;

/// Loaded asset data. Only textures are cached; scenes are loaded on demand.
pub enum AssetData {
    Texture(Ref<Texture2D>),
}

/// Editor-side asset manager that owns the asset registry and loaded GPU assets.
///
/// Provides methods to import, load, and query assets by handle (UUID).
pub struct EditorAssetManager {
    registry: AssetRegistry,
    loaded_assets: HashMap<AssetHandle, AssetData>,
    asset_directory: PathBuf,
}

const REGISTRY_FILENAME: &str = "AssetRegistry.ggregistry";

impl EditorAssetManager {
    /// Create a new asset manager rooted at the given asset directory.
    pub fn new(asset_directory: impl Into<PathBuf>) -> Self {
        Self {
            registry: AssetRegistry::new(),
            loaded_assets: HashMap::new(),
            asset_directory: asset_directory.into(),
        }
    }

    /// Load the registry from the `AssetRegistry.ggregistry` file in the asset directory.
    pub fn load_registry(&mut self) {
        let registry_path = self.asset_directory.join(REGISTRY_FILENAME);
        if registry_path.exists() {
            if let Some(reg) = AssetRegistry::load(&registry_path) {
                self.registry = reg;
            }
        } else {
            log::info!("No asset registry found at '{}', starting empty", registry_path.display());
        }
    }

    /// Save the registry to the `AssetRegistry.ggregistry` file in the asset directory.
    pub fn save_registry(&self) {
        let registry_path = self.asset_directory.join(REGISTRY_FILENAME);
        self.registry.save(&registry_path);
    }

    /// Import an asset by its path relative to the asset directory.
    ///
    /// Detects the asset type from the file extension, generates a handle,
    /// and registers it. Returns the new handle.
    pub fn import_asset(&mut self, relative_path: &str) -> AssetHandle {
        // Check if already imported.
        if let Some(handle) = self.registry.find_by_path(relative_path) {
            return handle;
        }

        let ext = Path::new(relative_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let asset_type = asset_type_from_extension(ext);

        let handle = Uuid::new();
        self.registry.insert(
            handle,
            AssetMetadata {
                file_path: relative_path.to_string(),
                asset_type,
            },
        );

        log::info!(
            "Imported asset '{}' as {:?} (handle: {})",
            relative_path,
            asset_type,
            handle
        );
        handle
    }

    /// Check if a relative path is already imported in the registry.
    pub fn is_imported(&self, relative_path: &str) -> bool {
        self.registry.find_by_path(relative_path).is_some()
    }

    /// Get the handle for an already-imported path.
    pub fn get_handle_for_path(&self, relative_path: &str) -> Option<AssetHandle> {
        self.registry.find_by_path(relative_path)
    }

    /// Get metadata for an asset handle.
    pub fn get_metadata(&self, handle: &AssetHandle) -> Option<&AssetMetadata> {
        self.registry.get(handle)
    }

    /// Get the absolute path for an asset handle.
    pub fn get_absolute_path(&self, handle: &AssetHandle) -> Option<PathBuf> {
        self.registry
            .get(handle)
            .map(|meta| self.asset_directory.join(&meta.file_path))
    }

    /// Get a loaded asset by handle.
    pub fn get_asset(&self, handle: &AssetHandle) -> Option<&AssetData> {
        self.loaded_assets.get(handle)
    }

    /// Convenience: get a loaded texture by handle.
    pub fn get_texture(&self, handle: &AssetHandle) -> Option<Ref<Texture2D>> {
        match self.loaded_assets.get(handle) {
            Some(AssetData::Texture(tex)) => Some(tex.clone()),
            _ => None,
        }
    }

    /// Load an asset into GPU memory if not already loaded.
    ///
    /// Returns `true` if the asset is now loaded (either freshly or already was).
    pub fn load_asset(&mut self, handle: &AssetHandle, renderer: &Renderer) -> bool {
        if self.loaded_assets.contains_key(handle) {
            return true;
        }

        let metadata = match self.registry.get(handle) {
            Some(m) => m.clone(),
            None => return false,
        };

        match metadata.asset_type {
            AssetType::Texture2D => {
                let abs_path = self.asset_directory.join(&metadata.file_path);
                if abs_path.exists() {
                    let texture = Ref::new(renderer.create_texture_from_file(&abs_path));
                    self.loaded_assets
                        .insert(*handle, AssetData::Texture(texture));
                    true
                } else {
                    log::warn!("Texture file not found: {}", abs_path.display());
                    false
                }
            }
            AssetType::Scene => {
                // Scenes are not cached in the asset manager.
                true
            }
            AssetType::None => false,
        }
    }

    /// Check if a handle exists in the registry.
    pub fn is_valid(&self, handle: &AssetHandle) -> bool {
        self.registry.contains(handle)
    }

    /// Check if an asset is loaded into GPU memory.
    pub fn is_loaded(&self, handle: &AssetHandle) -> bool {
        self.loaded_assets.contains_key(handle)
    }

    /// Get the asset type from registry metadata.
    pub fn get_asset_type(&self, handle: &AssetHandle) -> AssetType {
        self.registry
            .get(handle)
            .map(|m| m.asset_type)
            .unwrap_or(AssetType::None)
    }

    /// Access the underlying registry.
    pub fn registry(&self) -> &AssetRegistry {
        &self.registry
    }

    /// Mutable access to the registry (e.g. for removing assets).
    pub fn registry_mut(&mut self) -> &mut AssetRegistry {
        &mut self.registry
    }

    /// The root asset directory.
    pub fn asset_directory(&self) -> &Path {
        &self.asset_directory
    }
}
