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
