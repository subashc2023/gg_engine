mod cursor_test;
mod jobs_stress;
mod sandbox2d;
mod sandbox3d;

use gg_engine::prelude::*;

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    TwoD,
    ThreeD,
    Cursor,
}

struct Sandbox {
    mode: Mode,
    sandbox_2d: sandbox2d::Sandbox2D,
    sandbox_3d: sandbox3d::Sandbox3D,
    cursor_test: cursor_test::CursorTest,
    last_dt: f32,
}

impl Application for Sandbox {
    fn new(_layers: &mut LayerStack) -> Self {
        Self {
            mode: Mode::TwoD,
            sandbox_2d: sandbox2d::Sandbox2D::new(),
            sandbox_3d: sandbox3d::Sandbox3D::new(),
            cursor_test: cursor_test::CursorTest::new(),
            last_dt: 0.0,
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        self.sandbox_3d.on_attach(renderer);
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEngine Sandbox".into(),
            ..Default::default()
        }
    }

    fn present_mode(&self) -> PresentMode {
        PresentMode::Mailbox
    }

    fn camera(&self) -> Option<&OrthographicCamera> {
        match self.mode {
            Mode::TwoD => self.sandbox_2d.camera(),
            Mode::ThreeD | Mode::Cursor => None,
        }
    }

    fn clear_color(&self) -> [f32; 4] {
        match self.mode {
            Mode::TwoD => [0.1, 0.1, 0.1, 1.0],
            Mode::ThreeD => self.sandbox_3d.clear_color(),
            Mode::Cursor => [0.12, 0.12, 0.15, 1.0],
        }
    }

    fn cursor_mode(&self) -> CursorMode {
        match self.mode {
            Mode::Cursor => self.cursor_test.cursor_mode,
            _ => CursorMode::Normal,
        }
    }

    fn on_event(&mut self, event: &Event, input: &Input) {
        match self.mode {
            Mode::TwoD => self.sandbox_2d.on_event(event, input),
            Mode::ThreeD => self.sandbox_3d.on_event(event, input),
            Mode::Cursor => self.cursor_test.on_event(event, input),
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.last_dt = dt.seconds();
        match self.mode {
            Mode::TwoD => self.sandbox_2d.on_update(dt, input),
            Mode::ThreeD => self.sandbox_3d.on_update(dt, input),
            Mode::Cursor => self.cursor_test.on_update(dt, input),
        }
    }

    fn on_render_shadows(
        &mut self,
        renderer: &mut Renderer,
        cmd_buf: gg_engine::ash::vk::CommandBuffer,
        current_frame: usize,
    ) {
        if self.mode == Mode::ThreeD {
            self.sandbox_3d
                .on_render_shadows(renderer, cmd_buf, current_frame);
        }
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        match self.mode {
            Mode::TwoD => self.sandbox_2d.on_render(renderer),
            Mode::ThreeD => self.sandbox_3d.on_render(renderer),
            Mode::Cursor => self.cursor_test.on_render(renderer),
        }
    }

    fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        window: &gg_engine::winit::window::Window,
    ) {
        let fps = if self.last_dt > 0.0 {
            1.0 / self.last_dt
        } else {
            0.0
        };

        gg_engine::egui::Window::new("Sandbox").show(ctx, |ui| {
            ui.label(format!("{:.0} FPS", fps));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Mode:");
                ui.selectable_value(&mut self.mode, Mode::TwoD, "2D");
                ui.selectable_value(&mut self.mode, Mode::ThreeD, "3D");
                ui.selectable_value(&mut self.mode, Mode::Cursor, "Cursor");
            });
        });

        match self.mode {
            Mode::TwoD => self.sandbox_2d.on_egui(ctx, window),
            Mode::ThreeD => self.sandbox_3d.on_egui(ctx, window),
            Mode::Cursor => self.cursor_test.on_egui(ctx, window),
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--stress") {
        run::<jobs_stress::JobsStress>();
    } else {
        run::<Sandbox>();
    }
}
