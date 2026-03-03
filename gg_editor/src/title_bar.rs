use gg_engine::egui;
use gg_engine::winit::window::Window;

const TITLE_BAR_HEIGHT: f32 = 30.0;
const BUTTON_WIDTH: f32 = 46.0;

const BAR_BG: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const BUTTON_ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
const BUTTON_HOVER_BG: egui::Color32 = egui::Color32::from_rgb(0x40, 0x40, 0x40);
const CLOSE_HOVER_BG: egui::Color32 = egui::Color32::from_rgb(0xE8, 0x11, 0x23);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(0x96, 0x96, 0x96);

pub struct TitleBarResponse {
    pub close_requested: bool,
}

pub fn title_bar_ui(
    ctx: &egui::Context,
    window: &Window,
    title: &str,
    menu_contents: impl FnOnce(&mut egui::Ui),
) -> TitleBarResponse {
    let mut close_requested = false;
    let is_maximized = window.is_maximized();

    egui::TopBottomPanel::top("title_bar")
        .exact_height(TITLE_BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(BAR_BG)
                .inner_margin(egui::Margin::ZERO),
        )
        .show(ctx, |ui| {
            let title_bar_rect = ui.max_rect();

            // Skip layout when the window is too small (e.g. during minimize).
            if !title_bar_rect.is_finite()
                || title_bar_rect.width() < BUTTON_WIDTH * 3.0 + 50.0
                || title_bar_rect.height() < 1.0
            {
                return;
            }

            let buttons_x = title_bar_rect.right() - BUTTON_WIDTH * 3.0;

            // -- 1. Allocate window control buttons (right side) --
            let min_rect = egui::Rect::from_min_size(
                egui::pos2(buttons_x, title_bar_rect.top()),
                egui::vec2(BUTTON_WIDTH, TITLE_BAR_HEIGHT),
            );
            let min_resp = ui.allocate_rect(min_rect, egui::Sense::click());

            let max_rect = min_rect.translate(egui::vec2(BUTTON_WIDTH, 0.0));
            let max_resp = ui.allocate_rect(max_rect, egui::Sense::click());

            let close_rect = max_rect.translate(egui::vec2(BUTTON_WIDTH, 0.0));
            let close_resp = ui.allocate_rect(close_rect, egui::Sense::click());

            // -- 2. Allocate drag region (entire left area up to buttons) --
            // Added BEFORE the menu so menu widgets get higher interaction priority.
            let drag_rect = egui::Rect::from_min_max(
                title_bar_rect.left_top(),
                egui::pos2(buttons_x, title_bar_rect.bottom()),
            );
            let drag_resp = ui.allocate_rect(drag_rect, egui::Sense::click_and_drag());

            // -- 3. Menu bar (left side, on top of drag region) --
            let menu_area = egui::Rect::from_min_max(
                title_bar_rect.left_top(),
                egui::pos2(buttons_x, title_bar_rect.bottom()),
            );
            let mut menu_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(menu_area)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            menu_ui.set_height(TITLE_BAR_HEIGHT);
            menu_ui.add_space(8.0);
            egui::MenuBar::new().ui(&mut menu_ui, |ui| {
                menu_contents(ui);
            });

            // -- 4. Paint everything (immutable borrows only) --
            let painter = ui.painter();

            // Title: centered in window, hidden if it would overlap buttons
            let title_galley = painter.layout_no_wrap(
                title.to_string(),
                egui::FontId::proportional(13.0),
                TITLE_COLOR,
            );
            let title_w = title_galley.size().x;
            let title_x = title_bar_rect.center().x - title_w / 2.0;
            if title_x + title_w + 8.0 < buttons_x {
                let title_y = title_bar_rect.center().y - title_galley.size().y / 2.0;
                painter.galley(
                    egui::pos2(title_x, title_y),
                    title_galley,
                    egui::Color32::TRANSPARENT,
                );
            }

            // Button icons
            paint_minimize_icon(painter, &min_resp, min_rect);
            if is_maximized {
                paint_restore_icon(painter, &max_resp, max_rect);
            } else {
                paint_maximize_icon(painter, &max_resp, max_rect);
            }
            paint_close_icon(painter, &close_resp, close_rect);

            // -- 5. Handle interactions --
            if min_resp.clicked() {
                window.set_minimized(true);
            }
            if max_resp.clicked() {
                window.set_maximized(!is_maximized);
            }
            if close_resp.clicked() {
                close_requested = true;
            }
            if drag_resp.drag_started() {
                let _ = window.drag_window();
            }
            if drag_resp.double_clicked() {
                window.set_maximized(!is_maximized);
            }
        });

    TitleBarResponse { close_requested }
}

