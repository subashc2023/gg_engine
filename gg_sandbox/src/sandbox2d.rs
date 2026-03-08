use std::cell::Cell;
use std::collections::HashMap;

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

/// Generate a high-frequency checkerboard texture (RGBA8).
/// `cell_size` is how many pixels per checker square.
fn generate_checkerboard(width: u32, height: u32, cell_size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let checker = ((x / cell_size) + (y / cell_size)).is_multiple_of(2);
            if checker {
                pixels.extend_from_slice(&[255, 255, 255, 255]); // white
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 255]); // black
            }
        }
    }
    pixels
}

// ---------------------------------------------------------------------------
// Tilemap data
// ---------------------------------------------------------------------------

const MAP_WIDTH: u32 = 24;

/// ASCII tilemap: 'W' = water, 'D' = dirt, 'G' = grass.
/// Read top-to-bottom, left-to-right. Each row is MAP_WIDTH chars.
/// The map is a small island with a grass interior, dirt shores, and
/// a water lake in the center — surrounded by ocean.
#[rustfmt::skip]
const MAP_TILES: &str = "\
WWWWWWWWWWWWWWWWWWWWWWWW\
WWWWWWWDDDDDDDDWWWWWWWWW\
WWWWWDDDDDDDDDDDDWWWWWWW\
WWWWDDDDGGGGGGGDDDDDWWWW\
WWWDDDGGGGGGGGGGGDDDDWWW\
WWDDDDGGGGGGGGGGGDDDDDWW\
WWDDDDGGGGWWGGGGGDDDDDWW\
WWDDDDGGGGWWGGGGDDDDDDWW\
WWWDDDGGGGGGGGGDDDDDWWWW\
WWWWDDDDDGGGGGGDDDDDDWWW\
WWWWWDDDDDDDDDDDDDDWWWWW\
WWWWWWWDDDDDDDDDWWWWWWWW\
WWWWWWWWWWWWWWWWWWWWWWWW";

// ---------------------------------------------------------------------------
// Sandbox2D
// ---------------------------------------------------------------------------

pub struct Sandbox2D {
    camera_controller: OrthographicCameraController,
    last_dt: f32,
    last_stats: Cell<Renderer2DStats>,

    particle_props: ParticleProps,
    emit_rate: u32,
    window_width: u32,
    window_height: u32,

    // Tilemap rendering.
    tile_colors: HashMap<char, Vec4>,
    map_width: u32,
    map_height: u32,

    // Mipmap comparison textures.
    checker_no_mip: Option<Ref<Texture2D>>,
    checker_nearest_mip: Option<Ref<Texture2D>>,
    checker_linear_mip: Option<Ref<Texture2D>>,
}

