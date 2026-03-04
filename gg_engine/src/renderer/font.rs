use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::gpu_allocation::GpuAllocator;
use super::msdf;
use super::texture::{Texture2D, TextureSpecification};
use super::RendererResources;
use crate::profiling::ProfileTimer;

// ---------------------------------------------------------------------------
// GlyphInfo — per-glyph layout and UV data
// ---------------------------------------------------------------------------

/// Per-glyph metrics and atlas UV coordinates.
#[derive(Clone, Debug)]
pub struct GlyphInfo {
    /// UV coordinates for the 4 corners of the glyph quad in the atlas.
    /// Order: top-left, top-right, bottom-right, bottom-left.
    pub tex_coords: [[f32; 2]; 4],
    /// Horizontal advance (in normalized font units).
    pub advance_x: f32,
    /// Horizontal bearing (offset from cursor to left edge of glyph).
    pub bearing_x: f32,
    /// Vertical bearing (offset from baseline to top edge of glyph).
    pub bearing_y: f32,
    /// Width of the glyph in normalized font units.
    pub width: f32,
    /// Height of the glyph in normalized font units.
    pub height: f32,
}

// ---------------------------------------------------------------------------
// FontCpuData — CPU-side font data (Send-safe, no Vulkan types)
// ---------------------------------------------------------------------------

/// CPU-side font data produced by MSDF generation. Suitable for background
/// thread production. Contains everything needed to create a [`Font`] on the
/// main thread via [`Font::from_cpu_data`].
pub struct FontCpuData {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub atlas_pixels: Vec<u8>,
    pub glyphs: HashMap<char, GlyphInfo>,
    pub kerning_pairs: HashMap<(char, char), f32>,
    pub line_height: f32,
    pub ascender: f32,
    pub descender: f32,
}

