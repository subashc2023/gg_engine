//! Asset cooking — analyzes project references and produces a build manifest
//! listing exactly which files are needed for a distributable build.
//!
//! Used by the editor's build system to strip unused assets.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::uuid::Uuid;

use super::{AssetRegistry, AssetType};

// ---------------------------------------------------------------------------
// Build manifest
// ---------------------------------------------------------------------------

/// A file to include in the build.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    /// Relative path within the assets directory (forward slashes).
    pub path: String,
    /// Size in bytes on disk.
    pub size: u64,
    /// What kind of asset this is.
    pub category: FileCategory,
}

/// Classification of a file in the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    Scene,
    Texture,
    Audio,
    Mesh,
    Material,
    Prefab,
    Script,
    Font,
    Registry,
    Other,
}

impl FileCategory {
    pub fn label(self) -> &'static str {
        match self {
            FileCategory::Scene => "Scenes",
            FileCategory::Texture => "Textures",
            FileCategory::Audio => "Audio",
            FileCategory::Mesh => "Meshes",
            FileCategory::Material => "Materials",
            FileCategory::Prefab => "Prefabs",
            FileCategory::Script => "Scripts",
            FileCategory::Font => "Fonts",
            FileCategory::Registry => "Registry",
            FileCategory::Other => "Other",
        }
    }

    fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "png" | "jpg" | "jpeg" | "hdr" => FileCategory::Texture,
            "ggscene" => FileCategory::Scene,
            "wav" | "ogg" | "mp3" | "flac" => FileCategory::Audio,
            "gltf" | "glb" => FileCategory::Mesh,
            "ggmaterial" => FileCategory::Material,
            "ggprefab" => FileCategory::Prefab,
            "lua" => FileCategory::Script,
            "ttf" | "otf" => FileCategory::Font,
            "ggregistry" => FileCategory::Registry,
            _ => FileCategory::Other,
        }
    }
}

/// Result of asset cooking — determines what to include/exclude in a build.
#[derive(Debug)]
pub struct BuildManifest {
    /// Files to include (relative paths within assets dir).
    pub entries: Vec<ManifestEntry>,
    /// Files that exist on disk but are not referenced (relative paths).
    pub excluded: Vec<ManifestEntry>,
    /// Warnings about issues found during analysis.
    pub warnings: Vec<String>,
    /// Total bytes of included files.
    pub total_included_bytes: u64,
    /// Total bytes of excluded files.
    pub total_excluded_bytes: u64,
}

impl BuildManifest {
    /// Bytes per category for included files.
    pub fn size_by_category(&self) -> HashMap<FileCategory, u64> {
        let mut map = HashMap::new();
        for entry in &self.entries {
            *map.entry(entry.category).or_insert(0) += entry.size;
        }
        map
    }
}

// ---------------------------------------------------------------------------
// Scene/prefab YAML scanning
// ---------------------------------------------------------------------------

/// References extracted from a scene or prefab YAML file.
#[derive(Debug, Default)]
struct SceneFileRefs {
    /// Asset handles (textures, audio, meshes, materials, environments).
    asset_handles: HashSet<u64>,
    /// Script file paths (relative to assets dir).
    script_paths: HashSet<String>,
    /// Font file paths (relative to assets dir).
    font_paths: HashSet<String>,
}

