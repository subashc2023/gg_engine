use gg_engine::egui;
use gg_engine::winit::window::{ResizeDirection, Window};

const TITLE_BAR_HEIGHT: f32 = 30.0;
const RESIZE_BORDER: f32 = 5.0;
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
    pub pause_toggled: bool,
    pub step_pressed: bool,
}

fn handle_resize_borders(ctx: &egui::Context, window: &Window) {
    if window.is_maximized() {
        return;
    }
    let screen_rect = ctx.input(|i| i.viewport_rect());
    let pos = match ctx.input(|i| i.pointer.latest_pos()) {
        Some(p) => p,
        None => return,
    };

    let at_left = pos.x <= screen_rect.left() + RESIZE_BORDER;
    let at_right = pos.x >= screen_rect.right() - RESIZE_BORDER;
    let at_top = pos.y <= screen_rect.top() + RESIZE_BORDER;
    let at_bottom = pos.y >= screen_rect.bottom() - RESIZE_BORDER;

    use ResizeDirection::*;
    let direction = match (at_left, at_right, at_top, at_bottom) {
        (true, _, true, _) => Some(NorthWest),
        (_, true, true, _) => Some(NorthEast),
        (true, _, _, true) => Some(SouthWest),
        (_, true, _, true) => Some(SouthEast),
        (true, _, _, _) => Some(West),
        (_, true, _, _) => Some(East),
        (_, _, true, _) => Some(North),
        (_, _, _, true) => Some(South),
        _ => None,
    };

    if let Some(dir) = direction {
        ctx.set_cursor_icon(match dir {
            East => egui::CursorIcon::ResizeEast,
            West => egui::CursorIcon::ResizeWest,
            North => egui::CursorIcon::ResizeNorth,
            South => egui::CursorIcon::ResizeSouth,
            NorthEast => egui::CursorIcon::ResizeNorthEast,
            NorthWest => egui::CursorIcon::ResizeNorthWest,
            SouthEast => egui::CursorIcon::ResizeSouthEast,
            SouthWest => egui::CursorIcon::ResizeSouthWest,
        });
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)) {
            let _ = window.drag_resize_window(dir);
        }
    }
}

