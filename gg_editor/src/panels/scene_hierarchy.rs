use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn scene_hierarchy_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
) {
    let entities = scene.each_entity_with_tag();
    let mut entity_to_delete = None;

    for (entity, tag) in &entities {
        let selected = selection_context.is_some_and(|sel| sel == *entity);
        let response = ui.selectable_label(selected, tag);
        if response.clicked() {
            *selection_context = Some(*entity);
        }
        // Right-click on entity → delete.
        response.context_menu(|ui| {
            if ui.button("Delete Entity").clicked() {
                entity_to_delete = Some(*entity);
                ui.close();
            }
        });
    }

    // Click on blank space to deselect.
    let remaining = ui.available_rect_before_wrap();
    if remaining.width() > 0.0 && remaining.height() > 0.0 {
        let response = ui.allocate_rect(remaining, egui::Sense::click());
        if response.clicked() {
            *selection_context = None;
        }
        // Right-click on blank space → create entity.
        response.context_menu(|ui| {
            if ui.button("Create Empty Entity").clicked() {
                scene.create_entity_with_tag("Empty Entity");
                ui.close();
            }
        });
    }

    // Deferred entity deletion.
    if let Some(entity) = entity_to_delete {
        if *selection_context == Some(entity) {
            *selection_context = None;
        }
        let _ = scene.destroy_entity(entity);
    }
}
