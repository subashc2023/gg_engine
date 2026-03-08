use std::path::PathBuf;

use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;
use crate::selection::Selection;

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
    Reparent {
        child: Entity,
        new_parent: Entity,
    },
    DetachToRoot(Entity),
    ReorderSibling {
        child_uuid: u64,
        new_index: usize,
    },
    InstantiatePrefab {
        path: PathBuf,
        parent: Option<Entity>,
    },
}

/// Actions that the editor main loop must handle (require broader context).
pub(crate) enum HierarchyExternalAction {
    SaveAsPrefab(Entity),
    InstantiatePrefab {
        path: PathBuf,
        parent: Option<Entity>,
    },
}

/// Fraction of item height at top/bottom that triggers reorder vs reparent.
const REORDER_EDGE_FRACTION: f32 = 0.3;

// ---------------------------------------------------------------------------
// Main panel UI
// ---------------------------------------------------------------------------

pub(crate) fn scene_hierarchy_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection: &mut Selection,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
    filter: &mut String,
) -> Option<HierarchyExternalAction> {
    // Search box.
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.text_edit_singleline(filter);
        if !filter.is_empty() && ui.small_button("X").clicked() {
            filter.clear();
        }
    });
    ui.add_space(2.0);

    let filter_lower = filter.trim().to_lowercase();

    let root_entities = scene.root_entities();
    let mut deferred_action: Option<DeferredHierarchyAction> = None;
    let mut external_action: Option<HierarchyExternalAction> = None;

    for (entity, tag) in &root_entities {
        if !filter_lower.is_empty() && !entity_matches_filter(scene, *entity, tag, &filter_lower) {
            continue;
        }
        draw_entity_node(
            ui,
            scene,
            *entity,
            tag,
            selection,
            scene_dirty,
            &mut deferred_action,
            &mut external_action,
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
            selection.clear();
        }
        // Right-click on blank space → create entity.
        response.context_menu(|ui| {
            if ui.button("Create Empty Entity").clicked() {
                undo_system.record(scene, "Create entity");
                let e = scene.create_entity_with_tag("Empty Entity");
                selection.set(e);
                *scene_dirty = true;
                ui.close();
            }
        });

        // Drop target: detach dragged entity to root.
        if let Some(payload) = response.dnd_release_payload::<HierarchyDragPayload>() {
            deferred_action = Some(DeferredHierarchyAction::DetachToRoot(payload.entity));
        }
        // Drop target: instantiate prefab from content browser.
        if let Some(payload) = response.dnd_release_payload::<ContentBrowserPayload>() {
            if is_prefab_file(&payload.path) {
                deferred_action = Some(DeferredHierarchyAction::InstantiatePrefab {
                    path: payload.path.clone(),
                    parent: None,
                });
            }
        }
        let has_hierarchy_hover = response
            .dnd_hover_payload::<HierarchyDragPayload>()
            .is_some();
        let has_prefab_hover = response
            .dnd_hover_payload::<ContentBrowserPayload>()
            .is_some_and(|p| is_prefab_file(&p.path));
        if has_hierarchy_hover || has_prefab_hover {
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
                undo_system.record(scene, "Delete entity");
                selection.remove(entity);
                let _ = scene.destroy_entity(entity);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::CreateChild(parent) => {
                undo_system.record(scene, "Create child entity");
                let child = scene.create_entity_with_tag("Empty Entity");
                scene.set_parent(child, parent, false);
                selection.set(child);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::Reparent { child, new_parent } => {
                undo_system.record(scene, "Reparent entity");
                scene.set_parent(child, new_parent, true);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::DetachToRoot(entity) => {
                if scene.get_parent(entity).is_some() {
                    undo_system.record(scene, "Detach from parent");
                    scene.detach_from_parent(entity, true);
                    *scene_dirty = true;
                }
            }
            DeferredHierarchyAction::ReorderSibling {
                child_uuid,
                new_index,
            } => {
                undo_system.record(scene, "Reorder entity");
                scene.reorder_child(child_uuid, new_index);
                *scene_dirty = true;
            }
            DeferredHierarchyAction::InstantiatePrefab { path, parent } => {
                external_action = Some(HierarchyExternalAction::InstantiatePrefab { path, parent });
            }
        }
    }
    external_action
}

// ---------------------------------------------------------------------------
// Recursive entity node drawing
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_entity_node(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    tag: &str,
    selection: &mut Selection,
    scene_dirty: &mut bool,
    deferred_action: &mut Option<DeferredHierarchyAction>,
    external_action: &mut Option<HierarchyExternalAction>,
) {
    let children = scene.get_children(entity);
    let has_parent = scene.get_parent(entity).is_some();
    let selected = selection.contains(entity);

    if children.is_empty() {
        // Leaf node — simple selectable label.
        let response = ui.selectable_label(selected, tag);
        if response.clicked() {
            let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
            if ctrl {
                selection.toggle(entity);
            } else {
                selection.set(entity);
            }
        }
        entity_context_menu(
            &response,
            entity,
            deferred_action,
            external_action,
            scene_dirty,
            has_parent,
        );
        handle_drag_source(ui, &response, entity);
        handle_drop_target(&response, ui, entity, deferred_action, scene);
    } else {
        // Parent node — collapsing header with children.
        let id = ui.make_persistent_id(entity.id());
        let header =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let (_collapse_resp, header_ir, _body_ir) = header
            .show_header(ui, |ui| {
                let r = ui.selectable_label(selected, tag);
                if r.clicked() {
                    let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
                    if ctrl {
                        selection.toggle(entity);
                    } else {
                        selection.set(entity);
                    }
                }
                entity_context_menu(
                    &r,
                    entity,
                    deferred_action,
                    external_action,
                    scene_dirty,
                    has_parent,
                );
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
                            selection,
                            scene_dirty,
                            deferred_action,
                            external_action,
                        );
                    }
                }
            });

        // Drop target on the header label.
        handle_drop_target(&header_ir.inner, ui, entity, deferred_action, scene);
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

