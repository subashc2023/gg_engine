use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_ui_rect_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<UIRectComponent>(entity) {
        return false;
    }
    super::component_header(ui, "UI Rect", "ui_rect", bold_family, entity, |ui| {
        let (mut size, mut pivot, mut raycast) = {
            let r = scene.get_component::<UIRectComponent>(entity).unwrap();
            ([r.size.x, r.size.y], [r.pivot.x, r.pivot.y], r.raycast_target)
        };

        let mut changed = false;

        // Size controls.
        ui.horizontal(|ui| {
            ui.label("Size W:");
            if ui
                .add(egui::DragValue::new(&mut size[0]).speed(1.0).range(0.0..=f32::MAX))
                .changed()
            {
                changed = true;
            }
            ui.label("H:");
            if ui
                .add(egui::DragValue::new(&mut size[1]).speed(1.0).range(0.0..=f32::MAX))
                .changed()
            {
                changed = true;
            }
        });

        // Pivot presets.
        ui.label("Pivot Presets:");
        ui.horizontal(|ui| {
            if ui.small_button("TL").on_hover_text("Top-Left").clicked() {
                pivot = [0.0, 0.0];
                changed = true;
            }
            if ui.small_button("TC").on_hover_text("Top-Center").clicked() {
                pivot = [0.5, 0.0];
                changed = true;
            }
            if ui.small_button("TR").on_hover_text("Top-Right").clicked() {
                pivot = [1.0, 0.0];
                changed = true;
            }
            if ui.small_button("CL").on_hover_text("Center-Left").clicked() {
                pivot = [0.0, 0.5];
                changed = true;
            }
            if ui.small_button("C").on_hover_text("Center").clicked() {
                pivot = [0.5, 0.5];
                changed = true;
            }
            if ui.small_button("CR").on_hover_text("Center-Right").clicked() {
                pivot = [1.0, 0.5];
                changed = true;
            }
            if ui.small_button("BL").on_hover_text("Bottom-Left").clicked() {
                pivot = [0.0, 1.0];
                changed = true;
            }
            if ui.small_button("BC").on_hover_text("Bottom-Center").clicked() {
                pivot = [0.5, 1.0];
                changed = true;
            }
            if ui.small_button("BR").on_hover_text("Bottom-Right").clicked() {
                pivot = [1.0, 1.0];
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Pivot X:");
            if ui
                .add(egui::DragValue::new(&mut pivot[0]).speed(0.01).range(0.0..=1.0))
                .changed()
            {
                changed = true;
            }
            ui.label("Y:");
            if ui
                .add(egui::DragValue::new(&mut pivot[1]).speed(0.01).range(0.0..=1.0))
                .changed()
            {
                changed = true;
            }
        });

        // Raycast target checkbox.
        if ui.checkbox(&mut raycast, "Raycast Target").changed() {
            changed = true;
        }

        if changed {
            if let Some(mut r) = scene.get_component_mut::<UIRectComponent>(entity) {
                r.size = Vec2::from(size);
                r.pivot = Vec2::from(pivot);
                r.raycast_target = raycast;
            }
            *scene_dirty = true;
        }
    })
}
