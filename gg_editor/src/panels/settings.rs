use gg_engine::egui;
use gg_engine::prelude::*;

pub(crate) fn settings_ui(
    ui: &mut egui::Ui,
    scene: &Scene,
    frame_time_ms: f32,
    vsync: &mut bool,
    hovered_entity: i32,
) {
    ui.heading("Renderer");
    ui.separator();

    let fps = if frame_time_ms > 0.0 {
        1000.0 / frame_time_ms
    } else {
        0.0
    };
    ui.label(format!("Frame time: {:.2} ms", frame_time_ms));
    ui.label(format!("FPS: {:.0}", fps));

    ui.add_space(8.0);
    ui.checkbox(vsync, "VSync");

    ui.add_space(8.0);
    ui.heading("Mouse Picking");
    ui.separator();
    let hovered_name = if hovered_entity >= 0 {
        scene
            .find_entity_by_id(hovered_entity as u32)
            .and_then(|e| {
                scene
                    .get_component::<TagComponent>(e)
                    .map(|tag| tag.tag.clone())
            })
            .unwrap_or_else(|| format!("Entity({})", hovered_entity))
    } else {
        "None".to_string()
    };
    ui.label(format!("Hovered Entity: {}", hovered_name));
}
