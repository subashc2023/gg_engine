use gg_engine::prelude::*;

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

// Y-flipped for Vulkan NDC (Y+ is downward).
const TRIANGLE_VERTICES: [TriangleVertex; 3] = [
    TriangleVertex { position: [-0.5, 0.5, 0.0], color: [0.8, 0.2, 0.3, 1.0] },
    TriangleVertex { position: [0.5, 0.5, 0.0],  color: [0.2, 0.3, 0.8, 1.0] },
    TriangleVertex { position: [0.0, -0.5, 0.0], color: [0.8, 0.8, 0.2, 1.0] },
];

const TRIANGLE_INDICES: [u32; 3] = [0, 1, 2];

const SQUARE_VERTICES: [SquareVertex; 4] = [
    SquareVertex { position: [-0.75,  0.75, 0.0] },
    SquareVertex { position: [ 0.75,  0.75, 0.0] },
    SquareVertex { position: [ 0.75, -0.75, 0.0] },
    SquareVertex { position: [-0.75, -0.75, 0.0] },
];

const SQUARE_INDICES: [u32; 6] = [0, 1, 2, 2, 3, 0];

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

struct Sandbox {
    vsync: bool,
    frame_time_ms: f32,

    // Camera
    camera: OrthographicCamera,
    camera_position: Vec3,
    camera_rotation: f32,
    camera_move_speed: f32,
    camera_rotation_speed: f32,

    // Rendering resources (initialized in on_attach)
    triangle_shader: Option<Shader>,
    triangle_pipeline: Option<Pipeline>,
    triangle_va: Option<VertexArray>,
    square_shader: Option<Shader>,
    square_pipeline: Option<Pipeline>,
    square_va: Option<VertexArray>,
}

impl Application for Sandbox {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("Sandbox initialized");

        let aspect = 1280.0_f32 / 720.0;
        let camera = OrthographicCamera::new(-aspect, aspect, -1.0, 1.0);

        Sandbox {
            vsync: false,
            frame_time_ms: 0.0,
            camera,
            camera_position: Vec3::ZERO,
            camera_rotation: 0.0,
            camera_move_speed: 5.0,
            camera_rotation_speed: 180.0,
            triangle_shader: None,
            triangle_pipeline: None,
            triangle_va: None,
            square_shader: None,
            square_pipeline: None,
            square_va: None,
        }
    }

    fn on_attach(&mut self, renderer: &Renderer) {
        // ==== Square (flat blue) =============================================
        let square_shader = renderer.create_shader(
            "flat_color",
            include_bytes!("../../gg_engine/src/renderer/shaders/flat_color_vert.spv"),
            include_bytes!("../../gg_engine/src/renderer/shaders/flat_color_frag.spv"),
        );

        let mut square_vb = renderer.create_vertex_buffer(as_bytes(&SQUARE_VERTICES));
        square_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
        ]));

        let square_ib = renderer.create_index_buffer(&SQUARE_INDICES);

        let mut square_va = renderer.create_vertex_array();
        square_va.add_vertex_buffer(square_vb);
        square_va.set_index_buffer(square_ib);

        let square_pipeline = renderer.create_pipeline(&square_shader, &square_va);

        // ==== Triangle (vertex colors) =======================================
        let triangle_shader = renderer.create_shader(
            "triangle",
            include_bytes!("../../gg_engine/src/renderer/shaders/triangle_vert.spv"),
            include_bytes!("../../gg_engine/src/renderer/shaders/triangle_frag.spv"),
        );

        let mut triangle_vb = renderer.create_vertex_buffer(as_bytes(&TRIANGLE_VERTICES));
        triangle_vb.set_layout(BufferLayout::new(&[
            BufferElement::new(ShaderDataType::Float3, "a_position"),
            BufferElement::new(ShaderDataType::Float4, "a_color"),
        ]));

        let triangle_ib = renderer.create_index_buffer(&TRIANGLE_INDICES);

        let mut triangle_va = renderer.create_vertex_array();
        triangle_va.add_vertex_buffer(triangle_vb);
        triangle_va.set_index_buffer(triangle_ib);

        let triangle_pipeline = renderer.create_pipeline(&triangle_shader, &triangle_va);

        self.square_shader = Some(square_shader);
        self.square_pipeline = Some(square_pipeline);
        self.square_va = Some(square_va);
        self.triangle_shader = Some(triangle_shader);
        self.triangle_pipeline = Some(triangle_pipeline);
        self.triangle_va = Some(triangle_va);

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
        Some(&self.camera)
    }

    fn on_event(&mut self, _event: &Event, _input: &Input) {}

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.frame_time_ms = dt.millis();

        // Camera movement via input polling (speeds are per-second).
        let mut camera_changed = false;

        if input.is_key_pressed(KeyCode::Left) {
            self.camera_position.x -= self.camera_move_speed * dt;
            camera_changed = true;
        } else if input.is_key_pressed(KeyCode::Right) {
            self.camera_position.x += self.camera_move_speed * dt;
            camera_changed = true;
        }

        if input.is_key_pressed(KeyCode::Up) {
            self.camera_position.y += self.camera_move_speed * dt;
            camera_changed = true;
        } else if input.is_key_pressed(KeyCode::Down) {
            self.camera_position.y -= self.camera_move_speed * dt;
            camera_changed = true;
        }

        if input.is_key_pressed(KeyCode::A) {
            self.camera_rotation += (self.camera_rotation_speed * dt).to_radians();
            camera_changed = true;
        } else if input.is_key_pressed(KeyCode::D) {
            self.camera_rotation -= (self.camera_rotation_speed * dt).to_radians();
            camera_changed = true;
        }

        if camera_changed {
            self.camera.set_position(self.camera_position);
            self.camera.set_rotation(self.camera_rotation);
        }
    }

    fn on_render(&self, renderer: &Renderer) {
        // Draw square first (behind), then triangle on top.
        if let (Some(pipeline), Some(va)) = (&self.square_pipeline, &self.square_va) {
            renderer.submit(pipeline, va);
        }
        if let (Some(pipeline), Some(va)) = (&self.triangle_pipeline, &self.triangle_va) {
            renderer.submit(pipeline, va);
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

            let mut changed = false;
            changed |= ui
                .add(
                    gg_engine::egui::Slider::new(&mut self.camera_position.x, -2.0..=2.0)
                        .text("X"),
                )
                .changed();
            changed |= ui
                .add(
                    gg_engine::egui::Slider::new(&mut self.camera_position.y, -2.0..=2.0)
                        .text("Y"),
                )
                .changed();

            let mut rotation_deg = self.camera_rotation.to_degrees();
            if ui
                .add(
                    gg_engine::egui::Slider::new(&mut rotation_deg, -180.0..=180.0)
                        .text("Rotation"),
                )
                .changed()
            {
                self.camera_rotation = rotation_deg.to_radians();
                changed = true;
            }

            if changed {
                self.camera.set_position(self.camera_position);
                self.camera.set_rotation(self.camera_rotation);
            }

            ui.separator();
            ui.strong("Controls");
            ui.label("Arrow keys: Move camera");
            ui.label("A/D: Rotate camera");
        });
    }
}

fn main() {
    run::<Sandbox>();
}
