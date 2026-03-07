use gg_engine::egui;

use super::{GGEditor, SceneState};

impl GGEditor {
    pub(super) fn toolbar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(34.0)
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0x25, 0x25, 0x26))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                // 1px bottom border line.
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.min.x, rect.max.y),
                        egui::pos2(rect.max.x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3C, 0x3C, 0x3C)),
                );

                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true),
                    |ui| {
                        ui.add_space(3.0);

                        let is_edit = self.playback.scene_state == SceneState::Edit;
                        let has_play_button = is_edit;
                        let has_simulate_button = is_edit;
                        let has_stop_button = !is_edit;
                        let has_pause_button = !is_edit;
                        let has_step_button = !is_edit && self.playback.paused;

                        let btn_size = egui::vec2(28.0, 28.0);
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
                        let total_width =
                            btn_size.x * button_count + spacing * (button_count - 1.0).max(0.0);
                        let avail = ui.available_width();
                        ui.add_space((avail - total_width) / 2.0);

                        let hover_bg = egui::Color32::from_rgb(0x40, 0x40, 0x40);
                        let pause_active_bg = egui::Color32::from_rgb(0x2A, 0x50, 0x70);

                        // Allocate buttons in order.
                        let play_alloc = has_play_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            ui.add_space(spacing);
                            a
                        });
                        let sim_alloc = has_simulate_button
                            .then(|| ui.allocate_exact_size(btn_size, egui::Sense::click()));
                        let stop_alloc = has_stop_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            ui.add_space(spacing);
                            a
                        });
                        let pause_alloc = has_pause_button.then(|| {
                            let a = ui.allocate_exact_size(btn_size, egui::Sense::click());
                            if has_step_button {
                                ui.add_space(spacing);
                            }
                            a
                        });
                        let step_alloc = has_step_button
                            .then(|| ui.allocate_exact_size(btn_size, egui::Sense::click()));

                        // Paint icons.
                        if let Some((rect, ref resp)) = play_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            paint_play_triangle(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = sim_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            paint_gear_icon(ui.painter(), rect.center(), 8.0);
                        }
                        if let Some((rect, ref resp)) = stop_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            paint_stop_square(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = pause_alloc {
                            if self.playback.paused {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    pause_active_bg,
                                );
                            }
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            paint_pause_icon(ui.painter(), rect.center());
                        }
                        if let Some((rect, ref resp)) = step_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            paint_step_icon(ui.painter(), rect.center());
                        }

                        // Handle clicks.
                        if let Some((_, ref resp)) = play_alloc {
                            if resp.clicked() {
                                self.on_scene_play();
                            }
                        }
                        if let Some((_, ref resp)) = sim_alloc {
                            if resp.clicked() {
                                self.on_scene_simulate();
                            }
                        }
                        if let Some((_, ref resp)) = stop_alloc {
                            if resp.clicked() {
                                self.on_scene_stop();
                            }
                        }
                        if let Some((_, ref resp)) = pause_alloc {
                            if resp.clicked() {
                                self.on_scene_pause();
                            }
                        }
                        if let Some((_, ref resp)) = step_alloc {
                            if resp.clicked() {
                                self.on_scene_step();
                            }
                        }
                    },
                );
            });
    }
}

fn paint_play_triangle(painter: &egui::Painter, center: egui::Pos2) {
    let half = 7.0;
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
    let half = 6.0;
    let stop_rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
    painter.rect_filled(
        stop_rect,
        egui::CornerRadius::same(2),
        egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
    );
}

fn paint_pause_icon(painter: &egui::Painter, center: egui::Pos2) {
    let bar_w = 3.0;
    let bar_h = 12.0;
    let gap = 2.5;
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    painter.rect_filled(
        egui::Rect::from_center_size(
            egui::pos2(center.x - gap, center.y),
            egui::vec2(bar_w, bar_h),
        ),
        0.0,
        color,
    );
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
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let half = 5.0;
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
    let bar_x = center.x + half * 0.7;
    painter.rect_filled(
        egui::Rect::from_center_size(egui::pos2(bar_x, center.y), egui::vec2(2.5, half * 2.0)),
        0.0,
        color,
    );
}

fn paint_gear_icon(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let bg = egui::Color32::from_rgb(0x25, 0x25, 0x26);
    let teeth = 6;
    let inner_r = radius * 0.55;
    let outer_r = radius;
    let tooth_width = std::f32::consts::PI / (teeth as f32 * 2.0);

    let mut points = Vec::new();
    for i in 0..teeth {
        let angle = (i as f32 / teeth as f32) * std::f32::consts::TAU;
        let a1 = angle - tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a1.cos(),
            center.y + inner_r * a1.sin(),
        ));
        let a2 = angle - tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a2.cos(),
            center.y + outer_r * a2.sin(),
        ));
        let a3 = angle + tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a3.cos(),
            center.y + outer_r * a3.sin(),
        ));
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
    painter.circle_filled(center, radius * 0.25, bg);
}
