use egui::{
    epaint::Shadow, style::WidgetVisuals, Color32, FontData, FontDefinitions, FontFamily, FontId,
    Stroke, Style, TextStyle, Visuals,
};
use std::sync::Arc;

/// Font family name for the bold weight, usable with
/// `FontFamily::Name(BOLD_FONT.into())` or `RichText::new(...).font(...)`.
pub const BOLD_FONT: &str = "JetBrainsMono-Bold";

/// Available editor color themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum EditorTheme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

impl EditorTheme {
    pub const ALL: &'static [EditorTheme] = &[
        EditorTheme::Dark,
        EditorTheme::Light,
        EditorTheme::HighContrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EditorTheme::Dark => "Dark",
            EditorTheme::Light => "Light",
            EditorTheme::HighContrast => "High Contrast",
        }
    }
}

/// Apply the engine-wide dark theme and JetBrains Mono fonts to an egui context.
/// Called once during engine initialization, before the first frame.
pub fn apply_engine_theme(ctx: &egui::Context) {
    configure_fonts(ctx);
    apply_theme_colors(ctx, &DARK_THEME);
}

/// Apply a specific theme to the egui context (fonts are not changed).
pub fn apply_theme(ctx: &egui::Context, theme: EditorTheme) {
    match theme {
        EditorTheme::Dark => apply_theme_colors(ctx, &DARK_THEME),
        EditorTheme::Light => apply_theme_colors(ctx, &LIGHT_THEME),
        EditorTheme::HighContrast => apply_theme_colors(ctx, &HIGH_CONTRAST_THEME),
    }
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
// Data-driven theme system
// ---------------------------------------------------------------------------

/// Color and stroke parameters for a single widget state.
struct WidgetColors {
    bg: u32,
    weak_bg: u32,
    /// `None` means `Stroke::NONE`.
    bg_stroke: Option<(u32, f32)>,
    fg_color: u32,
    fg_width: f32,
    expansion: f32,
}

/// All color/stroke values that vary between themes.
struct ThemeColors {
    /// `true` → `Visuals::dark()`, `false` → `Visuals::light()`.
    dark_base: bool,

    // Core backgrounds.
    panel_fill: u32,
    window_fill: u32,
    extreme_bg: u32,
    faint_bg: u32,
    code_bg: u32,

    // Selection.
    selection_bg: u32,
    selection_stroke_color: u32,
    selection_stroke_width: f32,

    // Hyperlinks.
    hyperlink: u32,

    // Window chrome.
    window_stroke_color: u32,
    window_stroke_width: f32,
    /// `Some((blur, alpha))` for a popup shadow, `None` for `Shadow::NONE`.
    popup_shadow: Option<(u8, u8)>,

    // Widget visuals per state.
    noninteractive: WidgetColors,
    inactive: WidgetColors,
    hovered: WidgetColors,
    active: WidgetColors,
    open: WidgetColors,
}

// ---------------------------------------------------------------------------
// Theme definitions
// ---------------------------------------------------------------------------

static DARK_THEME: ThemeColors = ThemeColors {
    dark_base: true,
    panel_fill: 0x1E1E1E,
    window_fill: 0x252526,
    extreme_bg: 0x121212,
    faint_bg: 0x252526,
    code_bg: 0x1E1E1E,
    selection_bg: 0x264F78,
    selection_stroke_color: 0x007ACC,
    selection_stroke_width: 1.0,
    hyperlink: 0x3794FF,
    window_stroke_color: 0x3C3C3C,
    window_stroke_width: 1.0,
    popup_shadow: Some((8, 96)),
    noninteractive: WidgetColors {
        bg: 0x1E1E1E,
        weak_bg: 0x1E1E1E,
        bg_stroke: Some((0x3C3C3C, 1.0)),
        fg_color: 0xCCCCCC,
        fg_width: 1.0,
        expansion: 0.0,
    },
    inactive: WidgetColors {
        bg: 0x333436,
        weak_bg: 0x333436,
        bg_stroke: None,
        fg_color: 0xCCCCCC,
        fg_width: 1.0,
        expansion: 0.0,
    },
    hovered: WidgetColors {
        bg: 0x454547,
        weak_bg: 0x454547,
        bg_stroke: Some((0x007ACC, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 1.5,
        expansion: 1.0,
    },
    active: WidgetColors {
        bg: 0x007ACC,
        weak_bg: 0x007ACC,
        bg_stroke: Some((0xFFFFFF, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 2.0,
        expansion: 1.0,
    },
    open: WidgetColors {
        bg: 0x333436,
        weak_bg: 0x333436,
        bg_stroke: Some((0x007ACC, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 1.0,
        expansion: 0.0,
    },
};

static LIGHT_THEME: ThemeColors = ThemeColors {
    dark_base: false,
    panel_fill: 0xF3F3F3,
    window_fill: 0xFFFFFF,
    extreme_bg: 0xFFFFFF,
    faint_bg: 0xF3F3F3,
    code_bg: 0xE8E8E8,
    selection_bg: 0xADD6FF,
    selection_stroke_color: 0x0066B8,
    selection_stroke_width: 1.0,
    hyperlink: 0x006AB1,
    window_stroke_color: 0xCCCCCC,
    window_stroke_width: 1.0,
    popup_shadow: Some((8, 32)),
    noninteractive: WidgetColors {
        bg: 0xF3F3F3,
        weak_bg: 0xF3F3F3,
        bg_stroke: Some((0xCCCCCC, 1.0)),
        fg_color: 0x333333,
        fg_width: 1.0,
        expansion: 0.0,
    },
    inactive: WidgetColors {
        bg: 0xE0E0E0,
        weak_bg: 0xE0E0E0,
        bg_stroke: None,
        fg_color: 0x333333,
        fg_width: 1.0,
        expansion: 0.0,
    },
    hovered: WidgetColors {
        bg: 0xD0D0D0,
        weak_bg: 0xD0D0D0,
        bg_stroke: Some((0x0066B8, 1.0)),
        fg_color: 0x000000,
        fg_width: 1.5,
        expansion: 1.0,
    },
    active: WidgetColors {
        bg: 0x0066B8,
        weak_bg: 0x0066B8,
        bg_stroke: Some((0x000000, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 2.0,
        expansion: 1.0,
    },
    open: WidgetColors {
        bg: 0xE0E0E0,
        weak_bg: 0xE0E0E0,
        bg_stroke: Some((0x0066B8, 1.0)),
        fg_color: 0x000000,
        fg_width: 1.0,
        expansion: 0.0,
    },
};

static HIGH_CONTRAST_THEME: ThemeColors = ThemeColors {
    dark_base: true,
    panel_fill: 0x000000,
    window_fill: 0x000000,
    extreme_bg: 0x000000,
    faint_bg: 0x0A0A0A,
    code_bg: 0x000000,
    selection_bg: 0x264F78,
    selection_stroke_color: 0xFFFFFF,
    selection_stroke_width: 2.0,
    hyperlink: 0x3794FF,
    window_stroke_color: 0xFFFFFF,
    window_stroke_width: 2.0,
    popup_shadow: None,
    noninteractive: WidgetColors {
        bg: 0x000000,
        weak_bg: 0x000000,
        bg_stroke: Some((0xFFFFFF, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 1.0,
        expansion: 0.0,
    },
    inactive: WidgetColors {
        bg: 0x1A1A1A,
        weak_bg: 0x1A1A1A,
        bg_stroke: Some((0xFFFFFF, 1.0)),
        fg_color: 0xFFFFFF,
        fg_width: 1.0,
        expansion: 0.0,
    },
    hovered: WidgetColors {
        bg: 0x2A2A2A,
        weak_bg: 0x2A2A2A,
        bg_stroke: Some((0x00FFFF, 2.0)),
        fg_color: 0xFFFFFF,
        fg_width: 2.0,
        expansion: 1.0,
    },
    active: WidgetColors {
        bg: 0x00FFFF,
        weak_bg: 0x00FFFF,
        bg_stroke: Some((0xFFFFFF, 2.0)),
        fg_color: 0x000000,
        fg_width: 2.0,
        expansion: 1.0,
    },
    open: WidgetColors {
        bg: 0x1A1A1A,
        weak_bg: 0x1A1A1A,
        bg_stroke: Some((0x00FFFF, 2.0)),
        fg_color: 0xFFFFFF,
        fg_width: 1.0,
        expansion: 0.0,
    },
};

// ---------------------------------------------------------------------------
// Theme application
// ---------------------------------------------------------------------------

fn build_widget_visuals(w: &WidgetColors, corner: egui::CornerRadius) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: hex_color(w.bg),
        weak_bg_fill: hex_color(w.weak_bg),
        bg_stroke: match w.bg_stroke {
            Some((color, width)) => Stroke::new(width, hex_color(color)),
            None => Stroke::NONE,
        },
        fg_stroke: Stroke::new(w.fg_width, hex_color(w.fg_color)),
        corner_radius: corner,
        expansion: w.expansion,
    }
}

fn apply_theme_colors(ctx: &egui::Context, colors: &ThemeColors) {
    let mut style = base_text_style();

    // -- Spacing (identical across all themes) --------------------------------
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(6.0, 3.0);

    // -- Visuals --------------------------------------------------------------
    let mut visuals = if colors.dark_base {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    // Core backgrounds.
    visuals.panel_fill = hex_color(colors.panel_fill);
    visuals.window_fill = hex_color(colors.window_fill);
    visuals.extreme_bg_color = hex_color(colors.extreme_bg);
    visuals.faint_bg_color = hex_color(colors.faint_bg);

    // Code background.
    visuals.code_bg_color = hex_color(colors.code_bg);

    // Selection.
    visuals.selection.bg_fill = hex_color(colors.selection_bg);
    visuals.selection.stroke =
        Stroke::new(colors.selection_stroke_width, hex_color(colors.selection_stroke_color));

    // Hyperlinks.
    visuals.hyperlink_color = hex_color(colors.hyperlink);

    // Window chrome.
    visuals.window_stroke =
        Stroke::new(colors.window_stroke_width, hex_color(colors.window_stroke_color));
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = match colors.popup_shadow {
        Some((blur, alpha)) => Shadow {
            spread: 0,
            blur,
            offset: [0, 2],
            color: Color32::from_black_alpha(alpha),
        },
        None => Shadow::NONE,
    };

    // Collapsing headers get a visible frame.
    visuals.collapsing_header_frame = true;

    // -- Widget visuals -------------------------------------------------------
    let corner = egui::CornerRadius::same(3);

    visuals.widgets.noninteractive = build_widget_visuals(&colors.noninteractive, corner);
    visuals.widgets.inactive = build_widget_visuals(&colors.inactive, corner);
    visuals.widgets.hovered = build_widget_visuals(&colors.hovered, corner);
    visuals.widgets.active = build_widget_visuals(&colors.active, corner);
    visuals.widgets.open = build_widget_visuals(&colors.open, corner);

    style.visuals = visuals;
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Shared text style configuration
// ---------------------------------------------------------------------------

fn base_text_style() -> Style {
    Style {
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
    }
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
