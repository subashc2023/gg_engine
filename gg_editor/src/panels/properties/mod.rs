mod audio;
mod camera;
mod physics;
mod scripting;
mod sprite;
mod text;
mod tilemap;

#[cfg(feature = "lua-scripting")]
pub(crate) use scripting::clear_field_cache;

use gg_engine::egui;
use gg_engine::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn properties_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection_context: &mut Option<Entity>,
    asset_manager: &mut Option<EditorAssetManager>,
    is_playing: bool,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
    tilemap_paint: &mut crate::TilemapPaintState,
    egui_texture_map: &std::collections::HashMap<u64, egui::TextureId>,
) {
    if let Some(entity) = *selection_context {
        if scene.is_alive(entity) {
            draw_components(ui, scene, entity, asset_manager, is_playing, assets_root, scene_dirty, undo_system, tilemap_paint, egui_texture_map);
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

    ui.horizontal(|ui| {
        // Label column.
        ui.allocate_ui_with_layout(
            egui::vec2(column_width, 0.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(label);
            },
        );

        // Available width for 3 button+drag pairs. Reserve spacing.
        let avail = ui.available_width();
        let group_width = (avail - 2.0 * ui.spacing().item_spacing.x) / 3.0;
        let _drag_width = (group_width - 20.0).max(20.0);

        // --- X (red) ---
        {
            let x_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new("X")
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(egui::Color32::from_rgb(0xBB, 0x22, 0x22))
                .corner_radius(egui::CornerRadius {
                    nw: 2,
                    sw: 2,
                    ne: 0,
                    se: 0,
                })
                .min_size(egui::vec2(20.0, 0.0)),
            );
            if x_btn.clicked() {
                values.x = reset_value;
                changed = true;
            }
            if ui
                .add(
                    egui::DragValue::new(&mut values.x)
                        .speed(0.1)
                        .max_decimals(3)
                        .update_while_editing(false),
                )
                .changed()
            {
                changed = true;
            }
        }

        // --- Y (green) ---
        {
            let y_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new("Y")
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(egui::Color32::from_rgb(0x22, 0x88, 0x22))
                .corner_radius(egui::CornerRadius {
                    nw: 2,
                    sw: 2,
                    ne: 0,
                    se: 0,
                })
                .min_size(egui::vec2(20.0, 0.0)),
            );
            if y_btn.clicked() {
                values.y = reset_value;
                changed = true;
            }
            if ui
                .add(
                    egui::DragValue::new(&mut values.y)
                        .speed(0.1)
                        .max_decimals(3)
                        .update_while_editing(false),
                )
                .changed()
            {
                changed = true;
            }
        }

        // --- Z (blue) ---
        {
            let z_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new("Z")
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(egui::Color32::from_rgb(0x22, 0x44, 0xBB))
                .corner_radius(egui::CornerRadius {
                    nw: 2,
                    sw: 2,
                    ne: 0,
                    se: 0,
                })
                .min_size(egui::vec2(20.0, 0.0)),
            );
            if z_btn.clicked() {
                values.z = reset_value;
                changed = true;
            }
            if ui
                .add(
                    egui::DragValue::new(&mut values.z)
                        .speed(0.1)
                        .max_decimals(3)
                        .update_while_editing(false),
                )
                .changed()
            {
                changed = true;
            }
        }
    });

    changed
}

// ---------------------------------------------------------------------------
// Component inspector — dispatches to per-component modules
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_components(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    asset_manager: &mut Option<EditorAssetManager>,
    is_playing: bool,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
    tilemap_paint: &mut crate::TilemapPaintState,
    egui_texture_map: &std::collections::HashMap<u64, egui::TextureId>,
) {
    let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

    // Coalesced undo: detect any drag interaction starting/ending this frame.
    let any_drag_active = ui.ctx().dragged_id().is_some();
    if any_drag_active && !undo_system.is_editing() {
        undo_system.begin_edit(scene);
    } else if !any_drag_active && undo_system.is_editing() {
        undo_system.end_edit();
    }

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
                    undo_system.record(scene);
                    scene.add_component(entity, CameraComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<SpriteRendererComponent>(entity)
                    && ui.button("Sprite Renderer").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, SpriteRendererComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<CircleRendererComponent>(entity)
                    && ui.button("Circle Renderer").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, CircleRendererComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<SpriteAnimatorComponent>(entity)
                    && ui.button("Sprite Animator").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, SpriteAnimatorComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<TextComponent>(entity)
                    && ui.button("Text").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, TextComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<RigidBody2DComponent>(entity)
                    && ui.button("Rigidbody 2D").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, RigidBody2DComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<BoxCollider2DComponent>(entity)
                    && ui.button("Box Collider 2D").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, BoxCollider2DComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<CircleCollider2DComponent>(entity)
                    && ui.button("Circle Collider 2D").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, CircleCollider2DComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<TilemapComponent>(entity)
                    && ui.button("Tilemap").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, TilemapComponent::default());
                    *scene_dirty = true;
                }
                if !scene.has_component::<AudioSourceComponent>(entity)
                    && ui.button("Audio Source").clicked()
                {
                    undo_system.record(scene);
                    scene.add_component(entity, AudioSourceComponent::default());
                    *scene_dirty = true;
                }

                #[cfg(feature = "lua-scripting")]
                {
                    if !scene.has_component::<LuaScriptComponent>(entity)
                        && ui.button("Lua Script").clicked()
                    {
                        undo_system.record(scene);
                        scene.add_component(entity, LuaScriptComponent::default());
                        *scene_dirty = true;
                    }
                }
            });
        });
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

    // -- Removable components (delegated to sub-modules) --

    if camera::draw_camera_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<CameraComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_sprite_renderer_component(ui, scene, entity, &bold_family, asset_manager, assets_root, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<SpriteRendererComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_sprite_animator_component(ui, scene, entity, &bold_family, asset_manager, assets_root, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<SpriteAnimatorComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_circle_renderer_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<CircleRendererComponent>(entity);
        *scene_dirty = true;
    }

    if text::draw_text_component(ui, scene, entity, &bold_family, assets_root, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<TextComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_rigidbody2d_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<RigidBody2DComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_box_collider2d_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<BoxCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_circle_collider2d_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<CircleCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    if scripting::draw_native_script_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<NativeScriptComponent>(entity);
        *scene_dirty = true;
    }

    if audio::draw_audio_source_component(ui, scene, entity, &bold_family, asset_manager, assets_root, scene_dirty, undo_system) {
        undo_system.record(scene);
        scene.remove_component::<AudioSourceComponent>(entity);
        *scene_dirty = true;
    }

    if tilemap::draw_tilemap_component(ui, scene, entity, &bold_family, asset_manager, assets_root, scene_dirty, undo_system, tilemap_paint, egui_texture_map) {
        undo_system.record(scene);
        scene.remove_component::<TilemapComponent>(entity);
        *scene_dirty = true;
    }

    #[cfg(feature = "lua-scripting")]
    {
        if scripting::draw_lua_script_component(ui, scene, entity, &bold_family, assets_root, is_playing, scene_dirty, undo_system) {
            undo_system.record(scene);
            scene.remove_component::<LuaScriptComponent>(entity);
            *scene_dirty = true;
        }
    }
}
