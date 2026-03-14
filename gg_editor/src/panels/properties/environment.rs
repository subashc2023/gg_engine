use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_environment_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<EnvironmentComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Environment Map",
        "environment_map",
        bold_family,
        entity,
        |ui| {
            let (handle_raw, loaded, mut exposure, mut ibl_intensity, mut rotation, mut show_skybox) = {
                let ec = scene
                    .get_component::<EnvironmentComponent>(entity)
                    .unwrap();
                (
                    ec.environment_handle,
                    ec.loaded,
                    ec.skybox_exposure,
                    ec.ibl_intensity,
                    ec.skybox_rotation,
                    ec.show_skybox,
                )
            };

            let mut changed = false;

            // HDR map picker.
            ui.horizontal(|ui| {
                ui.label("HDR Map");
                match super::asset_handle_picker(
                    ui,
                    handle_raw,
                    asset_manager,
                    assets_root,
                    "hdri",
                    "HDR Environment Maps",
                    &["hdr"],
                ) {
                    super::AssetPickerAction::Selected(new_handle) => {
                        if let Some(mut ec) =
                            scene.get_component_mut::<EnvironmentComponent>(entity)
                        {
                            ec.environment_handle = new_handle.raw();
                            ec.loaded = false; // Trigger reload.
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::Cleared => {
                        if let Some(mut ec) =
                            scene.get_component_mut::<EnvironmentComponent>(entity)
                        {
                            ec.environment_handle = 0;
                            ec.loaded = false;
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::None => {}
                }
            });

            // Status indicator.
            if handle_raw == 0 {
                ui.label(
                    egui::RichText::new("No HDR map selected")
                        .weak()
                        .small(),
                );
            } else if !loaded {
                ui.label(
                    egui::RichText::new("Loading...")
                        .weak()
                        .small(),
                );
            }

            ui.separator();

            if ui.checkbox(&mut show_skybox, "Show Skybox").changed() {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut exposure)
                        .speed(0.01)
                        .range(0.0..=10.0)
                        .prefix("Exposure: "),
                )
                .changed()
            {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut ibl_intensity)
                        .speed(0.01)
                        .range(0.0..=10.0)
                        .prefix("IBL Intensity: "),
                )
                .changed()
            {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut rotation)
                        .speed(1.0)
                        .range(-360.0..=360.0)
                        .suffix("\u{00b0}")
                        .prefix("Rotation: "),
                )
                .changed()
            {
                changed = true;
            }

            if changed {
                if let Some(mut ec) =
                    scene.get_component_mut::<EnvironmentComponent>(entity)
                {
                    ec.skybox_exposure = exposure;
                    ec.ibl_intensity = ibl_intensity;
                    ec.skybox_rotation = rotation;
                    ec.show_skybox = show_skybox;
                }
                *scene_dirty = true;
            }
        },
    )
}
