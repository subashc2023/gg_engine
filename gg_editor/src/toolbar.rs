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
                            super::icons::paint_play_triangle(ui.painter(), rect.center(), 7.0);
                        }
                        if let Some((rect, ref resp)) = sim_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            super::icons::paint_gear_icon(ui.painter(), rect.center(), 8.0, egui::Color32::from_rgb(0x25, 0x25, 0x26));
                        }
                        if let Some((rect, ref resp)) = stop_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            super::icons::paint_stop_square(ui.painter(), rect.center(), 6.0);
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
                            super::icons::paint_pause_icon(ui.painter(), rect.center(), 12.0);
                        }
                        if let Some((rect, ref resp)) = step_alloc {
                            if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(3),
                                    hover_bg,
                                );
                            }
                            super::icons::paint_step_icon(ui.painter(), rect.center(), 5.0);
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

