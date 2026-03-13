use gg_engine::egui;
use gg_engine::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_mesh_renderer_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<MeshRendererComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Mesh Renderer",
        "mesh_renderer",
        bold_family,
        entity,
        |ui| {
            let (
                mesh_source,
                mut color_arr,
                mut metallic,
                mut roughness,
                mut emissive_arr,
                mut emissive_strength,
                texture_handle_raw,
                normal_texture_handle_raw,
            ) = {
                let mc = scene
                    .get_component::<MeshRendererComponent>(entity)
                    .unwrap();
                (
                    mc.mesh_source.clone(),
                    <[f32; 4]>::from(mc.color),
                    mc.metallic,
                    mc.roughness,
                    <[f32; 3]>::from(mc.emissive_color),
                    mc.emissive_strength,
                    mc.texture_handle.raw(),
                    mc.normal_texture_handle.raw(),
                )
            };

            let mut changed = false;
            let mut source_changed = false;

            // Mesh source selector: Primitive or Asset.
            let is_primitive = matches!(mesh_source, MeshSource::Primitive(_));
            let source_labels = ["Primitive", "Mesh Asset"];
            let current_source = if is_primitive {
                source_labels[0]
            } else {
                source_labels[1]
            };
            let mut source_idx: usize = if is_primitive { 0 } else { 1 };
            egui::ComboBox::from_label("Mesh Source")
                .selected_text(current_source)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut source_idx, 0, source_labels[0])
                        .changed()
                    {
                        source_changed = true;
                    }
                    if ui
                        .selectable_value(&mut source_idx, 1, source_labels[1])
                        .changed()
                    {
                        source_changed = true;
                    }
                });

            // Handle source type change.
            if source_changed {
                if source_idx == 0 && !is_primitive {
                    // Switch to primitive.
                    if let Some(mut mc) = scene.get_component_mut::<MeshRendererComponent>(entity) {
                        mc.mesh_source = MeshSource::Primitive(MeshPrimitive::Cube);
                    }
                    scene.invalidate_mesh(entity);
                    *scene_dirty = true;
                } else if source_idx == 1 && is_primitive {
                    // Switch to asset (no asset selected yet).
                    if let Some(mut mc) = scene.get_component_mut::<MeshRendererComponent>(entity) {
                        mc.mesh_source = MeshSource::Asset(Uuid::from_raw(0));
                    }
                    scene.invalidate_mesh(entity);
                    *scene_dirty = true;
                }
            }

            // Re-read current state after possible change.
            let current_source = scene
                .get_component::<MeshRendererComponent>(entity)
                .unwrap()
                .mesh_source
                .clone();

            let mut primitive = match &current_source {
                MeshSource::Primitive(p) => *p,
                _ => MeshPrimitive::Cube,
            };

            match &current_source {
                MeshSource::Primitive(_) => {
                    // Primitive selector.
                    let prim_labels = [
                        "Cube", "Sphere", "Plane", "Cylinder", "Cone", "Torus", "Capsule",
                    ];
                    let prim_variants = [
                        MeshPrimitive::Cube,
                        MeshPrimitive::Sphere,
                        MeshPrimitive::Plane,
                        MeshPrimitive::Cylinder,
                        MeshPrimitive::Cone,
                        MeshPrimitive::Torus,
                        MeshPrimitive::Capsule,
                    ];
                    let current_idx = prim_variants
                        .iter()
                        .position(|&v| v == primitive)
                        .unwrap_or(0);
                    egui::ComboBox::from_label("Primitive")
                        .selected_text(prim_labels[current_idx])
                        .show_ui(ui, |ui| {
                            for (i, (&variant, &label)) in
                                prim_variants.iter().zip(&prim_labels).enumerate()
                            {
                                let _ = i; // suppress unused warning
                                if ui
                                    .selectable_value(&mut primitive, variant, label)
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });
                }
                MeshSource::Asset(mesh_handle) => {
                    // Mesh asset picker.
                    ui.horizontal(|ui| {
                        ui.label("Mesh File");
                        match super::asset_handle_picker(
                            ui,
                            mesh_handle.raw(),
                            asset_manager,
                            assets_root,
                            "meshes",
                            "glTF files",
                            &["gltf", "glb"],
                        ) {
                            super::AssetPickerAction::Selected(handle) => {
                                if let Some(mut mc) =
                                    scene.get_component_mut::<MeshRendererComponent>(entity)
                                {
                                    mc.mesh_source = MeshSource::Asset(handle);
                                }
                                scene.invalidate_mesh(entity);
                                *scene_dirty = true;
                            }
                            super::AssetPickerAction::Cleared => {
                                if let Some(mut mc) =
                                    scene.get_component_mut::<MeshRendererComponent>(entity)
                                {
                                    mc.mesh_source = MeshSource::Asset(Uuid::from_raw(0));
                                }
                                scene.invalidate_mesh(entity);
                                *scene_dirty = true;
                            }
                            super::AssetPickerAction::None => {}
                        }
                    });

                    // Show mesh info if loaded.
                    let mc = scene
                        .get_component::<MeshRendererComponent>(entity)
                        .unwrap();
                    if let Some(ref mesh) = mc.loaded_mesh {
                        ui.label(format!(
                            "{}: {} verts, {} tris",
                            mesh.name,
                            mesh.vertices.len(),
                            mesh.indices.len() / 3
                        ));
                    } else if mc.mesh_asset_handle().is_some_and(|h| h.raw() != 0) {
                        ui.label("Loading...");
                    }
                }
            }

            // Color picker.
            if super::color_picker_rgba(ui, "Color", &mut color_arr) {
                changed = true;
            }

            // Albedo texture picker.
            ui.horizontal(|ui| {
                ui.label("Albedo Texture");
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
                        if let Some(mut mc) =
                            scene.get_component_mut::<MeshRendererComponent>(entity)
                        {
                            mc.texture_handle = handle;
                            mc.texture = None;
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::Cleared => {
                        if let Some(mut mc) =
                            scene.get_component_mut::<MeshRendererComponent>(entity)
                        {
                            mc.texture_handle = Uuid::from_raw(0);
                            mc.texture = None;
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::None => {}
                }
            });

            // Normal map texture picker.
            ui.horizontal(|ui| {
                ui.label("Normal Map");
                match super::asset_handle_picker(
                    ui,
                    normal_texture_handle_raw,
                    asset_manager,
                    assets_root,
                    "textures",
                    "Image files",
                    &["png", "jpg", "jpeg"],
                ) {
                    super::AssetPickerAction::Selected(handle) => {
                        if let Some(mut mc) =
                            scene.get_component_mut::<MeshRendererComponent>(entity)
                        {
                            mc.normal_texture_handle = handle;
                            mc.normal_texture = None;
                        }
                        scene.invalidate_texture_cache();
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::Cleared => {
                        if let Some(mut mc) =
                            scene.get_component_mut::<MeshRendererComponent>(entity)
                        {
                            mc.normal_texture_handle = Uuid::from_raw(0);
                            mc.normal_texture = None;
                        }
                        scene.invalidate_texture_cache();
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::None => {}
                }
            });

            ui.separator();

            // Material properties.
            if ui
                .add(egui::Slider::new(&mut metallic, 0.0..=1.0).text("Metallic"))
                .changed()
            {
                changed = true;
            }
            if ui
                .add(egui::Slider::new(&mut roughness, 0.0..=1.0).text("Roughness"))
                .changed()
            {
                changed = true;
            }

            // Emissive.
            ui.horizontal(|ui| {
                ui.label("Emissive Color");
                if ui.color_edit_button_rgb(&mut emissive_arr).changed() {
                    changed = true;
                }
            });
            if ui
                .add(
                    egui::Slider::new(&mut emissive_strength, 0.0..=10.0).text("Emissive Strength"),
                )
                .changed()
            {
                changed = true;
            }

            // Alpha shadow toggle.
            {
                let mut alpha_shadow = scene
                    .get_component::<MeshRendererComponent>(entity)
                    .unwrap()
                    .cast_alpha_shadow;
                if ui
                    .checkbox(&mut alpha_shadow, "Cast Alpha Shadow")
                    .on_hover_text(
                        "Use the alpha-tested shadow pipeline so transparent \
                         textures (foliage, fences) cast shaped shadows.",
                    )
                    .changed()
                {
                    if let Some(mut mc) = scene.get_component_mut::<MeshRendererComponent>(entity) {
                        mc.cast_alpha_shadow = alpha_shadow;
                    }
                    *scene_dirty = true;
                }
            }

            if changed {
                let needs_reupload = {
                    let mc = scene
                        .get_component::<MeshRendererComponent>(entity)
                        .unwrap();
                    let prim_changed = match &mc.mesh_source {
                        MeshSource::Primitive(p) => *p != primitive,
                        _ => false,
                    };
                    prim_changed || <[f32; 4]>::from(mc.color) != color_arr
                };
                if let Some(mut mc) = scene.get_component_mut::<MeshRendererComponent>(entity) {
                    if let MeshSource::Primitive(_) = &mc.mesh_source {
                        mc.mesh_source = MeshSource::Primitive(primitive);
                    }
                    mc.color = Vec4::from(color_arr);
                    mc.metallic = metallic;
                    mc.roughness = roughness;
                    mc.emissive_color = Vec3::from(emissive_arr);
                    mc.emissive_strength = emissive_strength;
                }
                if needs_reupload {
                    scene.invalidate_mesh(entity);
                }
                *scene_dirty = true;
            }
        },
    )
}
