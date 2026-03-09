# Asset System

The asset system lives in `gg_engine/src/asset/` and provides UUID-based asset management decoupled from file paths. Assets are identified by handles (UUIDs), registered in a persistent YAML registry, and loaded on demand with LRU caching and async background loading.

## Asset System Overview

All assets are referenced by `AssetHandle` (a `Uuid` type alias). A handle of `0` means "no asset" / null. File paths are stored in a registry, and handles are what components and serialization use. This decouples game data from the filesystem — renaming or moving a file only requires updating the registry entry, not every reference in every scene.

## Core Types

**File:** `asset/mod.rs`

### AssetHandle

```rust
pub type AssetHandle = Uuid;  // 0 = null/no asset
```

### AssetType

| Variant | Extensions | Description |
|---------|-----------|-------------|
| `None` | (unknown) | Unrecognized file type |
| `Scene` | `.ggscene` | Scene file |
| `Texture2D` | `.png`, `.jpg`, `.jpeg` | Image texture |
| `Audio` | `.wav`, `.ogg`, `.mp3`, `.flac` | Audio file |
| `Prefab` | `.ggprefab` | Entity template (prefab) |
| `Material` | `.ggmaterial` | Material definition |
| `Mesh` | `.gltf`, `.glb` | 3D mesh (glTF) |

Type detection is case-insensitive (`asset_type_from_extension`). Round-trip conversion via `as_str()` / `parse_str()`.

### AssetMetadata

```rust
pub struct AssetMetadata {
    pub file_path: String,      // Relative to asset directory
    pub asset_type: AssetType,
}
```

### Path Security

