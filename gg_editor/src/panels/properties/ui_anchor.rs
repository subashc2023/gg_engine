use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_ui_anchor_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<UIAnchorComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "UI Anchor",
        "ui_anchor",
        bold_family,
        entity,
        |ui| {
            let (mut anchor, mut offset) = {
                let ua = scene.get_component::<UIAnchorComponent>(entity).unwrap();
                ([ua.anchor.x, ua.anchor.y], [ua.offset.x, ua.offset.y])
            };

            let mut changed = false;

            // Preset buttons.
            ui.label("Presets:");
            ui.horizontal(|ui| {
                if ui.small_button("TL").on_hover_text("Top-Left").clicked() {
                    anchor = [0.0, 0.0];
                    changed = true;
                }
                if ui.small_button("TC").on_hover_text("Top-Center").clicked() {
                    anchor = [0.5, 0.0];
                    changed = true;
                }
                if ui.small_button("TR").on_hover_text("Top-Right").clicked() {
                    anchor = [1.0, 0.0];
                    changed = true;
                }
                if ui.small_button("CL").on_hover_text("Center-Left").clicked() {
                    anchor = [0.0, 0.5];
                    changed = true;
                }
                if ui.small_button("C").on_hover_text("Center").clicked() {
                    anchor = [0.5, 0.5];
                    changed = true;
                }
                if ui.small_button("CR").on_hover_text("Center-Right").clicked() {
                    anchor = [1.0, 0.5];
                    changed = true;
                }
                if ui.small_button("BL").on_hover_text("Bottom-Left").clicked() {
                    anchor = [0.0, 1.0];
                    changed = true;
                }
                if ui.small_button("BC").on_hover_text("Bottom-Center").clicked() {
                    anchor = [0.5, 1.0];
                    changed = true;
                }
                if ui.small_button("BR").on_hover_text("Bottom-Right").clicked() {
                    anchor = [1.0, 1.0];
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Anchor X:");
                if ui
                    .add(
                        egui::DragValue::new(&mut anchor[0])
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.label("Y:");
                if ui
                    .add(
                        egui::DragValue::new(&mut anchor[1])
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Offset X:");
                if ui
                    .add(egui::DragValue::new(&mut offset[0]).speed(0.05))
                    .changed()
                {
                    changed = true;
                }
                ui.label("Y:");
                if ui
                    .add(egui::DragValue::new(&mut offset[1]).speed(0.05))
                    .changed()
                {
                    changed = true;
                }
            });

            if changed {
                if let Some(mut ua) = scene.get_component_mut::<UIAnchorComponent>(entity) {
                    ua.anchor = Vec2::from(anchor);
                    ua.offset = Vec2::from(offset);
                }
                *scene_dirty = true;
            }
        },
    )
}
