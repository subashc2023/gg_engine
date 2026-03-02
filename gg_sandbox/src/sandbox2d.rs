use std::cell::Cell;

use gg_engine::prelude::*;

fn vec4_to_srgba(v: Vec4) -> [u8; 4] {
    [
        (v.x * 255.0) as u8,
        (v.y * 255.0) as u8,
        (v.z * 255.0) as u8,
        (v.w * 255.0) as u8,
    ]
}

fn srgba_to_vec4(s: [u8; 4]) -> Vec4 {
    Vec4::new(
        s[0] as f32 / 255.0,
        s[1] as f32 / 255.0,
        s[2] as f32 / 255.0,
        s[3] as f32 / 255.0,
    )
}

pub struct Sandbox2D {
    camera_controller: OrthographicCameraController,
    square_color: [f32; 4],
    checkerboard_texture: Option<Texture2D>,
    last_dt: f32,
    last_stats: Cell<Renderer2DStats>,

    particle_system: ParticleSystem,
    particle_props: ParticleProps,
    window_width: u32,
    window_height: u32,

    // Sprite sheet demo.
    sprite_sheet: Option<Texture2D>,
    sprite_red: Option<SubTexture2D>,
    sprite_green: Option<SubTexture2D>,
    sprite_blue: Option<SubTexture2D>,
    sprite_yellow: Option<SubTexture2D>,
    sprite_wide: Option<SubTexture2D>, // 2x1 multi-cell sprite
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

            particle_system: ParticleSystem::new(10_000),
            particle_props: ParticleProps::default(),
            window_width: 1280,
            window_height: 720,

            sprite_sheet: None,
            sprite_red: None,
            sprite_green: None,
            sprite_blue: None,
            sprite_yellow: None,
            sprite_wide: None,
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

        // Programmatic 4x4 sprite sheet (each cell = 1 pixel, 16 distinct colors).
        //  Row 0: Red,      Green,    Blue,     Yellow
        //  Row 1: Cyan,     Magenta,  Orange,   White
        //  Row 2: DarkRed,  DarkGreen,DarkBlue, Purple
        //  Row 3: Pink,     Lime,     Teal,     Gray
        #[rustfmt::skip]
        let sheet_pixels: [u8; 4 * 4 * 4] = [
            255,0,0,255,     0,255,0,255,     0,0,255,255,     255,255,0,255,
            0,255,255,255,   255,0,255,255,   255,165,0,255,   255,255,255,255,
            139,0,0,255,     0,100,0,255,     0,0,139,255,     128,0,128,255,
            255,182,193,255, 50,205,50,255,   0,128,128,255,   128,128,128,255,
        ];
        let sheet = renderer.create_texture_from_rgba8(4, 4, &sheet_pixels);

        // Pick individual sprites from the sheet using grid coordinates.
        let cell = Vec2::new(1.0, 1.0); // each cell is 1x1 pixel
        self.sprite_red = Some(SubTexture2D::from_coords(&sheet, Vec2::new(0.0, 0.0), cell, Vec2::ONE));
        self.sprite_green = Some(SubTexture2D::from_coords(&sheet, Vec2::new(1.0, 0.0), cell, Vec2::ONE));
        self.sprite_blue = Some(SubTexture2D::from_coords(&sheet, Vec2::new(2.0, 0.0), cell, Vec2::ONE));
        self.sprite_yellow = Some(SubTexture2D::from_coords(&sheet, Vec2::new(3.0, 0.0), cell, Vec2::ONE));
        // Multi-cell sprite: 2 cells wide (Cyan + Magenta from row 1).
        self.sprite_wide = Some(SubTexture2D::from_coords(&sheet, Vec2::new(0.0, 1.0), cell, Vec2::new(2.0, 1.0)));
        self.sprite_sheet = Some(sheet);

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

        if let Event::Window(WindowEvent::Resize { width, height }) = event {
            if *width > 0 && *height > 0 {
                self.window_width = *width;
                self.window_height = *height;
            }
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        profile_scope!("Sandbox2D::on_update");
        self.last_dt = dt.seconds();
        self.camera_controller.on_update(dt, input);

        // Continuously emit particles at the origin for benchmarking.
        self.particle_props.position = Vec2::ZERO;
        for _ in 0..5 {
            self.particle_system.emit(&self.particle_props);
        }

        self.particle_system.on_update(dt);
    }

