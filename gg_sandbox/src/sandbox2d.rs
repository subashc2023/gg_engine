use std::cell::Cell;

use gg_engine::prelude::*;

pub struct Sandbox2D {
    camera_controller: OrthographicCameraController,
    last_dt: f32,
    last_stats: Cell<Renderer2DStats>,
}

impl Sandbox2D {
    pub fn new() -> Self {
        let aspect = 1280.0_f32 / 720.0;
        let mut camera_controller = OrthographicCameraController::new(aspect, true);
        camera_controller.set_zoom_level(3.0);

        info!("Sandbox2D initialized");
        Self {
            camera_controller,
            last_dt: 0.0,
            last_stats: Cell::new(Renderer2DStats::default()),
        }
    }

    pub fn camera(&self) -> Option<&OrthographicCamera> {
        Some(self.camera_controller.camera())
    }

    pub fn on_event(&mut self, event: &Event, _input: &Input) {
        self.camera_controller.on_event(event);
    }

    pub fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.last_dt = dt.seconds();
        self.camera_controller.on_update(dt, input);
    }

    pub fn on_render(&mut self, renderer: &mut Renderer) {
        self.last_stats.set(renderer.stats_2d());

        renderer.draw_quad(
            &Vec3::new(0.0, 0.0, 0.0),
            &Vec2::new(1.0, 1.0),
            Vec4::new(0.2, 0.8, 0.3, 1.0),
        );
        renderer.draw_quad(
            &Vec3::new(1.5, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            Vec4::new(0.8, 0.2, 0.3, 1.0),
        );
        renderer.draw_quad(
            &Vec3::new(-1.5, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            Vec4::new(0.2, 0.3, 0.8, 1.0),
        );
    }

    pub fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        _window: &gg_engine::winit::window::Window,
    ) {
        let fps = if self.last_dt > 0.0 {
            1.0 / self.last_dt
        } else {
            0.0
        };
        let stats = self.last_stats.get();

        gg_engine::egui::Window::new("Sandbox 2D").show(ctx, |ui| {
            ui.label(format!("{:.2} ms ({:.0} FPS)", self.last_dt * 1000.0, fps));
            ui.separator();
            ui.label(format!("Draw calls: {}", stats.draw_calls));
            ui.label(format!("Quads: {}", stats.quad_count));
            ui.separator();
            ui.label("WASD: Move  |  Q/E: Rotate  |  Scroll: Zoom");
        });
    }
}
