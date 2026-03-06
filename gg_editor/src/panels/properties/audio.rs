use gg_engine::egui;
use gg_engine::prelude::*;

use crate::panels::content_browser::ContentBrowserPayload;

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_audio_source_component(
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

    if scene.has_component::<AudioSourceComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Audio Source")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("audio_source", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (audio_handle_raw, mut volume, mut pitch, mut looping, mut play_on_start,
                 mut streaming, mut spatial, mut min_distance, mut max_distance) = {
                let ac = scene.get_component::<AudioSourceComponent>(entity).unwrap();
                (ac.audio_handle.raw(), ac.volume, ac.pitch, ac.looping, ac.play_on_start,
                 ac.streaming, ac.spatial, ac.min_distance, ac.max_distance)
            };

            // Audio file label.
            let audio_label = if audio_handle_raw != 0 {
                if let Some(am) = asset_manager.as_ref() {
                    let handle = Uuid::from_raw(audio_handle_raw);
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

            let btn_width = ((audio_label.len() as f32) * 7.0 + 20.0).max(100.0);
            ui.horizontal(|ui| {
                ui.label("Audio File");
                let btn_resp = ui.add_sized(
                    [btn_width, 0.0],
                    egui::Button::new(&audio_label),
                );

                if btn_resp.clicked() {
                    if let Some(am) = asset_manager.as_mut() {
                        let audio_dir = assets_root.join("audio");
                        let audio_dir_str = audio_dir.to_string_lossy();
                        if let Some(path_str) =
                            FileDialogs::open_file_in("Audio files", &["wav", "ogg", "mp3", "flac"], &audio_dir_str)
                        {
                            let abs_path = std::path::PathBuf::from(&path_str);
                            let rel_path = abs_path
                                .strip_prefix(am.asset_directory())
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or(path_str);
                            let handle = am.import_asset(&rel_path);
                            if let Some(mut ac) =
                                scene.get_component_mut::<AudioSourceComponent>(entity)
                            {
                                ac.audio_handle = handle;
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
                        if matches!(ext.as_str(), "wav" | "ogg" | "mp3" | "flac") {
                            if let Some(am) = asset_manager.as_mut() {
                                let rel_path = payload.path
                                    .strip_prefix(am.asset_directory())
                                    .map(|p| p.to_string_lossy().to_string())
                                    .unwrap_or_else(|_| payload.path.to_string_lossy().to_string());
                                let handle = am.import_asset(&rel_path);
                                if let Some(mut ac) =
                                    scene.get_component_mut::<AudioSourceComponent>(entity)
                                {
                                    ac.audio_handle = handle;
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

                if audio_handle_raw != 0
                    && ui.small_button("X").clicked()
                {
                    if let Some(mut ac) =
                        scene.get_component_mut::<AudioSourceComponent>(entity)
                    {
                        ac.audio_handle = Uuid::from_raw(0);
                    }
                    *scene_dirty = true;
                }
            });

            // Volume slider.
            ui.horizontal(|ui| {
                ui.label("Volume");
                if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0)).changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.volume = volume;
                    }
                    *scene_dirty = true;
                }
            });

            // Pitch drag.
            ui.horizontal(|ui| {
                ui.label("Pitch");
                if ui.add(egui::DragValue::new(&mut pitch).range(0.1..=4.0).speed(0.01)).changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.pitch = pitch;
                    }
                    *scene_dirty = true;
                }
            });

            // Looping checkbox.
            ui.horizontal(|ui| {
                if ui.checkbox(&mut looping, "Looping").changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.looping = looping;
                    }
                    *scene_dirty = true;
                }
            });

            // Play on start checkbox.
            ui.horizontal(|ui| {
                if ui.checkbox(&mut play_on_start, "Play On Start").changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.play_on_start = play_on_start;
                    }
                    *scene_dirty = true;
                }
            });

            // Streaming checkbox.
            ui.horizontal(|ui| {
                if ui.checkbox(&mut streaming, "Streaming").on_hover_text(
                    "Stream from disk instead of loading into memory.\nBetter for long music tracks.",
                ).changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.streaming = streaming;
                    }
                    *scene_dirty = true;
                }
            });

            ui.separator();

            // Spatial audio checkbox.
            ui.horizontal(|ui| {
                if ui.checkbox(&mut spatial, "Spatial Audio").on_hover_text(
                    "Compute panning and distance attenuation\nbased on entity position relative to camera.",
                ).changed() {
                    if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                        ac.spatial = spatial;
                    }
                    *scene_dirty = true;
                }
            });

            if spatial {
                // Min distance.
                ui.horizontal(|ui| {
                    ui.label("Min Distance");
                    if ui.add(egui::DragValue::new(&mut min_distance).range(0.0..=max_distance).speed(0.1)).changed() {
                        if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                            ac.min_distance = min_distance;
                        }
                        *scene_dirty = true;
                    }
                });

                // Max distance.
                ui.horizontal(|ui| {
                    ui.label("Max Distance");
                    if ui.add(egui::DragValue::new(&mut max_distance).range(min_distance..=1000.0).speed(0.5)).changed() {
                        if let Some(mut ac) = scene.get_component_mut::<AudioSourceComponent>(entity) {
                            ac.max_distance = max_distance;
                        }
                        *scene_dirty = true;
                    }
                });
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

pub(crate) fn draw_audio_listener_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<AudioListenerComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Audio Listener")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("audio_listener", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let mut active = scene
                .get_component::<AudioListenerComponent>(entity)
                .map(|al| al.active)
                .unwrap_or(true);

            ui.horizontal(|ui| {
                if ui.checkbox(&mut active, "Active").on_hover_text(
                    "When active, this entity's position is used as the\n\
                     spatial audio listener instead of the primary camera.",
                ).changed() {
                    if let Some(mut al) = scene.get_component_mut::<AudioListenerComponent>(entity) {
                        al.active = active;
                    }
                    *scene_dirty = true;
                }
            });
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
