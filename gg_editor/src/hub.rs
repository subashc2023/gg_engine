use std::path::PathBuf;

use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::BOLD_FONT;

use crate::editor_settings::EditorSettings;

pub(crate) struct HubResponse {
    pub open_project_path: Option<PathBuf>,
    pub new_project_requested: bool,
}

pub(crate) fn hub_ui(ctx: &egui::Context, settings: &mut EditorSettings) -> HubResponse {
    let mut response = HubResponse {
        open_project_path: None,
        new_project_requested: false,
    };

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::from_rgb(0x1E, 0x1E, 0x1E)))
        .show(ctx, |ui| {
            let available = ui.available_size();
            let content_width = 500.0_f32.min(available.x - 60.0);
            let top_padding = (available.y * 0.15).max(40.0);

            ui.add_space(top_padding);

            ui.vertical_centered(|ui| {
                ui.set_max_width(content_width);

                // Title
                ui.label(
                    egui::RichText::new("GGEngine")
                        .font(egui::FontId::new(
                            28.0,
                            egui::FontFamily::Name(BOLD_FONT.into()),
                        ))
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Game Engine")
                        .color(egui::Color32::from_rgb(0x88, 0x88, 0x88))
                        .size(14.0),
                );

                ui.add_space(24.0);

                // Action buttons
                ui.horizontal(|ui| {
                    if ui.button("Open Project...").clicked() {
                        if let Some(path) =
                            FileDialogs::open_file("GGProject files", &["ggproject"])
                        {
                            response.open_project_path = Some(PathBuf::from(path));
                        }
                    }
                    if ui.button("New Project...").clicked() {
                        response.new_project_requested = true;
                    }
                });

                ui.add_space(16.0);

                // Recent Projects
                if !settings.recent_projects.is_empty() {
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Recent Projects")
                            .color(egui::Color32::from_rgb(0x88, 0x88, 0x88))
                            .small(),
                    );
                    ui.add_space(4.0);

                    let mut to_remove: Option<String> = None;
                    let mut to_open: Option<PathBuf> = None;

                    for recent in &settings.recent_projects {
                        let exists = std::path::Path::new(&recent.path).exists();

                        ui.horizontal(|ui| {
                            let name_text = if exists {
                                egui::RichText::new(&recent.name).color(egui::Color32::WHITE)
                            } else {
                                egui::RichText::new(&recent.name)
                                    .color(egui::Color32::from_rgb(0x66, 0x66, 0x66))
                                    .strikethrough()
                            };

                            if ui.selectable_label(false, name_text).clicked() && exists {
                                to_open = Some(PathBuf::from(&recent.path));
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("x").clicked() {
                                        to_remove = Some(recent.path.clone());
                                    }
                                    ui.label(
                                        egui::RichText::new(&recent.path)
                                            .color(egui::Color32::from_rgb(0x55, 0x55, 0x55))
                                            .small(),
                                    );
                                },
                            );
                        });
                    }

                    if let Some(path) = to_remove {
                        settings.remove_recent_project(&path);
                    }
                    if let Some(path) = to_open {
                        response.open_project_path = Some(path);
                    }
                } else {
                    ui.add_space(20.0);
                    ui.label(
                        egui::RichText::new("No recent projects")
                            .color(egui::Color32::from_rgb(0x66, 0x66, 0x66))
                            .italics(),
                    );
                }
            });
        });

    response
}