`validate_asset_path()` prevents directory traversal and absolute path attacks. Rejects:
- Empty paths
- `..` path components (e.g., `../../etc/passwd`)
- Unix absolute paths (starting with `/`)
- Windows absolute paths (e.g., `C:\`, `\\server\share`)

Single dots and dots in filenames are allowed (e.g., `./textures/player.png`, `file.name.png`).

## Asset Registry

**File:** `asset/asset_registry.rs`

`AssetRegistry` maps asset handles (UUIDs) to metadata (file path + type). Persisted as `AssetRegistry.ggregistry` YAML in the project's assets directory.

### Storage

Two internal maps for bidirectional lookup:

| Field | Type | Purpose |
|-------|------|---------|
| `assets` | `HashMap<AssetHandle, AssetMetadata>` | Handle-to-metadata (primary) |
| `path_index` | `HashMap<String, AssetHandle>` | Path-to-handle (reverse index, O(1)) |

Paths are normalized to forward slashes on insert and lookup (`\` to `/`) for cross-platform consistency.

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `() -> Self` | Empty registry |
| `insert` | `(handle, metadata)` | Register an asset, updates both maps |
| `get` | `(&handle) -> Option<&AssetMetadata>` | Lookup by handle |
| `remove` | `(&handle) -> Option<AssetMetadata>` | Unregister, cleans up both maps |
| `find_by_path` | `(&str) -> Option<AssetHandle>` | Reverse lookup by normalized path |
| `contains` | `(&handle) -> bool` | Check existence |
| `iter` | `() -> impl Iterator` | Iterate all `(handle, metadata)` pairs |
| `len` / `is_empty` | | Entry count |
| `load` | `(&Path) -> Option<Self>` | Deserialize from `.ggregistry` YAML |
| `save` | `(&Path) -> bool` | Serialize to `.ggregistry` YAML |

### Dependency Tracking

The registry maintains forward and reverse dependency maps between scenes/prefabs and the assets they reference.

| Field | Type | Purpose |
|-------|------|---------|
| `dependencies` | `HashMap<AssetHandle, HashSet<AssetHandle>>` | Scene/prefab → assets it references |
| `dependents` | `HashMap<AssetHandle, HashSet<AssetHandle>>` | Asset → scenes/prefabs that reference it |

| Method | Signature | Description |
|--------|-----------|-------------|
| `set_dependencies` | `(source, HashSet<AssetHandle>)` | Set deps for a scene, updates reverse index |
| `get_dependencies` | `(&handle) -> Option<&HashSet<AssetHandle>>` | Forward query: what does this scene use? |
| `get_dependents` | `(&handle) -> Option<&HashSet<AssetHandle>>` | Reverse query: who uses this asset? |
| `scan_scene_dependencies` | `(&str) -> HashSet<AssetHandle>` | Parse YAML for TextureHandle/AudioHandle/AlbedoTexture/MeshAsset |
| `rebuild_dependencies` | `(&Path)` | Scan all scene/prefab files and populate both maps |

Dependencies are rebuilt automatically on `EditorAssetManager::load_registry()`. When an asset is removed via `registry.remove()`, both forward and reverse entries are cleaned up.

### Persistence Format

The registry file uses serde intermediate structs (`RegistryFileData`, `RegistryEntryData`) with capitalized field names. Entries are sorted by file path for stable, diff-friendly output. File writes use `atomic_write` for crash safety.

```yaml
Assets:
- Handle: 1001
  FilePath: textures/hero.png
  Type: Texture2D
- Handle: 2002
  FilePath: scenes/level1.ggscene
  Type: Scene
```

## EditorAssetManager

**File:** `asset/asset_manager.rs`

`EditorAssetManager` wraps the registry with a GPU texture cache and async loading. It is the primary interface for the editor and player to interact with assets.

### Construction

```rust
let mut manager = EditorAssetManager::new("assets/");
manager.load_registry();  // Load AssetRegistry.ggregistry from asset directory
```

### AssetData

```rust
pub enum AssetData {
    Texture(Ref<Texture2D>),  // Ref = Arc
}
```

Only textures are cached in GPU memory. Scenes are loaded on demand via the scene serializer. Audio files are loaded by kira at playback time (the manager only verifies the file exists).

### Import and Load

| Method | Signature | Description |
|--------|-----------|-------------|
| `import_asset` | `(&str) -> AssetHandle` | Register a new asset by relative path. Detects type from extension. Returns existing handle if already imported. Validates path security. |
| `load_asset` | `(&handle, &Renderer) -> bool` | Synchronous GPU load. Returns `true` if asset is now loaded. Uses fallback texture on failure. |
| `save_registry` | `()` | Persist registry to disk |
| `load_registry` | `()` | Load registry from disk |

### Query Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `is_imported` | `(&str) -> bool` | Check if a path is in the registry |
| `is_valid` | `(&handle) -> bool` | Check if a handle exists in the registry |
| `is_loaded` | `(&handle) -> bool` | Check if an asset is in the GPU cache |
| `get_metadata` | `(&handle) -> Option<&AssetMetadata>` | Registry metadata lookup |
| `get_absolute_path` | `(&handle) -> Option<PathBuf>` | Resolve handle to full filesystem path |
| `get_asset_type` | `(&handle) -> AssetType` | Asset type from registry (returns `None` if unknown) |
| `get_handle_for_path` | `(&str) -> Option<AssetHandle>` | Reverse lookup |
| `get_texture` | `(&handle) -> Option<Ref<Texture2D>>` | Get cached texture, updates LRU access time |
| `get_asset` | `(&handle) -> Option<&AssetData>` | Get raw cached asset data |

### Fallback Texture

When a texture fails to load (file missing, corrupt image, GPU upload failure), the manager substitutes a **4x4 magenta/black checkerboard** texture. This provides a visible indicator of broken asset references without crashing. The fallback texture is created lazily on first use and shared across all failed assets.

## LRU Texture Cache

The manager implements LRU (Least Recently Used) eviction for GPU textures to bound memory usage.

### Configuration

| Field | Default | Description |
|-------|---------|-------------|
| `access_counter` | 0 | Monotonic counter, incremented on each access |
| `access_times` | `HashMap<AssetHandle, u64>` | Last-access timestamp per loaded asset |
| `max_cached_textures` | 256 | Maximum cached textures before eviction (0 = unlimited) |
| `gpu_memory_budget` | 0 | GPU memory budget in bytes (0 = unlimited, count-based only) |
| `asset_gpu_bytes` | `HashMap<AssetHandle, u64>` | Per-texture GPU memory size tracking |
| `total_gpu_bytes` | 0 | Running total of tracked GPU memory usage |

### GPU Memory Tracking

Each loaded texture's GPU memory is tracked as `width × height × 4` bytes (RGBA8). The running total is maintained across all insert/remove operations. When a `gpu_memory_budget` is set (non-zero), `evict_lru()` evicts until both the count limit and byte limit are satisfied.

### Eviction Rules

`evict_lru()` runs automatically after each `poll_loaded()` call. It evicts when either the count limit or the byte budget is exceeded. It only evicts textures where `Arc::strong_count == 1`, meaning the texture is held only by the cache and has no external references (no component is using it). Candidates are sorted by access time ascending (oldest first).

### Cache Management Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `evict_lru` | `()` | Evict oldest unreferenced textures until under both count and byte limits |
| `unload_asset` | `(&handle) -> bool` | Remove specific asset from cache |
| `unload_unused` | `() -> usize` | Remove all assets with `strong_count == 1`, returns eviction count |
| `unload_all` | `()` | Clear entire cache |
| `set_max_cached_textures` | `(usize)` | Set count limit (0 = unlimited) |
| `set_gpu_memory_budget` | `(u64)` | Set byte budget (0 = unlimited) |
| `gpu_memory_usage` | `() -> u64` | Current total GPU memory in bytes |
| `gpu_memory_budget` | `() -> u64` | Current budget setting |
| `loaded_count` | `() -> usize` | Number of currently cached assets |

## Async Loading

**File:** `asset/asset_loader.rs`

`AssetLoader` performs CPU-heavy asset loading (image decoding, font MSDF generation) on background threads, keeping the main thread responsive.

### Architecture

```
Main Thread                    Worker Threads (2)
    |                               |
    |-- request_texture(handle) --> request_rx (shared Mutex)
    |-- request_font(key) -------> [worker picks up request]
    |                               |
    |                          Texture2D::load_cpu_data()
    |                          generate_font_cpu_data()
    |                               |
    |<-- poll_results() ---------- result_tx
    |                               |
    |   GPU upload on main thread   |
```

- **Worker count**: `WORKER_COUNT = 2` threads, spawned lazily on first request
- **Channel**: `mpsc` channels for request/response. Workers share a single `Receiver` via `Arc<Mutex<_>>`
- **Panic safety**: Worker threads wrap load calls in `catch_unwind` to prevent panics from killing the thread pool

### Request/Result Types

```rust
enum LoadRequest {
    Texture { handle: Uuid, path: PathBuf, spec: TextureSpecification },
    Font { font_key: PathBuf, path: PathBuf },
    Shutdown,
}

pub enum LoadResult {
    Texture { handle: Uuid, data: Result<TextureCpuData, String> },
    Font { font_key: PathBuf, data: Result<FontCpuData, String> },
}
```

### CPU/GPU Split

Loading is split into two phases to keep Vulkan calls on the main thread:

1. **CPU phase** (worker thread): `Texture2D::load_cpu_data()` decodes the image file into `TextureCpuData` (width, height, pixels as `Vec<u8>`, spec). No Vulkan types, fully thread-safe.
2. **GPU phase** (main thread): `renderer.upload_texture(&cpu_data)` creates the Vulkan image, staging buffer, layout transitions, and descriptor writes.

For fonts, the CPU phase runs `generate_font_cpu_data()` which does the full MSDF atlas generation. GPU upload happens on the main thread when results are polled.

### Usage Flow

```rust
// Enqueue async load (via EditorAssetManager)
asset_manager.request_load(&handle);

// Each frame, poll for completed loads and upload to GPU
let font_results = asset_manager.poll_loaded(&renderer);
// font_results contains any completed font loads for the caller to process
```

### Deduplication

`AssetLoader` tracks pending requests via `HashSet<Uuid>` (textures) and `HashSet<PathBuf>` (fonts). Duplicate requests are silently dropped. Pending state is cleared when results are polled.

### Shutdown

On `Drop`, `AssetLoader` sends one `Shutdown` message per worker and joins all threads. This ensures clean teardown without leaked threads.

### Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `request_texture` | `(handle, path, spec) -> bool` | Enqueue texture load. Returns `false` if already pending. |
| `request_font` | `(font_key: PathBuf) -> bool` | Enqueue font load. Returns `false` if already pending. |
| `poll_results` | `() -> Vec<LoadResult>` | Non-blocking drain of completed results |
| `is_texture_pending` | `(&Uuid) -> bool` | Check if a texture load is in flight |
| `is_font_pending` | `(&PathBuf) -> bool` | Check if a font load is in flight |

## Scene Integration

Components reference assets by handle. At runtime, handles must be resolved to GPU resources.

### Texture Resolution

```rust
// Synchronous — blocks until all textures are loaded
scene.resolve_texture_handles(&mut asset_manager, &renderer);

// Asynchronous — enqueues background loads, caller polls each frame
scene.resolve_texture_handles_async(&mut asset_manager);
```

Both methods scan `SpriteRendererComponent` and `TilemapComponent` entities for non-zero `texture_handle` fields and resolve them to `Option<Ref<Texture2D>>`.

### Audio Resolution

```rust
scene.resolve_audio_handles(&mut asset_manager);
```

Scans `AudioSourceComponent` entities, resolves `audio_handle` to a file path stored on the component for use by the audio engine at playback time.

### Components with Asset Handles

| Component | Field | Asset Type |
|-----------|-------|------------|
| `SpriteRendererComponent` | `texture_handle: Uuid` | `Texture2D` |
| `TilemapComponent` | `texture_handle: Uuid` | `Texture2D` |
| `AudioSourceComponent` | `audio_handle: Uuid` | `Audio` |
