/// Engine-wide error type.
///
/// Covers I/O, serialization, rendering, and scripting failures that were
/// previously swallowed via `Option`/`bool` + `log::error!`.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    #[error("{0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Gpu(String),

    #[error("{0}")]
    Asset(String),

    #[cfg(feature = "lua-scripting")]
    #[error("{0}")]
    Script(#[from] mlua::Error),

    #[error("{0}")]
    Audio(String),
}

/// Convenience alias used throughout the engine.
pub type EngineResult<T> = Result<T, EngineError>;