/// Load a TTF font file, parse it, generate MSDF glyphs, and pack the atlas.
/// Returns CPU-only data — no Vulkan calls. Safe to call on a background thread.
pub(crate) fn generate_font_cpu_data(path: &Path) -> Result<FontCpuData, String> {
    let _timer = ProfileTimer::new("generate_font_cpu_data");

    let font_data = std::fs::read(path)
        .map_err(|e| format!("Failed to read font '{}': {e}", path.display()))?;

    let face = ttf_parser::Face::parse(&font_data, 0)
        .map_err(|e| format!("Failed to parse font '{}': {e}", path.display()))?;

    let units_per_em = face.units_per_em() as f64;
    let ascender = face.ascender() as f64 / units_per_em;
    let descender = face.descender() as f64 / units_per_em;
    let line_gap = face.line_gap() as f64 / units_per_em;
    let line_height = ascender - descender + line_gap;

    // Character set ranges (Basic Latin + Latin-1 Supplement).
    const CHAR_RANGES: &[(u32, u32)] = &[
        (0x0020, 0x007E),
        (0x00A0, 0x00FF),
    ];

    let mut chars: Vec<char> = Vec::new();
    for &(start, end) in CHAR_RANGES {
        for cp in start..=end {
            if let Some(ch) = char::from_u32(cp) {
                if face.glyph_index(ch).is_some() {
                    chars.push(ch);
                }
            }
        }
    }

    // Per-glyph MSDF generation.
    struct MsdfGlyph {
        ch: char,
        advance_x: f64,
        bearing_x: f64,
        bearing_y: f64,
        glyph_w: f64,
        glyph_h: f64,
        bitmap: Option<Vec<u8>>,
    }

    let mut msdf_glyphs: Vec<MsdfGlyph> = Vec::with_capacity(chars.len());

    for &ch in &chars {
        let glyph_id = face.glyph_index(ch).unwrap();
        let advance_x = face.glyph_hor_advance(glyph_id).unwrap_or(0) as f64 / units_per_em;

        let mut builder = msdf::OutlineBuilder::new();
        let bbox = face.outline_glyph(glyph_id, &mut builder);

        if bbox.is_none() {
            msdf_glyphs.push(MsdfGlyph {
                ch,
                advance_x,
                bearing_x: 0.0,
                bearing_y: 0.0,
                glyph_w: 0.0,
                glyph_h: 0.0,
                bitmap: None,
            });
            continue;
        }

        let mut shape = builder.build();

        let frame = msdf::autoframe(&shape, MSDF_GLYPH_SIZE, MSDF_GLYPH_SIZE, MSDF_RANGE_PX);
        if frame.is_none() {
            msdf_glyphs.push(MsdfGlyph {
                ch,
                advance_x,
                bearing_x: 0.0,
                bearing_y: 0.0,
                glyph_w: 0.0,
                glyph_h: 0.0,
                bitmap: None,
            });
            continue;
        }
        let (scale, translate) = frame.unwrap();

        let (min_x, min_y, max_x, max_y) = msdf::shape_bounds(&shape).unwrap();
        let range_fu_x = if scale.x.abs() > 1e-10 { MSDF_RANGE_PX / scale.x } else { 0.0 };
        let range_fu_y = if scale.y.abs() > 1e-10 { MSDF_RANGE_PX / scale.y } else { 0.0 };

        let bearing_x = (min_x - range_fu_x) / units_per_em;
        let bearing_y = (max_y + range_fu_y) / units_per_em;
        let glyph_w = (max_x - min_x + 2.0 * range_fu_x) / units_per_em;
        let glyph_h = (max_y - min_y + 2.0 * range_fu_y) / units_per_em;

        let bitmap = msdf::generate_msdf(
            &mut shape,
            MSDF_GLYPH_SIZE,
            MSDF_GLYPH_SIZE,
            MSDF_RANGE_PX,
            scale,
            translate,
        );

        msdf_glyphs.push(MsdfGlyph {
            ch,
            advance_x,
            bearing_x,
            bearing_y,
            glyph_w,
            glyph_h,
            bitmap: Some(bitmap),
        });
    }

    // Atlas packing: fixed MSDF_GLYPH_SIZE cells with padding.
    let cell_w = MSDF_GLYPH_SIZE + GLYPH_PADDING * 2;
    let cell_h = MSDF_GLYPH_SIZE + GLYPH_PADDING * 2;
    let visible_count = msdf_glyphs.iter().filter(|g| g.bitmap.is_some()).count() as u32;
    let cols = if visible_count == 0 { 1 } else { (visible_count as f64).sqrt().ceil() as u32 };
    let rows = if visible_count == 0 { 1 } else { (visible_count + cols - 1) / cols };
    let atlas_width = (cols * cell_w).max(1);
    let atlas_height = (rows * cell_h).max(1);

    let mut atlas_pixels = vec![0u8; (atlas_width * atlas_height * 4) as usize];
    let mut glyphs = HashMap::new();
    let mut visible_idx = 0u32;

    for glyph in &msdf_glyphs {
        if let Some(ref bitmap_data) = glyph.bitmap {
            let col = visible_idx % cols;
            let row = visible_idx / cols;
            let cell_x = col * cell_w + GLYPH_PADDING;
            let cell_y = row * cell_h + GLYPH_PADDING;

            for py in 0..MSDF_GLYPH_SIZE {
                let src_start = (py * MSDF_GLYPH_SIZE * 4) as usize;
                for px in 0..MSDF_GLYPH_SIZE {
                    let src_offset = src_start + (px * 4) as usize;
                    let dest_x = cell_x + px;
                    let dest_y = cell_y + py;
                    let dest_offset = ((dest_y * atlas_width + dest_x) * 4) as usize;
                    atlas_pixels[dest_offset..dest_offset + 4]
                        .copy_from_slice(&bitmap_data[src_offset..src_offset + 4]);
                }
            }

            let u0 = cell_x as f32 / atlas_width as f32;
            let v0 = cell_y as f32 / atlas_height as f32;
            let u1 = (cell_x + MSDF_GLYPH_SIZE) as f32 / atlas_width as f32;
            let v1 = (cell_y + MSDF_GLYPH_SIZE) as f32 / atlas_height as f32;

            glyphs.insert(
                glyph.ch,
                GlyphInfo {
                    tex_coords: [
                        [u0, v0],
                        [u1, v0],
                        [u1, v1],
                        [u0, v1],
                    ],
                    advance_x: glyph.advance_x as f32,
                    bearing_x: glyph.bearing_x as f32,
                    bearing_y: glyph.bearing_y as f32,
                    width: glyph.glyph_w as f32,
                    height: glyph.glyph_h as f32,
                },
            );
            visible_idx += 1;
        } else {
            glyphs.insert(
                glyph.ch,
                GlyphInfo {
                    tex_coords: [[0.0; 2]; 4],
                    advance_x: glyph.advance_x as f32,
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
            );
        }
    }

    // Extract kerning pairs.
    let mut kerning_pairs = HashMap::new();
    if let Some(kern_table) = face.tables().kern {
        for subtable in kern_table.subtables {
            if subtable.horizontal && !subtable.variable {
                for &left in &chars {
                    if let Some(left_gid) = face.glyph_index(left) {
                        for &right in &chars {
                            if let Some(right_gid) = face.glyph_index(right) {
                                if let Some(kern) = subtable.glyphs_kerning(left_gid, right_gid) {
                                    let kern_norm = kern as f64 / units_per_em;
                                    if kern_norm.abs() > f64::EPSILON {
                                        kerning_pairs.insert((left, right), kern_norm as f32);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    log::info!(
        "Font CPU data generated: '{}' — {} glyphs, {} kerning pairs, atlas {}x{} (MSDF)",
        path.display(),
        glyphs.len(),
        kerning_pairs.len(),
        atlas_width,
        atlas_height,
    );

    Ok(FontCpuData {
        atlas_width,
        atlas_height,
        atlas_pixels,
        glyphs,
        kerning_pairs,
        line_height: line_height as f32,
        ascender: ascender as f32,
        descender: descender as f32,
    })
}

// ---------------------------------------------------------------------------
// Font — MSDF font atlas
// ---------------------------------------------------------------------------

/// A font loaded from a TTF file with an MSDF atlas for resolution-independent rendering.
///
/// Contains a texture atlas with MSDF-rendered glyphs and per-character
/// layout information for text rendering.
pub struct Font {
    /// The MSDF atlas texture (R8G8B8A8_UNORM, LINEAR filtering).
    pub(crate) atlas_texture: Texture2D,
    /// Per-character glyph information.
    glyphs: HashMap<char, GlyphInfo>,
    /// Kerning pairs: (left, right) -> horizontal kern offset in normalized font units.
    kerning_pairs: HashMap<(char, char), f32>,
    /// Line height in normalized font units.
    pub line_height: f32,
    /// Ascender in normalized font units.
    pub ascender: f32,
    /// Descender in normalized font units (typically negative).
    pub descender: f32,
    /// Atlas texture width in pixels.
    pub atlas_width: u32,
    /// Atlas texture height in pixels.
    pub atlas_height: u32,
}

/// Size in pixels for each MSDF glyph cell. MSDF needs far less resolution
/// than single-channel SDF — 48px gives excellent quality.
const MSDF_GLYPH_SIZE: u32 = 48;
/// Padding around each glyph in the atlas (in pixels).
const GLYPH_PADDING: u32 = 2;
/// MSDF distance field range in pixels.
const MSDF_RANGE_PX: f64 = 4.0;

impl Font {
    /// Create a Font from pre-generated CPU data (GPU upload only).
    /// Call on the main thread after [`generate_font_cpu_data`] produced the data.
    pub(crate) fn from_cpu_data(
        res: &RendererResources<'_>,
        allocator: &Arc<Mutex<GpuAllocator>>,
        data: FontCpuData,
    ) -> Self {
        let atlas_texture = Texture2D::from_rgba8_with_spec(
            res,
            allocator,
            data.atlas_width,
            data.atlas_height,
            &data.atlas_pixels,
            &TextureSpecification::font_atlas(),
        );

        Self {
            atlas_texture,
            glyphs: data.glyphs,
            kerning_pairs: data.kerning_pairs,
            line_height: data.line_height,
            ascender: data.ascender,
            descender: data.descender,
            atlas_width: data.atlas_width,
            atlas_height: data.atlas_height,
        }
    }

    /// Load a font from a TTF file and generate an MSDF atlas (synchronous).
    /// Calls [`generate_font_cpu_data`] then [`Font::from_cpu_data`].
    pub(crate) fn load(res: &RendererResources<'_>, allocator: &Arc<Mutex<GpuAllocator>>, path: &Path) -> Self {
        let cpu_data = generate_font_cpu_data(path)
            .unwrap_or_else(|e| panic!("{e}"));
        Self::from_cpu_data(res, allocator, cpu_data)
    }

    /// Look up glyph information for a character.
    pub fn glyph(&self, ch: char) -> Option<&GlyphInfo> {
        self.glyphs.get(&ch)
    }

    /// Get the kerning offset between two characters (in normalized font units).
    /// Returns 0.0 if no kerning pair exists.
    pub fn kerning(&self, left: char, right: char) -> f32 {
        self.kerning_pairs.get(&(left, right)).copied().unwrap_or(0.0)
    }

    /// The bindless texture index for the font atlas.
    pub fn bindless_index(&self) -> u32 {
        self.atlas_texture.bindless_index()
    }
}
