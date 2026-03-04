use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn scene_hierarchy_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
) {
    let root_entities = scene.root_entities();
    let mut entity_to_delete = None;

    for (entity, tag) in &root_entities {
        draw_entity_node(
            ui,
            scene,
            *entity,
            tag,
            selection_context,
            scene_dirty,
            &mut entity_to_delete,
        );
    }

    // Click on blank space to deselect.
    // Clamp to visible remaining height so the blank area doesn't extend
    // infinitely inside the scroll area.
    let remaining = ui.available_rect_before_wrap();
    let visible_height = (ui.clip_rect().max.y - remaining.min.y).max(0.0);
    if remaining.width() > 0.0 && visible_height > 0.0 {
        let clamped =
            egui::Rect::from_min_size(remaining.min, egui::vec2(remaining.width(), visible_height));
        let response = ui.allocate_rect(clamped, egui::Sense::click());
        if response.clicked() {
            *selection_context = None;
        }
        // Right-click on blank space → create entity.
        response.context_menu(|ui| {
            if ui.button("Create Empty Entity").clicked() {
                undo_system.record(scene);
                scene.create_entity_with_tag("Empty Entity");
                *scene_dirty = true;
                ui.close();
            }
        });
    }

    // Deferred entity deletion.
    if let Some(entity) = entity_to_delete {
        undo_system.record(scene);
        if *selection_context == Some(entity) {
            *selection_context = None;
        }
        let _ = scene.destroy_entity(entity);
    }
}

fn draw_entity_node(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    tag: &str,
    selection_context: &mut Option<Entity>,
    scene_dirty: &mut bool,
    entity_to_delete: &mut Option<Entity>,
) {
    let children = scene.get_children(entity);
    let selected = selection_context.is_some_and(|sel| sel == entity);

    if children.is_empty() {
        // Leaf node — simple selectable label.
        let response = ui.selectable_label(selected, tag);
        if response.clicked() {
            *selection_context = Some(entity);
        }
        entity_context_menu(&response, entity, entity_to_delete, scene_dirty);
    } else {
        // Parent node — collapsing header with children.
        let id = ui.make_persistent_id(entity.id());
        let header = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            true,
        );
        header
            .show_header(ui, |ui| {
                let r = ui.selectable_label(selected, tag);
                if r.clicked() {
                    *selection_context = Some(entity);
                }
                entity_context_menu(&r, entity, entity_to_delete, scene_dirty);
            })
            .body(|ui| {
                // Render children recursively.
                for child_uuid in &children {
                    if let Some(child_entity) = scene.find_entity_by_uuid(*child_uuid) {
                        let child_tag = scene
                            .get_component::<TagComponent>(child_entity)
                            .map(|t| t.tag.clone())
                            .unwrap_or_else(|| "Entity".into());
                        draw_entity_node(
                            ui,
                            scene,
                            child_entity,
                            &child_tag,
                            selection_context,
                            scene_dirty,
                            entity_to_delete,
                        );
                    }
                }
            });
    }
}

fn entity_context_menu(
    response: &egui::Response,
    entity: Entity,
    entity_to_delete: &mut Option<Entity>,
    scene_dirty: &mut bool,
) {
    response.context_menu(|ui| {
        if ui.button("Delete Entity").clicked() {
            *entity_to_delete = Some(entity);
            *scene_dirty = true;
            ui.close();
        }
    });
}
