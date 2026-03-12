use gg_engine::prelude::*;

/// Interactive test for the cursor mode system.
///
/// Exercises all three modes (`Normal`, `Confined`, `Locked`) with live
/// telemetry showing mouse position, raw deltas, and a simple FPS-style
/// look-around demo in Locked mode.
pub struct CursorTest {
    pub cursor_mode: CursorMode,

    // FPS look-around state (used in Locked mode).
    look_yaw: f32,
    look_pitch: f32,

    // Telemetry.
    last_mouse_pos: (f64, f64),
    last_mouse_delta: (f64, f64),
    frame_count: u64,
}

impl CursorTest {
    pub fn new() -> Self {
        info!("CursorTest initialized — press 1/2/3 to switch modes");
        Self {
            cursor_mode: CursorMode::Normal,
            look_yaw: 0.0,
            look_pitch: 0.0,
            last_mouse_pos: (0.0, 0.0),
            last_mouse_delta: (0.0, 0.0),
            frame_count: 0,
        }
    }

    pub fn on_update(&mut self, _dt: Timestep, input: &Input) {
        self.frame_count += 1;

        // Snapshot telemetry.
        self.last_mouse_pos = input.mouse_position();
        self.last_mouse_delta = input.mouse_delta();

        // Keyboard shortcuts for mode switching.
        if input.is_key_just_pressed(KeyCode::Num1) || input.is_key_just_pressed(KeyCode::Escape) {
            self.cursor_mode = CursorMode::Normal;
            info!("Cursor mode → Normal");
        }
        if input.is_key_just_pressed(KeyCode::Num2) {
            self.cursor_mode = CursorMode::Confined;
            info!("Cursor mode → Confined");
        }
        if input.is_key_just_pressed(KeyCode::Num3) {
            self.cursor_mode = CursorMode::Locked;
            info!("Cursor mode → Locked");
        }

        // FPS look-around in Locked mode.
        if self.cursor_mode == CursorMode::Locked {
            let (dx, dy) = input.mouse_delta();
            let sensitivity = 0.003;
            self.look_yaw += dx as f32 * sensitivity;
            self.look_pitch = (self.look_pitch + dy as f32 * sensitivity).clamp(-1.5, 1.5);
        }
    }

    pub fn on_event(&mut self, _event: &Event, _input: &Input) {}

    pub fn on_render(&mut self, renderer: &mut Renderer) {
        // Draw a crosshair in the center of the screen to show where "forward" is.
        if self.cursor_mode == CursorMode::Locked {
            // Simple crosshair using quads.
            let size = 0.02;
            let thickness = 0.003;
            // Horizontal bar.
            renderer.draw_quad(
                &Vec3::new(0.0, 0.0, 0.0),
                &Vec2::new(size * 2.0, thickness),
                Vec4::new(1.0, 1.0, 1.0, 0.8),
            );
            // Vertical bar.
            renderer.draw_quad(
                &Vec3::new(0.0, 0.0, 0.0),
                &Vec2::new(thickness, size * 2.0),
                Vec4::new(1.0, 1.0, 1.0, 0.8),
            );
        }
    }

    pub fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        _window: &gg_engine::winit::window::Window,
    ) {
        gg_engine::egui::Window::new("Cursor Test")
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.heading("Cursor Mode System Test");
                ui.separator();

                // Mode selector.
                ui.label("Active Mode:");
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.cursor_mode == CursorMode::Normal, "1: Normal")
                        .clicked()
                    {
                        self.cursor_mode = CursorMode::Normal;
                    }
                    if ui
                        .selectable_label(self.cursor_mode == CursorMode::Confined, "2: Confined")
                        .clicked()
                    {
                        self.cursor_mode = CursorMode::Confined;
                    }
                    if ui
                        .selectable_label(self.cursor_mode == CursorMode::Locked, "3: Locked")
                        .clicked()
                    {
                        self.cursor_mode = CursorMode::Locked;
                    }
                });

                ui.separator();

                // Description of active mode.
                let desc = match self.cursor_mode {
                    CursorMode::Normal => {
                        "OS cursor visible, no grab.\n\
                         Standard behavior for editor/menus."
                    }
                    CursorMode::Confined => {
                        "OS cursor hidden, software cursor drawn.\n\
                         Cursor confined to window bounds.\n\
                         Move the mouse — you should see the engine's\n\
                         arrow cursor instead of the OS cursor."
                    }
                    CursorMode::Locked => {
                        "OS cursor hidden and locked in place.\n\
                         Raw deltas only — move the mouse to look around.\n\
                         A crosshair is shown at the center."
                    }
                };
                ui.label(desc);

                ui.separator();

                // Live telemetry.
                ui.heading("Telemetry");
                ui.monospace(format!(
                    "Mouse position: ({:.1}, {:.1})",
                    self.last_mouse_pos.0, self.last_mouse_pos.1,
                ));
                ui.monospace(format!(
                    "Mouse delta:    ({:.1}, {:.1})",
                    self.last_mouse_delta.0, self.last_mouse_delta.1,
                ));

                if self.cursor_mode == CursorMode::Locked {
                    ui.separator();
                    ui.label("FPS Look-Around");
                    ui.monospace(format!(
                        "Yaw: {:.1}\u{00b0}  Pitch: {:.1}\u{00b0}",
                        self.look_yaw.to_degrees(),
                        self.look_pitch.to_degrees(),
                    ));
                }

                ui.separator();
                ui.label("Keyboard: 1/Esc = Normal, 2 = Confined, 3 = Locked");
                ui.label("Alt+Tab or click outside to test focus loss/regain.");
            });
    }
}
