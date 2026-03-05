use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::EditorTheme;

const GRID_SIZE_OPTIONS: &[f32] = &[0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_ui(
    ui: &mut egui::Ui,
    scene: &Scene,
    frame_time_ms: f32,
    render_stats: Renderer2DStats,
    vsync: &mut bool,
    show_physics_colliders: &mut bool,
    hovered_entity: i32,
    show_grid: &mut bool,
    snap_to_grid: &mut bool,
    grid_size: &mut f32,
    scene_warnings: &[String],
    theme: &mut EditorTheme,
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
    ui.label(format!("Draw calls: {}", render_stats.draw_calls));
    ui.label(format!("Quads: {}", render_stats.quad_count));
    ui.label(format!(
        "Vertices: {}",
        render_stats.total_vertex_count()
    ));
    ui.label(format!(
        "Indices: {}",
        render_stats.total_index_count()
    ));

    ui.add_space(8.0);
    ui.checkbox(vsync, "VSync");

    ui.add_space(4.0);
    egui::ComboBox::from_label("Theme")
        .selected_text(theme.label())
        .show_ui(ui, |ui| {
            for &t in EditorTheme::ALL {
                if ui.selectable_value(theme, t, t.label()).changed() {
                    gg_engine::ui_theme::apply_theme(ui.ctx(), *theme);
                }
            }
        });

    ui.add_space(8.0);
    ui.heading("Debug");
    ui.separator();

    ui.checkbox(show_physics_colliders, "Show Physics Colliders");

    ui.add_space(8.0);
    ui.heading("Grid");
    ui.separator();

    ui.checkbox(show_grid, "Show Grid");
    ui.checkbox(snap_to_grid, "Snap to Grid");

    egui::ComboBox::from_label("Grid Size")
        .selected_text(format!("{}", grid_size))
        .show_ui(ui, |ui| {
            for &size in GRID_SIZE_OPTIONS {
                ui.selectable_value(grid_size, size, format!("{}", size));
            }
        });

    ui.add_space(8.0);
    ui.heading("Scene");
    ui.separator();

    let entity_count = scene.each_entity_with_tag().len();
    ui.label(format!("Entities: {}", entity_count));

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

    if !scene_warnings.is_empty() {
        ui.add_space(8.0);
        ui.heading("Warnings");
        ui.separator();

        for warning in scene_warnings {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("!")
                        .color(egui::Color32::from_rgb(0xFF, 0xCC, 0x00))
                        .strong(),
                );
                ui.label(warning);
            });
        }
    }
}
