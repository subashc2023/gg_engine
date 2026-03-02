use gg_engine::egui;
use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// Tab identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tab {
    Viewport,
    Settings,
}

// ---------------------------------------------------------------------------
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    dock_state: egui_dock::DockState<Tab>,
    scene_fb: Option<Framebuffer>,
    camera_controller: OrthographicCameraController,
    viewport_size: (u32, u32),
    viewport_focused: bool,
    viewport_hovered: bool,
    vsync: bool,
    frame_time_ms: f32,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // Initial layout: Viewport on the left (80%), Settings on the right (20%).
        let mut dock_state = egui_dock::DockState::new(vec![Tab::Viewport]);
        let surface = dock_state.main_surface_mut();
        let root = egui_dock::NodeIndex::root();
        surface.split_right(root, 0.8, vec![Tab::Settings]);

        GGEditor {
            dock_state,
            scene_fb: None,
            camera_controller: OrthographicCameraController::new(16.0 / 9.0, true),
            viewport_size: (0, 0),
            viewport_focused: false,
            viewport_hovered: false,
            vsync: true,
            frame_time_ms: 0.0,
        }
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEditor".into(),
            width: 1600,
            height: 900,
        }
    }

    fn on_attach(&mut self, renderer: &Renderer) {
        let fb = renderer.create_framebuffer(FramebufferSpec {
            width: 800,
            height: 600,
        });
        self.scene_fb = Some(fb);
    }

    fn scene_framebuffer(&self) -> Option<&Framebuffer> {
        self.scene_fb.as_ref()
    }

    fn scene_framebuffer_mut(&mut self) -> Option<&mut Framebuffer> {
        self.scene_fb.as_mut()
    }

    fn desired_viewport_size(&self) -> Option<(u32, u32)> {
        if self.viewport_size.0 > 0 && self.viewport_size.1 > 0 {
            Some(self.viewport_size)
        } else {
            None
        }
    }

    fn camera(&self) -> Option<&OrthographicCamera> {
        Some(self.camera_controller.camera())
    }

    fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Immediate
        }
    }

    fn block_events(&self) -> bool {
        // Allow events through to the engine when the viewport is focused
        // and hovered (e.g. scroll zoom). Block them otherwise so egui
        // widgets (checkboxes, sliders, etc.) work normally.
        !(self.viewport_focused && self.viewport_hovered)
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        // Only forward scroll/resize events to the camera when the viewport
        // is focused AND hovered. This prevents scrolling in the Settings
        // panel from zooming the scene camera.
        if self.viewport_focused && self.viewport_hovered {
            self.camera_controller.on_event(event);
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        // Only poll WASD/QE when the viewport is focused.
        if self.viewport_focused {
            self.camera_controller.on_update(dt, input);
        }

        // Exponential moving average for stable frame time display.
        // Alpha of 0.05 smooths out per-frame jitter from the double-buffered
        // fence-wait pattern while still responding to real changes.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        // Update camera projection if viewport size changed.
        if let Some(fb) = &self.scene_fb {
            let (w, h) = (fb.width(), fb.height());
            if w > 0 && h > 0 {
                let aspect = w as f32 / h as f32;
                let zoom = self.camera_controller.zoom_level();
                self.camera_controller.camera_mut().set_projection(
                    -aspect * zoom,
                    aspect * zoom,
                    -zoom,
                    zoom,
                );
            }
        }
    }

    fn on_render(&self, renderer: &Renderer) {
        // Test quads.
        renderer.draw_quad(
            &Vec3::new(0.0, 0.0, 0.0),
            &Vec2::new(1.0, 1.0),
            Vec4::new(0.8, 0.2, 0.3, 1.0),
        );
        renderer.draw_quad(
            &Vec3::new(1.5, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            Vec4::new(0.2, 0.3, 0.8, 1.0),
        );
        renderer.draw_quad(
            &Vec3::new(-1.5, 0.0, 0.0),
            &Vec2::new(0.8, 0.8),
            Vec4::new(0.2, 0.8, 0.3, 1.0),
        );

        // Checkerboard.
        for y in -5..5 {
            for x in -5..5 {
                let color = if (x + y) % 2 == 0 {
                    Vec4::new(0.3, 0.3, 0.3, 1.0)
                } else {
                    Vec4::new(0.5, 0.5, 0.5, 1.0)
                };
                renderer.draw_quad(
                    &Vec3::new(x as f32 * 0.25, y as f32 * 0.25, -0.1),
                    &Vec2::new(0.23, 0.23),
                    color,
                );
            }
        }
    }

    fn on_egui(&mut self, ctx: &egui::Context) {
        // Gather state the TabViewer needs.
        let fb_tex_id = self
            .scene_fb
            .as_ref()
            .and_then(|fb| fb.egui_texture_id());

        let mut viewer = EditorTabViewer {
            viewport_size: &mut self.viewport_size,
            viewport_focused: &mut self.viewport_focused,
            viewport_hovered: &mut self.viewport_hovered,
            fb_tex_id,
            vsync: &mut self.vsync,
            frame_time_ms: self.frame_time_ms,
        };

        egui_dock::DockArea::new(&mut self.dock_state)
            .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);
    }
}

// ---------------------------------------------------------------------------
// TabViewer implementation
// ---------------------------------------------------------------------------

struct EditorTabViewer<'a> {
    viewport_size: &'a mut (u32, u32),
    viewport_focused: &'a mut bool,
    viewport_hovered: &'a mut bool,
    fb_tex_id: Option<egui::TextureId>,
    vsync: &'a mut bool,
    frame_time_ms: f32,
}

impl egui_dock::TabViewer for EditorTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Viewport => "Viewport".into(),
            Tab::Settings => "Settings".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Viewport => {
                let available = ui.available_size();
                let w = available.x as u32;
                let h = available.y as u32;
                if w > 0 && h > 0 {
                    *self.viewport_size = (w, h);
                }

                // Hovered: mouse is over the viewport panel right now.
                *self.viewport_hovered = ui.ui_contains_pointer();

                // Focused: click-based, persists until another panel is clicked.
                // Clicking inside the viewport sets focus; clicking outside
                // (handled in other tab branches) clears it.
                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && *self.viewport_hovered {
                    *self.viewport_focused = true;
                }

                if let Some(tex_id) = self.fb_tex_id {
                    let size = egui::vec2(available.x, available.y);
                    ui.image(egui::load::SizedTexture::new(tex_id, size));
                }
            }
            Tab::Settings => {
                // Clicking in the Settings panel unfocuses the viewport.
                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && ui.ui_contains_pointer() {
                    *self.viewport_focused = false;
                }

                ui.heading("Renderer");
                ui.separator();

                let fps = if self.frame_time_ms > 0.0 {
                    1000.0 / self.frame_time_ms
                } else {
                    0.0
                };
                ui.label(format!("Frame time: {:.2} ms", self.frame_time_ms));
                ui.label(format!("FPS: {:.0}", fps));

                ui.add_space(8.0);
                ui.checkbox(self.vsync, "VSync");
            }
        }
    }

    fn is_closeable(&self, _tab: &Tab) -> bool {
        false
    }

    fn allowed_in_windows(&self, _tab: &mut Tab) -> bool {
        false
    }

    fn clear_background(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Viewport)
    }

    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false]
    }
}

fn main() {
    run::<GGEditor>();
}
