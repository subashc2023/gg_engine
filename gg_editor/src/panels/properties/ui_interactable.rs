use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_ui_interactable_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
    scene_dirty: &mut bool,
    _undo_system: &mut crate::undo::UndoSystem,
) -> bool {
    if !scene.has_component::<UIInteractableComponent>(entity) {
        return false;
    }
    super::component_header(
        ui,
        "UI Interactable",
        "ui_interactable",
        bold_family,
        entity,
        |ui| {
            let (mut interactable, mut hover_color, mut press_color, mut disabled_color) = {
                let inter = scene
                    .get_component::<UIInteractableComponent>(entity)
                    .unwrap();
                (
                    inter.interactable,
                    inter.hover_color.map(|c| [c.x, c.y, c.z, c.w]),
                    inter.press_color.map(|c| [c.x, c.y, c.z, c.w]),
                    inter.disabled_color.map(|c| [c.x, c.y, c.z, c.w]),
                )
            };

            let mut changed = false;

            if ui.checkbox(&mut interactable, "Interactable").changed() {
                changed = true;
            }

            // Optional color overrides with enable/disable checkboxes.
            {
                let mut has_hover = hover_color.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut has_hover, "Hover Color").changed() {
                        if has_hover {
                            hover_color = Some([0.9, 0.9, 0.9, 1.0]);
                        } else {
                            hover_color = None;
                        }
                        changed = true;
                    }
                    if let Some(ref mut c) = hover_color {
                        if super::color_picker_rgba(ui, "", c) {
                            changed = true;
                        }
                    }
                });
            }

            {
                let mut has_press = press_color.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut has_press, "Press Color").changed() {
                        if has_press {
                            press_color = Some([0.7, 0.7, 0.7, 1.0]);
                        } else {
                            press_color = None;
                        }
                        changed = true;
                    }
                    if let Some(ref mut c) = press_color {
                        if super::color_picker_rgba(ui, "", c) {
                            changed = true;
                        }
                    }
                });
            }

            {
                let mut has_disabled = disabled_color.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut has_disabled, "Disabled Color").changed() {
                        if has_disabled {
                            disabled_color = Some([0.5, 0.5, 0.5, 0.5]);
                        } else {
                            disabled_color = None;
                        }
                        changed = true;
                    }
                    if let Some(ref mut c) = disabled_color {
                        if super::color_picker_rgba(ui, "", c) {
                            changed = true;
                        }
                    }
                });
            }

            if changed {
                if let Some(mut inter) = scene.get_component_mut::<UIInteractableComponent>(entity)
                {
                    inter.interactable = interactable;
                    inter.hover_color = hover_color.map(Vec4::from);
                    inter.press_color = press_color.map(Vec4::from);
                    inter.disabled_color = disabled_color.map(Vec4::from);
                }
                *scene_dirty = true;
            }
        },
    )
}
