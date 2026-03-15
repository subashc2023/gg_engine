pub use gg_core::cursor::CursorMode;

/// Custom software cursor appearance.
///
/// When provided via [`Application::software_cursor()`](crate::Application::software_cursor),
/// replaces the default arrow cursor in [`CursorMode::Confined`] mode.
#[derive(Clone)]
pub struct SoftwareCursor {
    /// The egui texture to draw as the cursor.
    pub texture: egui::TextureId,
    /// Display size in logical pixels.
    pub size: egui::Vec2,
    /// Hotspot offset from top-left of the texture (logical pixels).
    /// (0, 0) = top-left corner is the click point.
    pub hotspot: egui::Vec2,
}

/// Generate the default arrow cursor as a 32x32 RGBA texture.
///
/// Classic Windows-style arrow: black outline, white fill, transparent background.
/// Pixel-art at 32x32 looks crisp and is universally recognizable.
fn generate_cursor_image() -> egui::ColorImage {
    // Each string is one row. B = black, W = white, . = transparent.
    #[rustfmt::skip]
    const ROWS: [&str; 26] = [
        "B...............",
        "BB..............",
        "BWB.............",
        "BWWB............",
        "BWWWB...........",
        "BWWWWB..........",
        "BWWWWWB.........",
        "BWWWWWWB........",
        "BWWWWWWWB.......",
        "BWWWWWWWWB......",
        "BWWWWWWWWWB.....",
        "BWWWWWWWWWWB....",
        "BWWWWWWBBBBB....",
        "BWWWBWWB........",
        "BWWBBWWB........",
        "BWB..BWWB.......",
        "BB...BWWB.......",
        "B.....BWWB......",
        "......BWWB......",
        ".......BWWB.....",
        ".......BWWB.....",
        "........BB......",
        "................",
        "................",
        "................",
        "................",
    ];

    let w = 16_usize;
    let h = 32_usize;
    let mut pixels = vec![egui::Color32::TRANSPARENT; w * h];
    for (y, row) in ROWS.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if x < w {
                pixels[y * w + x] = match ch {
                    'B' => egui::Color32::from_rgba_unmultiplied(0, 0, 0, 255),
                    'W' => egui::Color32::WHITE,
                    _ => egui::Color32::TRANSPARENT,
                };
            }
        }
    }
    // Fill remaining rows with transparent.
    egui::ColorImage::new([w, h], pixels)
}

/// Ensure the cursor texture is loaded into egui and return its texture ID.
///
/// Stores the `TextureHandle` (RAII guard) in egui's persistent data so the
/// texture stays alive across frames.
fn ensure_cursor_texture(ctx: &egui::Context) -> egui::TextureId {
    let id = egui::Id::new("__gg_default_cursor_tex");
    // Check if already loaded (read the handle, return its id).
    if let Some(handle) = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<egui::TextureHandle>>(id)) {
        return handle.id();
    }
    let image = generate_cursor_image();
    let handle = ctx.load_texture(
        "__gg_default_cursor",
        image,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Nearest,
            ..Default::default()
        },
    );
    let tex_id = handle.id();
    // Wrap in Arc so it's Clone (required by egui data storage) and persists.
    ctx.data_mut(|d| d.insert_temp(id, std::sync::Arc::new(handle)));
    tex_id
}

/// Draw the default arrow software cursor at the given screen position.
pub(crate) fn draw_default_cursor(ctx: &egui::Context, pos: (f64, f64)) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("__gg_software_cursor"),
    ));
    let tex_id = ensure_cursor_texture(ctx);
    let p = egui::pos2(pos.0 as f32, pos.1 as f32);
    // Render at native pixel size (16x32 logical pixels).
    let size = egui::vec2(16.0, 32.0);
    let rect = egui::Rect::from_min_size(p, size);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(tex_id, rect, uv, egui::Color32::WHITE);
}

/// Draw a custom texture software cursor at the given screen position.
pub(crate) fn draw_custom_cursor(ctx: &egui::Context, pos: (f64, f64), cursor: &SoftwareCursor) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("__gg_software_cursor"),
    ));
    let p = egui::pos2(pos.0 as f32, pos.1 as f32) - cursor.hotspot;
    let rect = egui::Rect::from_min_size(p, cursor.size);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(cursor.texture, rect, uv, egui::Color32::WHITE);
}
