use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_directional_light_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<DirectionalLightComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Directional Light",
        "directional_light",
        bold_family,
        entity,
        |ui| {
            let (mut color_arr, mut intensity) = {
                let dl = scene
                    .get_component::<DirectionalLightComponent>(entity)
                    .unwrap();
                (<[f32; 3]>::from(dl.color), dl.intensity)
            };

            // Show computed direction (read-only, derived from entity rotation).
            let world = scene.get_world_transform(entity);
            let (_, world_rot, _) = world.to_scale_rotation_translation();
            let dir = DirectionalLightComponent::direction(world_rot);
            ui.horizontal(|ui| {
                ui.label("Direction");
                ui.label(format!("({:.2}, {:.2}, {:.2})", dir.x, dir.y, dir.z));
            });
            ui.label(
                egui::RichText::new("Rotate the entity to aim the light.")
                    .weak()
                    .small(),
            );

            let mut changed = false;

            if ui.color_edit_button_rgb(&mut color_arr).changed() {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut intensity)
                        .speed(0.01)
                        .range(0.0..=100.0)
                        .prefix("Intensity: "),
                )
                .changed()
            {
                changed = true;
            }

            if changed {
                if let Some(mut dl) = scene.get_component_mut::<DirectionalLightComponent>(entity) {
                    dl.color = Vec3::from(color_arr);
                    dl.intensity = intensity;
                }
                *scene_dirty = true;
            }
        },
    )
}

pub(crate) fn draw_point_light_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<PointLightComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Point Light",
        "point_light",
        bold_family,
        entity,
        |ui| {
            let (mut color_arr, mut intensity, mut radius) = {
                let pl = scene.get_component::<PointLightComponent>(entity).unwrap();
                (<[f32; 3]>::from(pl.color), pl.intensity, pl.radius)
            };

            let mut changed = false;

            if ui.color_edit_button_rgb(&mut color_arr).changed() {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut intensity)
                        .speed(0.01)
                        .range(0.0..=100.0)
                        .prefix("Intensity: "),
                )
                .changed()
            {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut radius)
                        .speed(0.1)
                        .range(0.01..=1000.0)
                        .prefix("Radius: "),
                )
                .changed()
            {
                changed = true;
            }

            if changed {
                if let Some(mut pl) = scene.get_component_mut::<PointLightComponent>(entity) {
                    pl.color = Vec3::from(color_arr);
                    pl.intensity = intensity;
                    pl.radius = radius;
                }
                *scene_dirty = true;
            }
        },
    )
}

pub(crate) fn draw_ambient_light_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<AmbientLightComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "Ambient Light",
        "ambient_light",
        bold_family,
        entity,
        |ui| {
            let (mut color_arr, mut intensity) = {
                let al = scene
                    .get_component::<AmbientLightComponent>(entity)
                    .unwrap();
                (<[f32; 3]>::from(al.color), al.intensity)
            };

            let mut changed = false;

            if ui.color_edit_button_rgb(&mut color_arr).changed() {
                changed = true;
            }

            if ui
                .add(
                    egui::DragValue::new(&mut intensity)
                        .speed(0.01)
                        .range(0.0..=10.0)
                        .prefix("Intensity: "),
                )
                .changed()
            {
                changed = true;
            }

            if changed {
                if let Some(mut al) = scene.get_component_mut::<AmbientLightComponent>(entity) {
                    al.color = Vec3::from(color_arr);
                    al.intensity = intensity;
                }
                *scene_dirty = true;
            }
        },
    )
}
