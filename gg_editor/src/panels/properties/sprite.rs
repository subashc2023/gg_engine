use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;
use crate::panels::relative_asset_path;

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
    if !scene.has_component::<SpriteRendererComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Sprite Renderer",
        "sprite_renderer",
        bold_family,
        entity,
        |ui| {
            let (
                mut color_arr,
                texture_handle_raw,
                mut tiling_factor,
                mut sorting_layer,
                mut order_in_layer,
                mut atlas_min,
                mut atlas_max,
            ) = {
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

            if super::color_picker_rgba(ui, "Color", &mut color_arr) {
                if let Some(mut sprite) = scene.get_component_mut::<SpriteRendererComponent>(entity)
                {
                    sprite.color = Vec4::from(color_arr);
                }
            }

            ui.horizontal(|ui| {
                match super::asset_handle_picker(
                    ui,
                    texture_handle_raw,
                    asset_manager,
                    assets_root,
                    "textures",
                    "Image files",
                    &["png", "jpg", "jpeg"],
                ) {
                    super::AssetPickerAction::Selected(handle) => {
                        if let Some(mut sprite) =
                            scene.get_component_mut::<SpriteRendererComponent>(entity)
                        {
                            sprite.texture_handle = handle;
                            sprite.texture = None;
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::Cleared => {
                        if let Some(mut sprite) =
                            scene.get_component_mut::<SpriteRendererComponent>(entity)
                        {
                            sprite.texture_handle = Uuid::from_raw(0);
                            sprite.texture = None;
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::None => {}
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
                    *scene_dirty = true;
                }
            });

            // Sorting layer & order.
            if super::sorting_layer_controls(ui, &mut sorting_layer, &mut order_in_layer) {
                if let Some(mut sprite) = scene.get_component_mut::<SpriteRendererComponent>(entity)
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
                        .add(
                            egui::DragValue::new(&mut atlas_min.x)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("U: "),
                        )
                        .changed();
                    atlas_changed |= ui
                        .add(
                            egui::DragValue::new(&mut atlas_min.y)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("V: "),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Atlas Max UV");
                    atlas_changed |= ui
                        .add(
                            egui::DragValue::new(&mut atlas_max.x)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("U: "),
                        )
                        .changed();
                    atlas_changed |= ui
                        .add(
                            egui::DragValue::new(&mut atlas_max.y)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("V: "),
                        )
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
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_sprite_animator_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<SpriteAnimatorComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Sprite Animator",
        "sprite_animator",
        bold_family,
        entity,
        |ui| {
            let (
                mut cell_w,
                mut cell_h,
                mut columns,
                mut default_clip,
                mut speed_scale,
                clip_names,
            ) = {
                let sa = scene
                    .get_component::<SpriteAnimatorComponent>(entity)
                    .unwrap();
                (
                    sa.cell_size.x,
                    sa.cell_size.y,
                    sa.columns,
                    sa.default_clip.clone(),
                    sa.speed_scale,
                    sa.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                )
            };

            // --- Preview controls ---
            ui.horizontal(|ui| {
                let is_previewing = scene
                    .get_component::<SpriteAnimatorComponent>(entity)
                    .map(|sa| sa.is_previewing() && sa.is_playing())
                    .unwrap_or(false);

                if is_previewing {
                    if ui.button("Pause").clicked() {
                        if let Some(mut sa) =
                            scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                        {
                            sa.stop();
                        }
                    }
                } else if ui.button("Preview").clicked() {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.set_previewing(true);
                        // If no clip is selected, play the first clip or default.
                        if sa.current_clip_index().is_none() {
                            if !sa.default_clip.is_empty() {
                                let name = sa.default_clip.clone();
                                sa.play(&name);
                            } else if !sa.clips.is_empty() {
                                let name = sa.clips[0].name.clone();
                                sa.play(&name);
                            }
                        } else {
                            // Resume from where we paused.
                            let idx = sa.current_clip_index().unwrap();
                            if let Some(clip) = sa.clips.get(idx) {
                                let name = clip.name.clone();
                                sa.play(&name);
                            }
                        }
                    }
                }

                if ui.button("Stop").clicked() {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.reset();
                    }
                }

                // Show clip selector for preview.
                let mut preview_clip_idx: usize = scene
                    .get_component::<SpriteAnimatorComponent>(entity)
                    .and_then(|sa| sa.current_clip_index())
                    .unwrap_or(0);
                if !clip_names.is_empty() {
                    let prev_idx = preview_clip_idx;
                    egui::ComboBox::from_id_salt(("preview_clip", entity.id()))
                        .selected_text(
                            clip_names
                                .get(preview_clip_idx)
                                .cloned()
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in clip_names.iter().enumerate() {
                                ui.selectable_value(&mut preview_clip_idx, i, name);
                            }
                        });
                    if preview_clip_idx != prev_idx {
                        if let Some(mut sa) =
                            scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                        {
                            if let Some(clip) = sa.clips.get(preview_clip_idx) {
                                let name = clip.name.clone();
                                sa.play(&name);
                                sa.set_previewing(true);
                            }
                        }
                    }
                }
            });

            ui.separator();

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

                // Auto-detect columns from sprite texture size.
                if ui.button("Auto").clicked() && cell_w > 0.0 {
                    let tex_width = scene
                        .get_component::<SpriteRendererComponent>(entity)
                        .and_then(|sprite| sprite.texture.as_ref().map(|t| t.width()));
                    if let Some(w) = tex_width {
                        columns = (w as f32 / cell_w).floor().max(1.0) as u32;
                        changed = true;
                    }
                }
            });

            // Default clip selector.
            ui.horizontal(|ui| {
                ui.label("Default Clip");
                let mut default_changed = false;
                egui::ComboBox::from_id_salt(("default_clip", entity.id()))
                    .selected_text(if default_clip.is_empty() {
                        "(none)"
                    } else {
                        &default_clip
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(default_clip.is_empty(), "(none)")
                            .clicked()
                        {
                            default_clip.clear();
                            default_changed = true;
                        }
                        for name in &clip_names {
                            if ui.selectable_label(*name == default_clip, name).clicked() {
                                default_clip = name.clone();
                                default_changed = true;
                            }
                        }
                    });
                if default_changed {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.default_clip = default_clip.clone();
                        *scene_dirty = true;
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Speed");
                if ui
                    .add(
                        egui::DragValue::new(&mut speed_scale)
                            .range(0.0..=10.0)
                            .speed(0.01),
                    )
                    .changed()
                {
                    if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                    {
                        sa.speed_scale = speed_scale;
                        *scene_dirty = true;
                    }
                }
            });

            if changed {
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
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
                let (mut name, mut start, mut end, mut fps, mut looping, clip_tex_handle) = {
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
                        c.texture_handle.raw(),
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
                            .add(egui::DragValue::new(&mut end).prefix("End: ").speed(0.1))
                            .changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("FPS");
                        clip_changed |= ui
                            .add(egui::DragValue::new(&mut fps).range(0.1..=120.0).speed(0.1))
                            .changed();
                        clip_changed |= ui.checkbox(&mut looping, "Loop").changed();
                    });

                    // Per-clip texture picker (click or drag-and-drop).
                    ui.horizontal(|ui| {
                        let tex_label = if clip_tex_handle != 0 {
                            if let Some(am) = asset_manager.as_ref() {
                                let handle = Uuid::from_raw(clip_tex_handle);
                                am.get_metadata(&handle)
                                    .map(|m| {
                                        std::path::Path::new(&m.file_path)
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string()
                                    })
                                    .unwrap_or_else(|| format!("{}", clip_tex_handle))
                            } else {
                                format!("{}", clip_tex_handle)
                            }
                        } else {
                            "None (uses sprite)".to_string()
                        };

                        ui.label("Texture");
                        let btn_w = ((tex_label.len() as f32) * 7.0 + 20.0).max(100.0);
                        let btn_resp = ui.add_sized([btn_w, 0.0], egui::Button::new(&tex_label));

                        if btn_resp.clicked() {
                            if let Some(am) = asset_manager.as_mut() {
                                let start_dir = if assets_root.join("textures").exists() {
                                    assets_root.join("textures")
                                } else {
                                    assets_root.to_path_buf()
                                };
                                let start_dir_str = start_dir.to_string_lossy();
                                if let Some(path_str) = FileDialogs::open_file_in(
                                    "Image files",
                                    &["png", "jpg", "jpeg"],
                                    &start_dir_str,
                                ) {
                                    let abs_path = std::path::PathBuf::from(&path_str);
                                    let rel_path =
                                        relative_asset_path(&abs_path, am.asset_directory());
                                    let handle = am.import_asset(&rel_path);
                                    if let Some(mut sa) =
                                        scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                                    {
                                        if let Some(c) = sa.clips.get_mut(i) {
                                            c.texture_handle = handle;
                                            c.texture = None;
                                            *scene_dirty = true;
                                        }
                                    }
                                }
                            }
                        }

                        // Drag-and-drop from content browser.
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
                                        let rel_path = relative_asset_path(
                                            &payload.path,
                                            am.asset_directory(),
                                        );
                                        let handle = am.import_asset(&rel_path);
                                        if let Some(mut sa) = scene
                                            .get_component_mut::<SpriteAnimatorComponent>(entity)
                                        {
                                            if let Some(c) = sa.clips.get_mut(i) {
                                                c.texture_handle = handle;
                                                c.texture = None;
                                                *scene_dirty = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Visual drop target highlight.
                        if btn_resp
                            .dnd_hover_payload::<ContentBrowserPayload>()
                            .is_some()
                        {
                            ui.painter().rect_stroke(
                                btn_resp.rect,
                                egui::CornerRadius::same(2),
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(0x56, 0x9C, 0xD6)),
                                egui::StrokeKind::Inside,
                            );
                        }

                        if clip_tex_handle != 0 && ui.small_button("X").clicked() {
                            if let Some(mut sa) =
                                scene.get_component_mut::<SpriteAnimatorComponent>(entity)
                            {
                                if let Some(c) = sa.clips.get_mut(i) {
                                    c.texture_handle = Uuid::from_raw(0);
                                    c.texture = None;
                                    *scene_dirty = true;
                                }
                            }
                        }
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
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    sa.clips.remove(idx);
                    *scene_dirty = true;
                }
            }

            ui.separator();
            if ui.button("Add Clip").clicked() {
                if let Some(mut sa) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
                    let idx = sa.clips.len();
                    sa.clips.push(AnimationClip {
                        name: format!("clip_{}", idx),
                        ..Default::default()
                    });
                    *scene_dirty = true;
                }
            }
        },
    )
}

pub(crate) fn draw_circle_renderer_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<CircleRendererComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Circle Renderer",
        "circle_renderer",
        bold_family,
        entity,
        |ui| {
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

            if super::color_picker_rgba(ui, "Color", &mut color_arr) {
                if let Some(mut circle) = scene.get_component_mut::<CircleRendererComponent>(entity)
                {
                    circle.color = Vec4::from(color_arr);
                }
                *scene_dirty = true;
            }

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
                    *scene_dirty = true;
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
                    *scene_dirty = true;
                }
            });

            // Sorting layer & order.
            if super::sorting_layer_controls(ui, &mut sorting_layer, &mut order_in_layer) {
                if let Some(mut circle) = scene.get_component_mut::<CircleRendererComponent>(entity)
                {
                    circle.sorting_layer = sorting_layer;
                    circle.order_in_layer = order_in_layer;
                }
                *scene_dirty = true;
            }
        },
    )
}

