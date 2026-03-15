//! Binary cache for preprocessed IBL cubemap data.
//!
//! Saves irradiance + prefiltered specular cubemaps to a `.ggenv` file next to
//! the source HDR. On subsequent loads the GPU compute chain is skipped
//! entirely — cached data is uploaded directly to the cubemap textures.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use gg_core::error::{EngineError, EngineResult};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAGIC: &[u8; 6] = b"GGENV\0";
const VERSION: u32 = 1;

/// Bytes per pixel for R16G16B16A16_SFLOAT.
const BPP: usize = 8;

// ---------------------------------------------------------------------------
// Cache header (serialized as raw bytes, little-endian)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CacheHeader {
    pub source_file_len: u64,
    pub source_mtime_secs: i64,
    pub irradiance_size: u32,
    pub prefilter_size: u32,
    pub prefilter_mips: u32,
}

impl CacheHeader {
    const BYTE_SIZE: usize = 6 + 4 + 8 + 8 + 4 + 4 + 4; // 38 bytes

    fn write_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&self.source_file_len.to_le_bytes())?;
        w.write_all(&self.source_mtime_secs.to_le_bytes())?;
        w.write_all(&self.irradiance_size.to_le_bytes())?;
        w.write_all(&self.prefilter_size.to_le_bytes())?;
        w.write_all(&self.prefilter_mips.to_le_bytes())?;
        Ok(())
    }

    fn read_from(r: &mut impl Read) -> std::io::Result<Option<Self>> {
        let mut magic = [0u8; 6];
        if r.read_exact(&mut magic).is_err() {
            return Ok(None);
        }
        if &magic != MAGIC {
            return Ok(None);
        }
        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

        r.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != VERSION {
            return Ok(None);
        }

        r.read_exact(&mut buf8)?;
        let source_file_len = u64::from_le_bytes(buf8);
        r.read_exact(&mut buf8)?;
        let source_mtime_secs = i64::from_le_bytes(buf8);
        r.read_exact(&mut buf4)?;
        let irradiance_size = u32::from_le_bytes(buf4);
        r.read_exact(&mut buf4)?;
        let prefilter_size = u32::from_le_bytes(buf4);
        r.read_exact(&mut buf4)?;
        let prefilter_mips = u32::from_le_bytes(buf4);

        Ok(Some(Self {
            source_file_len,
            source_mtime_secs,
            irradiance_size,
            prefilter_size,
            prefilter_mips,
        }))
    }
}

// ---------------------------------------------------------------------------
// Cached IBL data
// ---------------------------------------------------------------------------

