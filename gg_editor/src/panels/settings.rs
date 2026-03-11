use gg_engine::egui;
use gg_engine::prelude::*;
use gg_engine::ui_theme::EditorTheme;

use crate::{GpuTimingSnapshot, PostProcessSettings};

const GRID_SIZE_OPTIONS: &[f32] = &[0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];

#[allow(clippy::too_many_arguments)]
pub(crate) fn settings_ui(
    ui: &mut egui::Ui,
    scene: &Scene,
    frame_time_ms: f32,
    render_stats: Renderer2DStats,
    vsync: &mut bool,
    _hovered_entity: i32,
    show_grid: &mut bool,
    show_xz_grid: &mut bool,
    snap_to_grid: &mut bool,
    grid_size: &mut f32,
    scene_warnings: &[String],
    theme: &mut EditorTheme,
    reload_shaders_requested: &mut bool,
    msaa_samples: &mut MsaaSamples,
    max_msaa_samples: MsaaSamples,
    msaa_changed: &mut bool,
    show_physics_colliders: &mut bool,
    pp_settings: &mut PostProcessSettings,
    gpu_timing: &mut GpuTimingSnapshot,
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
    ui.label(format!("Vertices: {}", render_stats.total_vertex_count()));
    ui.label(format!("Indices: {}", render_stats.total_index_count()));

    let cull = scene.culling_stats();
    if cull.total_cullable > 0 {
        ui.label(format!(
            "Frustum culling: {}/{} rendered ({} culled)",
            cull.rendered, cull.total_cullable, cull.culled
        ));
    }

    let entity_count = scene.each_entity_with_tag().len();
    ui.label(format!("Scene Entities: {}", entity_count));

    ui.add_space(8.0);
    ui.checkbox(vsync, "VSync");

    ui.add_space(4.0);
    let available = MsaaSamples::available_up_to(max_msaa_samples.to_vk());
    egui::ComboBox::from_label("MSAA")
        .selected_text(format!("{}", msaa_samples))
        .show_ui(ui, |ui| {
            for &s in &available {
                if ui
                    .selectable_value(msaa_samples, s, format!("{s}"))
                    .changed()
                {
                    *msaa_changed = true;
                }
            }
        });

    ui.add_space(4.0);
    if ui
        .button("Reload Shaders")
        .on_hover_text(
            "Recompile .glsl sources and rebuild all pipelines.\nRequires glslc on PATH.",
        )
        .clicked()
    {
        *reload_shaders_requested = true;
    }

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
    ui.heading("Grid");
    ui.separator();

    ui.checkbox(show_grid, "X-Y Grid");
    ui.checkbox(show_xz_grid, "X-Z Grid");
    ui.checkbox(snap_to_grid, "Snap to Grid");

    ui.add_space(8.0);
    ui.heading("Physics");
    ui.separator();

    ui.checkbox(show_physics_colliders, "Show Colliders")
        .on_hover_text("Visualize 2D physics colliders and velocity arrows in the viewport.");

    egui::ComboBox::from_label("Grid Size")
        .selected_text(format!("{}", grid_size))
        .show_ui(ui, |ui| {
            for &size in GRID_SIZE_OPTIONS {
                ui.selectable_value(grid_size, size, format!("{}", size));
            }
        });

    // Mouse picking debug (kept for future use)
    // ui.add_space(8.0);
    // ui.heading("Mouse Picking");
    // ui.separator();
    // let hovered_name = if _hovered_entity >= 0 {
    //     scene
    //         .find_entity_by_id(_hovered_entity as u32)
    //         .and_then(|e| {
    //             scene
    //                 .get_component::<TagComponent>(e)
    //                 .map(|tag| tag.tag.clone())
    //         })
    //         .unwrap_or_else(|| format!("Entity({})", _hovered_entity))
    // } else {
    //     "None".to_string()
    // };
    // ui.label(format!("Hovered Entity: {}", hovered_name));

    ui.add_space(8.0);
    ui.heading("Profiling");
    ui.separator();

    // GPU timestamp profiling.
    ui.checkbox(&mut gpu_timing.enabled, "GPU Timestamps");
    if gpu_timing.enabled && gpu_timing.total_frame_ms > 0.0 {
        ui.label(format!("GPU frame: {:.2} ms", gpu_timing.total_frame_ms));
        for (name, time_ms) in &gpu_timing.results {
            ui.label(format!("  {}: {:.3} ms", name, time_ms));
        }
    }

    ui.add_space(4.0);

    // On-demand Chrome Tracing capture for gg_tools analysis.
    let recording = gg_engine::profiling::is_session_active();
    let label = if recording {
        "Stop Capture"
    } else {
        "Capture Trace"
    };
    if ui.button(label).clicked() {
        if recording {
            gg_engine::profiling::end_session();
        } else {
            gg_engine::profiling::begin_session("Runtime", "gg_profile_runtime.json");
        }
    }
    if recording {
        ui.label(
            egui::RichText::new("Recording...")
                .color(egui::Color32::from_rgb(0xFF, 0x44, 0x44))
                .strong(),
        );
    }

    // -- Post-Processing --
    ui.add_space(8.0);
    ui.heading("Post-Processing");
    ui.separator();

    let pp = pp_settings;
    ui.checkbox(&mut pp.enabled, "Enable");
    if pp.enabled {
        ui.add_space(4.0);
        ui.checkbox(&mut pp.bloom_enabled, "Bloom");
        if pp.bloom_enabled {
            ui.add(egui::Slider::new(&mut pp.bloom_threshold, 0.0..=3.0).text("Threshold"));
            ui.add(egui::Slider::new(&mut pp.bloom_intensity, 0.0..=2.0).text("Intensity"));
            ui.add(egui::Slider::new(&mut pp.bloom_filter_radius, 0.1..=5.0).text("Radius"));
        }

        ui.add_space(4.0);
        egui::ComboBox::from_label("Tonemapping")
            .selected_text(format!("{}", pp.tonemapping))
            .show_ui(ui, |ui| {
                for &mode in TonemappingMode::ALL {
                    ui.selectable_value(&mut pp.tonemapping, mode, format!("{mode}"));
                }
            });

        ui.add(egui::Slider::new(&mut pp.exposure, -5.0..=5.0).text("Exposure"));
        ui.add(egui::Slider::new(&mut pp.contrast, 0.0..=3.0).text("Contrast"));
        ui.add(egui::Slider::new(&mut pp.saturation, 0.0..=3.0).text("Saturation"));

        ui.add_space(4.0);
        ui.checkbox(&mut pp.contact_shadows_enabled, "Contact Shadows");
        if pp.contact_shadows_enabled {
            ui.add(egui::Slider::new(&mut pp.contact_shadows_max_distance, 0.01..=3.0).text("Max Distance"));
            ui.add(egui::Slider::new(&mut pp.contact_shadows_thickness, 0.01..=1.0).text("Thickness"));
            ui.add(egui::Slider::new(&mut pp.contact_shadows_intensity, 0.0..=1.0).text("Intensity"));
            let mut steps = pp.contact_shadows_step_count;
            ui.add(egui::Slider::new(&mut steps, 4..=64).text("Steps"));
            pp.contact_shadows_step_count = steps;
            let debug_labels = ["Off", "Linear Depth", "Raw (no fade)", "Precision ULPs"];
            let mut debug_idx = (pp.contact_shadows_debug as usize).min(debug_labels.len() - 1);
            egui::ComboBox::from_label("Debug")
                .selected_text(debug_labels[debug_idx])
                .show_ui(ui, |ui| {
                    for (i, label) in debug_labels.iter().enumerate() {
                        ui.selectable_value(&mut debug_idx, i, *label);
                    }
                });
            pp.contact_shadows_debug = debug_idx as i32;
        }
    }

    // -- Shadow Debug --
    ui.add_space(8.0);
    ui.heading("Shadow Debug");
    ui.separator();
    {
        let debug_labels = [
            "Off",
            "Cascade Index",
            "Cascade 0 Shadow",
            "Cascade 1 Shadow",
            "Cascade 0 Coverage",
            "Cascade 1 Coverage",
            "Final Shadow",
            "Cascade Difference",
        ];
        let mut debug_idx = (pp.shadow_debug_mode as usize).min(debug_labels.len() - 1);
        egui::ComboBox::from_label("CSM Debug")
            .selected_text(debug_labels[debug_idx])
            .show_ui(ui, |ui| {
                for (i, label) in debug_labels.iter().enumerate() {
                    ui.selectable_value(&mut debug_idx, i, *label);
                }
            });
        pp.shadow_debug_mode = debug_idx as i32;
    }

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
