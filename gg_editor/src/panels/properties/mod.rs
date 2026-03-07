mod audio;
mod camera;
mod particles;
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
            draw_components(
                ui,
                scene,
                entity,
                asset_manager,
                is_playing,
                assets_root,
                scene_dirty,
                undo_system,
                tilemap_paint,
                egui_texture_map,
            );
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
// Shared UI helpers (used by per-component modules)
// ---------------------------------------------------------------------------

/// Draw a collapsing component header with a "Remove Component" context menu.
/// Returns `true` if the user requested removal.
fn component_header(
    ui: &mut egui::Ui,
    label: &str,
    id_salt: &str,
    bold_family: &egui::FontFamily,
    entity: Entity,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut remove = false;
    let cr = egui::CollapsingHeader::new(
        egui::RichText::new(label).font(egui::FontId::new(14.0, bold_family.clone())),
    )
    .id_salt((id_salt, entity.id()))
    .default_open(true)
    .show(ui, |ui| body(ui));

    cr.header_response.context_menu(|ui| {
        if ui.button("Remove Component").clicked() {
            remove = true;
            ui.close();
        }
    });
    remove
}

/// Draw a color picker for an RGBA f32 array. Returns `true` if changed.
fn color_picker_rgba(ui: &mut egui::Ui, label: &str, color: &mut [f32; 4]) -> bool {
    let mut egui_color = egui::Color32::from_rgba_unmultiplied(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    );
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if egui::color_picker::color_edit_button_srgba(
            ui,
            &mut egui_color,
            egui::color_picker::Alpha::OnlyBlend,
        )
        .changed()
        {
            let [r, g, b, a] = egui_color.to_srgba_unmultiplied();
            *color = [
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                a as f32 / 255.0,
            ];
            changed = true;
        }
    });
    changed
}

/// Draw sorting layer and order-in-layer drag values. Returns `true` if changed.
fn sorting_layer_controls(
    ui: &mut egui::Ui,
    sorting_layer: &mut i32,
    order_in_layer: &mut i32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Sorting Layer");
        changed |= ui
            .add(egui::DragValue::new(sorting_layer).speed(0.1))
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label("Order in Layer");
        changed |= ui
            .add(egui::DragValue::new(order_in_layer).speed(0.1))
            .changed();
    });
    changed
}

// ---------------------------------------------------------------------------
// Asset handle picker (shared by sprite texture + audio file pickers)
// ---------------------------------------------------------------------------

/// Result of an asset handle picker interaction.
pub(crate) enum AssetPickerAction {
    /// No change.
    None,
    /// User selected or dropped a new asset.
    Selected(Uuid),
    /// User clicked the clear (X) button.
    Cleared,
}

/// Reusable asset handle picker: shows asset filename, click to browse,
/// drag-drop from content browser, hover highlight, X clear button.
pub(crate) fn asset_handle_picker(
    ui: &mut egui::Ui,
    current_handle_raw: u64,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    subdirectory: &str,
    dialog_title: &str,
    extensions: &[&str],
) -> AssetPickerAction {
    // Resolve label from asset metadata.
    let label = if current_handle_raw != 0 {
        if let Some(am) = asset_manager.as_ref() {
            let handle = Uuid::from_raw(current_handle_raw);
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

    let btn_width = ((label.len() as f32) * 7.0 + 20.0).max(100.0);
    let btn_resp = ui.add_sized([btn_width, 0.0], egui::Button::new(&label));

    // Click to open file dialog.
    if btn_resp.clicked() {
        if let Some(am) = asset_manager.as_mut() {
            let start_dir = assets_root.join(subdirectory);
            let start_dir_str = start_dir.to_string_lossy();
            if let Some(path_str) =
                FileDialogs::open_file_in(dialog_title, extensions, &start_dir_str)
            {
                let abs_path = std::path::PathBuf::from(&path_str);
                let rel_path = super::relative_asset_path(&abs_path, am.asset_directory());
                let handle = am.import_asset(&rel_path);
                return AssetPickerAction::Selected(handle);
            }
        }
    }

    // Drag-and-drop from content browser.
    if let Some(payload) =
        btn_resp.dnd_release_payload::<super::content_browser::ContentBrowserPayload>()
    {
        if !payload.is_directory {
            let ext = payload
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if extensions.contains(&ext.as_str()) {
                if let Some(am) = asset_manager.as_mut() {
                    let rel_path =
                        super::relative_asset_path(&payload.path, am.asset_directory());
                    let handle = am.import_asset(&rel_path);
                    return AssetPickerAction::Selected(handle);
                }
            }
        }
    }

    // Drop target highlight.
    if btn_resp
        .dnd_hover_payload::<super::content_browser::ContentBrowserPayload>()
        .is_some()
    {
        ui.painter().rect_stroke(
            btn_resp.rect,
            egui::CornerRadius::same(2),
            egui::Stroke::new(2.0, egui::Color32::from_rgb(0x56, 0x9C, 0xD6)),
            egui::StrokeKind::Inside,
        );
    }

    // Clear button.
    if current_handle_raw != 0 && ui.small_button("X").clicked() {
        return AssetPickerAction::Cleared;
    }

    AssetPickerAction::None
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
                macro_rules! add_component_buttons {
                    ($(($type:path, $name:expr)),* $(,)?) => {
                        $(
                            if !scene.has_component::<$type>(entity)
                                && ui.button($name).clicked()
                            {
                                undo_system.record(scene);
                                scene.add_component(entity, <$type>::default());
                                *scene_dirty = true;
                            }
                        )*
                    };
                }
                gg_engine::for_each_addable_component!(add_component_buttons);

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

    if sprite::draw_sprite_renderer_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<SpriteRendererComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_sprite_animator_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<SpriteAnimatorComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_instanced_sprite_animator(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<InstancedSpriteAnimator>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_animation_controller(ui, scene, entity, &bold_family, scene_dirty, undo_system)
    {
        undo_system.record(scene);
        scene.remove_component::<AnimationControllerComponent>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_circle_renderer_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<CircleRendererComponent>(entity);
        *scene_dirty = true;
    }

    if text::draw_text_component(
        ui,
        scene,
        entity,
        &bold_family,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<TextComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_rigidbody2d_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<RigidBody2DComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_box_collider2d_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<BoxCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    if physics::draw_circle_collider2d_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<CircleCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    if scripting::draw_native_script_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<NativeScriptComponent>(entity);
        *scene_dirty = true;
    }

    if audio::draw_audio_source_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<AudioSourceComponent>(entity);
        *scene_dirty = true;
    }

    if audio::draw_audio_listener_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<AudioListenerComponent>(entity);
        *scene_dirty = true;
    }

    if particles::draw_particle_emitter_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene);
        scene.remove_component::<ParticleEmitterComponent>(entity);
        *scene_dirty = true;
    }

    if tilemap::draw_tilemap_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
        tilemap_paint,
        egui_texture_map,
    ) {
        undo_system.record(scene);
        scene.remove_component::<TilemapComponent>(entity);
        *scene_dirty = true;
    }

    #[cfg(feature = "lua-scripting")]
    {
        if scripting::draw_lua_script_component(
            ui,
            scene,
            entity,
            &bold_family,
            assets_root,
            is_playing,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene);
            scene.remove_component::<LuaScriptComponent>(entity);
            *scene_dirty = true;
        }
    }
}
