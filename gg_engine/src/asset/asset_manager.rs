use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::asset_loader::{AssetLoader, LoadResult};
use super::{asset_type_from_extension, validate_asset_path, AssetHandle, AssetMetadata, AssetRegistry, AssetType};
use crate::renderer::{Renderer, Texture2D, TextureSpecification};
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
    loader: AssetLoader,
    /// Lazily-created magenta/black checkerboard texture used for missing assets.
    fallback_texture: Option<Ref<Texture2D>>,
}

const REGISTRY_FILENAME: &str = "AssetRegistry.ggregistry";

impl EditorAssetManager {
    /// Create a new asset manager rooted at the given asset directory.
    pub fn new(asset_directory: impl Into<PathBuf>) -> Self {
        Self {
            registry: AssetRegistry::new(),
            loaded_assets: HashMap::new(),
            asset_directory: asset_directory.into(),
            loader: AssetLoader::new(),
            fallback_texture: None,
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
        // Normalize to forward slashes for cross-platform consistency.
        let normalized = relative_path.replace('\\', "/");

        // Reject paths that could escape the asset directory.
        if !validate_asset_path(&normalized) {
            log::warn!("Rejected unsafe asset path: '{}'", normalized);
            return Uuid::from_raw(0);
        }

        // Check if already imported.
        if let Some(handle) = self.registry.find_by_path(&normalized) {
            return handle;
        }

        let ext = Path::new(&normalized)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let asset_type = asset_type_from_extension(ext);

        let handle = Uuid::new();
        self.registry.insert(
            handle,
            AssetMetadata {
                file_path: normalized.clone(),
                asset_type,
            },
        );

        log::info!(
            "Imported asset '{}' as {:?} (handle: {})",
            normalized,
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

        if !validate_asset_path(&metadata.file_path) {
            log::warn!("Rejected unsafe asset path in registry: '{}'", metadata.file_path);
            return false;
        }

        match metadata.asset_type {
            AssetType::Texture2D => {
                let abs_path = self.asset_directory.join(&metadata.file_path);
                if abs_path.exists() {
                    if let Some(texture) = renderer.create_texture_from_file(&abs_path) {
                        self.loaded_assets
                            .insert(*handle, AssetData::Texture(Ref::new(texture)));
                        true
                    } else {
                        log::warn!("Failed to load texture '{}', using fallback", abs_path.display());
                        self.store_fallback(*handle, renderer);
                        true
                    }
                } else {
                    log::warn!("Texture file not found '{}', using fallback", abs_path.display());
                    self.store_fallback(*handle, renderer);
                    true
                }
            }
            AssetType::Scene => {
                // Scenes are not cached in the asset manager.
                true
            }
            AssetType::Audio => {
                // Audio files are loaded by kira at playback time.
                // Just verify the file exists.
                let abs_path = self.asset_directory.join(&metadata.file_path);
                if abs_path.exists() {
                    true
                } else {
                    log::warn!("Audio file not found: {}", abs_path.display());
                    false
                }
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

    /// Get or create the fallback texture (magenta/black 4x4 checkerboard).
    /// Used to visually indicate missing or broken asset references.
    fn get_fallback_texture(&mut self, renderer: &Renderer) -> Ref<Texture2D> {
        if let Some(ref tex) = self.fallback_texture {
            return tex.clone();
        }

        // 4x4 checkerboard: magenta (255,0,255) / black (0,0,0)
        let m = [255u8, 0, 255, 255]; // magenta
        let b = [0u8, 0, 0, 255]; // black
        let mut pixels = Vec::with_capacity(4 * 4 * 4);
        for row in 0..4u32 {
            for col in 0..4u32 {
                let cell = if (row + col) % 2 == 0 { &m } else { &b };
                pixels.extend_from_slice(cell);
            }
        }

        let texture = renderer.create_texture_from_rgba8(4, 4, &pixels);
        let tex_ref = Ref::new(texture);
        self.fallback_texture = Some(tex_ref.clone());
        log::info!("Created fallback checkerboard texture for missing assets");
        tex_ref
    }

    /// Store the fallback texture for a handle that failed to load.
    fn store_fallback(&mut self, handle: AssetHandle, renderer: &Renderer) {
        let tex = self.get_fallback_texture(renderer);
        self.loaded_assets.insert(handle, AssetData::Texture(tex));
    }

    // -------------------------------------------------------------------
    // Async loading API (used by editor)
    // -------------------------------------------------------------------

    /// Request async texture loading for an asset handle.
    /// Looks up the path from the registry and enqueues the work.
    pub fn request_load(&mut self, handle: &AssetHandle) {
        if self.loaded_assets.contains_key(handle) {
            return;
        }

        let metadata = match self.registry.get(handle) {
            Some(m) => m.clone(),
            None => return,
        };

        if metadata.asset_type != AssetType::Texture2D {
            return;
        }

        if !validate_asset_path(&metadata.file_path) {
            log::warn!("Rejected unsafe asset path in registry: '{}'", metadata.file_path);
            return;
        }

        let abs_path = self.asset_directory.join(&metadata.file_path);
        if !abs_path.exists() {
            log::warn!("Texture file not found: {}", abs_path.display());
            return;
        }

        self.loader.request_texture(*handle, abs_path, TextureSpecification::default());
    }

    /// Poll completed async loads, perform GPU upload for textures,
    /// and store them in `loaded_assets`. Returns any font results for
    /// the caller to process.
    pub fn poll_loaded(&mut self, renderer: &Renderer) -> Vec<LoadResult> {
        let results = self.loader.poll_results();
        let mut font_results = Vec::new();

        for result in results {
            match result {
                LoadResult::Texture { handle, data } => {
                    match data {
                        Ok(cpu_data) => {
                            let texture = Ref::new(renderer.upload_texture(&cpu_data));
                            self.loaded_assets.insert(handle, AssetData::Texture(texture));
                        }
                        Err(e) => {
                            log::warn!("Async texture load failed: {e}, using fallback");
                            let fallback = self.get_fallback_texture(renderer);
                            self.loaded_assets.insert(handle, AssetData::Texture(fallback));
                        }
                    }
                }
                font_result @ LoadResult::Font { .. } => {
                    font_results.push(font_result);
                }
            }
        }

        font_results
    }

    /// Unload a specific asset from GPU memory.
    pub fn unload_asset(&mut self, handle: &AssetHandle) -> bool {
        self.loaded_assets.remove(handle).is_some()
    }

    /// Unload assets that are only held by the cache (Arc strong_count == 1).
    /// Returns the number of assets evicted.
    pub fn unload_unused(&mut self) -> usize {
        let before = self.loaded_assets.len();
        self.loaded_assets.retain(|_, data| match data {
            AssetData::Texture(tex) => std::sync::Arc::strong_count(tex) > 1,
        });
        before - self.loaded_assets.len()
    }

    /// Unload all cached assets from GPU memory.
    pub fn unload_all(&mut self) {
        self.loaded_assets.clear();
    }

    /// Number of currently loaded assets.
    pub fn loaded_count(&self) -> usize {
        self.loaded_assets.len()
    }

    /// Access the underlying asset loader (e.g., for font loading from editor).
    pub fn loader(&mut self) -> &mut AssetLoader {
        &mut self.loader
    }
}
