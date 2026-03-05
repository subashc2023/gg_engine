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
}

impl AssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssetType::None => "None",
            AssetType::Scene => "Scene",
            AssetType::Texture2D => "Texture2D",
            AssetType::Audio => "Audio",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Scene" => AssetType::Scene,
            "Texture2D" => AssetType::Texture2D,
            "Audio" => AssetType::Audio,
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

/// Determine asset type from a file extension.
pub fn asset_type_from_extension(ext: &str) -> AssetType {
    match ext.to_lowercase().as_str() {
        "png" | "jpg" | "jpeg" => AssetType::Texture2D,
        "ggscene" => AssetType::Scene,
        "wav" | "ogg" | "mp3" | "flac" => AssetType::Audio,
        _ => AssetType::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_type_round_trip() {
        for ty in [AssetType::None, AssetType::Scene, AssetType::Texture2D, AssetType::Audio] {
            assert_eq!(AssetType::from_str(ty.as_str()), ty);
        }
    }

    #[test]
    fn asset_type_from_str_unknown_returns_none() {
        assert_eq!(AssetType::from_str("Mesh"), AssetType::None);
        assert_eq!(AssetType::from_str(""), AssetType::None);
    }

    #[test]
    fn asset_type_display() {
        assert_eq!(format!("{}", AssetType::Texture2D), "Texture2D");
        assert_eq!(format!("{}", AssetType::Scene), "Scene");
        assert_eq!(format!("{}", AssetType::Audio), "Audio");
        assert_eq!(format!("{}", AssetType::None), "None");
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
    fn extension_to_type_unknown() {
        assert_eq!(asset_type_from_extension("txt"), AssetType::None);
        assert_eq!(asset_type_from_extension("lua"), AssetType::None);
        assert_eq!(asset_type_from_extension("rs"), AssetType::None);
        assert_eq!(asset_type_from_extension(""), AssetType::None);
    }
}
