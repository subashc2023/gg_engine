use gg_engine::egui;
#[cfg(feature = "physics-3d")]
use gg_engine::glam::Vec3;
use gg_engine::prelude::*;

pub(crate) fn draw_rigidbody2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<RigidBody2DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Rigidbody 2D",
        "rigidbody_2d",
        bold_family,
        entity,
        |ui| {
            let (
                mut body_type,
                mut fixed_rotation,
                mut gravity_scale,
                mut linear_damping,
                mut angular_damping,
            ) = {
                let rb = scene.get_component::<RigidBody2DComponent>(entity).unwrap();
                (
                    rb.body_type,
                    rb.fixed_rotation,
                    rb.gravity_scale,
                    rb.linear_damping,
                    rb.angular_damping,
                )
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

            ui.horizontal(|ui| {
                ui.label("Gravity Scale");
                changed |= ui
                    .add(egui::DragValue::new(&mut gravity_scale).speed(0.01))
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Linear Damping");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut linear_damping)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Angular Damping");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut angular_damping)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            if changed {
                if let Some(mut rb) = scene.get_component_mut::<RigidBody2DComponent>(entity) {
                    rb.body_type = body_type;
                    rb.fixed_rotation = fixed_rotation;
                    rb.gravity_scale = gravity_scale;
                    rb.linear_damping = linear_damping;
                    rb.angular_damping = angular_damping;
                }
                *scene_dirty = true;
            }
        },
    )
}

pub(crate) fn draw_box_collider2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<BoxCollider2DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Box Collider 2D",
        "box_collider_2d",
        bold_family,
        entity,
        |ui| {
            let (
                mut offset,
                mut size,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
                mut is_sensor,
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
                    bc.is_sensor,
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

            changed |= draw_collider_material_ui(
                ui,
                &mut density,
                &mut friction,
                &mut restitution,
                &mut collision_layer,
                &mut collision_mask,
                &mut is_sensor,
            );

            if changed {
                if let Some(mut bc) = scene.get_component_mut::<BoxCollider2DComponent>(entity) {
                    bc.offset = offset;
                    bc.size = size;
                    bc.density = density;
                    bc.friction = friction;
                    bc.restitution = restitution;
                    bc.collision_layer = collision_layer;
                    bc.collision_mask = collision_mask;
                    bc.is_sensor = is_sensor;
                }
                *scene_dirty = true;
            }
        },
    )
}

pub(crate) fn draw_circle_collider2d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<CircleCollider2DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Circle Collider 2D",
        "circle_collider_2d",
        bold_family,
        entity,
        |ui| {
            let (
                mut offset,
                mut radius,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
                mut is_sensor,
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
                    cc.is_sensor,
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

            changed |= draw_collider_material_ui(
                ui,
                &mut density,
                &mut friction,
                &mut restitution,
                &mut collision_layer,
                &mut collision_mask,
                &mut is_sensor,
            );

            if changed {
                if let Some(mut cc) = scene.get_component_mut::<CircleCollider2DComponent>(entity) {
                    cc.offset = offset;
                    cc.radius = radius;
                    cc.density = density;
                    cc.friction = friction;
                    cc.restitution = restitution;
                    cc.collision_layer = collision_layer;
                    cc.collision_mask = collision_mask;
                    cc.is_sensor = is_sensor;
                }
                *scene_dirty = true;
            }
        },
    )
}

// ---------------------------------------------------------------------------
// 3D Physics Components
// ---------------------------------------------------------------------------

/// Shared UI for 3D collider material and collision filtering properties.
/// Returns true if any value changed.
fn draw_collider_material_ui(
    ui: &mut egui::Ui,
    density: &mut f32,
    friction: &mut f32,
    restitution: &mut f32,
    collision_layer: &mut u32,
    collision_mask: &mut u32,
    is_sensor: &mut bool,
) -> bool {
    let mut changed = false;

    ui.horizontal(|ui| {
        ui.label("Density");
        changed |= ui
            .add(
                egui::DragValue::new(density)
                    .speed(0.01)
                    .range(0.0..=f32::MAX),
            )
            .changed();
    });

    ui.horizontal(|ui| {
        ui.label("Friction");
        changed |= ui
            .add(egui::DragValue::new(friction).speed(0.01).range(0.0..=1.0))
            .changed();
    });

    ui.horizontal(|ui| {
        ui.label("Restitution");
        changed |= ui
            .add(
                egui::DragValue::new(restitution)
                    .speed(0.01)
                    .range(0.0..=1.0),
            )
            .changed();
    });

    ui.horizontal(|ui| {
        ui.label("Collision Layer");
        changed |= ui
            .add(egui::DragValue::new(collision_layer).hexadecimal(8, false, true))
            .changed();
    });

    ui.horizontal(|ui| {
        ui.label("Collision Mask");
        changed |= ui
            .add(egui::DragValue::new(collision_mask).hexadecimal(8, false, true))
            .changed();
    });

    changed |= ui.checkbox(is_sensor, "Is Sensor (Trigger)").changed();

    changed
}

