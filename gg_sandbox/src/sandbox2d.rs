use std::cell::Cell;

use gg_engine::prelude::*;

pub struct Sandbox2D {
    camera_controller: OrthographicCameraController,
    square_color: [f32; 4],
    checkerboard_texture: Option<Texture2D>,
    last_dt: f32,
    last_stats: Cell<Renderer2DStats>,
    stress_test: bool,
    grid_size: i32,
}

impl Application for Sandbox2D {
    fn new(_layers: &mut LayerStack) -> Self {
        let aspect = 1280.0_f32 / 720.0;
        info!("Sandbox2D initialized");
        Sandbox2D {
            camera_controller: OrthographicCameraController::new(aspect, true),
            square_color: [0.2, 0.3, 0.8, 1.0],
            checkerboard_texture: None,
            last_dt: 0.0,
            last_stats: Cell::new(Renderer2DStats::default()),
            stress_test: false,
            grid_size: 100,
        }
    }

    fn on_attach(&mut self, renderer: &Renderer) {
        profile_scope!("Sandbox2D::on_attach");
        // Programmatic 8x8 checkerboard texture (magenta / dark gray).
        let mut pixels = vec![0u8; 8 * 8 * 4];
        for y in 0..8u32 {
            for x in 0..8u32 {
                let idx = ((y * 8 + x) * 4) as usize;
                if (x + y) % 2 == 0 {
                    pixels[idx] = 255;
                    pixels[idx + 1] = 0;
                    pixels[idx + 2] = 255;
                    pixels[idx + 3] = 255;
                } else {
                    pixels[idx] = 40;
                    pixels[idx + 1] = 40;
                    pixels[idx + 2] = 40;
                    pixels[idx + 3] = 255;
                }
            }
        }
        self.checkerboard_texture = Some(renderer.create_texture_from_rgba8(8, 8, &pixels));
        info!("Sandbox2D rendering resources created");
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "Sandbox 2D".into(),
            ..Default::default()
        }
    }

    fn present_mode(&self) -> PresentMode {
        PresentMode::Mailbox
    }

    fn camera(&self) -> Option<&OrthographicCamera> {
        Some(self.camera_controller.camera())
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        self.camera_controller.on_event(event);
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        profile_scope!("Sandbox2D::on_update");
        self.last_dt = dt.seconds();
        self.camera_controller.on_update(dt, input);
    }

    fn on_render(&self, renderer: &Renderer) {
        profile_scope!("Sandbox2D::on_render");
        // Capture last frame's stats (snapshotted at end_scene).
        self.last_stats.set(renderer.stats_2d());

        if self.stress_test {
            // Stress test: NxN grid of colored quads.
            let n = self.grid_size;
            let spacing = 0.22_f32;
            let quad_size = 0.2_f32;
            let offset = n as f32 * spacing * 0.5;
            for y in 0..n {
                for x in 0..n {
                    let r = x as f32 / n as f32;
                    let g = y as f32 / n as f32;
                    let color = Vec4::new(r, g, 0.3, 0.75);
                    renderer.draw_quad(
                        &Vec3::new(
                            x as f32 * spacing - offset,
                            y as f32 * spacing - offset,
                            0.0,
                        ),
                        &Vec2::new(quad_size, quad_size),
                        color,
                    );
                }
            }
            return;
        }

        // Draw checkerboard background with 10x tiling (z = 0.1 pushes it behind the quads).
        if let Some(tex) = &self.checkerboard_texture {
            renderer.draw_textured_quad(
                &Vec3::new(0.0, 0.0, 0.1),
                &Vec2::new(10.0, 10.0),
                tex,
                10.0,
                Vec4::ONE,
            );
        }

        // Draw colored quads in front (z = 0).
        renderer.draw_quad(
            &Vec3::new(-1.0, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            Vec4::from(self.square_color),
        );
        renderer.draw_quad(
            &Vec3::new(0.5, -0.5, 0.0),
            &Vec2::new(0.5, 0.75),
            Vec4::new(0.8, 0.2, 0.3, 1.0),
        );

        // Rotated colored quad (45 degrees).
        renderer.draw_rotated_quad(
            &Vec3::new(-2.0, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            std::f32::consts::FRAC_PI_4,
            Vec4::new(0.2, 0.8, 0.3, 1.0),
        );

        // Tinted textured quad with slight red tint.
        if let Some(tex) = &self.checkerboard_texture {
            renderer.draw_rotated_textured_quad(
                &Vec3::new(1.5, 0.5, 0.0),
                &Vec2::new(1.0, 1.0),
                std::f32::consts::FRAC_PI_4,
                tex,
                1.0,
                Vec4::new(1.0, 0.8, 0.8, 1.0),
            );
        }
    }

    fn on_egui(&mut self, ctx: &gg_engine::egui::Context) {
        let dt_ms = self.last_dt * 1000.0;
        let fps = if self.last_dt > 0.0 {
            1.0 / self.last_dt
        } else {
            0.0
        };
        let stats = self.last_stats.get();

        gg_engine::egui::Window::new("Stats").show(ctx, |ui| {
            ui.label(format!("{:.2} ms ({:.0} FPS)", dt_ms, fps));
            ui.separator();
            ui.label(format!("Draw calls: {}", stats.draw_calls));
            ui.label(format!("Quads: {}", stats.quad_count));
            ui.label(format!("Vertices: {}", stats.total_vertex_count()));
            ui.label(format!("Indices: {}", stats.total_index_count()));
        });

        gg_engine::egui::Window::new("Settings").show(ctx, |ui| {
            ui.strong("Stress Test");
            ui.checkbox(&mut self.stress_test, "Enable grid");
            if self.stress_test {
                ui.add(
                    gg_engine::egui::Slider::new(&mut self.grid_size, 10..=200)
                        .text("Grid size"),
                );
                let total = self.grid_size as u32 * self.grid_size as u32;
                ui.label(format!("Total quads: {}", total));
            }

            ui.separator();
            ui.strong("Material");
            let mut srgba = [
                (self.square_color[0] * 255.0) as u8,
                (self.square_color[1] * 255.0) as u8,
                (self.square_color[2] * 255.0) as u8,
                (self.square_color[3] * 255.0) as u8,
            ];
            if ui
                .color_edit_button_srgba_unmultiplied(&mut srgba)
                .changed()
            {
                self.square_color = [
                    srgba[0] as f32 / 255.0,
                    srgba[1] as f32 / 255.0,
                    srgba[2] as f32 / 255.0,
                    srgba[3] as f32 / 255.0,
                ];
            }

            ui.separator();
            ui.strong("Controls");
            ui.label("WASD: Move camera");
            ui.label("Q/E: Rotate camera");
            ui.label("Scroll: Zoom");
        });
    }
}