impl Application for Sandbox2D {
    fn new(_layers: &mut LayerStack) -> Self {
        let aspect = 1280.0_f32 / 720.0;
        let mut camera_controller = OrthographicCameraController::new(aspect, true);
        camera_controller.set_zoom_level(6.0);

        info!("Sandbox2D initialized");
        Sandbox2D {
            camera_controller,
            last_dt: 0.0,
            last_stats: Cell::new(Renderer2DStats::default()),

            particle_props: ParticleProps::default(),
            emit_rate: 5,
            window_width: 1280,
            window_height: 720,

            tile_colors: HashMap::new(),
            map_width: MAP_WIDTH,
            map_height: 0,

            checker_no_mip: None,
            checker_nearest_mip: None,
            checker_linear_mip: None,
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        profile_scope!("Sandbox2D::on_attach");

        self.map_height = MAP_TILES.len() as u32 / self.map_width;

        // Tile type → color mapping (flat-colored quads, no texture atlas needed).
        self.tile_colors
            .insert('W', Vec4::new(0.157, 0.392, 0.784, 1.0)); // Water (blue)
        self.tile_colors
            .insert('D', Vec4::new(0.706, 0.510, 0.275, 1.0)); // Dirt  (brown)
        self.tile_colors
            .insert('G', Vec4::new(0.196, 0.706, 0.196, 1.0)); // Grass (green)

        // Create GPU particle system (100K max particles).
        if let Err(e) = renderer.create_gpu_particle_system(100_000) {
            error!("Failed to create GPU particle system: {e}");
        }

        // Create mipmap comparison textures: 256x256 checkerboard with 2px cells.
        // High frequency pattern makes aliasing very obvious without mipmaps.
        let checker_pixels = generate_checkerboard(256, 256, 2);

        // Without mipmaps (default).
        match renderer.create_texture_from_rgba8(256, 256, &checker_pixels) {
            Ok(tex) => self.checker_no_mip = Some(Ref::new(tex)),
            Err(e) => error!("Failed to create no-mip checker: {e}"),
        }

        // With mipmaps + NEAREST filter (pixel-art friendly).
        match renderer.create_texture_from_rgba8_with_spec(256, 256, &checker_pixels, TextureSpecification::nearest_mipmapped()) {
            Ok(tex) => self.checker_nearest_mip = Some(Ref::new(tex)),
            Err(e) => error!("Failed to create nearest-mipmapped checker: {e}"),
        }

        // With mipmaps + LINEAR filter (trilinear).
        match renderer.create_texture_from_rgba8_with_spec(256, 256, &checker_pixels, TextureSpecification::linear_mipmapped()) {
            Ok(tex) => self.checker_linear_mip = Some(Ref::new(tex)),
            Err(e) => error!("Failed to create linear-mipmapped checker: {e}"),
        }

        info!(
            "Tilemap loaded: {}x{} ({} tiles)",
            self.map_width,
            self.map_height,
            MAP_TILES.len()
        );
        info!("Mipmap comparison textures created (256x256, 2px checkerboard)");
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
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("Sandbox2D::on_render");
        self.last_stats.set(renderer.stats_2d());

        // Render tilemap.
        let half_w = self.map_width as f32 * 0.5;
        let half_h = self.map_height as f32 * 0.5;
        let bytes = MAP_TILES.as_bytes();

        for y in 0..self.map_height {
            for x in 0..self.map_width {
                let tile_char = bytes[(x + y * self.map_width) as usize] as char;

                let color = match self.tile_colors.get(&tile_char) {
                    Some(&c) => c,
                    None => continue,
                };

                renderer.draw_quad(
                    &Vec3::new(x as f32 - half_w, half_h - 1.0 - y as f32, 0.0),
                    &Vec2::new(1.0, 1.0),
                    color,
                );
            }
        }

        // Mipmap comparison: 3 columns at decreasing scales.
        // Left = no mipmaps, Center = NEAREST + mipmaps, Right = LINEAR + mipmaps.
        // Zoom out to see aliasing vs smooth mip blending.
        let scales: &[f32] = &[4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
        let col_x: [f32; 3] = [-19.0, -13.0, -7.0];

        for (i, &scale) in scales.iter().enumerate() {
            let y = 10.0 - i as f32 * 5.0;
            let size = scale * 3.0;

            let textures: [&Option<Ref<Texture2D>>; 3] = [
                &self.checker_no_mip,
                &self.checker_nearest_mip,
                &self.checker_linear_mip,
            ];

            for (col, tex_opt) in textures.iter().enumerate() {
                if let Some(ref tex) = tex_opt {
                    let transform = Mat4::from_scale_rotation_translation(
                        Vec3::new(size, size, 1.0),
                        Quat::IDENTITY,
                        Vec3::new(col_x[col], y, 0.1),
                    );
                    renderer.draw_textured_quad_transform(&transform, tex, 1.0, Vec4::ONE);
                }
            }
        }

        // Emit GPU particles at the origin.
        self.particle_props.position = Vec2::ZERO;
        for _ in 0..self.emit_rate {
            renderer.emit_particles(&self.particle_props);
        }

        // Render GPU particles (instanced draw, indirect).
        renderer.render_gpu_particles();
    }

    fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        _window: &gg_engine::winit::window::Window,
    ) {
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
            ui.separator();
            ui.label(format!(
                "Map: {}x{} ({} tiles)",
                self.map_width,
                self.map_height,
                self.map_width * self.map_height
            ));
        });

        gg_engine::egui::Window::new("Controls").show(ctx, |ui| {
            ui.label("WASD: Move camera");
            ui.label("Q/E: Rotate camera");
            ui.label("Scroll: Zoom");
            ui.separator();
            ui.strong("Mipmap Test (left side of scene)");
            ui.label("Left:   NEAREST, no mipmaps");
            ui.label("Center: NEAREST + mipmaps");
            ui.label("Right:  LINEAR + mipmaps");
            ui.label("Zoom out to see the difference");
        });

        gg_engine::egui::Window::new("GPU Particles").show(ctx, |ui| {
            ui.label("Compute shader simulation + instanced rendering");

            ui.separator();
            ui.strong("Emission");
            ui.add(gg_engine::egui::Slider::new(&mut self.emit_rate, 0..=100).text("Per frame"));

            ui.separator();
            ui.strong("Velocity");
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity.x).speed(0.1),
                );
                ui.label("Y");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity.y).speed(0.1),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Var X");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity_variation.x)
                        .speed(0.1),
                );
                ui.label("Y");
                ui.add(
                    gg_engine::egui::DragValue::new(&mut self.particle_props.velocity_variation.y)
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
