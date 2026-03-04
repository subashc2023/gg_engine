use gg_engine::egui;
use gg_engine::prelude::*;

use super::content_browser::ContentBrowserPayload;

#[cfg(feature = "lua-scripting")]
use std::cell::RefCell;
#[cfg(feature = "lua-scripting")]
use std::collections::HashMap;

// Cache for discovered script fields (avoids re-executing scripts every frame).
// Keyed by script path -> field list. Invalidated when script path changes.
#[cfg(feature = "lua-scripting")]
thread_local! {
    static FIELD_CACHE: RefCell<HashMap<String, Vec<(String, ScriptFieldValue)>>> =
        RefCell::new(HashMap::new());
}

#[cfg(feature = "lua-scripting")]
fn get_cached_fields(script_path: &str) -> Vec<(String, ScriptFieldValue)> {
    if script_path.is_empty() {
        return Vec::new();
    }
    FIELD_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(fields) = cache.get(script_path) {
            return fields.clone();
        }
        let fields = ScriptEngine::discover_fields(script_path);
        cache.insert(script_path.to_string(), fields.clone());
        fields
    })
}

pub(crate) fn properties_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    asset_manager: &mut Option<EditorAssetManager>,
    is_playing: bool,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
) {
    if let Some(entity) = *selection_context {
        if scene.is_alive(entity) {
            draw_components(ui, scene, entity, asset_manager, is_playing, assets_root, scene_dirty);
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
    asset_manager: &mut Option<EditorAssetManager>,
    is_playing: bool,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
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
                    *scene_dirty = true;
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
                    *scene_dirty = true;
                }
                if !scene.has_component::<SpriteRendererComponent>(entity)
                    && ui.button("Sprite Renderer").clicked()
                {
                    scene.add_component(entity, SpriteRendererComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<CircleRendererComponent>(entity)
                    && ui.button("Circle Renderer").clicked()
                {
                    scene.add_component(entity, CircleRendererComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<TextComponent>(entity)
                    && ui.button("Text").clicked()
                {
                    scene.add_component(entity, TextComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<RigidBody2DComponent>(entity)
                    && ui.button("Rigidbody 2D").clicked()
                {
                    scene.add_component(entity, RigidBody2DComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<BoxCollider2DComponent>(entity)
                    && ui.button("Box Collider 2D").clicked()
                {
                    scene.add_component(entity, BoxCollider2DComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<CircleCollider2DComponent>(entity)
                    && ui.button("Circle Collider 2D").clicked()
                {
                    scene.add_component(entity, CircleCollider2DComponent::default());
                    *scene_dirty = true;
                }
                #[cfg(feature = "lua-scripting")]
                if !scene.has_component::<LuaScriptComponent>(entity)
                    && ui.button("Lua Script").clicked()
                {
                    scene.add_component(entity, LuaScriptComponent::default());
                    *scene_dirty = true;
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
                    *scene_dirty = true;
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
        *scene_dirty = true;
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
            let (mut color_arr, texture_handle_raw, mut tiling_factor) = {
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
                    sprite.texture_handle.raw(),
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

            // Texture button label: show filename from asset metadata or "None".
            let texture_label = if texture_handle_raw != 0 {
                if let Some(am) = asset_manager.as_ref() {
                    let handle = Uuid::from_raw(texture_handle_raw);
                    am.get_metadata(&handle)
                        .map(|m| {
                            std::path::Path::new(&m.file_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| m.file_path.clone())
                        })
                        .unwrap_or_else(|| "Invalid".to_string())
                } else {
                    "No Asset Manager".to_string()
                }
            } else {
                "None".to_string()
            };

            // Calculate button width from text (approximate: 7px per char).
            let btn_width = ((texture_label.len() as f32) * 7.0 + 20.0).max(100.0);

            ui.horizontal(|ui| {
                let btn_resp = ui.add_sized(
                    [btn_width, 0.0],
                    egui::Button::new(&texture_label),
                );

                // Click to open file dialog in assets/textures.
                if btn_resp.clicked() {
                    if let Some(am) = asset_manager.as_mut() {
                        let textures_dir = assets_root.join("textures");
                        let textures_dir_str = textures_dir.to_string_lossy();
                        if let Some(path_str) =
                            FileDialogs::open_file_in("Image files", &["png", "jpg", "jpeg"], &textures_dir_str)
                        {
                            // Make path relative to the asset directory.
                            let abs_path = std::path::PathBuf::from(&path_str);
                            let rel_path = abs_path
                                .strip_prefix(am.asset_directory())
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or(path_str);
                            let handle = am.import_asset(&rel_path);
                            if let Some(mut sprite) =
                                scene.get_component_mut::<SpriteRendererComponent>(entity)
                            {
                                sprite.texture_handle = handle;
                                sprite.texture = None; // Will be resolved in on_render
                            }
                            *scene_dirty = true;
                        }
                    }
                }

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
                            if let Some(am) = asset_manager.as_mut() {
                                let rel_path = payload.path
                                    .strip_prefix(am.asset_directory())
                                    .map(|p| p.to_string_lossy().to_string())
                                    .unwrap_or_else(|_| payload.path.to_string_lossy().to_string());
                                let handle = am.import_asset(&rel_path);
                                if let Some(mut sprite) =
                                    scene.get_component_mut::<SpriteRendererComponent>(entity)
                                {
                                    sprite.texture_handle = handle;
                                    sprite.texture = None; // Will be resolved in on_render
                                }
                                *scene_dirty = true;
                            }
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

                // Clear button (X).
                if texture_handle_raw != 0 {
                    if ui.small_button("X").clicked() {
                        if let Some(mut sprite) =
                            scene.get_component_mut::<SpriteRendererComponent>(entity)
                        {
                            sprite.texture_handle = Uuid::from_raw(0);
                            sprite.texture = None;
                        }
                        *scene_dirty = true;
                    }
                }
            });

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
        *scene_dirty = true;
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
        *scene_dirty = true;
    }

    // -- Text Component (removable) --
    let mut remove_text = false;
    if scene.has_component::<TextComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Text").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("text", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut text_str, mut font_path, mut font_size, mut color_arr, mut line_spacing, mut kerning) = {
                let tc = scene.get_component::<TextComponent>(entity).unwrap();
                (
                    tc.text.clone(),
                    tc.font_path.clone(),
                    tc.font_size,
                    [tc.color.x, tc.color.y, tc.color.z, tc.color.w],
                    tc.line_spacing,
                    tc.kerning,
                )
            };

            // Text (multiline).
            ui.label("Text");
            if ui
                .add(egui::TextEdit::multiline(&mut text_str).desired_rows(3))
                .changed()
            {
                if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                    tc.text = text_str;
                    *scene_dirty = true;
                }
            }

            // Font path.
            ui.horizontal(|ui| {
                ui.label("Font");
                if ui
                    .add(egui::TextEdit::singleline(&mut font_path).desired_width(150.0))
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.font_path = font_path.clone();
                        tc.font = None; // Will be reloaded.
                        *scene_dirty = true;
                    }
                }
                if ui.button("...").clicked() {
                    let fonts_dir = assets_root.join("fonts");
                    let fonts_dir_str = fonts_dir.to_string_lossy();
                    if let Some(path_str) =
                        FileDialogs::open_file_in("Font files", &["ttf", "otf"], &fonts_dir_str)
                    {
                        if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                            tc.font_path = path_str;
                            tc.font = None;
                            *scene_dirty = true;
                        }
                    }
                }
            });

            // Color.
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
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.color = Vec4::from(color_arr);
                        *scene_dirty = true;
                    }
                }
            });

            // Font size.
            ui.horizontal(|ui| {
                ui.label("Font Size");
                if ui
                    .add(
                        egui::DragValue::new(&mut font_size)
                            .speed(0.01)
                            .range(0.01..=100.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.font_size = font_size;
                        *scene_dirty = true;
                    }
                }
            });

            // Line spacing.
            ui.horizontal(|ui| {
                ui.label("Line Spacing");
                if ui
                    .add(
                        egui::DragValue::new(&mut line_spacing)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.line_spacing = line_spacing;
                        *scene_dirty = true;
                    }
                }
            });

            // Kerning.
            ui.horizontal(|ui| {
                ui.label("Kerning");
                if ui
                    .add(
                        egui::DragValue::new(&mut kerning)
                            .speed(0.001)
                            .range(-1.0..=1.0),
                    )
                    .changed()
                {
                    if let Some(mut tc) = scene.get_component_mut::<TextComponent>(entity) {
                        tc.kerning = kerning;
                        *scene_dirty = true;
                    }
                }
            });
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_text = true;
                ui.close();
            }
        });
    }
    if remove_text {
        scene.remove_component::<TextComponent>(entity);
        *scene_dirty = true;
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
        *scene_dirty = true;
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
        *scene_dirty = true;
    }

    // -- Circle Collider 2D Component (removable) --
    let mut remove_cc2d = false;
    if scene.has_component::<CircleCollider2DComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Circle Collider 2D")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("circle_collider_2d", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut offset, mut radius, mut density, mut friction, mut restitution, mut restitution_threshold) = {
                let cc = scene
                    .get_component::<CircleCollider2DComponent>(entity)
                    .unwrap();
                (
                    cc.offset,
                    cc.radius,
                    cc.density,
                    cc.friction,
                    cc.restitution,
                    cc.restitution_threshold,
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
                if let Some(mut cc) =
                    scene.get_component_mut::<CircleCollider2DComponent>(entity)
                {
                    cc.offset = offset;
                    cc.radius = radius;
                    cc.density = density;
                    cc.friction = friction;
                    cc.restitution = restitution;
                    cc.restitution_threshold = restitution_threshold;
                }
            }
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_cc2d = true;
                ui.close();
            }
        });
    }
    if remove_cc2d {
        scene.remove_component::<CircleCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    // -- Native Script Component (removable, read-only display) --
    let mut remove_native_script = false;
    if scene.has_component::<NativeScriptComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Native Script")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("native_script", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Script");
                ui.label(
                    egui::RichText::new("(bound in code)")
                        .color(egui::Color32::from_rgb(0x96, 0x96, 0x96))
                        .italics(),
                );
            });
        });

        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_native_script = true;
                ui.close();
            }
        });
    }
    if remove_native_script {
        scene.remove_component::<NativeScriptComponent>(entity);
        *scene_dirty = true;
    }

    // -- Lua Script Component (removable) --
    #[cfg(feature = "lua-scripting")]
    {
        let mut remove_lua_script = false;
        if scene.has_component::<LuaScriptComponent>(entity) {
            let cr = egui::CollapsingHeader::new(
                egui::RichText::new("Lua Script")
                    .font(egui::FontId::new(14.0, bold_family.clone())),
            )
            .id_salt(("lua_script", entity.id()))
            .default_open(true)
            .show(ui, |ui| {
                let (script_path, field_overrides) = scene
                    .get_component::<LuaScriptComponent>(entity)
                    .map(|lsc| (lsc.script_path.clone(), lsc.field_overrides.clone()))
                    .unwrap_or_default();

                let mut new_script_path = None;

                ui.horizontal(|ui| {
                    ui.label("Script");

                    // Show filename or "None" on the button.
                    let display = if script_path.is_empty() {
                        "None".to_string()
                    } else {
                        std::path::Path::new(&script_path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| script_path.clone())
                    };
                    let btn_resp = ui.add_sized(
                        [ui.available_width(), 0.0],
                        egui::Button::new(display),
                    );

                    // Click to open file dialog in assets/scripts.
                    if btn_resp.clicked() {
                        let scripts_dir = assets_root.join("scripts");
                        let scripts_dir_str = scripts_dir.to_string_lossy();
                        if let Some(path) =
                            FileDialogs::open_file_in("Lua scripts", &["lua"], &scripts_dir_str)
                        {
                            new_script_path = Some(path);
                        }
                    }

                    // Accept script drag-and-drop from the content browser.
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
                            if ext == "lua" {
                                new_script_path =
                                    Some(payload.path.to_string_lossy().to_string());
                            }
                        }
                    }

                    // Visual feedback when dragging over the button.
                    if btn_resp
                        .dnd_hover_payload::<ContentBrowserPayload>()
                        .is_some()
                    {
                        ui.painter().rect_stroke(
                            btn_resp.rect,
                            egui::CornerRadius::same(2),
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgb(0x56, 0x9C, 0xD6),
                            ),
                            egui::StrokeKind::Inside,
                        );
                    }
                });

                // Apply script path change (clears overrides and cache).
                if let Some(path) = new_script_path {
                    FIELD_CACHE.with(|c| c.borrow_mut().remove(&path));
                    if let Some(mut lsc) =
                        scene.get_component_mut::<LuaScriptComponent>(entity)
                    {
                        if lsc.script_path != path {
                            lsc.field_overrides.clear();
                        }
                        lsc.script_path = path;
                    }
                }

                // ----- Script Fields -----
                if !script_path.is_empty() {
                    // Get entity UUID for runtime field access.
                    let entity_uuid = scene
                        .get_component::<IdComponent>(entity)
                        .map(|id| id.id.raw())
                        .unwrap_or(0);

                    // Determine fields to display.
                    // In play mode: read live values from the running Lua env.
                    // In edit mode: use discovery cache + stored overrides.
                    let fields: Vec<(String, ScriptFieldValue)> = if is_playing {
                        scene
                            .script_engine()
                            .and_then(|eng| eng.get_entity_fields(entity_uuid))
                            .unwrap_or_else(|| get_cached_fields(&script_path))
                    } else {
                        get_cached_fields(&script_path)
                    };

                    if !fields.is_empty() {
                        ui.separator();
                    }

                    for (name, default_value) in &fields {
                        // Determine display value: override > default.
                        let current = if is_playing {
                            default_value.clone() // Already live from get_entity_fields.
                        } else {
                            field_overrides
                                .get(name)
                                .cloned()
                                .unwrap_or_else(|| default_value.clone())
                        };

                        ui.horizontal(|ui| {
                            ui.label(name);

                            match current {
                                ScriptFieldValue::Float(mut v) => {
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut v)
                                                .speed(0.1),
                                        )
                                        .changed()
                                    {
                                        let new_val = ScriptFieldValue::Float(v);
                                        if is_playing {
                                            // Write directly to Lua env.
                                            if let Some(eng) = scene.script_engine() {
                                                eng.set_entity_field(
                                                    entity_uuid,
                                                    name,
                                                    &new_val,
                                                );
                                            }
                                        }
                                        // Always store override.
                                        if let Some(mut lsc) = scene
                                            .get_component_mut::<LuaScriptComponent>(entity)
                                        {
                                            lsc.field_overrides
                                                .insert(name.clone(), new_val);
                                        }
                                    }
                                }
                                ScriptFieldValue::Bool(mut v) => {
                                    if ui.checkbox(&mut v, "").changed() {
                                        let new_val = ScriptFieldValue::Bool(v);
                                        if is_playing {
                                            if let Some(eng) = scene.script_engine() {
                                                eng.set_entity_field(
                                                    entity_uuid,
                                                    name,
                                                    &new_val,
                                                );
                                            }
                                        }
                                        if let Some(mut lsc) = scene
                                            .get_component_mut::<LuaScriptComponent>(entity)
                                        {
                                            lsc.field_overrides
                                                .insert(name.clone(), new_val);
                                        }
                                    }
                                }
                                ScriptFieldValue::String(mut v) => {
                                    if ui
                                        .text_edit_singleline(&mut v)
                                        .changed()
                                    {
                                        let new_val = ScriptFieldValue::String(v);
                                        if is_playing {
                                            if let Some(eng) = scene.script_engine() {
                                                eng.set_entity_field(
                                                    entity_uuid,
                                                    name,
                                                    &new_val,
                                                );
                                            }
                                        }
                                        if let Some(mut lsc) = scene
                                            .get_component_mut::<LuaScriptComponent>(entity)
                                        {
                                            lsc.field_overrides
                                                .insert(name.clone(), new_val);
                                        }
                                    }
                                }
                            }
                        });
                    }
                }
            });

            cr.header_response.context_menu(|ui| {
                if ui.button("Remove Component").clicked() {
                    remove_lua_script = true;
                    ui.close();
                }
            });
        }
        if remove_lua_script {
            scene.remove_component::<LuaScriptComponent>(entity);
            *scene_dirty = true;
        }
    }
}
