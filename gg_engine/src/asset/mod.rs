mod asset_loader;
mod asset_manager;
mod asset_registry;

pub use asset_loader::{AssetLoader, LoadResult};
pub use asset_manager::EditorAssetManager;
pub use asset_registry::AssetRegistry;

use crate::uuid::Uuid;

/// Handle to an asset in the asset registry. An asset handle of 0 means "no asset".
pub type AssetHandle = Uuid;

/// The type of an asset, determined by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetType {
    None,
    Scene,
    Texture2D,
    Audio,
    Prefab,
    Material,
    Mesh,
}

impl AssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetType::None => "None",
            AssetType::Scene => "Scene",
            AssetType::Texture2D => "Texture2D",
            AssetType::Audio => "Audio",
            AssetType::Prefab => "Prefab",
            AssetType::Material => "Material",
            AssetType::Mesh => "Mesh",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "Scene" => AssetType::Scene,
            "Texture2D" => AssetType::Texture2D,
            "Audio" => AssetType::Audio,
            "Prefab" => AssetType::Prefab,
            "Material" => AssetType::Material,
            "Mesh" => AssetType::Mesh,
            _ => AssetType::None,
        }
    }
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata for a registered asset.
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    pub file_path: String,
    pub asset_type: AssetType,
}

/// Validate that a relative asset path does not escape the assets directory.
///
/// Rejects absolute paths, `..` components, and empty paths.
/// The path should already be normalized to forward slashes before calling.
pub fn validate_asset_path(relative_path: &str) -> bool {
    if relative_path.is_empty() {
        return false;
    }

    // Reject absolute paths (Unix or Windows style).
    if relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || relative_path.chars().nth(1) == Some(':')
    {
        return false;
    }

    // Reject any `..` path component that could escape the asset directory.
    // Split on both `/` and `\` for defense-in-depth (callers should normalize
    // to forward slashes, but we handle both just in case).
    for component in relative_path.split(['/', '\\']) {
        if component == ".." {
            return false;
        }
    }

    true
}

/// Determine asset type from a file extension.
pub fn asset_type_from_extension(ext: &str) -> AssetType {
    match ext.to_lowercase().as_str() {
        "png" | "jpg" | "jpeg" => AssetType::Texture2D,
        "ggscene" => AssetType::Scene,
        "wav" | "ogg" | "mp3" | "flac" => AssetType::Audio,
        "ggprefab" => AssetType::Prefab,
        "ggmaterial" => AssetType::Material,
        "gltf" | "glb" => AssetType::Mesh,
        _ => AssetType::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_type_round_trip() {
        for ty in [
            AssetType::None,
            AssetType::Scene,
            AssetType::Texture2D,
            AssetType::Audio,
            AssetType::Prefab,
            AssetType::Material,
            AssetType::Mesh,
        ] {
            assert_eq!(AssetType::parse_str(ty.as_str()), ty);
        }
    }

    #[test]
    fn asset_type_from_str_unknown_returns_none() {
        assert_eq!(AssetType::parse_str("Unknown"), AssetType::None);
        assert_eq!(AssetType::parse_str(""), AssetType::None);
        assert_eq!(AssetType::parse_str("Mesh"), AssetType::Mesh);
        assert_eq!(AssetType::parse_str("Material"), AssetType::Material);
    }

    #[test]
    fn asset_type_display() {
        assert_eq!(format!("{}", AssetType::Texture2D), "Texture2D");
        assert_eq!(format!("{}", AssetType::Scene), "Scene");
        assert_eq!(format!("{}", AssetType::Audio), "Audio");
        assert_eq!(format!("{}", AssetType::Prefab), "Prefab");
        assert_eq!(format!("{}", AssetType::Material), "Material");
        assert_eq!(format!("{}", AssetType::None), "None");
        assert_eq!(format!("{}", AssetType::Mesh), "Mesh");
    }

    #[test]
    fn extension_to_type_textures() {
        assert_eq!(asset_type_from_extension("png"), AssetType::Texture2D);
        assert_eq!(asset_type_from_extension("PNG"), AssetType::Texture2D);
        assert_eq!(asset_type_from_extension("jpg"), AssetType::Texture2D);
        assert_eq!(asset_type_from_extension("jpeg"), AssetType::Texture2D);
        assert_eq!(asset_type_from_extension("JPEG"), AssetType::Texture2D);
    }

    #[test]
    fn extension_to_type_scenes() {
        assert_eq!(asset_type_from_extension("ggscene"), AssetType::Scene);
    }

    #[test]
    fn extension_to_type_audio() {
        assert_eq!(asset_type_from_extension("wav"), AssetType::Audio);
        assert_eq!(asset_type_from_extension("ogg"), AssetType::Audio);
        assert_eq!(asset_type_from_extension("mp3"), AssetType::Audio);
        assert_eq!(asset_type_from_extension("flac"), AssetType::Audio);
    }

    #[test]
    fn extension_to_type_prefab() {
        assert_eq!(asset_type_from_extension("ggprefab"), AssetType::Prefab);
    }

    #[test]
    fn extension_to_type_material() {
        assert_eq!(asset_type_from_extension("ggmaterial"), AssetType::Material);
    }

    #[test]
    fn extension_to_type_mesh() {
        assert_eq!(asset_type_from_extension("gltf"), AssetType::Mesh);
        assert_eq!(asset_type_from_extension("GLTF"), AssetType::Mesh);
        assert_eq!(asset_type_from_extension("glb"), AssetType::Mesh);
        assert_eq!(asset_type_from_extension("GLB"), AssetType::Mesh);
    }

    #[test]
    fn extension_to_type_unknown() {
        assert_eq!(asset_type_from_extension("txt"), AssetType::None);
        assert_eq!(asset_type_from_extension("lua"), AssetType::None);
        assert_eq!(asset_type_from_extension("rs"), AssetType::None);
        assert_eq!(asset_type_from_extension(""), AssetType::None);
    }

    #[test]
    fn validate_path_accepts_normal_relative() {
        assert!(validate_asset_path("textures/player.png"));
        assert!(validate_asset_path("scenes/level1.ggscene"));
        assert!(validate_asset_path("music.ogg"));
        assert!(validate_asset_path("a/b/c/d.png"));
    }

    #[test]
    fn validate_path_rejects_empty() {
        assert!(!validate_asset_path(""));
    }

    #[test]
    fn validate_path_rejects_parent_traversal() {
        assert!(!validate_asset_path("../secret.txt"));
        assert!(!validate_asset_path("textures/../../etc/passwd"));
        assert!(!validate_asset_path("a/../b/../../../escape.txt"));
    }

    #[test]
    fn validate_path_rejects_absolute_unix() {
        assert!(!validate_asset_path("/etc/passwd"));
        assert!(!validate_asset_path("/home/user/file.png"));
    }

    #[test]
    fn validate_path_rejects_absolute_windows() {
        assert!(!validate_asset_path("C:\\Windows\\system32\\file.dll"));
        assert!(!validate_asset_path("D:\\data\\file.png"));
        assert!(!validate_asset_path("\\\\server\\share\\file.txt"));
    }

    #[test]
    fn validate_path_allows_dot_in_filenames() {
        // Single dots and dots in filenames are fine.
        assert!(validate_asset_path("file.name.png"));
        assert!(validate_asset_path("./textures/player.png"));
        assert!(validate_asset_path(".hidden/file.png"));
    }
}
