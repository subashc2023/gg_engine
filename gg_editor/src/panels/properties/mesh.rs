use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn draw_mesh_renderer_component(
    ui: &mut egui::Ui,
    scene: &mut Scene,
    entity: Entity,
    bold_family: &egui::FontFamily,
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
            let (mut primitive, mut color_arr) = {
                let mc = scene
                    .get_component::<MeshRendererComponent>(entity)
                    .unwrap();
                (mc.primitive, <[f32; 4]>::from(mc.color))
            };

            let mut changed = false;

            // Primitive selector.
            let prim_labels = ["Cube", "Sphere", "Plane"];
            let current_label = match primitive {
                MeshPrimitive::Cube => prim_labels[0],
                MeshPrimitive::Sphere => prim_labels[1],
                MeshPrimitive::Plane => prim_labels[2],
            };
            egui::ComboBox::from_label("Primitive")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut primitive, MeshPrimitive::Cube, prim_labels[0])
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut primitive, MeshPrimitive::Sphere, prim_labels[1])
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(&mut primitive, MeshPrimitive::Plane, prim_labels[2])
                        .changed()
                    {
                        changed = true;
                    }
                });

            // Color picker.
            if super::color_picker_rgba(ui, "Color", &mut color_arr) {
                changed = true;
            }

            if changed {
                let needs_reupload = {
                    let mc = scene
                        .get_component::<MeshRendererComponent>(entity)
                        .unwrap();
                    mc.primitive != primitive || <[f32; 4]>::from(mc.color) != color_arr
                };
                if let Some(mut mc) = scene.get_component_mut::<MeshRendererComponent>(entity) {
                    mc.primitive = primitive;
                    mc.color = Vec4::from(color_arr);
                }
                if needs_reupload {
                    scene.invalidate_mesh(entity);
                }
                *scene_dirty = true;
            }
        },
    )
}
