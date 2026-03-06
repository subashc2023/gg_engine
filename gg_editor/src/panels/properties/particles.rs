use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_particle_emitter_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    let mut remove = false;

    if scene.has_component::<ParticleEmitterComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Particle Emitter")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("particle_emitter", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (
                mut playing, mut emit_rate, mut max_particles,
                mut velocity, mut velocity_variation,
                mut color_begin, mut color_end,
                mut size_begin, mut size_end, mut size_variation,
                mut lifetime,
            ) = {
                let pe = scene.get_component::<ParticleEmitterComponent>(entity).unwrap();
                (
                    pe.playing, pe.emit_rate, pe.max_particles,
                    [pe.velocity.x, pe.velocity.y],
                    [pe.velocity_variation.x, pe.velocity_variation.y],
                    [pe.color_begin.x, pe.color_begin.y, pe.color_begin.z, pe.color_begin.w],
                    [pe.color_end.x, pe.color_end.y, pe.color_end.z, pe.color_end.w],
                    pe.size_begin, pe.size_end, pe.size_variation,
                    pe.lifetime,
                )
            };

            // Playing toggle.
            if ui.checkbox(&mut playing, "Playing").changed() {
                if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                    pe.playing = playing;
                }
                *scene_dirty = true;
            }

            // Emit rate.
            ui.horizontal(|ui| {
                ui.label("Emit Rate");
                if ui.add(egui::DragValue::new(&mut emit_rate).speed(1).range(0..=1000u32)).changed() {
                    if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                        pe.emit_rate = emit_rate;
                    }
                    *scene_dirty = true;
                }
            });

            // Max particles.
            ui.horizontal(|ui| {
                ui.label("Max Particles");
                if ui.add(egui::DragValue::new(&mut max_particles).speed(1000).range(1000..=1_000_000u32)).changed() {
                    if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                        pe.max_particles = max_particles;
                    }
                    *scene_dirty = true;
                }
            });

            ui.separator();
            ui.strong("Velocity");
            let mut vel_changed = false;
            ui.horizontal(|ui| {
                vel_changed |= ui.add(egui::DragValue::new(&mut velocity[0]).speed(0.1).prefix("X: ")).changed();
                vel_changed |= ui.add(egui::DragValue::new(&mut velocity[1]).speed(0.1).prefix("Y: ")).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Variation");
                vel_changed |= ui.add(egui::DragValue::new(&mut velocity_variation[0]).speed(0.1).prefix("X: ")).changed();
                vel_changed |= ui.add(egui::DragValue::new(&mut velocity_variation[1]).speed(0.1).prefix("Y: ")).changed();
            });
            if vel_changed {
                if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                    pe.velocity = Vec2::from(velocity);
                    pe.velocity_variation = Vec2::from(velocity_variation);
                }
                *scene_dirty = true;
            }

            ui.separator();
            ui.strong("Color");
            let mut color_changed = false;
            ui.horizontal(|ui| {
                ui.label("Begin");
                let mut c = egui::Color32::from_rgba_unmultiplied(
                    (color_begin[0] * 255.0) as u8, (color_begin[1] * 255.0) as u8,
                    (color_begin[2] * 255.0) as u8, (color_begin[3] * 255.0) as u8,
                );
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    color_begin = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0];
                    color_changed = true;
                }
                ui.label("End");
                let mut c2 = egui::Color32::from_rgba_unmultiplied(
                    (color_end[0] * 255.0) as u8, (color_end[1] * 255.0) as u8,
                    (color_end[2] * 255.0) as u8, (color_end[3] * 255.0) as u8,
                );
                if egui::color_picker::color_edit_button_srgba(ui, &mut c2, egui::color_picker::Alpha::OnlyBlend).changed() {
                    let [r, g, b, a] = c2.to_srgba_unmultiplied();
                    color_end = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0];
                    color_changed = true;
                }
            });
            if color_changed {
                if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                    pe.color_begin = Vec4::from(color_begin);
                    pe.color_end = Vec4::from(color_end);
                }
                *scene_dirty = true;
            }

            ui.separator();
            ui.strong("Size");
            let mut size_changed = false;
            ui.horizontal(|ui| {
                ui.label("Begin");
                size_changed |= ui.add(egui::DragValue::new(&mut size_begin).speed(0.01).range(0.01..=1.0)).changed();
                ui.label("End");
                size_changed |= ui.add(egui::DragValue::new(&mut size_end).speed(0.01).range(0.0..=1.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Variation");
                size_changed |= ui.add(egui::DragValue::new(&mut size_variation).speed(0.01).range(0.0..=0.5)).changed();
            });
            if size_changed {
                if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                    pe.size_begin = size_begin;
                    pe.size_end = size_end;
                    pe.size_variation = size_variation;
                }
                *scene_dirty = true;
            }

            // Lifetime.
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Lifetime");
                if ui.add(egui::DragValue::new(&mut lifetime).speed(0.1).range(0.1..=30.0).suffix(" s")).changed() {
                    if let Some(mut pe) = scene.get_component_mut::<ParticleEmitterComponent>(entity) {
                        pe.lifetime = lifetime;
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
