use gg_engine::egui;
use gg_engine::prelude::*;

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
    if !scene.has_component::<AudioSourceComponent>(entity) {
        return false;
    }
    super::component_header(ui, "Audio Source", "audio_source", bold_family, entity, |ui| {
        let (audio_handle_raw, mut volume, mut pitch, mut looping, mut play_on_start,
             mut streaming, mut spatial, mut min_distance, mut max_distance) = {
            let ac = scene.get_component::<AudioSourceComponent>(entity).unwrap();
            (ac.audio_handle.raw(), ac.volume, ac.pitch, ac.looping, ac.play_on_start,
             ac.streaming, ac.spatial, ac.min_distance, ac.max_distance)
        };

        ui.horizontal(|ui| {
            ui.label("Audio File");
            match super::asset_handle_picker(
                ui,
                audio_handle_raw,
                asset_manager,
                assets_root,
                "audio",
                "Audio files",
                &["wav", "ogg", "mp3", "flac"],
            ) {
                super::AssetPickerAction::Selected(handle) => {
                    if let Some(mut ac) =
                        scene.get_component_mut::<AudioSourceComponent>(entity)
                    {
                        ac.audio_handle = handle;
                    }
                    *scene_dirty = true;
                }
                super::AssetPickerAction::Cleared => {
                    if let Some(mut ac) =
                        scene.get_component_mut::<AudioSourceComponent>(entity)
                    {
                        ac.audio_handle = Uuid::from_raw(0);
                    }
                    *scene_dirty = true;
                }
                super::AssetPickerAction::None => {}
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
    })
}

pub(crate) fn draw_audio_listener_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<AudioListenerComponent>(entity) {
        return false;
    }
    super::component_header(ui, "Audio Listener", "audio_listener", bold_family, entity, |ui| {
        let mut active = scene
            .get_component::<AudioListenerComponent>(entity)
            .map(|al| al.active)
            .unwrap_or(true);

        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut active, "Active")
                .on_hover_text(
                    "When active, this entity's position is used as the\n\
                 spatial audio listener instead of the primary camera.",
                )
                .changed()
            {
                if let Some(mut al) = scene.get_component_mut::<AudioListenerComponent>(entity)
                {
                    al.active = active;
                }
                *scene_dirty = true;
            }
        });
    })
}
