use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::asset_loader::{AssetLoader, LoadResult};
use super::{
    asset_type_from_extension, validate_asset_path, AssetHandle, AssetMetadata, AssetRegistry,
    AssetType,
};
use crate::renderer::{Font, GltfSkinData, Mesh, Renderer, Texture2D, TextureSpecification};
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
    /// Cached fonts keyed by file path (shared across entities via Arc).
    loaded_fonts: HashMap<PathBuf, Ref<Font>>,
    /// Loaded mesh CPU data keyed by asset handle. Shared via `Ref<Mesh>` so
    /// multiple entities referencing the same glTF file share one copy.
    loaded_meshes: HashMap<AssetHandle, Ref<Mesh>>,
    /// Loaded skinned mesh data (skeleton + clips + mesh) keyed by asset handle.
    loaded_skinned_meshes: HashMap<AssetHandle, Ref<GltfSkinData>>,
    /// Lazily-created magenta/black checkerboard texture used for missing assets.
    fallback_texture: Option<Ref<Texture2D>>,
    /// Monotonic counter bumped on each asset access, used for LRU eviction.
    access_counter: u64,
    /// Last-access timestamp per loaded asset (for LRU ordering).
    access_times: HashMap<AssetHandle, u64>,
    /// Maximum number of cached textures before LRU eviction kicks in.
    /// 0 means unlimited.
    max_cached_textures: usize,
    /// Per-asset GPU memory size in bytes.
    asset_gpu_bytes: HashMap<AssetHandle, u64>,
    /// Total tracked GPU memory usage in bytes.
    total_gpu_bytes: u64,
    /// GPU memory budget in bytes. 0 means unlimited (eviction by count only).
    gpu_memory_budget: u64,
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
            loaded_fonts: HashMap::new(),
            loaded_meshes: HashMap::new(),
            loaded_skinned_meshes: HashMap::new(),
            fallback_texture: None,
            access_counter: 0,
            access_times: HashMap::new(),
            max_cached_textures: 256,
            asset_gpu_bytes: HashMap::new(),
            total_gpu_bytes: 0,
            gpu_memory_budget: 0,
        }
    }

    /// Load the registry from the `AssetRegistry.ggregistry` file in the asset directory.
    pub fn load_registry(&mut self) {
        let registry_path = self.asset_directory.join(REGISTRY_FILENAME);
        if registry_path.exists() {
            if let Ok(reg) = AssetRegistry::load(&registry_path) {
                self.registry = reg;
            }
        } else {
            log::info!(
                "No asset registry found at '{}', starting empty",
                registry_path.display()
            );
        }
        self.registry.rebuild_dependencies(&self.asset_directory);
    }

    /// Save the registry to the `AssetRegistry.ggregistry` file in the asset directory.
    pub fn save_registry(&self) {
        let registry_path = self.asset_directory.join(REGISTRY_FILENAME);
        if let Err(e) = self.registry.save(&registry_path) {
            log::error!("Failed to save asset registry: {}", e);
        }
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

    /// Return egui handles for all currently-loaded textures.
    /// Used by the editor to register thumbnail textures with egui.
    pub fn loaded_texture_egui_handles(&self) -> Vec<u64> {
        self.loaded_assets
            .values()
            .map(|data| {
                let AssetData::Texture(tex) = data;
                tex.egui_handle()
            })
            .collect()
    }

    /// Convenience: get a loaded texture by handle. Updates LRU access time.
    pub fn get_texture(&mut self, handle: &AssetHandle) -> Option<Ref<Texture2D>> {
        match self.loaded_assets.get(handle) {
            Some(AssetData::Texture(tex)) => {
                self.access_counter += 1;
                self.access_times.insert(*handle, self.access_counter);
                Some(tex.clone())
            }
            _ => None,
        }
    }

    /// Load an asset into GPU memory if not already loaded.
    ///
    /// Returns `true` if the asset is now loaded (either freshly or already was).
    pub fn load_asset(&mut self, handle: &AssetHandle, renderer: &Renderer) -> bool {
        if self.loaded_assets.contains_key(handle) {
            self.access_counter += 1;
            self.access_times.insert(*handle, self.access_counter);
            return true;
        }

        let metadata = match self.registry.get(handle) {
            Some(m) => m.clone(),
            None => return false,
        };

        if !validate_asset_path(&metadata.file_path) {
            log::warn!(
                "Rejected unsafe asset path in registry: '{}'",
                metadata.file_path
            );
            return false;
        }

        match metadata.asset_type {
            AssetType::Texture2D => {
                let abs_path = self.asset_directory.join(&metadata.file_path);
                if abs_path.exists() {
                    if let Some(texture) = renderer.create_texture_from_file(&abs_path) {
                        let bytes = texture.gpu_memory_bytes();
                        self.loaded_assets
                            .insert(*handle, AssetData::Texture(Ref::new(texture)));
                        self.track_gpu_bytes(*handle, bytes);
                        true
                    } else {
                        log::warn!(
                            "Failed to load texture '{}', using fallback",
                            abs_path.display()
                        );
                        self.store_fallback(*handle, renderer);
                        true
                    }
                } else {
                    log::warn!(
                        "Texture file not found '{}', using fallback",
                        abs_path.display()
                    );
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
            AssetType::Prefab => {
                // Prefabs are instantiated on demand, not cached.
                true
            }
            AssetType::Material => {
                // Materials are managed by MaterialLibrary, not the asset manager cache.
                true
            }
            AssetType::Mesh => {
                // Mesh CPU data is loaded async and cached separately.
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

        let texture = match renderer.create_texture_from_rgba8(4, 4, &pixels) {
            Ok(t) => t,
            Err(e) => {
                panic!("Failed to create fallback texture (GPU texture creation unavailable): {e}");
            }
        };
        let tex_ref = Ref::new(texture);
        self.fallback_texture = Some(tex_ref.clone());
        log::info!("Created fallback checkerboard texture for missing assets");
        tex_ref
    }

    /// Store the fallback texture for a handle that failed to load.
    fn store_fallback(&mut self, handle: AssetHandle, renderer: &Renderer) {
        let tex = self.get_fallback_texture(renderer);
        let bytes = tex.gpu_memory_bytes();
        self.loaded_assets.insert(handle, AssetData::Texture(tex));
        self.track_gpu_bytes(handle, bytes);
    }

    /// Record GPU memory usage for a loaded asset.
    fn track_gpu_bytes(&mut self, handle: AssetHandle, bytes: u64) {
        if let Some(old) = self.asset_gpu_bytes.insert(handle, bytes) {
            self.total_gpu_bytes -= old;
        }
        self.total_gpu_bytes += bytes;
    }

    /// Remove GPU memory tracking for an asset.
    fn untrack_gpu_bytes(&mut self, handle: &AssetHandle) {
        if let Some(bytes) = self.asset_gpu_bytes.remove(handle) {
            self.total_gpu_bytes -= bytes;
        }
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
            log::warn!(
                "Rejected unsafe asset path in registry: '{}'",
                metadata.file_path
            );
            return;
        }

        let abs_path = self.asset_directory.join(&metadata.file_path);
        if !abs_path.exists() {
            log::warn!("Texture file not found: {}", abs_path.display());
            return;
        }

        self.loader
            .request_texture(*handle, abs_path, TextureSpecification::default());
    }

    /// Get loaded mesh CPU data by asset handle.
    pub fn get_mesh(&self, handle: &AssetHandle) -> Option<Ref<Mesh>> {
        self.loaded_meshes.get(handle).cloned()
    }

    /// Request async mesh loading for an asset handle.
    /// Looks up the path from the registry and enqueues the work.
    pub fn request_mesh_load(&mut self, handle: &AssetHandle) {
        if self.loaded_meshes.contains_key(handle) {
            return;
        }

        let metadata = match self.registry.get(handle) {
            Some(m) => m.clone(),
            None => return,
        };

        if metadata.asset_type != AssetType::Mesh {
            return;
        }

        if !validate_asset_path(&metadata.file_path) {
            log::warn!("Rejected unsafe mesh asset path: '{}'", metadata.file_path);
            return;
        }

        let abs_path = self.asset_directory.join(&metadata.file_path);
        if !abs_path.exists() {
            log::warn!("Mesh file not found: {}", abs_path.display());
            return;
        }

        self.loader.request_mesh(*handle, abs_path);
    }

    /// Get loaded skinned mesh data by asset handle.
    pub fn get_skinned_mesh(&self, handle: &AssetHandle) -> Option<Ref<GltfSkinData>> {
        self.loaded_skinned_meshes.get(handle).cloned()
    }

    /// Request async skinned mesh loading for an asset handle.
    pub fn request_skinned_mesh_load(&mut self, handle: &AssetHandle) {
        if self.loaded_skinned_meshes.contains_key(handle) {
            return;
        }

        let metadata = match self.registry.get(handle) {
            Some(m) => m.clone(),
            None => return,
        };

        if metadata.asset_type != AssetType::Mesh {
            return;
        }

        if !validate_asset_path(&metadata.file_path) {
            log::warn!(
                "Rejected unsafe skinned mesh asset path: '{}'",
                metadata.file_path
            );
            return;
        }

        let abs_path = self.asset_directory.join(&metadata.file_path);
        if !abs_path.exists() {
            log::warn!("Skinned mesh file not found: {}", abs_path.display());
            return;
        }

        self.loader.request_skinned_mesh(*handle, abs_path);
    }

    /// Poll completed async loads and perform GPU uploads.
    ///
    /// Textures are stored in `loaded_assets`; fonts are cached in
    /// `loaded_fonts`. Call [`flush_transfers`](Renderer::flush_transfers)
    /// after this to submit the upload batch.
    pub fn poll_loaded(&mut self, renderer: &mut Renderer) {
        let results = self.loader.poll_results();

        for result in results {
            match result {
                LoadResult::Texture { handle, data } => {
                    self.access_counter += 1;
                    self.access_times.insert(handle, self.access_counter);
                    match data {
                        Ok(cpu_data) => match renderer.upload_texture(&cpu_data) {
                            Ok(tex) => {
                                let bytes = tex.gpu_memory_bytes();
                                self.loaded_assets
                                    .insert(handle, AssetData::Texture(Ref::new(tex)));
                                self.track_gpu_bytes(handle, bytes);
                            }
                            Err(e) => {
                                log::warn!("Texture GPU upload failed: {e}, using fallback");
                                self.store_fallback(handle, renderer);
                            }
                        },
                        Err(e) => {
                            log::warn!("Async texture load failed: {e}, using fallback");
                            self.store_fallback(handle, renderer);
                        }
                    }
                }
                LoadResult::Mesh { handle, data } => match data {
                    Ok(mesh) => {
                        log::info!(
                            "Loaded mesh '{}' ({} verts, {} indices)",
                            mesh.name,
                            mesh.vertices.len(),
                            mesh.indices.len()
                        );
                        self.loaded_meshes.insert(handle, Ref::new(mesh));
                    }
                    Err(e) => {
                        log::warn!("Async mesh load failed: {e}");
                    }
                },
                LoadResult::SkinnedMesh { handle, data } => match data {
                    Ok(skin_data) => {
                        log::info!(
                            "Loaded skinned mesh ({} verts, {} joints, {} clips)",
                            skin_data.mesh.vertices.len(),
                            skin_data.skeleton.joint_count(),
                            skin_data.clips.len(),
                        );
                        self.loaded_skinned_meshes
                            .insert(handle, Ref::new(skin_data));
                    }
                    Err(e) => {
                        log::warn!("Async skinned mesh load failed: {e}");
                    }
                },
                LoadResult::Font { font_key, data } => match data {
                    Ok(cpu_data) => match renderer.upload_font(cpu_data) {
                        Ok(font) => {
                            self.loaded_fonts.insert(font_key, Ref::new(font));
                        }
                        Err(e) => {
                            log::warn!("Font GPU upload failed: {e}");
                        }
                    },
                    Err(e) => {
                        log::warn!("Async font load failed: {e}");
                    }
                },
            }
        }

        self.evict_lru();
    }

    /// Unload a specific asset from GPU memory.
    pub fn unload_asset(&mut self, handle: &AssetHandle) -> bool {
        self.access_times.remove(handle);
        self.untrack_gpu_bytes(handle);
        self.loaded_assets.remove(handle).is_some()
    }

    /// Unload assets that are only held by the cache (Arc strong_count == 1).
    /// Returns the number of assets evicted.
    pub fn unload_unused(&mut self) -> usize {
        let to_remove: Vec<AssetHandle> = self
            .loaded_assets
            .iter()
            .filter_map(|(handle, data)| match data {
                AssetData::Texture(tex) if std::sync::Arc::strong_count(tex) == 1 => Some(*handle),
                _ => None,
            })
            .collect();

        let count = to_remove.len();
        for handle in &to_remove {
            self.loaded_assets.remove(handle);
            self.access_times.remove(handle);
            self.untrack_gpu_bytes(handle);
        }
        count
    }

    /// Unload all cached assets from GPU memory.
    pub fn unload_all(&mut self) {
        self.loaded_assets.clear();
        self.loaded_meshes.clear();
        self.access_times.clear();
        self.asset_gpu_bytes.clear();
        self.total_gpu_bytes = 0;
    }

    /// Number of currently loaded assets.
    pub fn loaded_count(&self) -> usize {
        self.loaded_assets.len()
    }

    /// Set the maximum number of cached textures. 0 = unlimited.
    pub fn set_max_cached_textures(&mut self, max: usize) {
        self.max_cached_textures = max;
    }

    /// Evict least-recently-used textures until the cache is within both the
    /// `max_cached_textures` count limit and the `gpu_memory_budget` byte limit.
    /// Only evicts textures that are not referenced elsewhere (Arc strong_count == 1).
    pub fn evict_lru(&mut self) {
        let over_count =
            self.max_cached_textures > 0 && self.loaded_assets.len() > self.max_cached_textures;
        let over_budget =
            self.gpu_memory_budget > 0 && self.total_gpu_bytes > self.gpu_memory_budget;

        if !over_count && !over_budget {
            return;
        }

        // Collect candidates: assets with Arc strong_count == 1 (only held by cache).
        let mut candidates: Vec<(AssetHandle, u64)> = self
            .loaded_assets
            .iter()
            .filter_map(|(handle, data)| match data {
                AssetData::Texture(tex) if std::sync::Arc::strong_count(tex) == 1 => {
                    let access_time = self.access_times.get(handle).copied().unwrap_or(0);
                    Some((*handle, access_time))
                }
                _ => None,
            })
            .collect();

        // Sort by access time ascending (oldest first).
        candidates.sort_by_key(|(_, time)| *time);

        let mut evict_count = 0;
        for (handle, _) in &candidates {
            let within_count = self.max_cached_textures == 0
                || self.loaded_assets.len() <= self.max_cached_textures;
            let within_budget =
                self.gpu_memory_budget == 0 || self.total_gpu_bytes <= self.gpu_memory_budget;
            if within_count && within_budget {
                break;
            }
            self.untrack_gpu_bytes(handle);
            self.loaded_assets.remove(handle);
            self.access_times.remove(handle);
            evict_count += 1;
        }

        if evict_count > 0 {
            log::debug!(
                "LRU cache eviction: removed {} textures (GPU mem: {:.1} MB)",
                evict_count,
                self.total_gpu_bytes as f64 / (1024.0 * 1024.0)
            );
        }
    }

    /// Get a cached font by file path.
    pub fn get_font(&self, path: &Path) -> Option<Ref<Font>> {
        self.loaded_fonts.get(path).cloned()
    }

    /// Request async font loading. No-op if already loaded or pending.
    pub fn request_font_load(&mut self, path: PathBuf) {
        if self.loaded_fonts.contains_key(&path) {
            return;
        }
        self.loader.request_font(path);
    }

    /// Number of pending (in-flight) load requests (textures + fonts).
    pub fn pending_load_count(&self) -> usize {
        self.loader.pending_count()
    }

    /// Access the underlying asset loader (e.g., for font loading from editor).
    pub fn loader(&mut self) -> &mut AssetLoader {
        &mut self.loader
    }

    /// Total GPU memory used by cached textures, in bytes.
    pub fn gpu_memory_usage(&self) -> u64 {
        self.total_gpu_bytes
    }

    /// Set the GPU memory budget in bytes. 0 = unlimited (count-based eviction only).
    pub fn set_gpu_memory_budget(&mut self, budget_bytes: u64) {
        self.gpu_memory_budget = budget_bytes;
    }

    /// Get the current GPU memory budget in bytes (0 = unlimited).
    pub fn gpu_memory_budget(&self) -> u64 {
        self.gpu_memory_budget
    }
}