/// In-memory representation of cached IBL textures ready for GPU upload.
pub struct CachedIblData {
    pub header: CacheHeader,
    /// Irradiance cubemap: 6 face blobs, each `irradiance_size² × BPP` bytes.
    pub irradiance_faces: Vec<Vec<u8>>,
    /// Prefiltered specular cubemap: `[mip][face]` blobs.
    pub prefilter_faces: Vec<Vec<Vec<u8>>>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Derive the `.ggenv` cache path from the source HDR path.
pub fn cache_path_for(source: &Path) -> PathBuf {
    source.with_extension("ggenv")
}

/// Check whether a valid cache exists for the given source HDR.
pub fn is_cache_valid(source: &Path) -> bool {
    let cache = cache_path_for(source);
    if !cache.exists() {
        return false;
    }
    match validate_cache(source, &cache) {
        Ok(valid) => valid,
        Err(_) => false,
    }
}

/// Load cached IBL data from disk. Returns `None` if the cache is missing,
/// invalid, or the source file has changed.
pub fn load_cache(source: &Path) -> EngineResult<Option<CachedIblData>> {
    let cache = cache_path_for(source);
    if !cache.exists() {
        return Ok(None);
    }

    let source_meta = fs::metadata(source)
        .map_err(|e| EngineError::Gpu(format!("Failed to stat source HDR: {e}")))?;

    let mut file = fs::File::open(&cache)
        .map_err(|e| EngineError::Gpu(format!("Failed to open cache: {e}")))?;

    let header = match CacheHeader::read_from(&mut file)
        .map_err(|e| EngineError::Gpu(format!("Failed to read cache header: {e}")))?
    {
        Some(h) => h,
        None => return Ok(None),
    };

    // Validate source hasn't changed.
    let source_mtime = source_meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if header.source_file_len != source_meta.len() || header.source_mtime_secs != source_mtime {
        log::info!(target: "gg_engine", "IBL cache invalidated (source changed)");
        return Ok(None);
    }

    // Read irradiance faces.
    let irr_face_bytes = (header.irradiance_size as usize).pow(2) * BPP;
    let mut irradiance_faces = Vec::with_capacity(6);
    for _ in 0..6 {
        let mut buf = vec![0u8; irr_face_bytes];
        file.read_exact(&mut buf)
            .map_err(|e| EngineError::Gpu(format!("Cache read irradiance: {e}")))?;
        irradiance_faces.push(buf);
    }

    // Read prefiltered faces (per mip, per face).
    let mut prefilter_faces = Vec::with_capacity(header.prefilter_mips as usize);
    for mip in 0..header.prefilter_mips {
        let mip_size = (header.prefilter_size >> mip).max(1) as usize;
        let face_bytes = mip_size * mip_size * BPP;
        let mut faces = Vec::with_capacity(6);
        for _ in 0..6 {
            let mut buf = vec![0u8; face_bytes];
            file.read_exact(&mut buf)
                .map_err(|e| EngineError::Gpu(format!("Cache read prefilter mip {mip}: {e}")))?;
            faces.push(buf);
        }
        prefilter_faces.push(faces);
    }

    log::info!(target: "gg_engine", "IBL cache loaded from {}", cache.display());
    Ok(Some(CachedIblData {
        header,
        irradiance_faces,
        prefilter_faces,
    }))
}

/// Save IBL cubemap data to the cache file.
pub fn save_cache(
    source: &Path,
    irradiance_size: u32,
    prefilter_size: u32,
    prefilter_mips: u32,
    irradiance_faces: &[Vec<u8>],
    prefilter_faces: &[Vec<Vec<u8>>],
) -> EngineResult<()> {
    let source_meta = fs::metadata(source)
        .map_err(|e| EngineError::Gpu(format!("Failed to stat source HDR: {e}")))?;

    let source_mtime = source_meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let header = CacheHeader {
        source_file_len: source_meta.len(),
        source_mtime_secs: source_mtime,
        irradiance_size,
        prefilter_size,
        prefilter_mips,
    };

    let cache = cache_path_for(source);
    let mut file = fs::File::create(&cache)
        .map_err(|e| EngineError::Gpu(format!("Failed to create cache file: {e}")))?;

    let mut writer = std::io::BufWriter::new(&mut file);
    header
        .write_to(&mut writer)
        .map_err(|e| EngineError::Gpu(format!("Failed to write cache header: {e}")))?;

    for face_data in irradiance_faces {
        writer
            .write_all(face_data)
            .map_err(|e| EngineError::Gpu(format!("Failed to write irradiance: {e}")))?;
    }
    for mip_faces in prefilter_faces {
        for face_data in mip_faces {
            writer
                .write_all(face_data)
                .map_err(|e| EngineError::Gpu(format!("Failed to write prefilter: {e}")))?;
        }
    }
    writer
        .flush()
        .map_err(|e| EngineError::Gpu(format!("Failed to flush cache: {e}")))?;

    let total_bytes: usize = CacheHeader::BYTE_SIZE
        + irradiance_faces.iter().map(|f| f.len()).sum::<usize>()
        + prefilter_faces
            .iter()
            .flat_map(|m| m.iter())
            .map(|f| f.len())
            .sum::<usize>();

    log::info!(
        target: "gg_engine",
        "IBL cache saved to {} ({:.1} MB)",
        cache.display(),
        total_bytes as f64 / (1024.0 * 1024.0),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validate_cache(source: &Path, cache: &Path) -> std::io::Result<bool> {
    let source_meta = fs::metadata(source)?;
    let mut file = fs::File::open(cache)?;
    let header = match CacheHeader::read_from(&mut file)? {
        Some(h) => h,
        None => return Ok(false),
    };
    let source_mtime = source_meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(header.source_file_len == source_meta.len() && header.source_mtime_secs == source_mtime)
}
