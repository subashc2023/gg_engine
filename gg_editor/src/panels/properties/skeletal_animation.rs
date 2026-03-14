use gg_engine::egui;
use gg_engine::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_skeletal_animation_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    asset_manager: &mut Option<EditorAssetManager>,
    assets_root: &std::path::Path,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<SkeletalAnimationComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Skeletal Animation",
        "skeletal_animation",
        bold_family,
        entity,
        |ui| {
            let (
                mesh_asset_raw,
                is_loaded,
                clip_names,
                current_clip_idx,
                mut speed,
                mut looping,
                mut playing,
                joint_count,
            ) = {
                let sac = scene
                    .get_component::<SkeletalAnimationComponent>(entity)
                    .unwrap();
                (
                    sac.mesh_asset.raw(),
                    sac.is_loaded(),
                    sac.clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
                    sac.current_clip,
                    sac.speed,
                    sac.looping,
                    sac.playing,
                    sac.skeleton.joint_count(),
                )
            };

            // Mesh asset picker.
            ui.horizontal(|ui| {
                ui.label("Mesh File");
                match super::asset_handle_picker(
                    ui,
                    mesh_asset_raw,
                    asset_manager,
                    assets_root,
                    "meshes",
                    "glTF files",
                    &["gltf", "glb"],
                ) {
                    super::AssetPickerAction::Selected(handle) => {
                        if let Some(mut sac) =
                            scene.get_component_mut::<SkeletalAnimationComponent>(entity)
                        {
                            // Reset to stub when asset changes.
                            *sac = SkeletalAnimationComponent::from_asset(handle);
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::Cleared => {
                        if let Some(mut sac) =
                            scene.get_component_mut::<SkeletalAnimationComponent>(entity)
                        {
                            *sac = SkeletalAnimationComponent::from_asset(Uuid::from_raw(0));
                        }
                        *scene_dirty = true;
                    }
                    super::AssetPickerAction::None => {}
                }
            });

            if !is_loaded {
                if mesh_asset_raw != 0 {
                    ui.label("Loading...");
                } else {
                    ui.label("No mesh asset assigned.");
                }
                return;
            }

            // Info.
            ui.label(format!(
                "{} joints, {} clips",
                joint_count,
                clip_names.len()
            ));

            // Clip selector.
            if !clip_names.is_empty() {
                let current_label = current_clip_idx
                    .and_then(|i| clip_names.get(i))
                    .map(|s| s.as_str())
                    .unwrap_or("None");
                let mut new_idx = current_clip_idx;
                egui::ComboBox::from_label("Clip")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for (i, name) in clip_names.iter().enumerate() {
                            if ui
                                .selectable_value(&mut new_idx, Some(i), name)
                                .changed()
                            {
                            }
                        }
                    });
                if new_idx != current_clip_idx {
                    if let Some(idx) = new_idx {
                        if let Some(mut sac) =
                            scene.get_component_mut::<SkeletalAnimationComponent>(entity)
                        {
                            sac.play(idx);
                        }
                    }
                    *scene_dirty = true;
                }
            }

            // Playback controls.
            let mut changed = false;
            if ui
                .add(egui::Slider::new(&mut speed, 0.0..=5.0).text("Speed"))
                .changed()
            {
                changed = true;
            }
            if ui.checkbox(&mut looping, "Looping").changed() {
                changed = true;
            }
            if ui.checkbox(&mut playing, "Playing").changed() {
                changed = true;
            }

            if changed {
                if let Some(mut sac) =
                    scene.get_component_mut::<SkeletalAnimationComponent>(entity)
                {
                    sac.speed = speed;
                    sac.looping = looping;
                    sac.playing = playing;
                }
                *scene_dirty = true;
            }
        },
    )
}
