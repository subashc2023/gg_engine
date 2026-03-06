//! Generates standard test textures for the engine.
//!
//! Run: `cargo run --example gen_textures -p gg_engine`
//!
//! Output goes to `assets/textures/`.

use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::Path;

fn main() {
    let out = Path::new("assets/textures");
    std::fs::create_dir_all(out).unwrap();

    generate_checkerboard(out);
    generate_circle(out);
    generate_default_placeholder(out);
    generate_color_grid(out);

    println!("Done — textures written to {}", out.display());
}

/// 256x256 checkerboard — 2x2 grid (4 tiles) so tiling factor maps to powers of 2.
/// Tiling 1.0 = 2x2, tiling 2.0 = 4x4, tiling 4.0 = 8x8, etc.
fn generate_checkerboard(out: &Path) {
    let size = 256u32;
    let cell = size / 2;
    let img = ImageBuffer::from_fn(size, size, |x, y| {
        let checker = ((x / cell) + (y / cell)).is_multiple_of(2);
        if checker {
            Rgba([255u8, 255, 255, 255])
        } else {
            Rgba([128u8, 128, 128, 255])
        }
    });
    let path = out.join("checkerboard.png");
    img.save(&path).unwrap();
    println!("  {}", path.display());
}

/// 128x128 soft white circle on a transparent background.
fn generate_circle(out: &Path) {
    let size = 128u32;
    let center = size as f32 / 2.0;
    let radius = center - 2.0; // 2px margin
    let softness = 2.0; // anti-alias band in pixels

    let img = ImageBuffer::from_fn(size, size, |x, y| {
        let dx = x as f32 - center;
        let dy = y as f32 - center;
        let dist = (dx * dx + dy * dy).sqrt();

        let alpha = if dist <= radius - softness {
            1.0
        } else if dist <= radius {
            1.0 - (dist - (radius - softness)) / softness
        } else {
            0.0
        };

        let a = (alpha * 255.0) as u8;
        Rgba([255u8, 255, 255, a])
    });
    let path = out.join("circle.png");
    img.save(&path).unwrap();
    println!("  {}", path.display());
}

/// 64x64 magenta/black checkerboard (2x2 grid) — the universal "missing texture" pattern.
fn generate_default_placeholder(out: &Path) {
    let size = 64u32;
    let cell = size / 2;
    let img = ImageBuffer::from_fn(size, size, |x, y| {
        let checker = ((x / cell) + (y / cell)).is_multiple_of(2);
        if checker {
            Rgba([255u8, 0, 255, 255]) // magenta
        } else {
            Rgba([0u8, 0, 0, 255]) // black
        }
    });
    let path = out.join("default.png");
    img.save(&path).unwrap();
    println!("  {}", path.display());
}

/// 256x256 sprite sheet — 4x4 grid of distinctly colored tiles.
/// Useful for testing SubTexture2D / atlas workflows.
fn generate_color_grid(out: &Path) {
    let size = 256u32;
    let cell = size / 4;

    #[rustfmt::skip]
    let colors: [[u8; 4]; 16] = [
        // Row 0
        [231,  76,  60, 255],  // red
        [ 46, 204, 113, 255],  // green
        [ 52, 152, 219, 255],  // blue
        [241, 196,  15, 255],  // yellow
        // Row 1
        [155,  89, 182, 255],  // purple
        [ 26, 188, 156, 255],  // teal
        [230, 126,  34, 255],  // orange
        [236, 240, 241, 255],  // light gray
        // Row 2
        [192,  57,  43, 255],  // dark red
        [ 39, 174,  96, 255],  // dark green
        [ 41, 128, 185, 255],  // dark blue
        [243, 156,  18, 255],  // dark yellow
        // Row 3
        [142,  68, 173, 255],  // dark purple
        [ 22, 160, 133, 255],  // dark teal
        [211,  84,   0, 255],  // dark orange
        [127, 140, 141, 255],  // medium gray
    ];

    let mut img: RgbaImage = ImageBuffer::new(size, size);

    for y in 0..size {
        for x in 0..size {
            let col = (x / cell) as usize;
            let row = (y / cell) as usize;
            let idx = row * 4 + col;

            // 1px dark border between cells
            let at_border = (x % cell == 0) || (y % cell == 0);
            let c: [u8; 4] = if at_border {
                [40, 40, 40, 255]
            } else {
                colors[idx]
            };

            img.put_pixel(x, y, Rgba(c));
        }
    }

    let path = out.join("color_grid.png");
    img.save(&path).unwrap();
    println!("  {}", path.display());
}
