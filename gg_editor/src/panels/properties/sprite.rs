use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_sprite_renderer_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<SpriteRendererComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Sprite Renderer")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("sprite_renderer", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut color_arr, texture_handle_raw, mut tiling_factor, mut sorting_layer, mut order_in_layer, mut atlas_min, mut atlas_max) = {
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
                    sprite.sorting_layer,
                    sprite.order_in_layer,
                    sprite.atlas_min,
                    sprite.atlas_max,
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

            let btn_width = ((texture_label.len() as f32) * 7.0 + 20.0).max(100.0);

            ui.horizontal(|ui| {
                let btn_resp = ui.add_sized(
                    [btn_width, 0.0],
                    egui::Button::new(&texture_label),
                );

                if btn_resp.clicked() {
                    if let Some(am) = asset_manager.as_mut() {
                        let textures_dir = assets_root.join("textures");
                        let textures_dir_str = textures_dir.to_string_lossy();
                        if let Some(path_str) =
                            FileDialogs::open_file_in("Image files", &["png", "jpg", "jpeg"], &textures_dir_str)
                        {
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
                                sprite.texture = None;
                            }
                            *scene_dirty = true;
                        }
                    }
                }

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
                                    sprite.texture = None;
                                }
                                *scene_dirty = true;
                            }
                        }
                    }
                }

                if btn_resp.dnd_hover_payload::<ContentBrowserPayload>().is_some() {
                    ui.painter().rect_stroke(
                        btn_resp.rect,
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(0x56, 0x9C, 0xD6)),
                        egui::StrokeKind::Inside,
                    );
                }

                if texture_handle_raw != 0
                    && ui.small_button("X").clicked()
                {
                    if let Some(mut sprite) =
                        scene.get_component_mut::<SpriteRendererComponent>(entity)
                    {
                        sprite.texture_handle = Uuid::from_raw(0);
                        sprite.texture = None;
                    }
                    *scene_dirty = true;
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

            // Sorting layer & order.
            let mut sort_changed = false;
            ui.horizontal(|ui| {
                ui.label("Sorting Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut sorting_layer).speed(0.1))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Order in Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut order_in_layer).speed(0.1))
                    .changed();
            });
            if sort_changed {
                if let Some(mut sprite) =
                    scene.get_component_mut::<SpriteRendererComponent>(entity)
                {
                    sprite.sorting_layer = sorting_layer;
                    sprite.order_in_layer = order_in_layer;
                }
                *scene_dirty = true;
            }

            // Atlas sub-texture UV region.
            if texture_handle_raw != 0 {
                let mut atlas_changed = false;
                ui.horizontal(|ui| {
                    ui.label("Atlas Min UV");
                    atlas_changed |= ui
                        .add(egui::DragValue::new(&mut atlas_min.x).speed(0.01).range(0.0..=1.0).prefix("U: "))
                        .changed();
                    atlas_changed |= ui
                        .add(egui::DragValue::new(&mut atlas_min.y).speed(0.01).range(0.0..=1.0).prefix("V: "))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Atlas Max UV");
                    atlas_changed |= ui
                        .add(egui::DragValue::new(&mut atlas_max.x).speed(0.01).range(0.0..=1.0).prefix("U: "))
                        .changed();
                    atlas_changed |= ui
                        .add(egui::DragValue::new(&mut atlas_max.y).speed(0.01).range(0.0..=1.0).prefix("V: "))
                        .changed();
                });
                if atlas_changed {
                    if let Some(mut sprite) =
                        scene.get_component_mut::<SpriteRendererComponent>(entity)
                    {
                        sprite.atlas_min = atlas_min;
                        sprite.atlas_max = atlas_max;
                    }
                    *scene_dirty = true;
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

pub(crate) fn draw_sprite_animator_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<SpriteAnimatorComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Sprite Animator")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("sprite_animator", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut cell_w, mut cell_h, mut columns) = {
                let sa = scene
                    .get_component::<SpriteAnimatorComponent>(entity)
                    .unwrap();
                (sa.cell_size.x, sa.cell_size.y, sa.columns)
            };

            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Cell Size");
                changed |= ui
                    .add(egui::DragValue::new(&mut cell_w).prefix("W: ").speed(1.0))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut cell_h).prefix("H: ").speed(1.0))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Columns");
                changed |= ui
                    .add(egui::DragValue::new(&mut columns).range(1..=256).speed(0.1))
                    .changed();
            });

            if changed {
                if let Some(mut sa) =
                    scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    sa.cell_size = Vec2::new(cell_w, cell_h);
                    sa.columns = columns;
                    *scene_dirty = true;
                }
            }

            // Clip list
            let clip_count = scene
                .get_component::<SpriteAnimatorComponent>(entity)
                .map(|sa| sa.clips.len())
                .unwrap_or(0);

            let mut clip_to_remove = None;
            for i in 0..clip_count {
                let (mut name, mut start, mut end, mut fps, mut looping) = {
                    let sa = scene
                        .get_component::<SpriteAnimatorComponent>(entity)
                        .unwrap();
                    let c = &sa.clips[i];
                    (
                        c.name.clone(),
                        c.start_frame,
                        c.end_frame,
                        c.fps,
                        c.looping,
                    )
                };

                ui.push_id(("clip", i), |ui| {
                    ui.separator();
                    let mut clip_changed = false;
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        clip_changed |= ui.text_edit_singleline(&mut name).changed();
                        if ui
                            .add(
                                egui::Button::new("X")
                                    .small()
                                    .fill(egui::Color32::from_rgb(0xCC, 0x33, 0x33)),
                            )
                            .clicked()
                        {
                            clip_to_remove = Some(i);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Frames");
                        clip_changed |= ui
                            .add(
                                egui::DragValue::new(&mut start)
                                    .prefix("Start: ")
                                    .speed(0.1),
                            )
                            .changed();
                        clip_changed |= ui
                            .add(
                                egui::DragValue::new(&mut end)
                                    .prefix("End: ")
                                    .speed(0.1),
                            )
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("FPS");
                        clip_changed |= ui
                            .add(
                                egui::DragValue::new(&mut fps)
                                    .range(0.1..=120.0)
                                    .speed(0.1),
                            )
                            .changed();
                        clip_changed |= ui.checkbox(&mut looping, "Loop").changed();
                    });

                    if clip_changed {
                        if let Some(mut sa) =
                            scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                        {
                            if let Some(c) = sa.clips.get_mut(i) {
                                c.name = name;
                                c.start_frame = start;
                                c.end_frame = end;
                                c.fps = fps;
                                c.looping = looping;
                                *scene_dirty = true;
                            }
                        }
                    }
                });
            }

            if let Some(idx) = clip_to_remove {
                if let Some(mut sa) =
                    scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    sa.clips.remove(idx);
                    *scene_dirty = true;
                }
            }

            ui.separator();
            if ui.button("Add Clip").clicked() {
                if let Some(mut sa) =
                    scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    let idx = sa.clips.len();
                    sa.clips.push(AnimationClip {
                        name: format!("clip_{}", idx),
                        ..Default::default()
                    });
                    *scene_dirty = true;
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

pub(crate) fn draw_circle_renderer_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    _scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<CircleRendererComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Circle Renderer")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("circle_renderer", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut color_arr, mut thickness, mut fade, mut sorting_layer, mut order_in_layer) = {
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
                    circle.sorting_layer,
                    circle.order_in_layer,
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

            // Sorting layer & order.
            let mut sort_changed = false;
            ui.horizontal(|ui| {
                ui.label("Sorting Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut sorting_layer).speed(0.1))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Order in Layer");
                sort_changed |= ui
                    .add(egui::DragValue::new(&mut order_in_layer).speed(0.1))
                    .changed();
            });
            if sort_changed {
                if let Some(mut circle) =
                    scene.get_component_mut::<CircleRendererComponent>(entity)
                {
                    circle.sorting_layer = sorting_layer;
                    circle.order_in_layer = order_in_layer;
                }
                *_scene_dirty = true;
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
