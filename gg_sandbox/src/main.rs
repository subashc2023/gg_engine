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
    /// When >0, counts down frames of runtime profiling, then exits.
    profile_frames: i32,
    profile_active: bool,
    /// Accumulated GPU timing samples for final summary.
    gpu_timing_samples: Vec<(String, f32)>,
    gpu_frame_times: Vec<f32>,
}

impl Application for Sandbox {
    fn new(_layers: &mut LayerStack) -> Self {
        let profiling = std::env::args().any(|a| a == "--profile");
        Self {
            mode: if profiling { Mode::ThreeD } else { Mode::TwoD },
            sandbox_2d: sandbox2d::Sandbox2D::new(),
            sandbox_3d: sandbox3d::Sandbox3D::new(),
            cursor_test: cursor_test::CursorTest::new(),
            last_dt: 0.0,
            profile_frames: if profiling { 2000 } else { -1 },
            profile_active: false,
            gpu_timing_samples: Vec::new(),
            gpu_frame_times: Vec::new(),
        }
    }

    fn on_attach(&mut self, renderer: &mut Renderer) {
        self.sandbox_3d.on_attach(renderer);
        // Enable GPU profiler when --profile is active.
        if self.profile_frames > 0 {
            if let Some(profiler) = renderer.gpu_profiler_mut() {
                profiler.set_enabled(true);
            }
        }
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

    fn should_exit(&self) -> bool {
        self.profile_frames == 0
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.last_dt = dt.seconds();
        // Start profiling session on first frame (after startup session is closed).
        if self.profile_frames > 0 && !self.profile_active {
            gg_engine::profiling::begin_session("Runtime", "gg_profile_runtime.json");
            self.profile_active = true;
            info!("Profiling: capturing {} frames", self.profile_frames);
        }
        match self.mode {
            Mode::TwoD => self.sandbox_2d.on_update(dt, input),
            Mode::ThreeD => self.sandbox_3d.on_update(dt, input),
            Mode::Cursor => self.cursor_test.on_update(dt, input),
        }
        if self.profile_frames > 0 {
            self.profile_frames -= 1;
            if self.profile_frames == 0 && self.profile_active {
                gg_engine::profiling::end_session();
                self.profile_active = false;
                info!("Profiling complete — wrote gg_profile_runtime.json");
                self.print_gpu_timing_summary();
            }
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
        // Collect GPU timing results each frame.
        if self.profile_active {
            if let Some(profiler) = renderer.gpu_profiler() {
                let total = profiler.total_frame_ms();
                if total > 0.0 {
                    self.gpu_frame_times.push(total);
                    for result in profiler.results() {
                        self.gpu_timing_samples
                            .push((result.name.to_string(), result.time_ms));
                    }
                }
            }
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

impl Sandbox {
    fn print_gpu_timing_summary(&self) {
        if self.gpu_frame_times.is_empty() {
            info!("No GPU timing data collected");
            return;
        }
        let n = self.gpu_frame_times.len();
        let avg: f32 = self.gpu_frame_times.iter().sum::<f32>() / n as f32;
        let mut sorted = self.gpu_frame_times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = sorted[n / 2];
        let p95 = sorted[(n as f32 * 0.95) as usize];
        let p99 = sorted[(n as f32 * 0.99) as usize];
        let max = sorted[n - 1];

        info!("=== GPU Timing Summary ({n} frames) ===");
        info!(
            "  Total GPU frame: avg={avg:.3}ms  P50={p50:.3}ms  P95={p95:.3}ms  P99={p99:.3}ms  max={max:.3}ms"
        );

        // Aggregate per-scope.
        let mut scope_data: std::collections::HashMap<String, Vec<f32>> =
            std::collections::HashMap::new();
        for (name, time) in &self.gpu_timing_samples {
            scope_data
                .entry(name.clone())
                .or_default()
                .push(*time);
        }
        let mut scopes: Vec<_> = scope_data.into_iter().collect();
        scopes.sort_by(|a, b| {
            b.1.iter().sum::<f32>().partial_cmp(&a.1.iter().sum::<f32>()).unwrap()
        });
        for (name, times) in &scopes {
            let count = times.len();
            let avg = times.iter().sum::<f32>() / count as f32;
            let mut s = times.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p50 = s[count / 2];
            let max = s[count - 1];
            info!("  {name:20}: avg={avg:.3}ms  P50={p50:.3}ms  max={max:.3}ms  ({count} samples)");
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
