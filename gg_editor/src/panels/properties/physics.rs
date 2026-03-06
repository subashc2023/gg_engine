use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_rigidbody2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<RigidBody2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Rigidbody 2D").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("rigidbody_2d", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut body_type, mut fixed_rotation) = {
                let rb = scene.get_component::<RigidBody2DComponent>(entity).unwrap();
                (rb.body_type, rb.fixed_rotation)
            };

            let mut changed = false;

            let body_type_strings = ["Static", "Dynamic", "Kinematic"];
            let current_label = match body_type {
                RigidBody2DType::Static => body_type_strings[0],
                RigidBody2DType::Dynamic => body_type_strings[1],
                RigidBody2DType::Kinematic => body_type_strings[2],
            };

            egui::ComboBox::from_label("Body Type")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut body_type, RigidBody2DType::Static, "Static")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut body_type, RigidBody2DType::Dynamic, "Dynamic")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut body_type, RigidBody2DType::Kinematic, "Kinematic")
                        .changed()
                    {
                        changed = true;
                    }
                });

            changed |= ui.checkbox(&mut fixed_rotation, "Fixed Rotation").changed();

            if changed {
                if let Some(mut rb) = scene.get_component_mut::<RigidBody2DComponent>(entity) {
                    rb.body_type = body_type;
                    rb.fixed_rotation = fixed_rotation;
                }
                *scene_dirty = true;
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove = true;
                ui.close();
            }
        });
    }

    remove
}

pub(crate) fn draw_box_collider2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<BoxCollider2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Box Collider 2D")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("box_collider_2d", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (
                mut offset,
                mut size,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
            ) = {
                let bc = scene
                    .get_component::<BoxCollider2DComponent>(entity)
                    .unwrap();
                (
                    bc.offset,
                    bc.size,
                    bc.density,
                    bc.friction,
                    bc.restitution,
                    bc.collision_layer,
                    bc.collision_mask,
                )
            };

            let mut changed = false;

            ui.horizontal(|ui| {
                ui.label("Offset");
                if ui
                    .add(
                        egui::DragValue::new(&mut offset.x)
                            .speed(0.01)
                            .prefix("X: "),
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut offset.y)
                            .speed(0.01)
                            .prefix("Y: "),
                    )
                    .changed()
                {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Size");
                if ui
                    .add(
                        egui::DragValue::new(&mut size.x)
                            .speed(0.01)
                            .range(0.01..=f32::MAX)
                            .prefix("X: "),
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut size.y)
                            .speed(0.01)
                            .range(0.01..=f32::MAX)
                            .prefix("Y: "),
                    )
                    .changed()
                {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Density");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut density)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Friction");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut friction)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Restitution");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut restitution)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Collision Layer");
                changed |= ui
                    .add(egui::DragValue::new(&mut collision_layer).hexadecimal(8, false, true))
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Collision Mask");
                changed |= ui
                    .add(egui::DragValue::new(&mut collision_mask).hexadecimal(8, false, true))
                    .changed();
            });

            if changed {
                if let Some(mut bc) = scene.get_component_mut::<BoxCollider2DComponent>(entity) {
                    bc.offset = offset;
                    bc.size = size;
                    bc.density = density;
                    bc.friction = friction;
                    bc.restitution = restitution;
                    bc.collision_layer = collision_layer;
                    bc.collision_mask = collision_mask;
                }
                *scene_dirty = true;
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove = true;
                ui.close();
            }
        });
    }

    remove
}

pub(crate) fn draw_circle_collider2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<CircleCollider2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Circle Collider 2D")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("circle_collider_2d", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (
                mut offset,
                mut radius,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
            ) = {
                let cc = scene
                    .get_component::<CircleCollider2DComponent>(entity)
                    .unwrap();
                (
                    cc.offset,
                    cc.radius,
                    cc.density,
                    cc.friction,
                    cc.restitution,
                    cc.collision_layer,
                    cc.collision_mask,
                )
            };

            let mut changed = false;

            ui.horizontal(|ui| {
                ui.label("Offset");
                if ui
                    .add(
                        egui::DragValue::new(&mut offset.x)
                            .speed(0.01)
                            .prefix("X: "),
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(
                        egui::DragValue::new(&mut offset.y)
                            .speed(0.01)
                            .prefix("Y: "),
                    )
                    .changed()
                {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Radius");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut radius)
                            .speed(0.01)
                            .range(0.001..=f32::MAX),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Density");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut density)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Friction");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut friction)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Restitution");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut restitution)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Collision Layer");
                changed |= ui
                    .add(egui::DragValue::new(&mut collision_layer).hexadecimal(8, false, true))
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Collision Mask");
                changed |= ui
                    .add(egui::DragValue::new(&mut collision_mask).hexadecimal(8, false, true))
                    .changed();
            });

            if changed {
                if let Some(mut cc) = scene.get_component_mut::<CircleCollider2DComponent>(entity) {
                    cc.offset = offset;
                    cc.radius = radius;
                    cc.density = density;
                    cc.friction = friction;
                    cc.restitution = restitution;
                    cc.collision_layer = collision_layer;
                    cc.collision_mask = collision_mask;
                }
                *scene_dirty = true;
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove = true;
                ui.close();
            }
        });
    }

    remove
}