#[cfg(feature = "physics-3d")]
/// Shared UI for a Vec3 offset field.
fn draw_offset_3d(ui: &mut egui::Ui, offset: &mut Vec3) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Offset");
        changed |= ui
            .add(
                egui::DragValue::new(&mut offset.x)
                    .speed(0.01)
                    .prefix("X: "),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut offset.y)
                    .speed(0.01)
                    .prefix("Y: "),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut offset.z)
                    .speed(0.01)
                    .prefix("Z: "),
            )
            .changed();
    });
    changed
}

#[cfg(feature = "physics-3d")]
pub(crate) fn draw_rigidbody3d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<RigidBody3DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Rigidbody 3D",
        "rigidbody_3d",
        bold_family,
        entity,
        |ui| {
            let (
                mut body_type,
                mut lock_rotation_x,
                mut lock_rotation_y,
                mut lock_rotation_z,
                mut gravity_scale,
                mut linear_damping,
                mut angular_damping,
            ) = {
                let rb = scene.get_component::<RigidBody3DComponent>(entity).unwrap();
                (
                    rb.body_type,
                    rb.lock_rotation_x,
                    rb.lock_rotation_y,
                    rb.lock_rotation_z,
                    rb.gravity_scale,
                    rb.linear_damping,
                    rb.angular_damping,
                )
            };

            let mut changed = false;

            let body_type_strings = ["Static", "Dynamic", "Kinematic"];
            let current_label = match body_type {
                RigidBody3DType::Static => body_type_strings[0],
                RigidBody3DType::Dynamic => body_type_strings[1],
                RigidBody3DType::Kinematic => body_type_strings[2],
            };

            egui::ComboBox::from_label("Body Type")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut body_type, RigidBody3DType::Static, "Static")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut body_type, RigidBody3DType::Dynamic, "Dynamic")
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut body_type, RigidBody3DType::Kinematic, "Kinematic")
                        .changed()
                    {
                        changed = true;
                    }
                });

            ui.label("Lock Rotation");
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut lock_rotation_x, "X").changed();
                changed |= ui.checkbox(&mut lock_rotation_y, "Y").changed();
                changed |= ui.checkbox(&mut lock_rotation_z, "Z").changed();
            });

            ui.horizontal(|ui| {
                ui.label("Gravity Scale");
                changed |= ui
                    .add(egui::DragValue::new(&mut gravity_scale).speed(0.01))
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Linear Damping");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut linear_damping)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            ui.horizontal(|ui| {
                ui.label("Angular Damping");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut angular_damping)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            if changed {
                if let Some(mut rb) = scene.get_component_mut::<RigidBody3DComponent>(entity) {
                    rb.body_type = body_type;
                    rb.lock_rotation_x = lock_rotation_x;
                    rb.lock_rotation_y = lock_rotation_y;
                    rb.lock_rotation_z = lock_rotation_z;
                    rb.gravity_scale = gravity_scale;
                    rb.linear_damping = linear_damping;
                    rb.angular_damping = angular_damping;
                }
                *scene_dirty = true;
            }
        },
    )
}

