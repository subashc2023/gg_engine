use gg_engine::egui;
use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// Bottom panel tab selection
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
enum BottomTab {
    Console,
    AssetBrowser,
}

// ---------------------------------------------------------------------------
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    bottom_tab: BottomTab,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");
        GGEditor {
            bottom_tab: BottomTab::Console,
        }
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEditor".into(),
            width: 1600,
            height: 900,
        }
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        trace!("{event}");
    }

    fn on_egui(&mut self, ctx: &egui::Context) {
        // Panel order matters! Outside-in: top → left → right → bottom → center.
        // Side panels declared before bottom so they span full height.

        // --- Menu bar (top) ---
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New Scene").clicked() {
                        info!("File > New Scene");
                        ui.close();
                    }
                    if ui.button("Open Scene...").clicked() {
                        info!("File > Open Scene");
                        ui.close();
                    }
                    if ui.button("Save Scene").clicked() {
                        info!("File > Save Scene");
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo").clicked() {
                        ui.close();
                    }
                    if ui.button("Redo").clicked() {
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Preferences...").clicked() {
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Reset Layout").clicked() {
                        ui.close();
                    }
                });
            });
        });

        // --- Left panel: Scene Hierarchy (top) + Properties (bottom) ---
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(240.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                let half = (ui.available_height() / 2.0 - ui.spacing().item_spacing.y).max(60.0);

                // Scene Hierarchy
                ui.strong("Scene Hierarchy");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("scene_hierarchy")
                    .max_height(half - 24.0)
                    .show(ui, |ui| {
                        let entities = [
                            "Main Camera",
                            "Directional Light",
                            "Point Light",
                            "Cube",
                            "Sphere",
                            "Ground Plane",
                            "Player",
                            "Enemy Spawner",
                        ];
                        for (i, entity) in entities.iter().enumerate() {
                            let _ = ui.selectable_label(i == 3, format!("  {entity}"));
                        }
                    });

                ui.add_space(4.0);
                ui.separator();

                // Properties
                ui.strong("Properties");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("properties")
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new("Transform")
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("transform_grid").show(ui, |ui| {
                                    ui.label("Position");
                                    ui.label("0.0, 2.0, 0.0");
                                    ui.end_row();
                                    ui.label("Rotation");
                                    ui.label("0.0, 45.0, 0.0");
                                    ui.end_row();
                                    ui.label("Scale");
                                    ui.label("1.0, 1.0, 1.0");
                                    ui.end_row();
                                });
                            });

                        egui::CollapsingHeader::new("Material")
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("material_grid").show(ui, |ui| {
                                    ui.label("Shader");
                                    ui.label("PBR Standard");
                                    ui.end_row();
                                    ui.label("Albedo");
                                    ui.colored_label(
                                        egui::Color32::from_rgb(180, 80, 80),
                                        "■ (0.7, 0.3, 0.3)",
                                    );
                                    ui.end_row();
                                    ui.label("Roughness");
                                    ui.label("0.5");
                                    ui.end_row();
                                    ui.label("Metallic");
                                    ui.label("0.0");
                                    ui.end_row();
                                });
                            });
                    });
            });

        // --- Right panel: Inspector (top) + World Settings (bottom) ---
        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(240.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                let half = (ui.available_height() / 2.0 - ui.spacing().item_spacing.y).max(60.0);

                // Inspector
                ui.strong("Inspector");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("inspector")
                    .max_height(half - 24.0)
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new("Mesh Renderer")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Mesh: Cube");
                                ui.label("Cast Shadows: true");
                                ui.label("Receive Shadows: true");
                            });
                        egui::CollapsingHeader::new("Rigid Body")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Type: Static");
                                ui.label("Mass: 1.0 kg");
                                ui.label("Friction: 0.5");
                            });
                        egui::CollapsingHeader::new("Box Collider")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Size: 1.0 x 1.0 x 1.0");
                                ui.label("Center: 0.0, 0.0, 0.0");
                            });
                    });

                ui.add_space(4.0);
                ui.separator();

                // World Settings
                ui.strong("World Settings");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("world_settings")
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new("Lighting")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Ambient Intensity: 0.1");
                                ui.label("Shadow Distance: 100.0");
                                ui.label("Skybox: None");
                            });
                        egui::CollapsingHeader::new("Physics")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Gravity: (0.0, -9.81, 0.0)");
                                ui.label("Fixed Timestep: 0.02s");
                                ui.label("Solver Iterations: 6");
                            });
                        egui::CollapsingHeader::new("Rendering")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("VSync: On");
                                ui.label("MSAA: 4x");
                                ui.label("Tonemapping: ACES");
                            });
                    });
            });

        // --- Bottom panel: Console / Asset Browser ---
        // Declared after side panels so it sits between them.
        egui::TopBottomPanel::bottom("bottom_panel")
            .resizable(true)
            .default_height(180.0)
            .min_height(80.0)
            .show(ctx, |ui| {
                // Tab bar
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.bottom_tab, BottomTab::Console, "Console");
                    ui.selectable_value(
                        &mut self.bottom_tab,
                        BottomTab::AssetBrowser,
                        "Asset Browser",
                    );
                });
                ui.separator();

                match self.bottom_tab {
                    BottomTab::Console => {
                        egui::ScrollArea::vertical()
                            .id_salt("console_scroll")
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                console_line(ui, "INFO", 0xAAAAAA, "GGEngine v0.1.0 initialized");
                                console_line(ui, "INFO", 0xAAAAAA, "Vulkan context created");
                                console_line(ui, "INFO", 0xAAAAAA, "Swapchain created (1600x900)");
                                console_line(
                                    ui,
                                    "INFO",
                                    0x7BC67E,
                                    "Scene loaded: Untitled.ggscene",
                                );
                                console_line(
                                    ui,
                                    "WARN",
                                    0xE8C44A,
                                    "No skybox assigned to scene",
                                );
                                console_line(
                                    ui,
                                    "WARN",
                                    0xE8C44A,
                                    "Physics world has no ground collider",
                                );
                                console_line(ui, "INFO", 0xAAAAAA, "Editor ready");
                            });
                    }
                    BottomTab::AssetBrowser => {
                        egui::ScrollArea::vertical()
                            .id_salt("asset_browser_scroll")
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for name in [
                                        "default.mat",
                                        "brick_diffuse.png",
                                        "metal_normal.png",
                                        "player.obj",
                                        "skybox.exr",
                                        "footstep_01.wav",
                                        "main_theme.ogg",
                                        "grass.mat",
                                        "character_rig.fbx",
                                    ] {
                                        ui.group(|ui| {
                                            ui.set_width(90.0);
                                            ui.vertical_centered(|ui| {
                                                ui.label("[file]");
                                                ui.small(name);
                                            });
                                        });
                                    }
                                });
                            });
                    }
                }
            });

        // --- Center: Viewport ---
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(25, 25, 30)))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();

                // Viewport label (centered).
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.heading("Viewport");
                    });
                });

                // Viewport dimensions (bottom-right corner).
                ui.painter().text(
                    rect.right_bottom() + egui::vec2(-8.0, -8.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{:.0} x {:.0}", rect.width(), rect.height()),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_gray(80),
                );
            });
    }
}

/// Draw a single colored console line.
fn console_line(ui: &mut egui::Ui, level: &str, color_hex: u32, message: &str) {
    let r = ((color_hex >> 16) & 0xFF) as u8;
    let g = ((color_hex >> 8) & 0xFF) as u8;
    let b = (color_hex & 0xFF) as u8;
    ui.colored_label(egui::Color32::from_rgb(r, g, b), format!("[{level}] {message}"));
}

fn main() {
    run::<GGEditor>();
}
