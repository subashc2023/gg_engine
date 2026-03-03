use gg_engine::egui;
use gg_engine::glam::EulerRot;
use gg_engine::prelude::*;
use transform_gizmo_egui::math::{DQuat, DVec3, Transform as GizmoTransform};
use transform_gizmo_egui::{Gizmo, GizmoConfig, GizmoExt, GizmoOrientation};

use crate::gizmo::{gizmo_modes_for, mat4_to_f64, GizmoOperation};
use crate::panels::content_browser::ContentBrowserPayload;

#[allow(clippy::too_many_arguments)]
pub(crate) fn viewport_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    viewport_size: &mut (u32, u32),
    viewport_focused: &mut bool,
    viewport_hovered: &mut bool,
    fb_tex_id: Option<egui::TextureId>,
    gizmo: &mut Gizmo,
    gizmo_operation: GizmoOperation,
    editor_camera: &EditorCamera,
    scene_fb: &mut Option<Framebuffer>,
    hovered_entity: i32,
    pending_open_path: &mut Option<std::path::PathBuf>,
    is_playing: bool,
) {
    let available = ui.available_size();
    if available.x > 0.0 && available.y > 0.0 {
        // Scale by DPI so the framebuffer renders at physical
        // pixel resolution (crisp on high-DPI displays).
        let ppp = ui.ctx().pixels_per_point();
        *viewport_size = ((available.x * ppp) as u32, (available.y * ppp) as u32);
    }

    *viewport_hovered = ui.ui_contains_pointer();

    let clicked = ui.input(|i| i.pointer.any_pressed());
    if clicked && *viewport_hovered {
        *viewport_focused = true;

        // Mouse picking — select entity on left click (edit mode only).
        if !is_playing {
            let left_click =
                ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let alt_held = ui.input(|i| i.modifiers.alt);
            if left_click && !gizmo.is_focused() && !alt_held {
                if hovered_entity >= 0 {
                    *selection_context = scene
                        .find_entity_by_id(hovered_entity as u32)
                        .filter(|e| scene.is_alive(*e));
                } else {
                    *selection_context = None;
                }
            }
        }
    }

    let viewport_rect = if let Some(tex_id) = fb_tex_id {
        let size = egui::vec2(available.x, available.y);
        let response = ui.image(egui::load::SizedTexture::new(tex_id, size));

        // Accept .ggscene drops from the content browser.
        if let Some(payload) = response.dnd_release_payload::<ContentBrowserPayload>() {
            if !payload.is_directory
                && payload
                    .path
                    .extension()
                    .is_some_and(|ext| ext == "ggscene")
            {
                *pending_open_path = Some(payload.path.clone());
            }
        }

        // Visual feedback: blue border when hovering with a valid
        // scene file payload.
        if let Some(payload) = response.dnd_hover_payload::<ContentBrowserPayload>() {
            if !payload.is_directory
                && payload
                    .path
                    .extension()
                    .is_some_and(|ext| ext == "ggscene")
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

    // -- Mouse picking: schedule pixel readback (edit mode only) --
    if *viewport_hovered && !is_playing {
        if let Some(viewport_rect) = viewport_rect {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                let ppp = ui.ctx().pixels_per_point();
                let mx = ((pos.x - viewport_rect.min.x) * ppp) as i32;
                let my = ((pos.y - viewport_rect.min.y) * ppp) as i32;

                if mx >= 0
                    && my >= 0
                    && mx < viewport_size.0 as i32
                    && my < viewport_size.1 as i32
                {
                    if let Some(fb) = scene_fb.as_mut() {
                        fb.schedule_pixel_readback(1, mx, my);
                    }
                }
            }
        }
    }

    // -- Gizmos (edit mode only) --
    if let Some(viewport_rect) = viewport_rect {
        if !is_playing {
        if let Some(entity) = *selection_context {
            if scene.is_alive(entity) && gizmo_operation != GizmoOperation::None {
                // Use the editor camera for gizmo view/projection.
                let camera_view = *editor_camera.view_matrix();
                // Undo Vulkan Y-flip for the gizmo library.
                let mut camera_projection = *editor_camera.projection();
                camera_projection.y_axis.y *= -1.0;

                // Read entity transform.
                let entity_transform = {
                    let tc = scene.get_component::<TransformComponent>(entity);
                    tc.map(|tc| {
                        let original_rotation = tc.rotation;
                        let quat = Quat::from_euler(
                            EulerRot::XYZ,
                            tc.rotation.x,
                            tc.rotation.y,
                            tc.rotation.z,
                        );
                        (tc.translation, quat, tc.scale, original_rotation)
                    })
                };

                if let Some((translation, quat, scale, original_rotation)) = entity_transform {
                    // Snapping: Ctrl held enables snap.
                    let snapping = ui.input(|i| i.modifiers.ctrl);

                    // Configure the gizmo.
                    gizmo.update_config(GizmoConfig {
                        view_matrix: mat4_to_f64(&camera_view).into(),
                        projection_matrix: mat4_to_f64(&camera_projection).into(),
                        viewport: viewport_rect,
                        modes: gizmo_modes_for(gizmo_operation),
                        orientation: GizmoOrientation::Local,
                        snapping,
                        snap_angle: std::f32::consts::FRAC_PI_4, // 45 degrees
                        snap_distance: 0.5_f32,
                        snap_scale: 0.5_f32,
                        ..Default::default()
                    });

                    // Build gizmo Transform from entity data.
                    let gizmo_transform = GizmoTransform::from_scale_rotation_translation(
                        DVec3::new(scale.x as f64, scale.y as f64, scale.z as f64),
                        DQuat::from_xyzw(
                            quat.x as f64,
                            quat.y as f64,
                            quat.z as f64,
                            quat.w as f64,
                        ),
                        DVec3::new(
                            translation.x as f64,
                            translation.y as f64,
                            translation.z as f64,
                        ),
                    );

                    // Interact (renders gizmo + returns new transforms).
                    if let Some((_result, new_transforms)) =
                        gizmo.interact(ui, &[gizmo_transform])
                    {
                        if let Some(new_t) = new_transforms.first() {
                            // Read back translation & scale from mint types.
                            let new_translation = Vec3::new(
                                new_t.translation.x as f32,
                                new_t.translation.y as f32,
                                new_t.translation.z as f32,
                            );
                            let new_scale = Vec3::new(
                                new_t.scale.x as f32,
                                new_t.scale.y as f32,
                                new_t.scale.z as f32,
                            );

                            // Rotation: use delta approach to avoid
                            // gimbal lock snapping.
                            let new_quat = Quat::from_xyzw(
                                new_t.rotation.v.x as f32,
                                new_t.rotation.v.y as f32,
                                new_t.rotation.v.z as f32,
                                new_t.rotation.s as f32,
                            );
                            let (nx, ny, nz) = new_quat.to_euler(EulerRot::XYZ);
                            let (ox, oy, oz) = quat.to_euler(EulerRot::XYZ);
                            let delta_rotation = Vec3::new(nx - ox, ny - oy, nz - oz);
                            let new_rotation = original_rotation + delta_rotation;

                            // Write back to component.
                            if let Some(mut tc) =
                                scene.get_component_mut::<TransformComponent>(entity)
                            {
                                tc.translation = new_translation;
                                tc.rotation = new_rotation;
                                tc.scale = new_scale;
                            }
                        }
                    }
                }
            }
        }
        }
    }
}