// ---------------------------------------------------------------------------
// InstancedSpriteAnimator
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_instanced_sprite_animator(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<InstancedSpriteAnimator>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Instanced Sprite Animator",
        "instanced_sprite_animator",
        bold_family,
        entity,
        |ui| {
            let (
                mut cell_w,
                mut cell_h,
                mut columns,
                mut default_clip,
                mut speed_scale,
                clip_count,
            ) = {
                let ia = scene
                    .get_component::<InstancedSpriteAnimator>(entity)
                    .unwrap();
                (
                    ia.cell_size.x,
                    ia.cell_size.y,
                    ia.columns,
                    ia.default_clip.clone(),
                    ia.speed_scale,
                    ia.clips.len(),
                )
            };

            ui.label(
                egui::RichText::new("Stateless (mass entities)")
                    .italics()
                    .color(egui::Color32::from_gray(160)),
            );

            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Cell Size");
                changed |= ui
                    .add(egui::DragValue::new(&mut cell_w).speed(1.0).prefix("W: "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut cell_h).speed(1.0).prefix("H: "))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Columns");
                changed |= ui
                    .add(egui::DragValue::new(&mut columns).speed(0.1).range(1..=256))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Speed Scale");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut speed_scale)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Default Clip");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut default_clip).desired_width(100.0))
                    .changed();
            });

            if changed {
                if let Some(mut ia) = scene.get_component_mut::<InstancedSpriteAnimator>(entity) {
                    ia.cell_size.x = cell_w;
                    ia.cell_size.y = cell_h;
                    ia.columns = columns;
                    ia.speed_scale = speed_scale;
                    ia.default_clip = default_clip;
                }
                *scene_dirty = true;
            }

            // --- Clip list ---
            ui.separator();
            ui.label(egui::RichText::new("Clips").strong());

            if ui.button("+ Add Clip").clicked() {
                if let Some(mut ia) = scene.get_component_mut::<InstancedSpriteAnimator>(entity) {
                    let idx = ia.clips.len();
                    ia.clips.push(AnimationClip {
                        name: format!("clip_{idx}"),
                        ..Default::default()
                    });
                }
                *scene_dirty = true;
            }

            let mut clip_to_remove: Option<usize> = None;
            for i in 0..clip_count {
                let clip_data = {
                    let ia = scene
                        .get_component::<InstancedSpriteAnimator>(entity)
                        .unwrap();
                    let c = &ia.clips[i];
                    (c.name.clone(), c.start_frame, c.end_frame, c.fps, c.looping)
                };
                let (mut name, mut start, mut end, mut fps, mut looping) = clip_data;

                let id = ui.make_persistent_id(("instanced_clip", entity.id(), i));
                egui::CollapsingHeader::new(&name)
                    .id_salt(id)
                    .default_open(false)
                    .show(ui, |ui| {
                        let mut clip_changed = false;
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            clip_changed |= ui
                                .add(egui::TextEdit::singleline(&mut name).desired_width(100.0))
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Frames");
                            clip_changed |= ui
                                .add(
                                    egui::DragValue::new(&mut start)
                                        .speed(0.1)
                                        .prefix("Start: "),
                                )
                                .changed();
                            clip_changed |= ui
                                .add(egui::DragValue::new(&mut end).speed(0.1).prefix("End: "))
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("FPS");
                            clip_changed |= ui
                                .add(egui::DragValue::new(&mut fps).speed(0.1).range(0.1..=120.0))
                                .changed();
                        });
                        clip_changed |= ui.checkbox(&mut looping, "Looping").changed();

                        if clip_changed {
                            if let Some(mut ia) =
                                scene.get_component_mut::<InstancedSpriteAnimator>(entity)
                            {
                                if let Some(c) = ia.clips.get_mut(i) {
                                    c.name = name;
                                    c.start_frame = start;
                                    c.end_frame = end;
                                    c.fps = fps;
                                    c.looping = looping;
                                }
                            }
                            *scene_dirty = true;
                        }

                        if ui
                            .button(egui::RichText::new("Remove Clip").color(egui::Color32::RED))
                            .clicked()
                        {
                            clip_to_remove = Some(i);
                        }
                    });
            }

            if let Some(idx) = clip_to_remove {
                if let Some(mut ia) = scene.get_component_mut::<InstancedSpriteAnimator>(entity) {
                    ia.clips.remove(idx);
                }
                *scene_dirty = true;
            }

            let _ = (asset_manager, assets_root);
        },
    )
}