/// Determines drop zone based on cursor Y position relative to item rect.
/// Returns (is_reorder, insert_before) — if not reorder, it's a reparent.
fn drop_zone(response: &egui::Response, cursor_y: f32) -> (bool, bool) {
    let rect = response.rect;
    let edge = rect.height() * REORDER_EDGE_FRACTION;
    if cursor_y < rect.min.y + edge {
        (true, true) // top edge → insert before
    } else if cursor_y > rect.max.y - edge {
        (true, false) // bottom edge → insert after
    } else {
        (false, false) // center → reparent
    }
}

fn handle_drop_target(
    response: &egui::Response,
    ui: &egui::Ui,
    entity: Entity,
    deferred_action: &mut Option<DeferredHierarchyAction>,
    scene: &Scene,
) {
    let cursor_y = ui.input(|i| i.pointer.hover_pos().map(|p| p.y));

    if let Some(payload) = response.dnd_release_payload::<HierarchyDragPayload>() {
        if payload.entity != entity {
            if let Some(cy) = cursor_y {
                let (is_reorder, insert_before) = drop_zone(response, cy);
                if is_reorder {
                    if let Some(action) =
                        compute_reorder_action(scene, entity, &payload, insert_before)
                    {
                        *deferred_action = Some(action);
                    } else {
                        // Fallback to reparent if not siblings.
                        *deferred_action = Some(DeferredHierarchyAction::Reparent {
                            child: payload.entity,
                            new_parent: entity,
                        });
                    }
                } else {
                    *deferred_action = Some(DeferredHierarchyAction::Reparent {
                        child: payload.entity,
                        new_parent: entity,
                    });
                }
            } else {
                *deferred_action = Some(DeferredHierarchyAction::Reparent {
                    child: payload.entity,
                    new_parent: entity,
                });
            }
        }
    }
    // Content browser drop: instantiate prefab as child of this entity.
    if let Some(payload) = response.dnd_release_payload::<ContentBrowserPayload>() {
        if is_prefab_file(&payload.path) {
            *deferred_action = Some(DeferredHierarchyAction::InstantiatePrefab {
                path: payload.path.clone(),
                parent: Some(entity),
            });
        }
    }

    let has_hierarchy_hover = response
        .dnd_hover_payload::<HierarchyDragPayload>()
        .is_some_and(|p| p.entity != entity);
    let has_prefab_hover = response
        .dnd_hover_payload::<ContentBrowserPayload>()
        .is_some_and(|p| is_prefab_file(&p.path));

    if has_hierarchy_hover {
        let accent = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);
        if let Some(cy) = cursor_y {
            let (is_reorder, insert_before) = drop_zone(response, cy);
            if is_reorder
                && are_siblings(
                    scene,
                    entity,
                    response
                        .dnd_hover_payload::<HierarchyDragPayload>()
                        .unwrap()
                        .entity,
                )
            {
                let line_y = if insert_before {
                    response.rect.min.y
                } else {
                    response.rect.max.y
                };
                ui.painter().hline(
                    response.rect.min.x..=response.rect.max.x,
                    line_y,
                    egui::Stroke::new(2.0, accent),
                );
            } else {
                ui.painter().rect_stroke(
                    response.rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(2.0, accent),
                    egui::StrokeKind::Inside,
                );
            }
        } else {
            ui.painter().rect_stroke(
                response.rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(2.0, accent),
                egui::StrokeKind::Inside,
            );
        }
    } else if has_prefab_hover {
        let accent = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);
        ui.painter().rect_stroke(
            response.rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(2.0, accent),
            egui::StrokeKind::Inside,
        );
    }
}

