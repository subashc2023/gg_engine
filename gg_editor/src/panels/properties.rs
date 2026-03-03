use gg_engine::egui;
use gg_engine::prelude::*;

use super::content_browser::ContentBrowserPayload;

pub(crate) fn properties_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    pending_texture_loads: &mut Vec<(Entity, std::path::PathBuf)>,
) {
    if let Some(entity) = *selection_context {
        if scene.is_alive(entity) {
            draw_components(ui, scene, entity, pending_texture_loads);
        } else {
            *selection_context = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Vec3 control (colored XYZ drag values with reset buttons)
// ---------------------------------------------------------------------------

/// Draw a labeled Vec3 control with colored X/Y/Z buttons that reset to
/// `reset_value` on click. `column_width` sets the label column width.
/// Returns `true` if any value changed.
fn draw_vec3_control(
    ui: &mut egui::Ui,
    label: &str,
    values: &mut Vec3,
    reset_value: f32,
    column_width: f32,
) -> bool {
    let mut changed = false;

    ui.push_id(label, |ui| {
        // Compute sizes based on current line height.
        let line_height =
            ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y;
        let button_size = egui::vec2(line_height + 3.0, line_height);

        ui.horizontal(|ui| {
            // Fixed-width label — takes exactly column_width, left-aligned.
            let (_, label_resp) =
                ui.allocate_exact_size(egui::vec2(column_width, line_height), egui::Sense::hover());
            ui.painter().text(
                label_resp.rect.left_center(),
                egui::Align2::LEFT_CENTER,
                label,
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().text_color(),
            );

            ui.spacing_mut().item_spacing.x = 0.0;

            let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

            // Compute a fixed width for each DragValue so all 3 groups fit.
            let spacing = 4.0 * 2.0; // two 4px gaps between XYZ groups
            let available = ui.available_width() - spacing - 3.0 * button_size.x;
            let drag_width = (available / 3.0).max(20.0);

            // --- X (red) ---
            let x_color = egui::Color32::from_rgba_unmultiplied(204, 26, 38, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("X")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family.clone())),
                    )
                    .fill(x_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.x = reset_value;
                changed = true;
            }

            // Drag value for X.
            let drag_x = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.x)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_x.changed() {
                changed = true;
            }

            ui.add_space(4.0);

            // --- Y (green) ---
            let y_color = egui::Color32::from_rgba_unmultiplied(47, 153, 47, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Y")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family.clone())),
                    )
                    .fill(y_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.y = reset_value;
                changed = true;
            }

            let drag_y = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.y)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_y.changed() {
                changed = true;
            }

            ui.add_space(4.0);

            // --- Z (blue) ---
            let z_color = egui::Color32::from_rgba_unmultiplied(20, 64, 204, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Z")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family)),
                    )
                    .fill(z_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.z = reset_value;
                changed = true;
            }

            let drag_z = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.z)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_z.changed() {
                changed = true;
            }
        });
    });

    changed
}

// ---------------------------------------------------------------------------
// Component inspector
// ---------------------------------------------------------------------------