/// Scan a scene/prefab YAML string for all external asset references.
///
/// Extracts:
/// - Handle-based: TextureHandle, AudioHandle, AlbedoTexture, NormalTexture,
///   MeshAsset, EnvironmentMap
/// - Path-based: ScriptPath, FontPath
fn scan_scene_references(yaml: &str) -> SceneFileRefs {
    let mut refs = SceneFileRefs::default();

    for line in yaml.lines() {
        let trimmed = line.trim();

        // Handle-based references (u64).
        let handle_raw = if let Some(rest) = trimmed.strip_prefix("TextureHandle:") {
            rest.trim().parse::<u64>().ok()
        } else if let Some(rest) = trimmed.strip_prefix("AudioHandle:") {
            rest.trim().parse::<u64>().ok()
        } else if let Some(rest) = trimmed.strip_prefix("AlbedoTexture:") {
            rest.trim().parse::<u64>().ok()
        } else if let Some(rest) = trimmed.strip_prefix("MeshAsset:") {
            rest.trim().parse::<u64>().ok()
        } else if let Some(rest) = trimmed.strip_prefix("NormalTexture:") {
            rest.trim().parse::<u64>().ok()
        } else if let Some(rest) = trimmed.strip_prefix("EnvironmentMap:") {
            rest.trim().parse::<u64>().ok()
        } else {
            None
        };

        if let Some(raw) = handle_raw {
            if raw != 0 {
                refs.asset_handles.insert(raw);
            }
        }

        // Path-based references (strings).
        if let Some(rest) = trimmed.strip_prefix("ScriptPath:") {
            let path = rest.trim().trim_matches(|c| c == '\'' || c == '"');
            if !path.is_empty() {
                refs.script_paths.insert(path.to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("FontPath:") {
            let path = rest.trim().trim_matches(|c| c == '\'' || c == '"');
            if !path.is_empty() {
                refs.font_paths.insert(path.to_string());
            }
        }
    }

    refs
}

// ---------------------------------------------------------------------------
// Lua script scanning
// ---------------------------------------------------------------------------

/// References found in a Lua source file.
#[derive(Debug, Default)]
struct LuaRefs {
    /// Module names from `require("module.name")` or `require('module.name')`.
    requires: HashSet<String>,
    /// Scene paths from `Engine.load_scene("path")` or `load_scene('path')`.
    scene_loads: HashSet<String>,
}

/// Scan a Lua source file for `require()` and `Engine.load_scene()` calls.
///
/// Uses simple pattern matching (not a full parser) — handles the common cases:
/// - `require("module.name")`, `require('module.name')`
/// - `Engine.load_scene("scenes/level2.ggscene")`, `load_scene('...')`
fn scan_lua_references(source: &str) -> LuaRefs {
    let mut refs = LuaRefs::default();

    for line in source.lines() {
        let trimmed = line.trim();

        // Skip comments.
        if trimmed.starts_with("--") {
            continue;
        }

        // Scan for require("...") or require('...').
        let mut search = trimmed;
        while let Some(pos) = search.find("require") {
            let after = &search[pos + 7..];
            if let Some(module) = extract_string_arg(after) {
                if !module.is_empty() {
                    refs.requires.insert(module);
                }
            }
            // Advance past this match.
            search = &search[pos + 7..];
        }

        // Scan for load_scene("...") or load_scene('...').
        let mut search = trimmed;
        while let Some(pos) = search.find("load_scene") {
            let after = &search[pos + 10..];
            if let Some(scene_path) = extract_string_arg(after) {
                if !scene_path.is_empty() {
                    refs.scene_loads.insert(scene_path);
                }
            }
            search = &search[pos + 10..];
        }
    }

    refs
}

/// Extract a string argument from patterns like `("value")` or `('value')`.
/// Returns the string content if found.
fn extract_string_arg(s: &str) -> Option<String> {
    let s = s.trim_start();
    if !s.starts_with('(') {
        return None;
    }
    let s = s[1..].trim_start();
    let (quote, rest) = if s.starts_with('"') {
        ('"', &s[1..])
    } else if s.starts_with('\'') {
        ('\'', &s[1..])
    } else {
        return None;
    };

    if let Some(end) = rest.find(quote) {
        Some(rest[..end].to_string())
    } else {
        None
    }
}

/// Resolve a Lua `require("module.name")` module name to a file path relative
/// to the assets directory.
///
/// - Dots become path separators: `module.name` → `module/name.lua`
/// - The path is relative to `script_module_subdir` (the script module path
///   relative to the assets directory, e.g. `scripts`).
fn resolve_require_path(module_name: &str, script_module_subdir: &str) -> String {
    let relative = module_name.replace('.', "/");
    let module_dir = script_module_subdir.trim_end_matches('/');
    format!("{}/{}.lua", module_dir, relative)
}

// ---------------------------------------------------------------------------
// Main cooking function
// ---------------------------------------------------------------------------

/// Analyze the project's asset directory and produce a build manifest.
///
/// Walks all scenes and prefabs, traces asset references (handles and paths),
/// follows Lua `require()` and `Engine.load_scene()` chains, and determines
/// exactly which files are needed for a shipping build.
///
/// # Arguments
/// - `asset_dir` — absolute path to the project's assets directory
/// - `script_module_subdir` — the script module path *relative to assets dir*
///   (e.g. `"scripts"`)
pub fn cook_assets(asset_dir: &Path, script_module_subdir: &str) -> BuildManifest {
    let mut warnings = Vec::new();

    // 1. Load the asset registry.
    let registry_path = asset_dir.join("AssetRegistry.ggregistry");
    let registry = match AssetRegistry::load(&registry_path) {
        Ok(r) => r,
        Err(e) => {
            warnings.push(format!("Failed to load asset registry: {}", e));
            // Fall back: include everything.
            return cook_all_files(asset_dir, warnings);
        }
    };

    // 2. Collect all scene + prefab file paths from the registry.
    let mut scenes_to_scan: Vec<String> = Vec::new();
    for (_handle, meta) in registry.iter() {
        if matches!(meta.asset_type, AssetType::Scene | AssetType::Prefab) {
            scenes_to_scan.push(meta.file_path.clone());
        }
    }

    // 3. Fixpoint loop: scan scenes/prefabs and scripts, discover new references.
    let mut needed_handles: HashSet<u64> = HashSet::new();
    let mut needed_paths: HashSet<String> = HashSet::new(); // relative to assets dir
    let mut scanned_scenes: HashSet<String> = HashSet::new();
    let mut scanned_scripts: HashSet<String> = HashSet::new();
    let mut scripts_to_scan: Vec<String> = Vec::new();

    // All scenes/prefabs in the registry are included (any might be loaded at runtime).
    for path in &scenes_to_scan {
        needed_paths.insert(path.clone());
    }

    loop {
        let mut found_new = false;

        // Scan scenes/prefabs.
        let current_scenes: Vec<String> = scenes_to_scan
            .drain(..)
            .filter(|p| scanned_scenes.insert(p.clone()))
            .collect();

        for scene_path in &current_scenes {
            let abs_path = asset_dir.join(scene_path);
            match fs::read_to_string(&abs_path) {
                Ok(yaml) => {
                    let refs = scan_scene_references(&yaml);
                    for h in refs.asset_handles {
                        if needed_handles.insert(h) {
                            found_new = true;
                        }
                    }
                    for sp in refs.script_paths {
                        if needed_paths.insert(sp.clone()) {
                            scripts_to_scan.push(sp);
                            found_new = true;
                        }
                    }
                    for fp in refs.font_paths {
                        if needed_paths.insert(fp.clone()) {
                            found_new = true;
                        }
                    }
                }
                Err(e) => {
                    warnings.push(format!("Cannot read scene '{}': {}", scene_path, e));
                }
            }
        }

        // Scan Lua scripts.
        let current_scripts: Vec<String> = scripts_to_scan
            .drain(..)
            .filter(|p| scanned_scripts.insert(p.clone()))
            .collect();

        for script_path in &current_scripts {
            let abs_path = asset_dir.join(script_path);
            match fs::read_to_string(&abs_path) {
                Ok(source) => {
                    let refs = scan_lua_references(&source);
                    for module_name in refs.requires {
                        let resolved = resolve_require_path(&module_name, script_module_subdir);
                        if needed_paths.insert(resolved.clone()) {
                            scripts_to_scan.push(resolved);
                            found_new = true;
                        }
                    }
                    for scene_path in refs.scene_loads {
                        if needed_paths.insert(scene_path.clone()) {
                            scenes_to_scan.push(scene_path);
                            found_new = true;
                        }
                    }
                }
                Err(e) => {
                    warnings.push(format!("Cannot read script '{}': {}", script_path, e));
                }
            }
        }

        if !found_new && scenes_to_scan.is_empty() && scripts_to_scan.is_empty() {
            break;
        }
    }

    // 4. Resolve asset handles to file paths.
    for handle_raw in &needed_handles {
        let handle = Uuid::from_raw(*handle_raw);
        if let Some(meta) = registry.get(&handle) {
            needed_paths.insert(meta.file_path.clone());
        } else {
            warnings.push(format!(
                "Asset handle {} referenced but not found in registry",
                handle_raw
            ));
        }
    }

    // Always include the registry itself.
    needed_paths.insert("AssetRegistry.ggregistry".to_string());

    // 5. Walk the assets directory and classify files.
    let all_files = walk_directory(asset_dir);
    let mut entries = Vec::new();
    let mut excluded = Vec::new();
    let mut total_included = 0u64;
    let mut total_excluded = 0u64;

    for (relative_path, size) in &all_files {
        let normalized = relative_path.replace('\\', "/");

        // Skip autosave files.
        if normalized.contains(".autosave.") {
            continue;
        }

        let ext = normalized
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_lowercase();
        let category = FileCategory::from_extension(&ext);

        let entry = ManifestEntry {
            path: normalized.clone(),
            size: *size,
            category,
        };

        if needed_paths.contains(&normalized) {
            total_included += size;
            entries.push(entry);
        } else {
            total_excluded += size;
            excluded.push(entry);
        }
    }

    // Sort entries by path for stable output.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    excluded.sort_by(|a, b| a.path.cmp(&b.path));

    // Validate: check for needed paths that don't exist on disk.
    let on_disk: HashSet<String> = all_files.iter().map(|(p, _)| p.replace('\\', "/")).collect();
    for path in &needed_paths {
        if path == "AssetRegistry.ggregistry" {
            continue;
        }
        if !on_disk.contains(path) {
            warnings.push(format!("Referenced file '{}' not found on disk", path));
        }
    }

    BuildManifest {
        entries,
        excluded,
        warnings,
        total_included_bytes: total_included,
        total_excluded_bytes: total_excluded,
    }
}

/// Fallback: include all files when registry can't be loaded.
fn cook_all_files(asset_dir: &Path, warnings: Vec<String>) -> BuildManifest {
    let all_files = walk_directory(asset_dir);
    let mut entries = Vec::new();
    let mut total = 0u64;

    for (relative_path, size) in all_files {
        let normalized = relative_path.replace('\\', "/");
        if normalized.contains(".autosave.") {
            continue;
        }
        let ext = normalized.rsplit('.').next().unwrap_or("");
        let category = FileCategory::from_extension(ext);
        total += size;
        entries.push(ManifestEntry {
            path: normalized,
            size,
            category,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    BuildManifest {
        entries,
        excluded: Vec::new(),
        warnings,
        total_included_bytes: total,
        total_excluded_bytes: 0,
    }
}

/// Recursively list all files in a directory, returning (relative_path, size_bytes).
fn walk_directory(dir: &Path) -> Vec<(String, u64)> {
    let mut results = Vec::new();
    walk_directory_inner(dir, dir, &mut results);
    results
}

fn walk_directory_inner(root: &Path, current: &Path, results: &mut Vec<(String, u64)>) {
    let entries = match fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_directory_inner(root, &path, results);
        } else if let Ok(meta) = path.metadata() {
            if let Ok(relative) = path.strip_prefix(root) {
                let rel_str = relative.to_string_lossy().replace('\\', "/");
                results.push((rel_str, meta.len()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_scene_references_handles() {
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
    NormalTexture: 9001
  EnvironmentComponent:
    EnvironmentMap: 10001
"#;
        let refs = scan_scene_references(yaml);
        assert_eq!(refs.asset_handles.len(), 6);
        assert!(refs.asset_handles.contains(&5001));
        assert!(refs.asset_handles.contains(&6001));
        assert!(refs.asset_handles.contains(&7001));
        assert!(refs.asset_handles.contains(&8001));
        assert!(refs.asset_handles.contains(&9001));
        assert!(refs.asset_handles.contains(&10001));
    }

    #[test]
    fn scan_scene_references_skips_zero() {
        let yaml = "  TextureHandle: 0\n  AudioHandle: 0\n";
        let refs = scan_scene_references(yaml);
        assert!(refs.asset_handles.is_empty());
    }

    #[test]
    fn scan_scene_references_paths() {
        let yaml = r#"
  NativeScriptComponent:
    ScriptPath: scripts/player.lua
  TextComponent:
    FontPath: fonts/default.ttf
"#;
        let refs = scan_scene_references(yaml);
        assert!(refs.script_paths.contains("scripts/player.lua"));
        assert!(refs.font_paths.contains("fonts/default.ttf"));
    }

    #[test]
    fn scan_lua_require() {
        let source = r#"
local utils = require("utils.math")
local ai = require('enemy.ai')
-- require("commented.out")
"#;
        let refs = scan_lua_references(source);
        assert_eq!(refs.requires.len(), 2);
        assert!(refs.requires.contains("utils.math"));
        assert!(refs.requires.contains("enemy.ai"));
    }

    #[test]
    fn scan_lua_load_scene() {
        let source = r#"
Engine.load_scene("scenes/level2.ggscene")
Engine.load_scene('scenes/boss.ggscene')
-- Engine.load_scene("commented.ggscene")
"#;
        let refs = scan_lua_references(source);
        assert_eq!(refs.scene_loads.len(), 2);
        assert!(refs.scene_loads.contains("scenes/level2.ggscene"));
        assert!(refs.scene_loads.contains("scenes/boss.ggscene"));
    }

    #[test]
    fn extract_string_arg_double_quotes() {
        assert_eq!(
            extract_string_arg(r#"("hello")"#),
            Some("hello".to_string())
        );
    }

    #[test]
    fn extract_string_arg_single_quotes() {
        assert_eq!(
            extract_string_arg("('world')"),
            Some("world".to_string())
        );
    }

    #[test]
    fn extract_string_arg_no_parens() {
        assert_eq!(extract_string_arg("\"hello\""), None);
    }

    #[test]
    fn extract_string_arg_with_spaces() {
        assert_eq!(
            extract_string_arg(r#"( "spaced" )"#),
            Some("spaced".to_string())
        );
    }

    #[test]
    fn resolve_require_path_simple() {
        assert_eq!(
            resolve_require_path("utils.math", "scripts"),
            "scripts/utils/math.lua"
        );
    }

    #[test]
    fn resolve_require_path_single_module() {
        assert_eq!(
            resolve_require_path("helpers", "scripts"),
            "scripts/helpers.lua"
        );
    }

    #[test]
    fn resolve_require_path_trailing_slash() {
        assert_eq!(
            resolve_require_path("a.b", "scripts/"),
            "scripts/a/b.lua"
        );
    }

    #[test]
    fn file_category_from_extension() {
        assert_eq!(FileCategory::from_extension("png"), FileCategory::Texture);
        assert_eq!(FileCategory::from_extension("JPG"), FileCategory::Texture);
        assert_eq!(FileCategory::from_extension("hdr"), FileCategory::Texture);
        assert_eq!(FileCategory::from_extension("ggscene"), FileCategory::Scene);
        assert_eq!(FileCategory::from_extension("ogg"), FileCategory::Audio);
        assert_eq!(FileCategory::from_extension("glb"), FileCategory::Mesh);
        assert_eq!(
            FileCategory::from_extension("ggmaterial"),
            FileCategory::Material
        );
        assert_eq!(
            FileCategory::from_extension("ggprefab"),
            FileCategory::Prefab
        );
        assert_eq!(FileCategory::from_extension("lua"), FileCategory::Script);
        assert_eq!(FileCategory::from_extension("ttf"), FileCategory::Font);
        assert_eq!(FileCategory::from_extension("txt"), FileCategory::Other);
    }

    #[test]
    fn cook_assets_with_temp_project() {
        let dir = std::env::temp_dir().join("gg_cook_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::create_dir_all(dir.join("textures")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::create_dir_all(dir.join("unused")).unwrap();

        // Create a simple registry with one scene and one texture.
        let mut registry = AssetRegistry::new();
        registry.insert(
            Uuid::from_raw(1001),
            super::super::AssetMetadata {
                file_path: "scenes/main.ggscene".to_string(),
                asset_type: AssetType::Scene,
            },
        );
        registry.insert(
            Uuid::from_raw(2001),
            super::super::AssetMetadata {
                file_path: "textures/hero.png".to_string(),
                asset_type: AssetType::Texture2D,
            },
        );
        registry
            .save(&dir.join("AssetRegistry.ggregistry"))
            .unwrap();

        // Scene references the texture and a script.
        let scene_yaml = r#"Version: 2
Scene: main
Entities:
- Entity: 100
  SpriteRendererComponent:
    TextureHandle: 2001
  NativeScriptComponent:
    ScriptPath: scripts/player.lua
"#;
        fs::write(dir.join("scenes/main.ggscene"), scene_yaml).unwrap();

        // Script file.
        fs::write(dir.join("scripts/player.lua"), "-- player script\n").unwrap();

        // Texture file (just some bytes).
        fs::write(dir.join("textures/hero.png"), &[0u8; 100]).unwrap();

        // Unused file that should be excluded.
        fs::write(dir.join("unused/junk.txt"), "unused").unwrap();

        let manifest = cook_assets(&dir, "scripts");

        // Should include: registry, scene, texture, script.
        assert_eq!(manifest.entries.len(), 4);
        let included_paths: HashSet<&str> =
            manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(included_paths.contains("AssetRegistry.ggregistry"));
        assert!(included_paths.contains("scenes/main.ggscene"));
        assert!(included_paths.contains("textures/hero.png"));
        assert!(included_paths.contains("scripts/player.lua"));

        // Unused file should be excluded.
        assert!(!manifest.excluded.is_empty());
        let excluded_paths: HashSet<&str> =
            manifest.excluded.iter().map(|e| e.path.as_str()).collect();
        assert!(excluded_paths.contains("unused/junk.txt"));

        assert!(manifest.warnings.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_assets_follows_require_chain() {
        let dir = std::env::temp_dir().join("gg_cook_require_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::create_dir_all(dir.join("scripts/utils")).unwrap();

        let mut registry = AssetRegistry::new();
        registry.insert(
            Uuid::from_raw(1001),
            super::super::AssetMetadata {
                file_path: "scenes/main.ggscene".to_string(),
                asset_type: AssetType::Scene,
            },
        );
        registry
            .save(&dir.join("AssetRegistry.ggregistry"))
            .unwrap();

        // Scene references a script.
        let scene_yaml = r#"Version: 2
Scene: main
Entities:
- Entity: 100
  NativeScriptComponent:
    ScriptPath: scripts/player.lua
"#;
        fs::write(dir.join("scenes/main.ggscene"), scene_yaml).unwrap();

        // Script requires a module.
        fs::write(
            dir.join("scripts/player.lua"),
            "local math = require('utils.helpers')\n",
        )
        .unwrap();

        // The module file.
        fs::write(dir.join("scripts/utils/helpers.lua"), "return {}\n").unwrap();

        let manifest = cook_assets(&dir, "scripts");

        let included_paths: HashSet<&str> =
            manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(included_paths.contains("scripts/player.lua"));
        assert!(included_paths.contains("scripts/utils/helpers.lua"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_assets_follows_load_scene() {
        let dir = std::env::temp_dir().join("gg_cook_loadscene_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();
        fs::create_dir_all(dir.join("scripts")).unwrap();
        fs::create_dir_all(dir.join("textures")).unwrap();

        let mut registry = AssetRegistry::new();
        registry.insert(
            Uuid::from_raw(1001),
            super::super::AssetMetadata {
                file_path: "scenes/main.ggscene".to_string(),
                asset_type: AssetType::Scene,
            },
        );
        registry.insert(
            Uuid::from_raw(1002),
            super::super::AssetMetadata {
                file_path: "scenes/level2.ggscene".to_string(),
                asset_type: AssetType::Scene,
            },
        );
        registry.insert(
            Uuid::from_raw(2001),
            super::super::AssetMetadata {
                file_path: "textures/boss.png".to_string(),
                asset_type: AssetType::Texture2D,
            },
        );
        registry
            .save(&dir.join("AssetRegistry.ggregistry"))
            .unwrap();

        // Main scene has a script that loads level2.
        let scene1 = r#"Version: 2
Scene: main
Entities:
- Entity: 100
  NativeScriptComponent:
    ScriptPath: scripts/game.lua
"#;
        fs::write(dir.join("scenes/main.ggscene"), scene1).unwrap();

        // Level2 references a texture not referenced by main.
        let scene2 = r#"Version: 2
Scene: level2
Entities:
- Entity: 200
  SpriteRendererComponent:
    TextureHandle: 2001
"#;
        fs::write(dir.join("scenes/level2.ggscene"), scene2).unwrap();

        // Script loads level2 scene.
        fs::write(
            dir.join("scripts/game.lua"),
            r#"Engine.load_scene("scenes/level2.ggscene")"#,
        )
        .unwrap();

        fs::write(dir.join("textures/boss.png"), &[0u8; 50]).unwrap();

        let manifest = cook_assets(&dir, "scripts");

        let included_paths: HashSet<&str> =
            manifest.entries.iter().map(|e| e.path.as_str()).collect();
        // Should follow: main scene → script → load_scene → level2 → texture.
        assert!(included_paths.contains("scenes/main.ggscene"));
        assert!(included_paths.contains("scripts/game.lua"));
        assert!(included_paths.contains("scenes/level2.ggscene"));
        assert!(included_paths.contains("textures/boss.png"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cook_assets_warns_missing_file() {
        let dir = std::env::temp_dir().join("gg_cook_missing_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scenes")).unwrap();

        let mut registry = AssetRegistry::new();
        registry.insert(
            Uuid::from_raw(1001),
            super::super::AssetMetadata {
                file_path: "scenes/main.ggscene".to_string(),
                asset_type: AssetType::Scene,
            },
        );
        registry
            .save(&dir.join("AssetRegistry.ggregistry"))
            .unwrap();

        // Scene references a texture that doesn't exist on disk.
        let scene_yaml = r#"Version: 2
Scene: main
Entities:
- Entity: 100
  SpriteRendererComponent:
    TextureHandle: 9999
"#;
        fs::write(dir.join("scenes/main.ggscene"), scene_yaml).unwrap();

        let manifest = cook_assets(&dir, "scripts");

        // Should have a warning about handle 9999 not in registry.
        assert!(manifest.warnings.iter().any(|w| w.contains("9999")));

        let _ = fs::remove_dir_all(&dir);
    }
}