#[cfg(feature = "physics-3d")]
pub(crate) fn draw_box_collider3d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<BoxCollider3DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Box Collider 3D",
        "box_collider_3d",
        bold_family,
        entity,
        |ui| {
            let (
                mut offset,
                mut size,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
                mut is_sensor,
            ) = {
                let bc = scene
                    .get_component::<BoxCollider3DComponent>(entity)
                    .unwrap();
                (
                    bc.offset,
                    bc.size,
                    bc.density,
                    bc.friction,
                    bc.restitution,
                    bc.collision_layer,
                    bc.collision_mask,
                    bc.is_sensor,
                )
            };

            let mut changed = false;

            changed |= draw_offset_3d(ui, &mut offset);

            ui.horizontal(|ui| {
                ui.label("Size");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut size.x)
                            .speed(0.01)
                            .range(0.01..=f32::MAX)
                            .prefix("X: "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut size.y)
                            .speed(0.01)
                            .range(0.01..=f32::MAX)
                            .prefix("Y: "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut size.z)
                            .speed(0.01)
                            .range(0.01..=f32::MAX)
                            .prefix("Z: "),
                    )
                    .changed();
            });

            changed |= draw_collider_material_ui(
                ui,
                &mut density,
                &mut friction,
                &mut restitution,
                &mut collision_layer,
                &mut collision_mask,
                &mut is_sensor,
            );

            if changed {
                if let Some(mut bc) = scene.get_component_mut::<BoxCollider3DComponent>(entity) {
                    bc.offset = offset;
                    bc.size = size;
                    bc.density = density;
                    bc.friction = friction;
                    bc.restitution = restitution;
                    bc.collision_layer = collision_layer;
                    bc.collision_mask = collision_mask;
                    bc.is_sensor = is_sensor;
                }
                *scene_dirty = true;
            }
        },
    )
}

#[cfg(feature = "physics-3d")]
pub(crate) fn draw_sphere_collider3d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<SphereCollider3DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Sphere Collider 3D",
        "sphere_collider_3d",
        bold_family,
        entity,
        |ui| {
            let (
                mut offset,
                mut radius,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
                mut is_sensor,
            ) = {
                let sc = scene
                    .get_component::<SphereCollider3DComponent>(entity)
                    .unwrap();
                (
                    sc.offset,
                    sc.radius,
                    sc.density,
                    sc.friction,
                    sc.restitution,
                    sc.collision_layer,
                    sc.collision_mask,
                    sc.is_sensor,
                )
            };

            let mut changed = false;

            changed |= draw_offset_3d(ui, &mut offset);

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

            changed |= draw_collider_material_ui(
                ui,
                &mut density,
                &mut friction,
                &mut restitution,
                &mut collision_layer,
                &mut collision_mask,
                &mut is_sensor,
            );

            if changed {
                if let Some(mut sc) = scene.get_component_mut::<SphereCollider3DComponent>(entity) {
                    sc.offset = offset;
                    sc.radius = radius;
                    sc.density = density;
                    sc.friction = friction;
                    sc.restitution = restitution;
                    sc.collision_layer = collision_layer;
                    sc.collision_mask = collision_mask;
                    sc.is_sensor = is_sensor;
                }
                *scene_dirty = true;
            }
        },
    )
}

#[cfg(feature = "physics-3d")]
pub(crate) fn draw_capsule_collider3d_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<CapsuleCollider3DComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Capsule Collider 3D",
        "capsule_collider_3d",
        bold_family,
        entity,
        |ui| {
            let (
                mut offset,
                mut half_height,
                mut radius,
                mut density,
                mut friction,
                mut restitution,
                mut collision_layer,
                mut collision_mask,
                mut is_sensor,
            ) = {
                let cc = scene
                    .get_component::<CapsuleCollider3DComponent>(entity)
                    .unwrap();
                (
                    cc.offset,
                    cc.half_height,
                    cc.radius,
                    cc.density,
                    cc.friction,
                    cc.restitution,
                    cc.collision_layer,
                    cc.collision_mask,
                    cc.is_sensor,
                )
            };

            let mut changed = false;

            changed |= draw_offset_3d(ui, &mut offset);

            ui.horizontal(|ui| {
                ui.label("Half Height");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut half_height)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
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

            changed |= draw_collider_material_ui(
                ui,
                &mut density,
                &mut friction,
                &mut restitution,
                &mut collision_layer,
                &mut collision_mask,
                &mut is_sensor,
            );

            if changed {
                if let Some(mut cc) = scene.get_component_mut::<CapsuleCollider3DComponent>(entity)
                {
                    cc.offset = offset;
                    cc.half_height = half_height;
                    cc.radius = radius;
                    cc.density = density;
                    cc.friction = friction;
                    cc.restitution = restitution;
                    cc.collision_layer = collision_layer;
                    cc.collision_mask = collision_mask;
                    cc.is_sensor = is_sensor;
                }
                *scene_dirty = true;
            }
        },
    )
}
