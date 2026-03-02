mod sandbox2d;

use gg_engine::prelude::*;
use gg_engine::shaders;

// ---------------------------------------------------------------------------
// Vertex types
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct TriangleVertex {
    position: [f32; 3],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SquareVertex {
    position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TexturedVertex {
    position: [f32; 3],
    tex_coord: [f32; 2],
}

const TRIANGLE_VERTICES: [TriangleVertex; 3] = [
    TriangleVertex {
        position: [-0.5, -0.5, 0.0],
        color: [0.8, 0.2, 0.3, 1.0],
    },
    TriangleVertex {
        position: [0.5, -0.5, 0.0],
        color: [0.2, 0.3, 0.8, 1.0],
    },
    TriangleVertex {
        position: [0.0, 0.5, 0.0],
        color: [0.8, 0.8, 0.2, 1.0],
    },
];

const TRIANGLE_INDICES: [u32; 3] = [0, 1, 2];

const SQUARE_VERTICES: [SquareVertex; 4] = [
    SquareVertex {
        position: [-0.5, 0.5, 0.0],
    },
    SquareVertex {
        position: [0.5, 0.5, 0.0],
    },
    SquareVertex {
        position: [0.5, -0.5, 0.0],
    },
    SquareVertex {
        position: [-0.5, -0.5, 0.0],
    },
];

const SQUARE_INDICES: [u32; 6] = [0, 1, 2, 2, 3, 0];

const TEXTURED_QUAD_VERTICES: [TexturedVertex; 4] = [
    TexturedVertex {
        position: [-0.5, 0.5, 0.0],
        tex_coord: [0.0, 0.0],
    },
    TexturedVertex {
        position: [0.5, 0.5, 0.0],
        tex_coord: [1.0, 0.0],
    },
    TexturedVertex {
        position: [0.5, -0.5, 0.0],
        tex_coord: [1.0, 1.0],
    },
    TexturedVertex {
        position: [-0.5, -0.5, 0.0],
        tex_coord: [0.0, 1.0],
    },
];

const TEXTURED_QUAD_INDICES: [u32; 6] = [0, 1, 2, 2, 3, 0];

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct ExampleApp {
    vsync: bool,
    frame_time_ms: f32,

    // Camera
    camera_controller: OrthographicCameraController,

    // Material
    square_color: [f32; 4],

    // Shader library — central registry for all shaders
    shader_library: ShaderLibrary,

    // Rendering resources (initialized in on_attach)
    triangle_pipeline: Option<Ref<Pipeline>>,
    triangle_va: Option<VertexArray>,
    square_pipeline: Option<Ref<Pipeline>>,
    square_va: Option<VertexArray>,

    // Textured quad
    texture_pipeline: Option<Ref<Pipeline>>,
    texture_va: Option<VertexArray>,
    checkerboard_texture: Option<Texture2D>,
}

#[allow(dead_code)]
impl Application for ExampleApp {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("ExampleApp initialized");

        let aspect = 1280.0_f32 / 720.0;
        let camera_controller = OrthographicCameraController::new(aspect, true);

        ExampleApp {
            vsync: false,
            frame_time_ms: 0.0,
            camera_controller,
            square_color: [0.2, 0.3, 0.8, 1.0],
            shader_library: ShaderLibrary::new(),
            triangle_pipeline: None,
            triangle_va: None,
            square_pipeline: None,
            square_va: None,
            texture_pipeline: None,
            texture_va: None,
            checkerboard_texture: None,
        }
    }

    fn on_attach(&mut self, renderer: &Renderer) {
        // Register all shaders in the library — the central registry owns them.
        self.shader_library.add(renderer.create_shader(
            "flat_color",
            shaders::FLAT_COLOR_VERT_SPV,
            shaders::FLAT_COLOR_FRAG_SPV,
        ));
        self.shader_library.add(renderer.create_shader(
            "triangle",
            shaders::TRIANGLE_VERT_SPV,
            shaders::TRIANGLE_FRAG_SPV,
        ));
        self.shader_library.add(renderer.create_shader(
            "texture",
            shaders::TEXTURE_VERT_SPV,
            shaders::TEXTURE_FRAG_SPV,
        ));

        // ==== Square (flat color) ============================================
        let flat_color_shader = self.shader_library.get("flat_color").unwrap();

        let mut square_vb = renderer.create_vertex_buffer(as_bytes(&SQUARE_VERTICES));
        square_vb.set_layout(BufferLayout::new(&[BufferElement::new(
            ShaderDataType::Float3,
            "a_position",
        )]));

        let square_ib = renderer.create_index_buffer(&SQUARE_INDICES);

        let mut square_va = renderer.create_vertex_array();
        square_va.add_vertex_buffer(square_vb);
        square_va.set_index_buffer(square_ib);

        let square_pipeline = renderer.create_pipeline(&flat_color_shader, &square_va, true, true);

        // ==== Triangle (vertex colors) =======================================
        let triangle_shader = self.shader_library.get("triangle").unwrap();

        let mut triangle_vb = renderer.create_vertex_buffer(as_bytes(&TRIANGLE_VERTICES));
        triangle_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float4, "a_color"),
        ]));

        let triangle_ib = renderer.create_index_buffer(&TRIANGLE_INDICES);

        let mut triangle_va = renderer.create_vertex_array();
        triangle_va.add_vertex_buffer(triangle_vb);
        triangle_va.set_index_buffer(triangle_ib);

        let triangle_pipeline =
            renderer.create_pipeline(&triangle_shader, &triangle_va, false, false);

        // ==== Textured quad ==================================================
        let texture_shader = self.shader_library.get("texture").unwrap();

        let mut texture_vb = renderer.create_vertex_buffer(as_bytes(&TEXTURED_QUAD_VERTICES));
        texture_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float2, "a_tex_coord"),
        ]));

        let texture_ib = renderer.create_index_buffer(&TEXTURED_QUAD_INDICES);

        let mut texture_va = renderer.create_vertex_array();
        texture_va.add_vertex_buffer(texture_vb);
        texture_va.set_index_buffer(texture_ib);

        let texture_pipeline = renderer.create_texture_pipeline(&texture_shader, &texture_va);

        // Programmatic 8x8 checkerboard texture (magenta/dark gray).
        let mut checker_pixels = vec![0u8; 8 * 8 * 4];
        for y in 0..8u32 {
            for x in 0..8u32 {
                let idx = ((y * 8 + x) * 4) as usize;
                if (x + y) % 2 == 0 {
                    // Magenta
                    checker_pixels[idx] = 255;
                    checker_pixels[idx + 1] = 0;
                    checker_pixels[idx + 2] = 255;
                    checker_pixels[idx + 3] = 255;
                } else {
                    // Dark gray
                    checker_pixels[idx] = 40;
                    checker_pixels[idx + 1] = 40;
                    checker_pixels[idx + 2] = 40;
                    checker_pixels[idx + 3] = 255;
                }
            }
        }
        let checkerboard_texture = renderer.create_texture_from_rgba8(8, 8, &checker_pixels);

        self.square_pipeline = Some(square_pipeline);
        self.square_va = Some(square_va);
        self.triangle_pipeline = Some(triangle_pipeline);
        self.triangle_va = Some(triangle_va);
        self.texture_pipeline = Some(texture_pipeline);
        self.texture_va = Some(texture_va);
        self.checkerboard_texture = Some(checkerboard_texture);

        info!("Sandbox rendering resources created");
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEngine Sandbox".into(),
            ..Default::default()
        }
    }

    fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Mailbox
        }
    }

    fn camera(&self) -> Option<&OrthographicCamera> {
        Some(self.camera_controller.camera())
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        self.camera_controller.on_event(event);
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.frame_time_ms = dt.millis();
        self.camera_controller.on_update(dt, input);
    }

    fn on_render(&self, renderer: &Renderer) {
        let scale = Mat4::from_scale(Vec3::splat(0.1));

        // Draw a 20×20 grid of small squares with a dynamic color.
        let color = Vec4::from(self.square_color);

        if let (Some(pipeline), Some(va)) = (&self.square_pipeline, &self.square_va) {
            for y in 0..20 {
                for x in 0..20 {
                    let pos = Vec3::new(x as f32 * 0.11, y as f32 * 0.11, 0.0);
                    let transform = Mat4::from_translation(pos) * scale;
                    renderer.submit(pipeline, va, &transform, Some(color));
                }
            }
        }

        // Draw triangle on top at the origin.
        if let (Some(pipeline), Some(va)) = (&self.triangle_pipeline, &self.triangle_va) {
            renderer.submit(pipeline, va, &Mat4::IDENTITY, None);
        }

        // Draw textured quad.
        if let (Some(pipeline), Some(va), Some(tex)) = (
            &self.texture_pipeline,
            &self.texture_va,
            &self.checkerboard_texture,
        ) {
            let transform = Mat4::from_translation(Vec3::new(-1.5, 0.5, 0.0));
            renderer.submit_textured(pipeline, va, &transform, tex);
        }
    }

    fn on_egui(&mut self, ctx: &gg_engine::egui::Context) {
        gg_engine::egui::Window::new("Settings").show(ctx, |ui| {
            ui.checkbox(&mut self.vsync, "VSync");
            let fps = if self.frame_time_ms > 0.0 {
                1000.0_f32 / self.frame_time_ms
            } else {
                0.0_f32
            };
            ui.label(format!("{:.2} ms ({:.0} FPS)", self.frame_time_ms, fps));

            ui.separator();
            ui.strong("Camera");

            let mut pos = self.camera_controller.position();
            let mut pos_changed = false;
            ui.horizontal(|ui| {
                ui.label("X");
                pos_changed |= ui
                    .add(gg_engine::egui::DragValue::new(&mut pos.x).speed(0.05))
                    .changed();
                ui.label("Y");
                pos_changed |= ui
                    .add(gg_engine::egui::DragValue::new(&mut pos.y).speed(0.05))
                    .changed();
            });
            if pos_changed {
                self.camera_controller.set_position(pos);
            }

            let mut rotation_deg = self.camera_controller.rotation().to_degrees();
            ui.horizontal(|ui| {
                ui.label("Rotation");
                if ui
                    .add(gg_engine::egui::DragValue::new(&mut rotation_deg).speed(0.5))
                    .changed()
                {
                    self.camera_controller
                        .set_rotation(rotation_deg.to_radians());
                }
            });

            let mut zoom = self.camera_controller.zoom_level();
            ui.horizontal(|ui| {
                ui.label("Zoom");
                if ui
                    .add(
                        gg_engine::egui::DragValue::new(&mut zoom)
                            .speed(0.05)
                            .range(0.25..=10.0),
                    )
                    .changed()
                {
                    self.camera_controller.set_zoom_level(zoom);
                }
            });

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

fn main() {
    // Switch between ExampleApp (full test scene) and Sandbox2D (2D renderer prep).
    // run::<ExampleApp>();
    run::<sandbox2d::Sandbox2D>();
}
