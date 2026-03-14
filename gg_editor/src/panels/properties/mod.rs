mod audio;
mod camera;
mod environment;
mod lighting;
mod mesh;
mod particles;
mod physics;
mod scripting;
mod skeletal_animation;
mod sprite;
mod text;
mod tilemap;
mod ui_anchor;
mod ui_image;
mod ui_interactable;
mod ui_layout;
mod ui_rect;

#[cfg(feature = "lua-scripting")]
pub(crate) use scripting::clear_field_cache;

use gg_engine::egui;
use gg_engine::prelude::*;

use crate::selection::Selection;

#[allow(clippy::too_many_arguments)]
pub(crate) fn properties_ui(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    selection: &mut Selection,
    asset_manager: &mut Option<EditorAssetManager>,
    is_playing: bool,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    undo_system: &mut crate::undo::UndoSystem,
    tilemap_paint: &mut crate::TilemapPaintState,
    egui_texture_map: &std::collections::HashMap<u64, egui::TextureId>,
) {
    // Remove dead entities from selection.
    selection.retain(|e| scene.is_alive(*e));

    match selection.len() {
        0 => {}
        1 => {
            let entity = selection.single().unwrap();
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
        }
        n => {
            ui.centered_and_justified(|ui| {
                ui.label(format!("{} entities selected", n));
            });
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
                    let rel_path = super::relative_asset_path(&payload.path, am.asset_directory());
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

    // --- Undo: capture pre-frame entity snapshot before any modifications ---
    // Only serializes the selected entity (not the entire scene) for ~100× speedup
    // on large scenes.
    if !undo_system.is_editing() {
        undo_system.capture_pre_frame_entity(scene, entity);
    }

    // --- Undo: temporarily reset scene_dirty to detect changes this frame ---
    let dirty_before_properties = *scene_dirty;
    *scene_dirty = false;

    // Track whether a text field has keyboard focus (for coalescing text edits).
    let mut text_field_focused = false;

    // -- Tag Component + Add Component button (inline) --
    if scene.has_component::<TagComponent>(entity) {
        let mut tag = scene
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_default();

        ui.horizontal(|ui| {
            let tag_resp = ui.text_edit_singleline(&mut tag);
            text_field_focused |= tag_resp.has_focus();
            if tag_resp.changed() {
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
                                undo_system.record(scene, concat!("Add ", $name));
                                scene.add_component(entity, <$type>::default());
                                *scene_dirty = true;
                            }
                        )*
                    };
                }
                gg_engine::for_each_addable_component!(add_component_buttons);

                // 3D colliders — mesh-aware defaults so colliders match primitives.
                #[cfg(feature = "physics-3d")]
                {
                    let mesh_prim = scene
                        .get_component::<MeshRendererComponent>(entity)
                        .and_then(|m| m.primitive());

                    // Box Collider 3D
                    if !scene.has_component::<BoxCollider3DComponent>(entity)
                        && ui.button("Box Collider 3D").clicked()
                    {
                        undo_system.record(scene, "Add Box Collider 3D");
                        let mut comp = BoxCollider3DComponent::default();
                        if mesh_prim == Some(MeshPrimitive::Plane) {
                            // Plane mesh is infinitely thin at Y=0.
                            // Use a very thin collider offset so the top surface aligns with Y=0.
                            comp.size = Vec3::new(0.5, 0.025, 0.5);
                            comp.offset = Vec3::new(0.0, -0.025, 0.0);
                        }
                        scene.add_component(entity, comp);
                        *scene_dirty = true;
                    }

                    // Sphere Collider 3D
                    if !scene.has_component::<SphereCollider3DComponent>(entity)
                        && ui.button("Sphere Collider 3D").clicked()
                    {
                        undo_system.record(scene, "Add Sphere Collider 3D");
                        scene.add_component(entity, SphereCollider3DComponent::default());
                        *scene_dirty = true;
                    }

                    // Capsule Collider 3D
                    if !scene.has_component::<CapsuleCollider3DComponent>(entity)
                        && ui.button("Capsule Collider 3D").clicked()
                    {
                        undo_system.record(scene, "Add Capsule Collider 3D");
                        scene.add_component(entity, CapsuleCollider3DComponent::default());
                        *scene_dirty = true;
                    }

                    // Mesh Collider 3D
                    if !scene.has_component::<MeshCollider3DComponent>(entity)
                        && ui.button("Mesh Collider 3D").clicked()
                    {
                        undo_system.record(scene, "Add Mesh Collider 3D");
                        scene.add_component(entity, MeshCollider3DComponent::default());
                        *scene_dirty = true;
                    }
                }

                // Skeletal Animation — requires asset, not Default-constructible.
                if !scene.has_component::<SkeletalAnimationComponent>(entity)
                    && ui.button("Skeletal Animation").clicked()
                {
                    undo_system.record(scene, "Add Skeletal Animation");
                    scene.add_component(
                        entity,
                        SkeletalAnimationComponent::from_asset(Uuid::from_raw(0)),
                    );
                    *scene_dirty = true;
                }

                #[cfg(feature = "lua-scripting")]
                {
                    if !scene.has_component::<LuaScriptComponent>(entity)
                        && ui.button("Lua Script").clicked()
                    {
                        undo_system.record(scene, "Add Lua Script");
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
                let euler = tc.euler_angles_stable();
                (
                    tc.translation,
                    Vec3::new(
                        euler.x.to_degrees(),
                        euler.y.to_degrees(),
                        euler.z.to_degrees(),
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
                    tc.set_euler_angles(Vec3::new(
                        rotation_deg.x.to_radians(),
                        rotation_deg.y.to_radians(),
                        rotation_deg.z.to_radians(),
                    ));
                    tc.scale = scale;
                    *scene_dirty = true;
                }
            }
        });
    }

    // -- Removable components (delegated to sub-modules) --

    if camera::draw_camera_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene, "Remove Camera");
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
        undo_system.record(scene, "Remove Sprite Renderer");
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
        undo_system.record(scene, "Remove Sprite Animator");
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
        undo_system.record(scene, "Remove Instanced Animator");
        scene.remove_component::<InstancedSpriteAnimator>(entity);
        *scene_dirty = true;
    }

    if sprite::draw_animation_controller(ui, scene, entity, &bold_family, scene_dirty, undo_system)
    {
        undo_system.record(scene, "Remove Animation Controller");
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
        undo_system.record(scene, "Remove Circle Renderer");
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
        undo_system.record(scene, "Remove Text");
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
        undo_system.record(scene, "Remove Rigidbody 2D");
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
        undo_system.record(scene, "Remove Box Collider 2D");
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
        undo_system.record(scene, "Remove Circle Collider 2D");
        scene.remove_component::<CircleCollider2DComponent>(entity);
        *scene_dirty = true;
    }

    #[cfg(feature = "physics-3d")]
    {
        if physics::draw_rigidbody3d_component(
            ui,
            scene,
            entity,
            &bold_family,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene, "Remove Rigidbody 3D");
            scene.remove_component::<RigidBody3DComponent>(entity);
            *scene_dirty = true;
        }

        if physics::draw_box_collider3d_component(
            ui,
            scene,
            entity,
            &bold_family,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene, "Remove Box Collider 3D");
            scene.remove_component::<BoxCollider3DComponent>(entity);
            *scene_dirty = true;
        }

        if physics::draw_sphere_collider3d_component(
            ui,
            scene,
            entity,
            &bold_family,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene, "Remove Sphere Collider 3D");
            scene.remove_component::<SphereCollider3DComponent>(entity);
            *scene_dirty = true;
        }

        if physics::draw_capsule_collider3d_component(
            ui,
            scene,
            entity,
            &bold_family,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene, "Remove Capsule Collider 3D");
            scene.remove_component::<CapsuleCollider3DComponent>(entity);
            *scene_dirty = true;
        }

        if physics::draw_mesh_collider3d_component(
            ui,
            scene,
            entity,
            &bold_family,
            scene_dirty,
            undo_system,
        ) {
            undo_system.record(scene, "Remove Mesh Collider 3D");
            scene.remove_component::<MeshCollider3DComponent>(entity);
            *scene_dirty = true;
        }
    }

    if scripting::draw_native_script_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Native Script");
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
        undo_system.record(scene, "Remove Audio Source");
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
        undo_system.record(scene, "Remove Audio Listener");
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
        undo_system.record(scene, "Remove Particle Emitter");
        scene.remove_component::<ParticleEmitterComponent>(entity);
        *scene_dirty = true;
    }

    if mesh::draw_mesh_renderer_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Mesh Renderer");
        scene.remove_component::<MeshRendererComponent>(entity);
        *scene_dirty = true;
    }

    if skeletal_animation::draw_skeletal_animation_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Skeletal Animation");
        scene.remove_component::<SkeletalAnimationComponent>(entity);
        *scene_dirty = true;
    }

    if lighting::draw_directional_light_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Directional Light");
        scene.remove_component::<DirectionalLightComponent>(entity);
        *scene_dirty = true;
    }

    if lighting::draw_point_light_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Point Light");
        scene.remove_component::<PointLightComponent>(entity);
        *scene_dirty = true;
    }

    if lighting::draw_ambient_light_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Ambient Light");
        scene.remove_component::<AmbientLightComponent>(entity);
        *scene_dirty = true;
    }

    if environment::draw_environment_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove Environment Map");
        scene.remove_component::<EnvironmentComponent>(entity);
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
        undo_system.record(scene, "Remove Tilemap");
        scene.remove_component::<TilemapComponent>(entity);
        *scene_dirty = true;
    }

    if ui_anchor::draw_ui_anchor_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove UI Anchor");
        scene.remove_component::<UIAnchorComponent>(entity);
        *scene_dirty = true;
    }

    if ui_rect::draw_ui_rect_component(ui, scene, entity, &bold_family, scene_dirty, undo_system) {
        undo_system.record(scene, "Remove UI Rect");
        scene.remove_component::<UIRectComponent>(entity);
        *scene_dirty = true;
    }

    if ui_image::draw_ui_image_component(
        ui,
        scene,
        entity,
        &bold_family,
        asset_manager,
        assets_root,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove UI Image");
        scene.remove_component::<UIImageComponent>(entity);
        *scene_dirty = true;
    }

    if ui_interactable::draw_ui_interactable_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove UI Interactable");
        scene.remove_component::<UIInteractableComponent>(entity);
        *scene_dirty = true;
    }

    if ui_layout::draw_ui_layout_component(
        ui,
        scene,
        entity,
        &bold_family,
        scene_dirty,
        undo_system,
    ) {
        undo_system.record(scene, "Remove UI Layout");
        scene.remove_component::<UILayoutComponent>(entity);
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
            undo_system.record(scene, "Remove Lua Script");
            scene.remove_component::<LuaScriptComponent>(entity);
            *scene_dirty = true;
        }
    }

    // --- Undo: detect changes and handle undo recording ---
    let properties_changed = *scene_dirty;
    *scene_dirty = dirty_before_properties || properties_changed;

    let any_drag = ui.ctx().dragged_id().is_some();
    let continuous_edit = any_drag || text_field_focused;

    if continuous_edit && !undo_system.is_editing() {
        // A continuous interaction started (drag or text input).
        undo_system.begin_edit_from_pre_frame("Modify properties");
    } else if !continuous_edit && undo_system.is_editing() {
        // A continuous interaction ended.
        undo_system.end_edit();
    } else if !continuous_edit && !undo_system.is_editing() && properties_changed {
        // A discrete change (checkbox, combobox, asset picker, etc.).
        undo_system.push_pre_frame("Change properties");
    }
}
