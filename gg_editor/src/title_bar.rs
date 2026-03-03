use gg_engine::egui;
use gg_engine::winit::window::Window;

const TITLE_BAR_HEIGHT: f32 = 30.0;
const BUTTON_WIDTH: f32 = 46.0;

const BAR_BG: egui::Color32 = egui::Color32::from_rgb(0x18, 0x18, 0x18);
const BUTTON_ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
const BUTTON_HOVER_BG: egui::Color32 = egui::Color32::from_rgb(0x40, 0x40, 0x40);
const CLOSE_HOVER_BG: egui::Color32 = egui::Color32::from_rgb(0xE8, 0x11, 0x23);


#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayState {
    Edit,
    Play,
    Simulate,
}

pub struct TitleBarResponse {
    pub close_requested: bool,
    pub play_toggled: bool,
    pub simulate_toggled: bool,
}

pub fn title_bar_ui(
    ctx: &egui::Context,
    window: &Window,
    play_state: PlayState,
    menu_contents: impl FnOnce(&mut egui::Ui),
) -> TitleBarResponse {
    let mut close_requested = false;
    let mut play_toggled = false;
    let mut simulate_toggled = false;
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

            // -- 4. Play/Stop + Simulate buttons (centered) --
            let btn_size = egui::vec2(28.0, 22.0);
            let spacing = 4.0;
            let total_width = btn_size.x * 2.0 + spacing;
            let center_x = title_bar_rect.center().x;
            let center_y = title_bar_rect.center().y;

            // Play button (left of center pair).
            let play_rect = egui::Rect::from_center_size(
                egui::pos2(center_x - (total_width / 2.0 - btn_size.x / 2.0), center_y),
                btn_size,
            );
            let play_resp = ui.allocate_rect(play_rect, egui::Sense::click());

            // Simulate button (right of center pair).
            let sim_rect = egui::Rect::from_center_size(
                egui::pos2(center_x + (total_width / 2.0 - btn_size.x / 2.0), center_y),
                btn_size,
            );
            let sim_resp = ui.allocate_rect(sim_rect, egui::Sense::click());

            // -- 5. Paint everything (immutable borrows only) --
            let painter = ui.painter();

            // Play/stop button hover.
            if play_resp.hovered() {
                painter.rect_filled(
                    play_rect,
                    egui::CornerRadius::same(3),
                    BUTTON_HOVER_BG,
                );
            }

            // Play/stop icon.
            let play_center = play_rect.center();
            let has_active_scene = true; // always true since we always have a scene now
            let play_icon = match play_state {
                PlayState::Edit | PlayState::Simulate => true,  // show play triangle
                PlayState::Play => false,                        // show stop square
            };
            if has_active_scene {
                if play_icon {
                    // Green play triangle.
                    let half = 6.0;
                    let points = vec![
                        egui::pos2(play_center.x - half * 0.7, play_center.y - half),
                        egui::pos2(play_center.x + half, play_center.y),
                        egui::pos2(play_center.x - half * 0.7, play_center.y + half),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        points,
                        egui::Color32::from_rgb(0x4E, 0xC9, 0x4E),
                        egui::Stroke::NONE,
                    ));
                } else {
                    // Blue stop square.
                    let half = 5.0;
                    let stop_rect = egui::Rect::from_center_size(
                        play_center,
                        egui::vec2(half * 2.0, half * 2.0),
                    );
                    painter.rect_filled(
                        stop_rect,
                        egui::CornerRadius::same(2),
                        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
                    );
                }
            }

            // Simulate button hover.
            if sim_resp.hovered() {
                painter.rect_filled(
                    sim_rect,
                    egui::CornerRadius::same(3),
                    BUTTON_HOVER_BG,
                );
            }

            // Simulate icon: gear shape (or stop square when simulating).
            let sim_center = sim_rect.center();
            match play_state {
                PlayState::Simulate => {
                    // Blue stop square (same as play stop).
                    let half = 5.0;
                    let stop_rect = egui::Rect::from_center_size(
                        sim_center,
                        egui::vec2(half * 2.0, half * 2.0),
                    );
                    painter.rect_filled(
                        stop_rect,
                        egui::CornerRadius::same(2),
                        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
                    );
                }
                _ => {
                    // Gear icon for simulate.
                    paint_gear_icon(painter, sim_center, 7.0);
                }
            }

            if play_resp.clicked() {
                play_toggled = true;
            }
            if sim_resp.clicked() {
                simulate_toggled = true;
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

    TitleBarResponse {
        close_requested,
        play_toggled,
        simulate_toggled,
    }
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

fn paint_gear_icon(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let teeth = 6;
    let inner_r = radius * 0.55;
    let outer_r = radius;
    let tooth_width = std::f32::consts::PI / (teeth as f32 * 2.0);

    // Build gear outline as a polygon.
    let mut points = Vec::new();
    for i in 0..teeth {
        let angle = (i as f32 / teeth as f32) * std::f32::consts::TAU;

        // Inner edge leading.
        let a1 = angle - tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a1.cos(),
            center.y + inner_r * a1.sin(),
        ));
        // Outer edge leading.
        let a2 = angle - tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a2.cos(),
            center.y + outer_r * a2.sin(),
        ));
        // Outer edge trailing.
        let a3 = angle + tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a3.cos(),
            center.y + outer_r * a3.sin(),
        ));
        // Inner edge trailing.
        let a4 = angle + tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a4.cos(),
            center.y + inner_r * a4.sin(),
        ));
    }

    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));

    // Center hole.
    let hole_r = radius * 0.25;
    painter.circle_filled(center, hole_r, BAR_BG);
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
