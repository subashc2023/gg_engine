use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::BOLD_FONT;

use crate::editor_settings::EditorSettings;

// Cache of `Path::exists()` results so we don't issue a syscall per entry per frame.
// Cleared when the recent-projects list changes (remove action).
thread_local! {
    static EXISTS_CACHE: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
}

// Thread-local state for the new-project wizard form.
thread_local! {
    static WIZARD_STATE: RefCell<WizardState> = RefCell::new(WizardState::default());
}

struct WizardState {
    active: bool,
    project_name: String,
    location: String,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            active: false,
            project_name: "NewProject".into(),
            location: String::new(),
        }
    }
}

pub(crate) struct HubResponse {
    pub open_project_path: Option<PathBuf>,
    /// Path for a project to create and then open (from the wizard).
    pub create_project_path: Option<PathBuf>,
}

/// Returns `true` if every character in `name` is alphanumeric, a space, hyphen, or underscore.
fn is_valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
}

pub(crate) fn hub_ui(ctx: &egui::Context, settings: &mut EditorSettings) -> HubResponse {
    let mut response = HubResponse {
        open_project_path: None,
        create_project_path: None,
    };

    let wizard_active = WIZARD_STATE.with(|s| s.borrow().active);

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

                if wizard_active {
                    new_project_wizard_ui(ui, &mut response);
                } else {
                    main_hub_ui(ui, settings, &mut response);
                }
            });
        });

    response
}

/// The main hub view with Open/New buttons and recent projects list.
fn main_hub_ui(
    ui: &mut egui::Ui,
    settings: &mut EditorSettings,
    response: &mut HubResponse,
) {
    // Action buttons
    ui.horizontal(|ui| {
        if ui.button("Open Project...").clicked() {
            if let Some(path) = FileDialogs::open_file("GGProject files", &["ggproject"]) {
                response.open_project_path = Some(PathBuf::from(path));
            }
        }
        if ui.button("New Project...").clicked() {
            WIZARD_STATE.with(|s| {
                let mut state = s.borrow_mut();
                state.active = true;
                state.project_name = "NewProject".into();
                state.location = String::new();
            });
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
            let exists = EXISTS_CACHE.with(|cache| {
                *cache
                    .borrow_mut()
                    .entry(recent.path.clone())
                    .or_insert_with(|| std::path::Path::new(&recent.path).exists())
            });

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
            EXISTS_CACHE.with(|c| c.borrow_mut().clear());
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
}

/// Inline new-project wizard form.
fn new_project_wizard_ui(ui: &mut egui::Ui, response: &mut HubResponse) {
    // Read current state for rendering.
    let (mut project_name, mut location) =
        WIZARD_STATE.with(|s| {
            let st = s.borrow();
            (st.project_name.clone(), st.location.clone())
        });

    // Section header
    ui.label(
        egui::RichText::new("Create New Project")
            .font(egui::FontId::new(
                18.0,
                egui::FontFamily::Name(BOLD_FONT.into()),
            ))
            .color(egui::Color32::WHITE),
    );
    ui.add_space(16.0);

    let label_color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);

    // -- Project Name --
    ui.label(egui::RichText::new("Project Name").color(label_color));
    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::singleline(&mut project_name)
            .desired_width(ui.available_width())
            .hint_text("Enter project name..."),
    );

    let name_valid = is_valid_project_name(&project_name);
    if !project_name.is_empty() && !name_valid {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("Name may only contain letters, digits, spaces, hyphens, and underscores.")
                .color(egui::Color32::from_rgb(0xF4, 0x80, 0x71))
                .small(),
        );
    }

    ui.add_space(12.0);

    // -- Location --
    ui.label(egui::RichText::new("Location").color(label_color));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut location)
                .desired_width(ui.available_width() - 80.0)
                .hint_text("Select a folder..."),
        );
        if ui.button("Browse...").clicked() {
            if let Some(folder) = FileDialogs::pick_folder() {
                location = folder;
            }
        }
    });

    ui.add_space(16.0);

    // -- Path preview --
    let location_valid = !location.is_empty();
    let can_create = name_valid && location_valid;

    if can_create {
        let resolved = PathBuf::from(&location)
            .join(&project_name)
            .join(format!("{}.ggproject", &project_name));
        ui.label(
            egui::RichText::new(format!("Project file: {}", resolved.display()))
                .color(egui::Color32::from_rgb(0x88, 0x88, 0x88))
                .small(),
        );
    } else {
        ui.label(
            egui::RichText::new("Fill in the fields above to see the project path.")
                .color(egui::Color32::from_rgb(0x66, 0x66, 0x66))
                .small()
                .italics(),
        );
    }

    ui.add_space(16.0);

    // -- Create / Cancel --
    ui.horizontal(|ui| {
        let create_btn = egui::Button::new(
            egui::RichText::new("Create").color(egui::Color32::WHITE),
        )
        .fill(egui::Color32::from_rgb(0x00, 0x7A, 0xCC));

        if ui.add_enabled(can_create, create_btn).clicked() && can_create {
            let resolved = PathBuf::from(&location)
                .join(&project_name)
                .join(format!("{}.ggproject", &project_name));
            response.create_project_path = Some(resolved);
            WIZARD_STATE.with(|s| s.borrow_mut().active = false);
        }

        if ui.button("Cancel").clicked() {
            WIZARD_STATE.with(|s| s.borrow_mut().active = false);
        }
    });

    // Write back text edits.
    WIZARD_STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.project_name = project_name;
        st.location = location;
    });
}