pub fn title_bar_ui(
    ctx: &egui::Context,
    window: &Window,
    play_state: PlayState,
    is_paused: bool,
    project_title: &str,
    menu_contents: impl FnOnce(&mut egui::Ui),
) -> TitleBarResponse {
    handle_resize_borders(ctx, window);

    let mut close_requested = false;
    let mut play_toggled = false;
    let mut simulate_toggled = false;
    let mut pause_toggled = false;
    let mut step_pressed = false;
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

            // -- 4. Centered toolbar: [title] [Play] [Simulate] --
            // Layout depends on scene state:
            //   Edit:                     [title] [Play] [Simulate]
            //   Play/Simulate:            [title] [Stop] [Pause]
            //   Play/Simulate + paused:   [title] [Stop] [Pause*] [Step]
            let is_edit = play_state == PlayState::Edit;
            let has_play_button = is_edit;
            let has_simulate_button = is_edit;
            let has_stop_button = !is_edit;
            let has_pause_button = !is_edit;
            let has_step_button = !is_edit && is_paused;

            let btn_size = egui::vec2(28.0, 22.0);
            let spacing = 4.0;
            let button_count = [
                has_play_button,
                has_simulate_button,
                has_stop_button,
                has_pause_button,
                has_step_button,
            ]
            .iter()
            .filter(|&&b| b)
            .count() as f32;
            let buttons_width = btn_size.x * button_count + spacing * (button_count - 1.0).max(0.0);

            // Measure title text width so we can center the whole group.
            let title_font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
            let title_galley = if !project_title.is_empty() {
                Some(ui.painter().layout_no_wrap(
                    project_title.to_string(),
                    title_font.clone(),
                    egui::Color32::WHITE, // color doesn't affect measurement
                ))
            } else {
                None
            };
            let title_width = title_galley.as_ref().map_or(0.0, |g| g.size().x);
            let title_gap = if title_width > 0.0 { 10.0 } else { 0.0 };
            let total_width = title_width + title_gap + buttons_width;

            let center_x = title_bar_rect.center().x;
            let center_y = title_bar_rect.center().y;
            let group_left = center_x - total_width / 2.0;
            let buttons_left = group_left + title_width + title_gap;
            let start_x = buttons_left + btn_size.x / 2.0;

            // Allocate button rects left-to-right.
            let mut btn_idx = 0;
            let mut next_rect = || -> egui::Rect {
                let rect = egui::Rect::from_center_size(
                    egui::pos2(start_x + btn_idx as f32 * (btn_size.x + spacing), center_y),
                    btn_size,
                );
                btn_idx += 1;
                rect
            };

            let play_rect_resp = has_play_button.then(|| {
                let r = next_rect();
                (r, ui.allocate_rect(r, egui::Sense::click()))
            });
            let sim_rect_resp = has_simulate_button.then(|| {
                let r = next_rect();
                (r, ui.allocate_rect(r, egui::Sense::click()))
            });
            let stop_rect_resp = has_stop_button.then(|| {
                let r = next_rect();
                (r, ui.allocate_rect(r, egui::Sense::click()))
            });
            let pause_rect_resp = has_pause_button.then(|| {
                let r = next_rect();
                (r, ui.allocate_rect(r, egui::Sense::click()))
            });
            let step_rect_resp = has_step_button.then(|| {
                let r = next_rect();
                (r, ui.allocate_rect(r, egui::Sense::click()))
            });

            // -- 5. Paint everything (immutable borrows only) --
            let painter = ui.painter();

            // Project title text (left of buttons, centered as a group).
            if let Some(galley) = title_galley {
                let title_color = egui::Color32::from_rgb(0x88, 0x88, 0x88);
                let title_pos = egui::pos2(group_left, center_y - galley.size().y / 2.0);
                painter.galley(title_pos, galley, title_color);
            }

            // Play button (green triangle).
            if let Some((rect, ref resp)) = play_rect_resp {
                if resp.hovered() {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), BUTTON_HOVER_BG);
                }
                paint_play_triangle(painter, rect.center());
            }

            // Simulate button (gear icon).
            if let Some((rect, ref resp)) = sim_rect_resp {
                if resp.hovered() {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), BUTTON_HOVER_BG);
                }
                paint_gear_icon(painter, rect.center(), 7.0);
            }

            // Stop button (blue square).
            if let Some((rect, ref resp)) = stop_rect_resp {
                if resp.hovered() {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), BUTTON_HOVER_BG);
                }
                paint_stop_square(painter, rect.center());
            }

            // Pause button (two vertical bars, highlighted when paused).
            if let Some((rect, ref resp)) = pause_rect_resp {
                if is_paused {
                    // Highlight background to indicate active pause.
                    painter.rect_filled(
                        rect,
                        egui::CornerRadius::same(3),
                        egui::Color32::from_rgb(0x2A, 0x50, 0x70),
                    );
                }
                if resp.hovered() {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), BUTTON_HOVER_BG);
                }
                paint_pause_icon(painter, rect.center());
            }

            // Step button (play triangle + vertical bar).
            if let Some((rect, ref resp)) = step_rect_resp {
                if resp.hovered() {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), BUTTON_HOVER_BG);
                }
                paint_step_icon(painter, rect.center());
            }

            // Handle clicks.
            if let Some((_, ref resp)) = play_rect_resp {
                if resp.clicked() {
                    play_toggled = true;
                }
            }
            if let Some((_, ref resp)) = sim_rect_resp {
                if resp.clicked() {
                    simulate_toggled = true;
                }
            }
            if let Some((_, ref resp)) = stop_rect_resp {
                if resp.clicked() {
                    // Stop returns to edit — reuse the appropriate toggle.
                    match play_state {
                        PlayState::Play => play_toggled = true,
                        PlayState::Simulate => simulate_toggled = true,
                        _ => {}
                    }
                }
            }
            if let Some((_, ref resp)) = pause_rect_resp {
                if resp.clicked() {
                    pause_toggled = true;
                }
            }
            if let Some((_, ref resp)) = step_rect_resp {
                if resp.clicked() {
                    step_pressed = true;
                }
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
        pause_toggled,
        step_pressed,
    }
}

