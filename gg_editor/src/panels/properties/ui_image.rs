use gg_engine::egui;
use gg_engine::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_ui_image_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<UIImageComponent>(entity) {
        return false;
    }
    super::component_header(ui, "UI Image", "ui_image", bold_family, entity, |ui| {
        let (mut color, tex_handle_raw, mut border, mut fill_center) = {
            let img = scene.get_component::<UIImageComponent>(entity).unwrap();
            (
                [img.color.x, img.color.y, img.color.z, img.color.w],
                img.texture_handle.raw(),
                img.border,
                img.fill_center,
            )
        };

        let mut changed = false;

        // Color picker.
        if super::color_picker_rgba(ui, "Color", &mut color) {
            changed = true;
        }

        // Texture picker.
        ui.horizontal(|ui| {
            ui.label("Texture:");
            match super::asset_handle_picker(
                ui,
                tex_handle_raw,
                asset_manager,
                assets_root,
                "textures",
                "Select UI Texture",
                &["png", "jpg", "jpeg"],
            ) {
                super::AssetPickerAction::Selected(handle) => {
                    if let Some(mut img) = scene.get_component_mut::<UIImageComponent>(entity) {
                        img.texture_handle = handle;
                        img.texture = None;
                    }
                    changed = true;
                }
                super::AssetPickerAction::Cleared => {
                    if let Some(mut img) = scene.get_component_mut::<UIImageComponent>(entity) {
                        img.texture_handle = Uuid::default();
                        img.texture = None;
                    }
                    changed = true;
                }
                super::AssetPickerAction::None => {}
            }
        });

        // 9-slice border insets.
        ui.label("9-Slice Border (texels):");
        ui.horizontal(|ui| {
            ui.label("L:");
            if ui
                .add(
                    egui::DragValue::new(&mut border[0])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("R:");
            if ui
                .add(
                    egui::DragValue::new(&mut border[1])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("T:");
            if ui
                .add(
                    egui::DragValue::new(&mut border[2])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("B:");
            if ui
                .add(
                    egui::DragValue::new(&mut border[3])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
        });

        // Fill center.
        if ui.checkbox(&mut fill_center, "Fill Center").changed() {
            changed = true;
        }

        if changed {
            if let Some(mut img) = scene.get_component_mut::<UIImageComponent>(entity) {
                img.color = Vec4::from(color);
                img.border = border;
                img.fill_center = fill_center;
            }
            *scene_dirty = true;
        }
    })
}
