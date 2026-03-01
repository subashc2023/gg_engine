use std::time::Instant;

use gg_engine::prelude::*;

struct Sandbox {
    vsync: bool,
    last_frame: Instant,
    frame_time_ms: f64,
}

impl Application for Sandbox {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("Sandbox initialized");
        Sandbox {
            vsync: false,
            last_frame: Instant::now(),
            frame_time_ms: 0.0,
        }
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

    fn on_event(&mut self, _event: &Event, _input: &Input) {}

    fn on_update(&mut self, _input: &Input) {
        let now = Instant::now();
        self.frame_time_ms = now.duration_since(self.last_frame).as_secs_f64() * 1000.0;
        self.last_frame = now;
    }

    fn on_egui(&mut self, ctx: &gg_engine::egui::Context) {
        gg_engine::egui::Window::new("Settings").show(ctx, |ui| {
            ui.checkbox(&mut self.vsync, "VSync");
            let fps = if self.frame_time_ms > 0.0 {
                1000.0 / self.frame_time_ms
            } else {
                0.0
            };
            ui.label(format!("{:.2} ms ({:.0} FPS)", self.frame_time_ms, fps));
        });
    }
}

fn main() {
    run::<Sandbox>();
}
