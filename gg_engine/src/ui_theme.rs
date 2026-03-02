use egui::{
    epaint::Shadow, style::WidgetVisuals, Color32, FontData, FontDefinitions, FontFamily, FontId,
    Stroke, Style, TextStyle, Visuals,
};
use std::sync::Arc;

/// Font family name for the bold weight, usable with
/// `FontFamily::Name(BOLD_FONT.into())` or `RichText::new(...).font(...)`.
pub const BOLD_FONT: &str = "JetBrainsMono-Bold";

/// Apply the engine-wide dark theme and JetBrains Mono fonts to an egui context.
/// Called once during engine initialization, before the first frame.
pub fn apply_engine_theme(ctx: &egui::Context) {
    configure_fonts(ctx);
    configure_style(ctx);
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::empty();

    // Regular weight — used for Proportional and Monospace.
    fonts.font_data.insert(
        "JetBrainsMono-Regular".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Regular.ttf"
        ))),
    );

    // Bold weight — registered under a custom family name.
    fonts.font_data.insert(
        BOLD_FONT.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/JetBrainsMono-Bold.ttf"
        ))),
    );

    // Proportional family → Regular (primary).
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .push("JetBrainsMono-Regular".to_owned());

    // Monospace family → Regular (primary).
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("JetBrainsMono-Regular".to_owned());

    // Custom "Bold" family → Bold (primary), Regular (fallback).
    fonts
        .families
        .entry(FontFamily::Name(BOLD_FONT.into()))
        .or_default()
        .extend([BOLD_FONT.to_owned(), "JetBrainsMono-Regular".to_owned()]);

    ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------------------
// Style / Visuals  (VS Code Dark+ inspired)
// ---------------------------------------------------------------------------

fn configure_style(ctx: &egui::Context) {
    let mut style = Style {
        text_styles: [
            (
                TextStyle::Heading,
                FontId::new(18.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(14.0, FontFamily::Monospace),
            ),
        ]
        .into(),
        ..Style::default()
    };

    // -- Spacing -------------------------------------------------------------
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(6.0, 3.0);

    // -- Visuals (dark theme) ------------------------------------------------
    let mut visuals = Visuals::dark();

    // Core backgrounds.
    visuals.panel_fill = hex_color(0x1E1E1E); // editor.background
    visuals.window_fill = hex_color(0x252526); // sideBar.background
    visuals.extreme_bg_color = hex_color(0x121212); // text edits, tab bar bg
    visuals.faint_bg_color = hex_color(0x252526);

    // Code background.
    visuals.code_bg_color = hex_color(0x1E1E1E);

    // Selection.
    visuals.selection.bg_fill = hex_color(0x264F78); // editor.selectionBackground
    visuals.selection.stroke = Stroke::new(1.0, hex_color(0x007ACC));

    // Hyperlinks.
    visuals.hyperlink_color = hex_color(0x3794FF);

    // Window chrome.
    visuals.window_stroke = Stroke::new(1.0, hex_color(0x3C3C3C));
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow {
        spread: 0,
        blur: 8,
        offset: [0, 2],
        color: Color32::from_black_alpha(96),
    };

    // Collapsing headers get a visible frame.
    visuals.collapsing_header_frame = true;

    // -- Widget visuals ------------------------------------------------------

    let corner = egui::CornerRadius::same(3);

    // Non-interactive (labels, separators, panel borders).
    visuals.widgets.noninteractive = WidgetVisuals {
        bg_fill: hex_color(0x1E1E1E),
        weak_bg_fill: hex_color(0x1E1E1E),
        bg_stroke: Stroke::new(1.0, hex_color(0x3C3C3C)),
        fg_stroke: Stroke::new(1.0, hex_color(0xCCCCCC)),
        corner_radius: corner,
        expansion: 0.0,
    };

    // Inactive (buttons at rest).
    visuals.widgets.inactive = WidgetVisuals {
        bg_fill: hex_color(0x333436),
        weak_bg_fill: hex_color(0x333436),
        bg_stroke: Stroke::NONE,
        fg_stroke: Stroke::new(1.0, hex_color(0xCCCCCC)),
        corner_radius: corner,
        expansion: 0.0,
    };

    // Hovered.
    visuals.widgets.hovered = WidgetVisuals {
        bg_fill: hex_color(0x454547),
        weak_bg_fill: hex_color(0x454547),
        bg_stroke: Stroke::new(1.0, hex_color(0x007ACC)),
        fg_stroke: Stroke::new(1.5, Color32::WHITE),
        corner_radius: corner,
        expansion: 1.0,
    };

    // Active (pressed / dragging).
    visuals.widgets.active = WidgetVisuals {
        bg_fill: hex_color(0x007ACC),
        weak_bg_fill: hex_color(0x007ACC),
        bg_stroke: Stroke::new(1.0, Color32::WHITE),
        fg_stroke: Stroke::new(2.0, Color32::WHITE),
        corner_radius: corner,
        expansion: 1.0,
    };

    // Open (e.g. combo-box with menu open).
    visuals.widgets.open = WidgetVisuals {
        bg_fill: hex_color(0x333436),
        weak_bg_fill: hex_color(0x333436),
        bg_stroke: Stroke::new(1.0, hex_color(0x007ACC)),
        fg_stroke: Stroke::new(1.0, Color32::WHITE),
        corner_radius: corner,
        expansion: 0.0,
    };

    style.visuals = visuals;
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a 24-bit hex color (0xRRGGBB) to an opaque `Color32`.
const fn hex_color(hex: u32) -> Color32 {
    Color32::from_rgb(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}
