use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::math::{DQuat, DVec3, Transform as GizmoTransform};
use transform_gizmo_egui::{GizmoConfig, GizmoExt, GizmoOrientation};

use crate::gizmo::{gizmo_modes_for, mat4_to_f64, GizmoOperation};
use crate::panels::content_browser::ContentBrowserPayload;
use crate::panels::{tile_uv_max, tile_uv_min};
use crate::selection::Selection;
use crate::TilemapPaintState;

/// Convert a viewport pixel position to a tilemap grid cell (col, row).
///
/// Returns `Some((col, row))` if the pixel maps to a valid cell, `None` otherwise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn screen_to_tile_grid(
    pixel_x: f32,
    pixel_y: f32,
    viewport_size: (u32, u32),
    vp_matrix: &Mat4,
    entity_world_transform: &Mat4,
    tilemap_z: f32,
    tile_size: Vec2,
    grid_width: u32,
    grid_height: u32,
) -> Option<(u32, u32)> {
    let (vw, vh) = (viewport_size.0 as f32, viewport_size.1 as f32);
    if vw <= 0.0 || vh <= 0.0 {
        return None;
    }

    // Pixel -> NDC.
    let ndc_x = (pixel_x / vw) * 2.0 - 1.0;
    let ndc_y = (pixel_y / vh) * 2.0 - 1.0;

    // Unproject near and far points through inverse(VP).
    let inv_vp = vp_matrix.inverse();
    let near_clip = inv_vp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    let far_clip = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
    if near_clip.w.abs() < 1e-8 || far_clip.w.abs() < 1e-8 {
        return None;
    }
    let near_world = near_clip.truncate() / near_clip.w;
    let far_world = far_clip.truncate() / far_clip.w;

    // Ray-plane intersection at Z = tilemap_z.
    let dir = far_world - near_world;
    if dir.z.abs() < 1e-8 {
        return None;
    }
    let t = (tilemap_z - near_world.z) / dir.z;
    let hit = near_world + dir * t;

    // World -> local (inverse of entity world transform).
    let inv_entity = entity_world_transform.inverse();
    let local_4 = inv_entity * Vec4::new(hit.x, hit.y, hit.z, 1.0);
    let local_x = local_4.x;
    let local_y = local_4.y;

    // Local -> grid. Tiles are centered at integer positions, so the quad
    // for tile (0,0) spans [-0.5*tile_size .. +0.5*tile_size].
    let col = ((local_x / tile_size.x) + 0.5).floor() as i32;
    let row = ((local_y / tile_size.y) + 0.5).floor() as i32;

    if col >= 0 && row >= 0 && (col as u32) < grid_width && (row as u32) < grid_height {
        Some((col as u32, row as u32))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn viewport_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection: &mut Selection,
    vs: &mut super::ViewportState<'_>,
    pending_open_path: &mut Option<std::path::PathBuf>,
    is_playing: bool,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
    tilemap_paint: &mut TilemapPaintState,
) {
    let viewport_size = &mut *vs.size;
    let viewport_focused = &mut *vs.focused;
    let viewport_hovered = &mut *vs.hovered;
    let fb_tex_id = vs.fb_tex_id;
    let gizmo = &mut *vs.gizmo;
    let gizmo_operation = &mut *vs.gizmo_operation;
    let editor_camera = vs.editor_camera;
    let scene_fb = &mut *vs.scene_fb;
    let hovered_entity = vs.hovered_entity;
    let viewport_mouse_pos = &mut *vs.mouse_pos;
    let tileset_preview = &vs.tileset_preview;
    let snap_to_grid = vs.snap_to_grid;
    let grid_size = vs.grid_size;
    let gizmo_local = &mut *vs.gizmo_local;
    let gizmo_editing = &mut *vs.gizmo_editing;
    let available = ui.available_size();
    if available.x > 0.0 && available.y > 0.0 {
        // Scale by DPI so the framebuffer renders at physical
        // pixel resolution (crisp on high-DPI displays).
        let ppp = ui.ctx().pixels_per_point();
        *viewport_size = ((available.x * ppp) as u32, (available.y * ppp) as u32);
    }

    *viewport_hovered = ui.ui_contains_pointer();

    // Determine if tilemap paint mode is active: brush is set AND selected
    // entity has a TilemapComponent.
    let paint_mode_active = !is_playing
        && tilemap_paint.is_active()
        && selection
            .single()
            .map(|e| scene.has_component::<TilemapComponent>(e))
            .unwrap_or(false);

    let clicked = ui.input(|i| i.pointer.any_pressed());
    if clicked && *viewport_hovered {
        *viewport_focused = true;

        // Mouse picking / paint — left click (edit mode only).
        if !is_playing {
            let left_click = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let alt_held = ui.input(|i| i.modifiers.alt);
            if left_click && !gizmo.is_focused() && !alt_held {
                if paint_mode_active {
                    // Begin a paint stroke.
                    undo_system.begin_edit(scene, "Paint tilemap");
                    tilemap_paint.painting_in_progress = true;
                    tilemap_paint.painted_this_stroke.clear();
                } else {
                    // Normal entity selection.
                    let ctrl = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
                    if hovered_entity >= 0 {
                        if let Some(e) = scene
                            .find_entity_by_id(hovered_entity as u32)
                            .filter(|e| scene.is_alive(*e))
                        {
                            if ctrl {
                                selection.toggle(e);
                            } else {
                                selection.set(e);
                            }
                        }
                    } else if !ctrl {
                        selection.clear();
                    }
                }
            }
        }
    }

    // End paint stroke on mouse release.
    if tilemap_paint.painting_in_progress {
        let left_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        if left_released || !ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
            tilemap_paint.painting_in_progress = false;
            undo_system.end_edit();
        }
    }

    let viewport_rect = if let Some(tex_id) = fb_tex_id {
        let size = egui::vec2(available.x, available.y);
        let response = ui.image(egui::load::SizedTexture::new(tex_id, size));

        // Accept .ggscene drops from the content browser.
        if let Some(payload) = response.dnd_release_payload::<ContentBrowserPayload>() {
            if !payload.is_directory && payload.path.extension().is_some_and(|ext| ext == "ggscene")
            {
                *pending_open_path = Some(payload.path.clone());
            }
        }

        // Visual feedback: blue border when hovering with a valid
        // scene file payload.
        if let Some(payload) = response.dnd_hover_payload::<ContentBrowserPayload>() {
            if !payload.is_directory && payload.path.extension().is_some_and(|ext| ext == "ggscene")
            {
                ui.painter().rect_stroke(
                    response.rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(0x00, 0x7A, 0xCC)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        Some(response.rect)
    } else {
        None
    };

    // -- Mouse position + pixel readback --
    // Track viewport-relative mouse position for tilemap painting and UI interaction.
    *viewport_mouse_pos = None;
    if *viewport_hovered {
        if let Some(viewport_rect) = viewport_rect {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                let ppp = ui.ctx().pixels_per_point();
                let mx = ((pos.x - viewport_rect.min.x) * ppp) as i32;
                let my = ((pos.y - viewport_rect.min.y) * ppp) as i32;

                if mx >= 0 && my >= 0 && mx < viewport_size.0 as i32 && my < viewport_size.1 as i32
                {
                    // Pixel readback for entity picking (edit mode only).
                    if !is_playing {
                        if let Some(fb) = scene_fb.as_mut() {
                            fb.schedule_pixel_readback(1, mx, my);
                        }
                    }
                    *viewport_mouse_pos = Some((mx as f32, my as f32));
                }
            }
        }
    }

    // -- Tilemap painting (continuous while dragging) --
    if tilemap_paint.painting_in_progress && paint_mode_active {
        if let (Some(entity), Some((px, py))) = (selection.single(), *viewport_mouse_pos) {
            let vp = editor_camera.view_projection();
            let world_transform = scene.get_world_transform(entity);
            let tilemap_z = {
                scene
                    .get_component::<TransformComponent>(entity)
                    .map(|tc| tc.translation.z)
                    .unwrap_or(0.0)
            };
            let (tile_size, grid_w, grid_h) = {
                let tm = scene.get_component::<TilemapComponent>(entity).unwrap();
                (tm.tile_size, tm.width, tm.height)
            };

            if let Some((col, row)) = screen_to_tile_grid(
                px,
                py,
                *viewport_size,
                &vp,
                &world_transform,
                tilemap_z,
                tile_size,
                grid_w,
                grid_h,
            ) {
                if !tilemap_paint.painted_this_stroke.contains(&(col, row)) {
                    tilemap_paint.painted_this_stroke.insert((col, row));
                    let value = tilemap_paint.composed_value();
                    if let Some(mut tm) = scene.get_component_mut::<TilemapComponent>(entity) {
                        tm.set_tile(col, row, value);
                    }
                    *scene_dirty = true;
                }
            }
        }
    }

    // -- Gizmo mode toolbar (edit mode only) --
    if !is_playing {
        if let Some(viewport_rect) = viewport_rect {
            let btn_size = egui::vec2(24.0, 24.0);
            let padding = 6.0;
            let spacing = 2.0;
            let toolbar_x = viewport_rect.min.x + padding;
            let toolbar_y = viewport_rect.min.y + padding;

            let operations = [
                (GizmoOperation::None, "Q", "Select (Q)"),
                (GizmoOperation::Translate, "W", "Translate (W)"),
                (GizmoOperation::Rotate, "E", "Rotate (E)"),
                (GizmoOperation::Scale, "R", "Scale (R)"),
            ];

            let active_bg = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);
            let inactive_bg = egui::Color32::from_rgba_premultiplied(0x30, 0x30, 0x30, 0xCC);
            let hover_bg = egui::Color32::from_rgb(0x50, 0x50, 0x50);

            for (i, (op, label, tooltip)) in operations.iter().enumerate() {
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(toolbar_x + i as f32 * (btn_size.x + spacing), toolbar_y),
                    btn_size,
                );
                let resp = ui.allocate_rect(btn_rect, egui::Sense::click());
                let is_active = *gizmo_operation == *op;

                let bg = if is_active {
                    active_bg
                } else if resp.hovered() {
                    hover_bg
                } else {
                    inactive_bg
                };

                ui.painter()
                    .rect_filled(btn_rect, egui::CornerRadius::same(3), bg);
                let text_color = if is_active {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_rgb(0xCC, 0xCC, 0xCC)
                };
                ui.painter().text(
                    btn_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::new(12.0, egui::FontFamily::Monospace),
                    text_color,
                );

                if resp.clicked() {
                    *gizmo_operation = *op;
                }
                resp.on_hover_text(*tooltip);
            }

            // Local / World toggle — after a small gap.
            let toggle_x = toolbar_x + operations.len() as f32 * (btn_size.x + spacing) + padding;
            let toggle_label = if *gizmo_local { "L" } else { "W" };
            let toggle_tooltip = if *gizmo_local {
                "Local space (click for World)"
            } else {
                "World space (click for Local)"
            };
            let toggle_rect = egui::Rect::from_min_size(egui::pos2(toggle_x, toolbar_y), btn_size);
            let toggle_resp = ui.allocate_rect(toggle_rect, egui::Sense::click());
            let toggle_bg = if toggle_resp.hovered() {
                hover_bg
            } else {
                inactive_bg
            };
            ui.painter()
                .rect_filled(toggle_rect, egui::CornerRadius::same(3), toggle_bg);
            ui.painter().text(
                toggle_rect.center(),
                egui::Align2::CENTER_CENTER,
                toggle_label,
                egui::FontId::new(12.0, egui::FontFamily::Monospace),
                egui::Color32::from_rgb(0xCC, 0xCC, 0xCC),
            );
            if toggle_resp.clicked() {
                *gizmo_local = !*gizmo_local;
            }
            toggle_resp.on_hover_text(toggle_tooltip);
        }
    }

    // -- Tile brush overlay (top-right of viewport) --
    if !is_playing {
        if let Some(viewport_rect) = viewport_rect {
            if tilemap_paint.brush_tile_id >= 0 {
                if let Some(preview) = tileset_preview {
                    let tid = tilemap_paint.brush_tile_id as usize;
                    let cols = preview.tileset_columns.max(1) as usize;
                    let ts_col = tid % cols;
                    let ts_row = tid / cols;

                    let preview_size = 48.0;
                    let pad = 8.0;
                    let text_h = 14.0;
                    let lines = if tilemap_paint.brush_flip_h || tilemap_paint.brush_flip_v {
                        3
                    } else {
                        2
                    };
                    let box_h = pad * 2.0 + preview_size + 4.0 + text_h * lines as f32;
                    let box_w = pad * 2.0 + preview_size.max(80.0);

                    let box_rect = egui::Rect::from_min_size(
                        egui::pos2(
                            viewport_rect.right() - box_w - 6.0,
                            viewport_rect.top() + 6.0,
                        ),
                        egui::vec2(box_w, box_h),
                    );

                    // Background
                    ui.painter().rect_filled(
                        box_rect,
                        egui::CornerRadius::same(4),
                        egui::Color32::from_rgba_premultiplied(0x1E, 0x1E, 0x1E, 0xDD),
                    );

                    // Tile image
                    let img_rect = egui::Rect::from_min_size(
                        egui::pos2(
                            box_rect.center().x - preview_size / 2.0,
                            box_rect.top() + pad,
                        ),
                        egui::vec2(preview_size, preview_size),
                    );
                    let uv_min = tile_uv_min(
                        ts_col,
                        ts_row,
                        preview.cell_size,
                        preview.spacing,
                        preview.margin,
                        preview.tex_w,
                        preview.tex_h,
                    );
                    let uv_max = tile_uv_max(
                        ts_col,
                        ts_row,
                        preview.cell_size,
                        preview.spacing,
                        preview.margin,
                        preview.tex_w,
                        preview.tex_h,
                    );
                    let mut mesh = egui::Mesh::with_texture(preview.egui_tex);
                    mesh.add_rect_with_uv(
                        img_rect,
                        egui::Rect::from_min_max(
                            egui::pos2(uv_min.0, uv_min.1),
                            egui::pos2(uv_max.0, uv_max.1),
                        ),
                        egui::Color32::WHITE,
                    );
                    ui.painter().add(egui::Shape::mesh(mesh));

                    // Labels
                    let label_y = img_rect.bottom() + 4.0;
                    let font = egui::FontId::new(11.0, egui::FontFamily::Monospace);
                    let text_color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
                    ui.painter().text(
                        egui::pos2(box_rect.center().x, label_y),
                        egui::Align2::CENTER_TOP,
                        format!("Tile {}", tilemap_paint.brush_tile_id),
                        font.clone(),
                        egui::Color32::WHITE,
                    );
                    ui.painter().text(
                        egui::pos2(box_rect.center().x, label_y + text_h),
                        egui::Align2::CENTER_TOP,
                        format!("col {}, row {}", ts_col, ts_row),
                        font.clone(),
                        text_color,
                    );
                    if tilemap_paint.brush_flip_h || tilemap_paint.brush_flip_v {
                        let mut flags = Vec::new();
                        if tilemap_paint.brush_flip_h {
                            flags.push("H-Flip");
                        }
                        if tilemap_paint.brush_flip_v {
                            flags.push("V-Flip");
                        }
                        ui.painter().text(
                            egui::pos2(box_rect.center().x, label_y + text_h * 2.0),
                            egui::Align2::CENTER_TOP,
                            flags.join(", "),
                            font,
                            egui::Color32::from_rgb(0xE8, 0xA8, 0x48),
                        );
                    }
                }
            }
        }
    }

    // -- Gizmos (edit mode only) --
    if let Some(viewport_rect) = viewport_rect {
        if !is_playing {
            if let Some(entity) = selection.single() {
                if scene.is_alive(entity) && *gizmo_operation != GizmoOperation::None {
                    // Use the editor camera for gizmo view/projection.
                    let camera_view = *editor_camera.view_matrix();
                    // Undo Vulkan Y-flip for the gizmo library.
                    let mut camera_projection = *editor_camera.projection();
                    camera_projection.y_axis.y *= -1.0;

                    // Read entity world transform for gizmo display.
                    let world_transform = scene.get_world_transform(entity);
                    let (world_scale, world_quat, world_translation) =
                        world_transform.to_scale_rotation_translation();

                    // Get parent's world transform for local→world conversion.
                    let parent_world = scene.get_parent(entity).and_then(|puuid| {
                        scene
                            .find_entity_by_uuid(puuid)
                            .map(|pe| scene.get_world_transform(pe))
                    });

                    {
                        // Snapping: always on if snap_to_grid, or Ctrl held.
                        let snapping = snap_to_grid || ui.input(|i| i.modifiers.ctrl);
                        let snap_dist = if snap_to_grid { grid_size } else { 0.5_f32 };

                        // Directional lights: allow translate + rotate (rotation
                        // controls direction), suppress scale.
                        // Ambient lights: translate only (no direction/scale).
                        let is_dir_light = scene.has_component::<DirectionalLightComponent>(entity);
                        let is_ambient_light = scene.has_component::<AmbientLightComponent>(entity);
                        let modes = if is_ambient_light {
                            gizmo_modes_for(GizmoOperation::Translate)
                        } else if is_dir_light && *gizmo_operation == GizmoOperation::Scale {
                            // Redirect scale → translate for directional lights.
                            gizmo_modes_for(GizmoOperation::Translate)
                        } else {
                            gizmo_modes_for(*gizmo_operation)
                        };

                        // Configure the gizmo.
                        gizmo.update_config(GizmoConfig {
                            view_matrix: mat4_to_f64(&camera_view).into(),
                            projection_matrix: mat4_to_f64(&camera_projection).into(),
                            viewport: viewport_rect,
                            modes,
                            orientation: if *gizmo_local {
                                GizmoOrientation::Local
                            } else {
                                GizmoOrientation::Global
                            },
                            snapping,
                            snap_angle: std::f32::consts::FRAC_PI_4, // 45 degrees
                            snap_distance: snap_dist,
                            snap_scale: 0.5_f32,
                            ..Default::default()
                        });

                        // Build gizmo Transform from world data.
                        let gizmo_transform = GizmoTransform::from_scale_rotation_translation(
                            DVec3::new(
                                world_scale.x as f64,
                                world_scale.y as f64,
                                world_scale.z as f64,
                            ),
                            DQuat::from_xyzw(
                                world_quat.x as f64,
                                world_quat.y as f64,
                                world_quat.z as f64,
                                world_quat.w as f64,
                            ),
                            DVec3::new(
                                world_translation.x as f64,
                                world_translation.y as f64,
                                world_translation.z as f64,
                            ),
                        );

                        // Interact (renders gizmo + returns new world transforms).
                        // Track gizmo focus transitions for undo bracketing.
                        let was_editing = *gizmo_editing;
                        if gizmo.is_focused() && !was_editing {
                            undo_system.begin_edit(scene, "Transform entity");
                            *gizmo_editing = true;
                        } else if !gizmo.is_focused() && was_editing {
                            undo_system.end_edit();
                            *gizmo_editing = false;
                        }
                        if let Some((_result, new_transforms)) =
                            gizmo.interact(ui, &[gizmo_transform])
                        {
                            if let Some(new_t) = new_transforms.first() {
                                // Read back new world transform.
                                let new_world_translation = Vec3::new(
                                    new_t.translation.x as f32,
                                    new_t.translation.y as f32,
                                    new_t.translation.z as f32,
                                );
                                let new_world_scale = Vec3::new(
                                    new_t.scale.x as f32,
                                    new_t.scale.y as f32,
                                    new_t.scale.z as f32,
                                );
                                let new_world_quat = Quat::from_xyzw(
                                    new_t.rotation.v.x as f32,
                                    new_t.rotation.v.y as f32,
                                    new_t.rotation.v.z as f32,
                                    new_t.rotation.s as f32,
                                );

                                // Build new world matrix.
                                let new_world_mat = Mat4::from_scale_rotation_translation(
                                    new_world_scale,
                                    new_world_quat,
                                    new_world_translation,
                                );

                                // Convert world → local if entity has a parent.
                                let new_local_mat = match parent_world {
                                    Some(pw) => pw.inverse() * new_world_mat,
                                    None => new_world_mat,
                                };

                                let (local_scale, local_quat, local_translation) =
                                    new_local_mat.to_scale_rotation_translation();

                                // Write back local transform — direct quaternion storage.
                                if let Some(mut tc) =
                                    scene.get_component_mut::<TransformComponent>(entity)
                                {
                                    tc.translation = local_translation;
                                    tc.set_rotation_quat(local_quat);
                                    tc.scale = local_scale;
                                    *scene_dirty = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
