use glam::Vec2;

use super::texture::Texture2D;

// ---------------------------------------------------------------------------
// SubTexture2D
// ---------------------------------------------------------------------------

/// A sub-region of a [`Texture2D`], defined by pre-computed texture coordinates.
///
/// Used for sprite sheets / texture atlases. Stores the bindless index of the
/// parent texture and four UV coordinates that define the sub-region. The parent
/// texture must outlive any draw calls using this sub-texture.
///
/// Create via [`SubTexture2D::new`] for explicit UV bounds, or
/// [`SubTexture2D::from_coords`] for grid-based sprite sheet access.
pub struct SubTexture2D {
    tex_coords: [[f32; 2]; 4],
    bindless_index: u32,
}

impl SubTexture2D {
    /// Create a sub-texture from explicit normalized texture coordinates.
    ///
    /// `min` is the top-left UV corner and `max` is the bottom-right UV corner,
    /// both in the range `[0.0, 1.0]`.
    pub fn new(texture: &Texture2D, min: Vec2, max: Vec2) -> Self {
        // Vertex order: top-left, top-right, bottom-right, bottom-left
        // (matches QUAD_TEX_COORDS / QUAD_POSITIONS in renderer.rs)
        let tex_coords = [
            [min.x, min.y], // top-left
            [max.x, min.y], // top-right
            [max.x, max.y], // bottom-right
            [min.x, max.y], // bottom-left
        ];

        Self {
            tex_coords,
            bindless_index: texture.bindless_index(),
        }
    }

    /// Create a sub-texture from grid coordinates in a sprite sheet.
    ///
    /// - `texture`: the sprite sheet texture
    /// - `coords`: grid position (column, row), zero-indexed from top-left
    /// - `cell_size`: size of each grid cell in pixels (e.g. `Vec2::new(128.0, 128.0)`)
    /// - `sprite_size`: how many cells this sprite spans (e.g. `Vec2::ONE` for a
    ///   single cell, `Vec2::new(1.0, 2.0)` for a sprite that is 1 cell wide
    ///   and 2 cells tall)
    pub fn from_coords(
        texture: &Texture2D,
        coords: Vec2,
        cell_size: Vec2,
        sprite_size: Vec2,
    ) -> Self {
        let tw = texture.width() as f32;
        let th = texture.height() as f32;

        let min = Vec2::new((coords.x * cell_size.x) / tw, (coords.y * cell_size.y) / th);
        let max = Vec2::new(
            ((coords.x + sprite_size.x) * cell_size.x) / tw,
            ((coords.y + sprite_size.y) * cell_size.y) / th,
        );

        Self::new(texture, min, max)
    }

    /// The four pre-computed texture coordinates (top-left, top-right,
    /// bottom-right, bottom-left).
    pub fn tex_coords(&self) -> &[[f32; 2]; 4] {
        &self.tex_coords
    }

    /// The bindless descriptor array index of the parent texture.
    pub fn bindless_index(&self) -> u32 {
        self.bindless_index
    }
}