// ---------------------------------------------------------------------------
// AnimationControllerComponent
// ---------------------------------------------------------------------------

pub(crate) fn draw_animation_controller(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<AnimationControllerComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Animation Controller",
        "animation_controller",
        bold_family,
        entity,
        |ui| {
            let transition_count = {
                let ctrl = scene
                    .get_component::<AnimationControllerComponent>(entity)
                    .unwrap();
                ctrl.transitions.len()
            };

            // --- Parameters ---
            ui.label(egui::RichText::new("Parameters").strong());
            {
                let ctrl = scene
                    .get_component::<AnimationControllerComponent>(entity)
                    .unwrap();
                let bools: Vec<(String, bool)> = ctrl
                    .bool_params
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                let floats: Vec<(String, f32)> = ctrl
                    .float_params
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                drop(ctrl);

                let mut remove_bool: Option<String> = None;
                let mut remove_float: Option<String> = None;
                for (name, val) in &bools {
                    ui.horizontal(|ui| {
                        ui.label(format!("{name} (bool)"));
                        let mut v = *val;
                        if ui.checkbox(&mut v, "").changed() {
                            if let Some(mut ctrl) =
                                scene.get_component_mut::<AnimationControllerComponent>(entity)
                            {
                                ctrl.bool_params.insert(name.clone(), v);
                            }
                            *scene_dirty = true;
                        }
                        if ui.small_button("X").clicked() {
                            remove_bool = Some(name.clone());
                        }
                    });
                }
                for (name, val) in &floats {
                    ui.horizontal(|ui| {
                        ui.label(format!("{name} (float)"));
                        let mut v = *val;
                        if ui
                            .add(egui::DragValue::new(&mut v).speed(0.1).max_decimals(2))
                            .changed()
                        {
                            if let Some(mut ctrl) =
                                scene.get_component_mut::<AnimationControllerComponent>(entity)
                            {
                                ctrl.float_params.insert(name.clone(), v);
                            }
                            *scene_dirty = true;
                        }
                        if ui.small_button("X").clicked() {
                            remove_float = Some(name.clone());
                        }
                    });
                }
                if let Some(key) = remove_bool {
                    if let Some(mut ctrl) =
                        scene.get_component_mut::<AnimationControllerComponent>(entity)
                    {
                        ctrl.bool_params.remove(&key);
                    }
                    *scene_dirty = true;
                }
                if let Some(key) = remove_float {
                    if let Some(mut ctrl) =
                        scene.get_component_mut::<AnimationControllerComponent>(entity)
                    {
                        ctrl.float_params.remove(&key);
                    }
                    *scene_dirty = true;
                }
            }

            ui.horizontal(|ui| {
                if ui.button("+ Bool Param").clicked() {
                    if let Some(mut ctrl) =
                        scene.get_component_mut::<AnimationControllerComponent>(entity)
                    {
                        let name = format!("param_{}", ctrl.bool_params.len());
                        ctrl.bool_params.insert(name, false);
                    }
                    *scene_dirty = true;
                }
                if ui.button("+ Float Param").clicked() {
                    if let Some(mut ctrl) =
                        scene.get_component_mut::<AnimationControllerComponent>(entity)
                    {
                        let name = format!("param_{}", ctrl.float_params.len());
                        ctrl.float_params.insert(name, 0.0);
                    }
                    *scene_dirty = true;
                }
            });

            // --- Transitions ---
            ui.separator();
            ui.label(egui::RichText::new("Transitions").strong());

            if ui.button("+ Add Transition").clicked() {
                if let Some(mut ctrl) =
                    scene.get_component_mut::<AnimationControllerComponent>(entity)
                {
                    ctrl.transitions.push(AnimationTransition {
                        from: String::new(),
                        to: String::new(),
                        condition: TransitionCondition::OnFinished,
                    });
                }
                *scene_dirty = true;
            }

            let mut transition_to_remove: Option<usize> = None;
            for i in 0..transition_count {
                let t_data = {
                    let ctrl = scene
                        .get_component::<AnimationControllerComponent>(entity)
                        .unwrap();
                    let t = &ctrl.transitions[i];
                    let cond_idx = match &t.condition {
                        TransitionCondition::OnFinished => 0usize,
                        TransitionCondition::ParamBool(_, _) => 1,
                        TransitionCondition::ParamFloat(_, _, _) => 2,
                    };
                    let (pname, bval, ford_idx, fthresh) = match &t.condition {
                        TransitionCondition::OnFinished => (String::new(), false, 0usize, 0.0f32),
                        TransitionCondition::ParamBool(n, v) => (n.clone(), *v, 0, 0.0),
                        TransitionCondition::ParamFloat(n, ord, th) => {
                            let oi = match ord {
                                FloatOrdering::Greater => 0,
                                FloatOrdering::Less => 1,
                                FloatOrdering::GreaterOrEqual => 2,
                                FloatOrdering::LessOrEqual => 3,
                            };
                            (n.clone(), false, oi, *th)
                        }
                    };
                    (
                        t.from.clone(),
                        t.to.clone(),
                        cond_idx,
                        pname,
                        bval,
                        ford_idx,
                        fthresh,
                    )
                };

                let (
                    mut from,
                    mut to,
                    mut cond_idx,
                    mut pname,
                    mut bval,
                    mut ford_idx,
                    mut fthresh,
                ) = t_data;

                let id = ui.make_persistent_id(("anim_transition", entity.id(), i));
                egui::CollapsingHeader::new(format!("Transition {i}"))
                    .id_salt(id)
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.label("From");
                            changed |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut from)
                                        .desired_width(80.0)
                                        .hint_text("(any)"),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("To");
                            changed |= ui
                                .add(egui::TextEdit::singleline(&mut to).desired_width(80.0))
                                .changed();
                        });

                        let cond_labels = ["OnFinished", "ParamBool", "ParamFloat"];
                        ui.horizontal(|ui| {
                            ui.label("Condition");
                            changed |= egui::ComboBox::from_id_salt(("cond_type", entity.id(), i))
                                .selected_text(cond_labels[cond_idx])
                                .show_index(ui, &mut cond_idx, cond_labels.len(), |idx| {
                                    cond_labels[idx].to_string()
                                })
                                .changed();
                        });

                        if cond_idx == 1 {
                            ui.horizontal(|ui| {
                                ui.label("Param");
                                changed |= ui
                                    .add(egui::TextEdit::singleline(&mut pname).desired_width(80.0))
                                    .changed();
                                changed |= ui.checkbox(&mut bval, "Value").changed();
                            });
                        } else if cond_idx == 2 {
                            ui.horizontal(|ui| {
                                ui.label("Param");
                                changed |= ui
                                    .add(egui::TextEdit::singleline(&mut pname).desired_width(80.0))
                                    .changed();
                            });
                            let ord_labels = [">", "<", ">=", "<="];
                            ui.horizontal(|ui| {
                                ui.label("Ordering");
                                changed |=
                                    egui::ComboBox::from_id_salt(("float_ord", entity.id(), i))
                                        .selected_text(ord_labels[ford_idx])
                                        .show_index(ui, &mut ford_idx, ord_labels.len(), |idx| {
                                            ord_labels[idx].to_string()
                                        })
                                        .changed();
                                ui.label("Threshold");
                                changed |= ui
                                    .add(egui::DragValue::new(&mut fthresh).speed(0.1))
                                    .changed();
                            });
                        }

                        if changed {
                            let condition = match cond_idx {
                                0 => TransitionCondition::OnFinished,
                                1 => TransitionCondition::ParamBool(pname, bval),
                                2 => {
                                    let ordering = match ford_idx {
                                        0 => FloatOrdering::Greater,
                                        1 => FloatOrdering::Less,
                                        2 => FloatOrdering::GreaterOrEqual,
                                        3 => FloatOrdering::LessOrEqual,
                                        _ => FloatOrdering::Greater,
                                    };
                                    TransitionCondition::ParamFloat(pname, ordering, fthresh)
                                }
                                _ => TransitionCondition::OnFinished,
                            };
                            if let Some(mut ctrl) =
                                scene.get_component_mut::<AnimationControllerComponent>(entity)
                            {
                                if let Some(t) = ctrl.transitions.get_mut(i) {
                                    t.from = from;
                                    t.to = to;
                                    t.condition = condition;
                                }
                            }
                            *scene_dirty = true;
                        }

                        if ui
                            .button(
                                egui::RichText::new("Remove Transition").color(egui::Color32::RED),
                            )
                            .clicked()
                        {
                            transition_to_remove = Some(i);
                        }
                    });
            }

            if let Some(idx) = transition_to_remove {
                if let Some(mut ctrl) =
                    scene.get_component_mut::<AnimationControllerComponent>(entity)
                {
                    ctrl.transitions.remove(idx);
                }
                *scene_dirty = true;
            }
        },
    )
}