fn draw_components(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    pending_texture_loads: &mut Vec<(Entity, std::path::PathBuf)>,
) {
    let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

    // -- Tag Component + Add Component button (inline) --
    if scene.has_component::<TagComponent>(entity) {
        let mut tag = scene
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_default();

        ui.horizontal(|ui| {
            if ui.text_edit_singleline(&mut tag).changed() {
                if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
                    tc.tag = tag;
                }
            }

            let add_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new("Add")
                        .color(egui::Color32::WHITE)
                        .font(egui::FontId::new(12.0, bold_family.clone())),
                )
                .fill(egui::Color32::from_rgb(0x00, 0x7A, 0xCC))
                .corner_radius(egui::CornerRadius::same(2)),
            );

            egui::Popup::from_toggle_button_response(&add_btn).show(|ui| {
                if !scene.has_component::<CameraComponent>(entity) && ui.button("Camera").clicked()
                {
                    scene.add_component(entity, CameraComponent::default());
                }
                if !scene.has_component::<SpriteRendererComponent>(entity)
                    && ui.button("Sprite Renderer").clicked()
                {
                    scene.add_component(entity, SpriteRendererComponent::default());
                }
                if !scene.has_component::<CircleRendererComponent>(entity)
                    && ui.button("Circle Renderer").clicked()
                {
                    scene.add_component(entity, CircleRendererComponent::default());
                }
                if !scene.has_component::<RigidBody2DComponent>(entity)
                    && ui.button("Rigidbody 2D").clicked()
                {
                    scene.add_component(entity, RigidBody2DComponent::default());
                }
                if !scene.has_component::<BoxCollider2DComponent>(entity)
                    && ui.button("Box Collider 2D").clicked()
                {
                    scene.add_component(entity, BoxCollider2DComponent::default());
                }
            });
        });
        ui.separator();
    }

    // -- Transform Component (not removable) --
    if scene.has_component::<TransformComponent>(entity) {
        egui::CollapsingHeader::new(
            egui::RichText::new("Transform").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("transform", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut translation, mut rotation_deg, mut scale) = {
                let tc = scene.get_component::<TransformComponent>(entity).unwrap();
                (
                    tc.translation,
                    Vec3::new(
                        tc.rotation.x.to_degrees(),
                        tc.rotation.y.to_degrees(),
                        tc.rotation.z.to_degrees(),
                    ),
                    tc.scale,
                )
            };

            let mut changed = false;
            changed |= draw_vec3_control(ui, "Translate", &mut translation, 0.0, 70.0);
            changed |= draw_vec3_control(ui, "Rotate", &mut rotation_deg, 0.0, 70.0);
            changed |= draw_vec3_control(ui, "Scale", &mut scale, 1.0, 70.0);

            if changed {
                if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(entity) {
                    tc.translation = translation;
                    tc.rotation = Vec3::new(
                        rotation_deg.x.to_radians(),
                        rotation_deg.y.to_radians(),
                        rotation_deg.z.to_radians(),
                    );
                    tc.scale = scale;
                }
            }
        });
    }

    // -- Camera Component (removable) --
    let mut remove_camera = false;
    if scene.has_component::<CameraComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Camera").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("camera", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            // Read all camera state up front.
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

            // Primary camera toggle — uses set_primary_camera to ensure
            // only one camera is primary at a time.
            if ui.checkbox(&mut primary, "Primary").changed() {
                if primary {
                    scene.set_primary_camera(entity);
                } else if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.primary = false;
                }
            }

            // Projection type combo box.
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

            // Projection-type-specific controls.
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

            // Fixed aspect ratio (applies to both projection types).
            changed |= ui
                .checkbox(&mut fixed_aspect, "Fixed Aspect Ratio")
                .changed();

            // Write back all changes.
            if changed {
                if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.fixed_aspect_ratio = fixed_aspect;

                    if cam.camera.projection_type() != proj_type {
                        cam.camera.set_projection_type(proj_type);
                    }

                    // Perspective parameters.
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

                    // Orthographic parameters.
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

        // Right-click header to remove.
        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_camera = true;
                ui.close();
            }
        });
    }
    if remove_camera {
        scene.remove_component::<CameraComponent>(entity);
    }

    // -- Sprite Renderer Component (removable) --
    let mut remove_sprite = false;
    if scene.has_component::<SpriteRendererComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Sprite Renderer")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("sprite_renderer", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut color_arr, _has_texture, mut tiling_factor) = {
                let sprite = scene
                    .get_component::<SpriteRendererComponent>(entity)
                    .unwrap();
                (
                    [
                        sprite.color.x,
                        sprite.color.y,
                        sprite.color.z,
                        sprite.color.w,
                    ],
                    sprite.texture.is_some(),
                    sprite.tiling_factor,
                )
            };

            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                (color_arr[0] * 255.0) as u8,
                (color_arr[1] * 255.0) as u8,
                (color_arr[2] * 255.0) as u8,
                (color_arr[3] * 255.0) as u8,
            );

            ui.horizontal(|ui| {
                ui.label("Color");
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut egui_color,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .changed()
                {
                    let [r, g, b, a] = egui_color.to_srgba_unmultiplied();
                    color_arr = [
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                        a as f32 / 255.0,
                    ];
                    if let Some(mut sprite) =
                        scene.get_component_mut::<SpriteRendererComponent>(entity)
                    {
                        sprite.color = Vec4::from(color_arr);
                    }
                }
            });

            // Texture drop target.
            let btn_resp = ui.add_sized(
                [100.0, 0.0],
                egui::Button::new("Texture"),
            );

            // Accept texture drag-and-drop from the content browser.
            if let Some(payload) =
                btn_resp.dnd_release_payload::<ContentBrowserPayload>()
            {
                if !payload.is_directory {
                    let ext = payload
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                        pending_texture_loads.push((entity, payload.path.clone()));
                    }
                }
            }

            // Visual feedback when dragging over the button.
            if btn_resp.dnd_hover_payload::<ContentBrowserPayload>().is_some() {
                ui.painter().rect_stroke(
                    btn_resp.rect,
                    egui::CornerRadius::same(2),
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(0x56, 0x9C, 0xD6)),
                    egui::StrokeKind::Inside,
                );
            }

            // Tiling factor.
            ui.horizontal(|ui| {
                ui.label("Tiling Factor");
                if ui
                    .add(
                        egui::DragValue::new(&mut tiling_factor)
                            .speed(0.1)
                            .range(0.0..=100.0),
                    )
                    .changed()
                {
                    if let Some(mut sprite) =
                        scene.get_component_mut::<SpriteRendererComponent>(entity)
                    {
                        sprite.tiling_factor = tiling_factor;
                    }
                }
            });
        });

        // Right-click header to remove.
        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_sprite = true;
                ui.close();
            }
        });
    }
    if remove_sprite {
        scene.remove_component::<SpriteRendererComponent>(entity);
    }

    // -- Circle Renderer Component (removable) --
    let mut remove_circle = false;
    if scene.has_component::<CircleRendererComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Circle Renderer")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("circle_renderer", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut color_arr, mut thickness, mut fade) = {
                let circle = scene
                    .get_component::<CircleRendererComponent>(entity)
                    .unwrap();
                (
                    [
                        circle.color.x,
                        circle.color.y,
                        circle.color.z,
                        circle.color.w,
                    ],
                    circle.thickness,
                    circle.fade,
                )
            };

            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                (color_arr[0] * 255.0) as u8,
                (color_arr[1] * 255.0) as u8,
                (color_arr[2] * 255.0) as u8,
                (color_arr[3] * 255.0) as u8,
            );

            ui.horizontal(|ui| {
                ui.label("Color");
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut egui_color,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .changed()
                {
                    let [r, g, b, a] = egui_color.to_srgba_unmultiplied();
                    color_arr = [
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                        a as f32 / 255.0,
                    ];
                    if let Some(mut circle) =
                        scene.get_component_mut::<CircleRendererComponent>(entity)
                    {
                        circle.color = Vec4::from(color_arr);
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Thickness");
                if ui
                    .add(
                        egui::DragValue::new(&mut thickness)
                            .speed(0.025)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    if let Some(mut circle) =
                        scene.get_component_mut::<CircleRendererComponent>(entity)
                    {
                        circle.thickness = thickness;
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Fade");
                if ui
                    .add(
                        egui::DragValue::new(&mut fade)
                            .speed(0.00025)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    if let Some(mut circle) =
                        scene.get_component_mut::<CircleRendererComponent>(entity)
                    {
                        circle.fade = fade;
                    }
                }
            });
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_circle = true;
                ui.close();
            }
        });
    }
    if remove_circle {
        scene.remove_component::<CircleRendererComponent>(entity);
    }

    // -- Rigidbody 2D Component (removable) --
    let mut remove_rb2d = false;
    if scene.has_component::<RigidBody2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Rigidbody 2D")
                .font(egui::FontId::new(14.0, bold_family.clone())),
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
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_rb2d = true;
                ui.close();
            }
        });
    }
    if remove_rb2d {
        scene.remove_component::<RigidBody2DComponent>(entity);
    }

    // -- Box Collider 2D Component (removable) --
    let mut remove_bc2d = false;
    if scene.has_component::<BoxCollider2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Box Collider 2D")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("box_collider_2d", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut offset, mut size, mut density, mut friction, mut restitution, mut restitution_threshold) = {
                let bc = scene
                    .get_component::<BoxCollider2DComponent>(entity)
                    .unwrap();
                (
                    bc.offset,
                    bc.size,
                    bc.density,
                    bc.friction,
                    bc.restitution,
                    bc.restitution_threshold,
                )
            };

            let mut changed = false;

            ui.horizontal(|ui| {
                ui.label("Offset");
                if ui
                    .add(egui::DragValue::new(&mut offset.x).speed(0.01).prefix("X: "))
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .add(egui::DragValue::new(&mut offset.y).speed(0.01).prefix("Y: "))
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
                ui.label("Restitution Threshold");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut restitution_threshold)
                            .speed(0.01)
                            .range(0.0..=f32::MAX),
                    )
                    .changed();
            });

            if changed {
                if let Some(mut bc) =
                    scene.get_component_mut::<BoxCollider2DComponent>(entity)
                {
                    bc.offset = offset;
                    bc.size = size;
                    bc.density = density;
                    bc.friction = friction;
                    bc.restitution = restitution;
                    bc.restitution_threshold = restitution_threshold;
                }
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_bc2d = true;
                ui.close();
            }
        });
    }
    if remove_bc2d {
        scene.remove_component::<BoxCollider2DComponent>(entity);
    }
}