// ---------------------------------------------------------------------------
// Procedural icon painting
// ---------------------------------------------------------------------------

fn paint_minimize_icon(painter: &egui::Painter, resp: &egui::Response, rect: egui::Rect) {
    if resp.hovered() {
        painter.rect_filled(rect, 0.0, BUTTON_HOVER_BG);
    }
    let color = if resp.hovered() {
        egui::Color32::WHITE
    } else {
        BUTTON_ICON_COLOR
    };
    let center = rect.center();
    let half_w = 5.0;
    painter.line_segment(
        [
            egui::pos2(center.x - half_w, center.y),
            egui::pos2(center.x + half_w, center.y),
        ],
        egui::Stroke::new(1.0, color),
    );
}

fn paint_maximize_icon(painter: &egui::Painter, resp: &egui::Response, rect: egui::Rect) {
    if resp.hovered() {
        painter.rect_filled(rect, 0.0, BUTTON_HOVER_BG);
    }
    let color = if resp.hovered() {
        egui::Color32::WHITE
    } else {
        BUTTON_ICON_COLOR
    };
    let center = rect.center();
    let half = 5.0;
    let icon_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_stroke(
        icon_rect,
        0.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

fn paint_restore_icon(painter: &egui::Painter, resp: &egui::Response, rect: egui::Rect) {
    if resp.hovered() {
        painter.rect_filled(rect, 0.0, BUTTON_HOVER_BG);
    }
    let color = if resp.hovered() {
        egui::Color32::WHITE
    } else {
        BUTTON_ICON_COLOR
    };
    let center = rect.center();
    let size = 8.0;
    let offset = 2.0;

    // Back (upper-right) rectangle
    let back_min = egui::pos2(center.x - size / 2.0 + offset, center.y - size / 2.0 - offset);
    let back_max = egui::pos2(back_min.x + size, back_min.y + size);
    let back_rect = egui::Rect::from_min_max(back_min, back_max);
    painter.rect_stroke(
        back_rect,
        0.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );

    // Front (lower-left) rectangle — filled with bar bg to occlude back rect
    let front_min = egui::pos2(center.x - size / 2.0 - offset, center.y - size / 2.0 + offset);
    let front_max = egui::pos2(front_min.x + size, front_min.y + size);
    let front_rect = egui::Rect::from_min_max(front_min, front_max);
    painter.rect_filled(front_rect, 0.0, BAR_BG);
    painter.rect_stroke(
        front_rect,
        0.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

fn paint_close_icon(painter: &egui::Painter, resp: &egui::Response, rect: egui::Rect) {
    if resp.hovered() {
        painter.rect_filled(rect, 0.0, CLOSE_HOVER_BG);
    }
    let color = if resp.hovered() {
        egui::Color32::WHITE
    } else {
        BUTTON_ICON_COLOR
    };
    let center = rect.center();
    let half = 5.0;
    painter.line_segment(
        [
            egui::pos2(center.x - half, center.y - half),
            egui::pos2(center.x + half, center.y + half),
        ],
        egui::Stroke::new(1.0, color),
    );
    painter.line_segment(
        [
            egui::pos2(center.x + half, center.y - half),
            egui::pos2(center.x - half, center.y + half),
        ],
        egui::Stroke::new(1.0, color),
    );
}
