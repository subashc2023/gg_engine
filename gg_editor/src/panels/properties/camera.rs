use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_camera_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    _scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<CameraComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Camera").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("camera", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (
                mut primary,
                mut fixed_aspect,
                mut proj_type,
                mut ortho_size,
                mut ortho_near,
                mut ortho_far,
                mut persp_fov_deg,
                mut persp_near,
                mut persp_far,
            ) = {
                let cam = scene.get_component::<CameraComponent>(entity).unwrap();
                (
                    cam.primary,
                    cam.fixed_aspect_ratio,
                    cam.camera.projection_type(),
                    cam.camera.orthographic_size(),
                    cam.camera.orthographic_near(),
                    cam.camera.orthographic_far(),
                    cam.camera.perspective_vertical_fov().to_degrees(),
                    cam.camera.perspective_near(),
                    cam.camera.perspective_far(),
                )
            };

            let mut changed = false;

            if ui.checkbox(&mut primary, "Primary").changed() {
                if primary {
                    scene.set_primary_camera(entity);
                } else if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.primary = false;
                }
            }

            let proj_type_strings = ["Perspective", "Orthographic"];
            let current_label = proj_type_strings[proj_type as usize];
            egui::ComboBox::from_label("Projection")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut proj_type,
                            ProjectionType::Perspective,
                            proj_type_strings[0],
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(
                            &mut proj_type,
                            ProjectionType::Orthographic,
                            proj_type_strings[1],
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });

            match proj_type {
                ProjectionType::Perspective => {
                    ui.horizontal(|ui| {
                        ui.label("Vertical FOV");
                        if ui
                            .add(
                                egui::DragValue::new(&mut persp_fov_deg)
                                    .speed(0.1)
                                    .range(1.0..=179.0)
                                    .suffix("°"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near");
                        if ui
                            .add(
                                egui::DragValue::new(&mut persp_near)
                                    .speed(0.01)
                                    .range(0.001..=f32::MAX),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far");
                        if ui
                            .add(egui::DragValue::new(&mut persp_far).speed(1.0))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                }
                ProjectionType::Orthographic => {
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        if ui
                            .add(
                                egui::DragValue::new(&mut ortho_size)
                                    .speed(0.1)
                                    .range(0.1..=1000.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near");
                        if ui
                            .add(egui::DragValue::new(&mut ortho_near).speed(0.1))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far");
                        if ui
                            .add(egui::DragValue::new(&mut ortho_far).speed(0.1))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                }
            }

            changed |= ui
                .checkbox(&mut fixed_aspect, "Fixed Aspect Ratio")
                .changed();

            if changed {
                if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.fixed_aspect_ratio = fixed_aspect;

                    if cam.camera.projection_type() != proj_type {
                        cam.camera.set_projection_type(proj_type);
                    }

                    let new_fov_rad = persp_fov_deg.to_radians();
                    if (cam.camera.perspective_vertical_fov() - new_fov_rad).abs() > f32::EPSILON {
                        cam.camera.set_perspective_vertical_fov(new_fov_rad);
                    }
                    if (cam.camera.perspective_near() - persp_near).abs() > f32::EPSILON {
                        cam.camera.set_perspective_near(persp_near);
                    }
                    if (cam.camera.perspective_far() - persp_far).abs() > f32::EPSILON {
                        cam.camera.set_perspective_far(persp_far);
                    }

                    if (cam.camera.orthographic_size() - ortho_size).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_size(ortho_size);
                    }
                    if (cam.camera.orthographic_near() - ortho_near).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_near(ortho_near);
                    }
                    if (cam.camera.orthographic_far() - ortho_far).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_far(ortho_far);
                    }
                }
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