pub fn hub_title_bar_ui(ctx: &egui::Context, window: &Window) -> bool {
    handle_resize_borders(ctx, window);

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

            if !title_bar_rect.is_finite()
                || title_bar_rect.width() < BUTTON_WIDTH * 3.0 + 50.0
                || title_bar_rect.height() < 1.0
            {
                return;
            }

            let buttons_x = title_bar_rect.right() - BUTTON_WIDTH * 3.0;

            // Window control buttons (right side).
            let min_rect = egui::Rect::from_min_size(
                egui::pos2(buttons_x, title_bar_rect.top()),
                egui::vec2(BUTTON_WIDTH, TITLE_BAR_HEIGHT),
            );
            let min_resp = ui.allocate_rect(min_rect, egui::Sense::click());

            let max_rect = min_rect.translate(egui::vec2(BUTTON_WIDTH, 0.0));
            let max_resp = ui.allocate_rect(max_rect, egui::Sense::click());

            let close_rect = max_rect.translate(egui::vec2(BUTTON_WIDTH, 0.0));
            let close_resp = ui.allocate_rect(close_rect, egui::Sense::click());

            // Drag region.
            let drag_rect = egui::Rect::from_min_max(
                title_bar_rect.left_top(),
                egui::pos2(buttons_x, title_bar_rect.bottom()),
            );
            let drag_resp = ui.allocate_rect(drag_rect, egui::Sense::click_and_drag());

            // Centered "GGEngine" title.
            let title_font = egui::FontId::new(12.0, egui::FontFamily::Proportional);
            let title_galley = ui.painter().layout_no_wrap(
                "GGEngine".to_string(),
                title_font,
                egui::Color32::from_rgb(0x88, 0x88, 0x88),
            );
            let center = title_bar_rect.center();
            let title_pos = egui::pos2(
                center.x - title_galley.size().x / 2.0,
                center.y - title_galley.size().y / 2.0,
            );
            ui.painter().galley(
                title_pos,
                title_galley,
                egui::Color32::from_rgb(0x88, 0x88, 0x88),
            );

            // Paint window chrome.
            let painter = ui.painter();
            paint_minimize_icon(painter, &min_resp, min_rect);
            if is_maximized {
                paint_restore_icon(painter, &max_resp, max_rect);
            } else {
                paint_maximize_icon(painter, &max_resp, max_rect);
            }
            paint_close_icon(painter, &close_resp, close_rect);

            // Handle interactions.
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

    close_requested
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
    let back_min = egui::pos2(
        center.x - size / 2.0 + offset,
        center.y - size / 2.0 - offset,
    );
    let back_max = egui::pos2(back_min.x + size, back_min.y + size);
    let back_rect = egui::Rect::from_min_max(back_min, back_max);
    painter.rect_stroke(
        back_rect,
        0.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );

    // Front (lower-left) rectangle — filled with bar bg to occlude back rect
    let front_min = egui::pos2(
        center.x - size / 2.0 - offset,
        center.y - size / 2.0 + offset,
    );
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

fn paint_play_triangle(painter: &egui::Painter, center: egui::Pos2) {
    let half = 6.0;
    let points = vec![
        egui::pos2(center.x - half * 0.7, center.y - half),
        egui::pos2(center.x + half, center.y),
        egui::pos2(center.x - half * 0.7, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::from_rgb(0x4E, 0xC9, 0x4E),
        egui::Stroke::NONE,
    ));
}

fn paint_stop_square(painter: &egui::Painter, center: egui::Pos2) {
    let half = 5.0;
    let stop_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(
        stop_rect,
        egui::CornerRadius::same(2),
        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
    );
}

fn paint_pause_icon(painter: &egui::Painter, center: egui::Pos2) {
    let bar_w = 3.0;
    let bar_h = 10.0;
    let gap = 2.5;
    let color = BUTTON_ICON_COLOR;
    // Left bar.
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x - gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
    // Right bar.
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x + gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
}

fn paint_step_icon(painter: &egui::Painter, center: egui::Pos2) {
    let color = BUTTON_ICON_COLOR;
    // Small play triangle (left half).
    let half = 4.5;
    let offset_x = -2.0;
    let points = vec![
        egui::pos2(center.x + offset_x - half * 0.6, center.y - half),
        egui::pos2(center.x + offset_x + half * 0.7, center.y),
        egui::pos2(center.x + offset_x - half * 0.6, center.y + half),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    // Vertical bar (right half).
    let bar_x = center.x + half * 0.7;
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(bar_x, center.y), egui::vec2(2.5, half * 2.0)),
        0.0,
        color,
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
