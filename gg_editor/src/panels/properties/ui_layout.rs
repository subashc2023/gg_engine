use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_ui_layout_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<UILayoutComponent>(entity) {
        return false;
    }
    super::component_header(ui, "UI Layout", "ui_layout", bold_family, entity, |ui| {
        let (mut direction, mut spacing, mut alignment, mut padding) = {
            let layout = scene.get_component::<UILayoutComponent>(entity).unwrap();
            (
                layout.direction,
                layout.spacing,
                layout.alignment,
                layout.padding,
            )
        };

        let mut changed = false;

        // Direction combo.
        ui.horizontal(|ui| {
            ui.label("Direction");
            let current_label = match direction {
                UILayoutDirection::Vertical => "Vertical",
                UILayoutDirection::Horizontal => "Horizontal",
            };
            egui::ComboBox::from_id_salt(("ui_layout_dir", entity.id()))
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut direction, UILayoutDirection::Vertical, "Vertical")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(
                            &mut direction,
                            UILayoutDirection::Horizontal,
                            "Horizontal",
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });
        });

        // Spacing drag.
        ui.horizontal(|ui| {
            ui.label("Spacing");
            if ui
                .add(
                    egui::DragValue::new(&mut spacing)
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
        });

        // Alignment combo.
        ui.horizontal(|ui| {
            ui.label("Alignment");
            let current_label = match alignment {
                UILayoutAlignment::Start => "Start",
                UILayoutAlignment::Center => "Center",
                UILayoutAlignment::End => "End",
            };
            egui::ComboBox::from_id_salt(("ui_layout_align", entity.id()))
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut alignment, UILayoutAlignment::Start, "Start")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut alignment, UILayoutAlignment::Center, "Center")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut alignment, UILayoutAlignment::End, "End")
                        .changed()
                    {
                        changed = true;
                    }
                });
        });

        // Padding fields [top, right, bottom, left].
        ui.label("Padding");
        ui.horizontal(|ui| {
            ui.label("Top:");
            if ui
                .add(
                    egui::DragValue::new(&mut padding[0])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("Right:");
            if ui
                .add(
                    egui::DragValue::new(&mut padding[1])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Bottom:");
            if ui
                .add(
                    egui::DragValue::new(&mut padding[2])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("Left:");
            if ui
                .add(
                    egui::DragValue::new(&mut padding[3])
                        .speed(0.5)
                        .range(0.0..=f32::MAX),
                )
                .changed()
            {
                changed = true;
            }
        });

        if changed {
            if let Some(mut layout) = scene.get_component_mut::<UILayoutComponent>(entity) {
                layout.direction = direction;
                layout.spacing = spacing;
                layout.alignment = alignment;
                layout.padding = padding;
            }
            *scene_dirty = true;
        }
    })
}
