use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_text_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<TextComponent>(entity) {
        return false;
    }
    super::component_header(ui, "Text", "text", bold_family, entity, |ui| {
            let (
                mut text_str,
                mut font_path,
                mut font_size,
                mut color_arr,
                mut line_spacing,
                mut kerning,
                mut sorting_layer,
                mut order_in_layer,
            ) = {
                let tc = scene.get_component::<TextComponent>(entity).unwrap();
                (
                    tc.text.clone(),
                    tc.font_path.clone(),
                    tc.font_size,
                    [tc.color.x, tc.color.y, tc.color.z, tc.color.w],
                    tc.line_spacing,
                    tc.kerning,
                    tc.sorting_layer,
                    tc.order_in_layer,
                )
            };

            // Text (multiline).
            ui.label("Text");
            if ui
                .add(egui::TextEdit::multiline(&mut text_str).desired_rows(3))
                .changed()
            {
                if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                    tc.text = text_str;
                    *scene_dirty = true;
                }
            }

            // Font path.
            ui.horizontal(|ui| {
                ui.label("Font");
                if ui
                    .add(egui::TextEdit::singleline(&mut font_path).desired_width(150.0))
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.font_path = font_path.clone();
                        tc.font = None;
                        *scene_dirty = true;
                    }
                }
                if ui.button("...").clicked() {
                    let fonts_dir = assets_root.join("fonts");
                    let fonts_dir_str = fonts_dir.to_string_lossy();
                    if let Some(path_str) =
                        FileDialogs::open_file_in("Font files", &["ttf", "otf"], &fonts_dir_str)
                    {
                        if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                            tc.font_path = path_str;
                            tc.font = None;
                            *scene_dirty = true;
                        }
                    }
                }
            });

            // Color.
            if super::color_picker_rgba(ui, "Color", &mut color_arr) {
                if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                    tc.color = Vec4::from(color_arr);
                    *scene_dirty = true;
                }
            }

            // Font size.
            ui.horizontal(|ui| {
                ui.label("Font Size");
                if ui
                    .add(
                        egui::DragValue::new(&mut font_size)
                            .speed(0.01)
                            .range(0.01..=100.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.font_size = font_size;
                        *scene_dirty = true;
                    }
                }
            });

            // Line spacing.
            ui.horizontal(|ui| {
                ui.label("Line Spacing");
                if ui
                    .add(
                        egui::DragValue::new(&mut line_spacing)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.line_spacing = line_spacing;
                        *scene_dirty = true;
                    }
                }
            });

            // Kerning.
            ui.horizontal(|ui| {
                ui.label("Kerning");
                if ui
                    .add(
                        egui::DragValue::new(&mut kerning)
                            .speed(0.001)
                            .range(-1.0..=1.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.kerning = kerning;
                        *scene_dirty = true;
                    }
                }
            });

            // Sorting layer & order.
            if super::sorting_layer_controls(ui, &mut sorting_layer, &mut order_in_layer) {
                if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                    tc.sorting_layer = sorting_layer;
                    tc.order_in_layer = order_in_layer;
                }
                *scene_dirty = true;
            }
    })
}