/// Check if two entities share the same parent.
fn are_siblings(scene: &Scene, a: Entity, b: Entity) -> bool {
    let pa = scene.get_parent(a);
    let pb = scene.get_parent(b);
    pa.is_some() && pa == pb
}

/// Compute a ReorderSibling action for dropping a payload entity relative to
/// the target entity (insert before or after).
fn compute_reorder_action(
    scene: &Scene,
    target: Entity,
    payload: &HierarchyDragPayload,
    insert_before: bool,
) -> Option<DeferredHierarchyAction> {
    // Both must share the same parent.
    let target_parent = scene.get_parent(target)?;
    let payload_parent = scene.get_parent(payload.entity)?;
    if target_parent != payload_parent {
        return None;
    }

    let parent_entity = scene.find_entity_by_uuid(target_parent)?;
    let children = scene.get_children(parent_entity);

    let target_uuid = scene
        .get_component::<IdComponent>(target)
        .map(|id| id.id.raw())?;
    let child_uuid = scene
        .get_component::<IdComponent>(payload.entity)
        .map(|id| id.id.raw())?;

    let target_idx = children.iter().position(|&c| c == target_uuid)?;

    let new_index = if insert_before {
        target_idx
    } else {
        target_idx + 1
    };

    Some(DeferredHierarchyAction::ReorderSibling {
        child_uuid,
        new_index,
    })
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

fn entity_context_menu(
    response: &egui::Response,
    entity: Entity,
    deferred_action: &mut Option<DeferredHierarchyAction>,
    external_action: &mut Option<HierarchyExternalAction>,
    scene_dirty: &mut bool,
    has_parent: bool,
) {
    response.context_menu(|ui| {
        if ui.button("Create Child Entity").clicked() {
            *deferred_action = Some(DeferredHierarchyAction::CreateChild(entity));
            *scene_dirty = true;
            ui.close();
        }

        if has_parent && ui.button("Detach from Parent").clicked() {
            *deferred_action = Some(DeferredHierarchyAction::DetachToRoot(entity));
            *scene_dirty = true;
            ui.close();
        }

        ui.separator();

        if ui.button("Save as Prefab...").clicked() {
            *external_action = Some(HierarchyExternalAction::SaveAsPrefab(entity));
            ui.close();
        }

        ui.separator();

        if ui.button("Delete Entity").clicked() {
            *deferred_action = Some(DeferredHierarchyAction::DeleteEntity(entity));
            *scene_dirty = true;
            ui.close();
        }
    });
}

// ---------------------------------------------------------------------------
// Filter helper — returns true if entity name or any descendant matches
// ---------------------------------------------------------------------------

fn is_prefab_file(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|ext| ext == "ggprefab")
}

fn entity_matches_filter(scene: &Scene, entity: Entity, tag: &str, filter: &str) -> bool {
    if tag.to_lowercase().contains(filter) {
        return true;
    }
    // Check children recursively.
    for child_uuid in scene.get_children(entity) {
        if let Some(child_entity) = scene.find_entity_by_uuid(child_uuid) {
            let child_tag = scene
                .get_component::<TagComponent>(child_entity)
                .map(|t| t.tag.clone())
                .unwrap_or_default();
            if entity_matches_filter(scene, child_entity, &child_tag, filter) {
                return true;
            }
        }
    }
    false
}