    fn on_render(&self, renderer: &Renderer) {
        profile_scope!("Sandbox2D::on_render");
        // Capture last frame's stats (snapshotted at end_scene).
        self.last_stats.set(renderer.stats_2d());

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

        // Sub-texture / sprite sheet demo: individual sprites from a 4x4 color grid.
        if let Some(red) = &self.sprite_red {
            renderer.draw_sub_textured_quad(
                &Vec3::new(-2.0, -1.5, 0.0),
                &Vec2::new(0.6, 0.6),
                red,
                Vec4::ONE,
            );
        }
        if let Some(green) = &self.sprite_green {
            renderer.draw_sub_textured_quad(
                &Vec3::new(-1.2, -1.5, 0.0),
                &Vec2::new(0.6, 0.6),
                green,
                Vec4::ONE,
            );
        }
        if let Some(blue) = &self.sprite_blue {
            renderer.draw_sub_textured_quad(
                &Vec3::new(-0.4, -1.5, 0.0),
                &Vec2::new(0.6, 0.6),
                blue,
                Vec4::ONE,
            );
        }
        if let Some(yellow) = &self.sprite_yellow {
            renderer.draw_sub_textured_quad(
                &Vec3::new(0.4, -1.5, 0.0),
                &Vec2::new(0.6, 0.6),
                yellow,
                Vec4::ONE,
            );
        }
        // Multi-cell sprite (2x1) rendered wider to match its aspect ratio.
        if let Some(wide) = &self.sprite_wide {
            renderer.draw_sub_textured_quad(
                &Vec3::new(1.5, -1.5, 0.0),
                &Vec2::new(1.2, 0.6),
                wide,
                Vec4::ONE,
            );
        }

        // Render particles (z = -0.1, in front of scene geometry).
        self.particle_system.on_render(renderer);
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
            ui.label("Left click: Emit particles");
        });

        gg_engine::egui::Window::new("Particles").show(ctx, |ui| {
            ui.label(format!(
                "Active: {}",
                self.particle_system.active_count()
            ));

            ui.separator();
            ui.strong("Velocity");
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity.x)
                        .speed(0.1),
                );
                ui.label("Y");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity.y)
                        .speed(0.1),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Var X");
                ui.add(
                    gg_engine::egui::DragValue::new(
                        &mut self.particle_props.velocity_variation.x,
                    )
                    .speed(0.1),
                );
                ui.label("Y");
                ui.add(
                    gg_engine::egui::DragValue::new(
                        &mut self.particle_props.velocity_variation.y,
                    )
                    .speed(0.1),
                );
            });

            ui.separator();
            ui.strong("Color");
            ui.horizontal(|ui| {
                ui.label("Begin");
                let mut begin = vec4_to_srgba(self.particle_props.color_begin);
                if ui
                    .color_edit_button_srgba_unmultiplied(&mut begin)
                    .changed()
                {
                    self.particle_props.color_begin = srgba_to_vec4(begin);
                }
                ui.label("End");
                let mut end = vec4_to_srgba(self.particle_props.color_end);
                if ui.color_edit_button_srgba_unmultiplied(&mut end).changed() {
                    self.particle_props.color_end = srgba_to_vec4(end);
                }
            });

            ui.separator();
            ui.strong("Size");
            ui.add(
                gg_engine::egui::Slider::new(&mut self.particle_props.size_begin, 0.01..=0.5)
                    .text("Begin"),
            );
            ui.add(
                gg_engine::egui::Slider::new(&mut self.particle_props.size_end, 0.0..=0.5)
                    .text("End"),
            );
            ui.add(
                gg_engine::egui::Slider::new(&mut self.particle_props.size_variation, 0.0..=0.2)
                    .text("Variation"),
            );

            ui.separator();
            ui.strong("Lifetime");
            ui.add(
                gg_engine::egui::Slider::new(&mut self.particle_props.lifetime, 0.1..=5.0)
                    .text("Seconds"),
            );
        });
    }
}
