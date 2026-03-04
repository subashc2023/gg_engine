use gg_engine::egui;
use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// Drag-and-drop payload for hierarchy reparenting
// ---------------------------------------------------------------------------

struct HierarchyDragPayload {
    entity: Entity,
}

// ---------------------------------------------------------------------------
// Deferred actions — collected during UI iteration, applied afterwards
// ---------------------------------------------------------------------------

enum DeferredHierarchyAction {
    DeleteEntity(Entity),
    CreateChild(Entity),
    Reparent { child: Entity, new_parent: Entity },
    DetachToRoot(Entity),
}

// ---------------------------------------------------------------------------
// Main panel UI
// ---------------------------------------------------------------------------

pub(crate) fn scene_hierarchy_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
) {
    let root_entities = scene.root_entities();
    let mut deferred_action: Option<DeferredHierarchyAction> = None;

    for (entity, tag) in &root_entities {
        draw_entity_node(
            ui,
            scene,
            *entity,
            tag,
            selection_context,
            scene_dirty,
            &mut deferred_action,
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

        // Drop target: detach dragged entity to root.
        if let Some(payload) = response.dnd_release_payload::<HierarchyDragPayload>() {
            deferred_action = Some(DeferredHierarchyAction::DetachToRoot(payload.entity));
        }
        if response.dnd_hover_payload::<HierarchyDragPayload>().is_some() {
            ui.painter().rect_stroke(
                clamped,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x00, 0x7A, 0xCC)),
                egui::StrokeKind::Inside,
            );
        }
    }

    // Process deferred action.
    if let Some(action) = deferred_action {
        match action {
            DeferredHierarchyAction::DeleteEntity(entity) => {
                undo_system.record(scene);
                if *selection_context == Some(entity) {
                    *selection_context = None;
                }
                let _ = scene.destroy_entity(entity);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::CreateChild(parent) => {
                undo_system.record(scene);
                let child = scene.create_entity_with_tag("Empty Entity");
                scene.set_parent(child, parent, false);
                *selection_context = Some(child);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::Reparent { child, new_parent } => {
                undo_system.record(scene);
                scene.set_parent(child, new_parent, true);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::DetachToRoot(entity) => {
                if scene.get_parent(entity).is_some() {
                    undo_system.record(scene);
                    scene.detach_from_parent(entity, true);
                    *scene_dirty = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Recursive entity node drawing
// ---------------------------------------------------------------------------

fn draw_entity_node(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    tag: &str,
    selection_context: &mut Option<Entity>,
    scene_dirty: &mut bool,
    deferred_action: &mut Option<DeferredHierarchyAction>,
) {
    let children = scene.get_children(entity);
    let has_parent = scene.get_parent(entity).is_some();
    let selected = selection_context.is_some_and(|sel| sel == entity);

    if children.is_empty() {
        // Leaf node — simple selectable label.
        let response = ui.selectable_label(selected, tag);
        if response.clicked() {
            *selection_context = Some(entity);
        }
        entity_context_menu(&response, entity, deferred_action, scene_dirty, has_parent);
        handle_drag_source(ui, &response, entity);
        handle_drop_target(&response, ui, entity, deferred_action);
    } else {
        // Parent node — collapsing header with children.
        let id = ui.make_persistent_id(entity.id());
        let header = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            true,
        );
        let (_collapse_resp, header_ir, _body_ir) = header
            .show_header(ui, |ui| {
                let r = ui.selectable_label(selected, tag);
                if r.clicked() {
                    *selection_context = Some(entity);
                }
                entity_context_menu(&r, entity, deferred_action, scene_dirty, has_parent);
                handle_drag_source(ui, &r, entity);
                r
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
                            deferred_action,
                        );
                    }
                }
            });

        // Drop target on the header label.
        handle_drop_target(&header_ir.inner, ui, entity, deferred_action);
    }
}

// ---------------------------------------------------------------------------
// Drag-and-drop helpers
// ---------------------------------------------------------------------------

fn handle_drag_source(ui: &egui::Ui, response: &egui::Response, entity: Entity) {
    if response.drag_started() || response.dragged() {
        egui::DragAndDrop::set_payload(ui.ctx(), HierarchyDragPayload { entity });
    }
}

fn handle_drop_target(
    response: &egui::Response,
    ui: &egui::Ui,
    entity: Entity,
    deferred_action: &mut Option<DeferredHierarchyAction>,
) {
    if let Some(payload) = response.dnd_release_payload::<HierarchyDragPayload>() {
        if payload.entity != entity {
            *deferred_action = Some(DeferredHierarchyAction::Reparent {
                child: payload.entity,
                new_parent: entity,
            });
        }
    }
    if let Some(payload) = response.dnd_hover_payload::<HierarchyDragPayload>() {
        if payload.entity != entity {
            ui.painter().rect_stroke(
                response.rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0x00, 0x7A, 0xCC)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

fn entity_context_menu(
    response: &egui::Response,
    entity: Entity,
    deferred_action: &mut Option<DeferredHierarchyAction>,
    scene_dirty: &mut bool,
    has_parent: bool,
) {
    response.context_menu(|ui| {
        if ui.button("Create Child Entity").clicked() {
            *deferred_action = Some(DeferredHierarchyAction::CreateChild(entity));
            *scene_dirty = true;
            ui.close();
        }

        if has_parent {
            if ui.button("Detach from Parent").clicked() {
                *deferred_action = Some(DeferredHierarchyAction::DetachToRoot(entity));
                *scene_dirty = true;
                ui.close();
            }
        }

        ui.separator();

        if ui.button("Delete Entity").clicked() {
            *deferred_action = Some(DeferredHierarchyAction::DeleteEntity(entity));
            *scene_dirty = true;
            ui.close();
        }
    });
}
