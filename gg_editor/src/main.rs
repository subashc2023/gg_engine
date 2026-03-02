use gg_engine::egui;
use gg_engine::prelude::*;

// ---------------------------------------------------------------------------
// Tab identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tab {
    Viewport,
    Properties,
    Settings,
}

// ---------------------------------------------------------------------------
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    dock_state: egui_dock::DockState<Tab>,
    scene_fb: Option<Framebuffer>,
    viewport_size: (u32, u32),
    viewport_focused: bool,
    viewport_hovered: bool,
    vsync: bool,
    frame_time_ms: f32,
    scene: Scene,
    square_entity: Entity,
    camera_entity: Entity,
    second_camera_entity: Entity,
    square_color: [f32; 4],
    camera_transform: [f32; 3],
    primary_camera: bool, // true = Camera A, false = Camera B
    second_camera_ortho_size: f32,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // Initial layout: Viewport on the left (70%), right column (30%)
        // split into Properties (top) and Settings (bottom).
        let mut dock_state = egui_dock::DockState::new(vec![Tab::Viewport]);
        let surface = dock_state.main_surface_mut();
        let root = egui_dock::NodeIndex::root();
        let [_viewport, right] = surface.split_right(root, 0.7, vec![Tab::Properties]);
        surface.split_below(right, 0.6, vec![Tab::Settings]);

        // Create scene.
        let mut scene = Scene::new();

        // Green square entity.
        let square_entity = scene.create_entity_with_tag("Green Square");
        let green = Vec4::new(0.2, 0.8, 0.3, 1.0);
        scene.add_component(square_entity, SpriteRendererComponent::new(green));

        // Camera A — primary, default orthographic (size 10).
        let camera_entity = scene.create_entity_with_tag("Camera A");
        scene.add_component(camera_entity, CameraComponent::default());

        // Camera B — clip space camera (secondary).
        let second_camera_entity = scene.create_entity_with_tag("Clip Space Camera");
        scene.add_component(
            second_camera_entity,
            CameraComponent::new(SceneCamera::default(), false),
        );

        GGEditor {
            dock_state,
            scene_fb: None,
            viewport_size: (0, 0),
            viewport_focused: false,
            viewport_hovered: false,
            vsync: true,
            frame_time_ms: 0.0,
            scene,
            square_entity,
            camera_entity,
            second_camera_entity,
            square_color: [green.x, green.y, green.z, green.w],
            camera_transform: [0.0, 0.0, 0.0],
            primary_camera: true,
            second_camera_ortho_size: 10.0,
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

    fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::Fifo
        } else {
            PresentMode::Immediate
        }
    }

    fn block_events(&self) -> bool {
        !(self.viewport_focused && self.viewport_hovered)
    }

    fn on_event(&mut self, _event: &Event, _input: &Input) {}

    fn on_update(&mut self, dt: Timestep, _input: &Input) {
        // Exponential moving average for stable frame time display.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        // Notify scene cameras of viewport resize.
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        self.scene.on_update(renderer);
    }

    fn on_egui(&mut self, ctx: &egui::Context) {
        // Gather state the TabViewer needs.
        let fb_tex_id = self
            .scene_fb
            .as_ref()
            .and_then(|fb| fb.egui_texture_id());

        // Read entity tag from the ECS (clone to avoid borrow conflicts).
        let entity_tag = self
            .scene
            .get_component::<TagComponent>(self.square_entity)
            .map(|t| t.tag.clone())
            .unwrap_or_default();

        let mut viewer = EditorTabViewer {
            viewport_size: &mut self.viewport_size,
            viewport_focused: &mut self.viewport_focused,
            viewport_hovered: &mut self.viewport_hovered,
            fb_tex_id,
            vsync: &mut self.vsync,
            frame_time_ms: self.frame_time_ms,
            square_color: &mut self.square_color,
            entity_tag: &entity_tag,
            camera_transform: &mut self.camera_transform,
            primary_camera: &mut self.primary_camera,
            second_camera_ortho_size: &mut self.second_camera_ortho_size,
        };

        egui_dock::DockArea::new(&mut self.dock_state)
            .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
            .show(ctx, &mut viewer);

        // Write the (possibly edited) color back into the ECS component.
        if let Some(mut sprite) = self
            .scene
            .get_component_mut::<SpriteRendererComponent>(self.square_entity)
        {
            sprite.color = Vec4::from(self.square_color);
        }

        // Write Camera A's transform back from the UI controls.
        if let Some(mut transform) = self
            .scene
            .get_component_mut::<TransformComponent>(self.camera_entity)
        {
            transform.transform = Mat4::from_translation(Vec3::from(self.camera_transform));
        }

        // Sync primary flag to the camera entities.
        if let Some(mut cam) = self
            .scene
            .get_component_mut::<CameraComponent>(self.camera_entity)
        {
            cam.primary = self.primary_camera;
        }
        if let Some(mut cam) = self
            .scene
            .get_component_mut::<CameraComponent>(self.second_camera_entity)
        {
            cam.primary = !self.primary_camera;

            // Sync orthographic size from UI.
            let current = cam.camera.orthographic_size();
            if (current - self.second_camera_ortho_size).abs() > f32::EPSILON {
                cam.camera.set_orthographic_size(self.second_camera_ortho_size);
            }
        }
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
    square_color: &'a mut [f32; 4],
    entity_tag: &'a str,
    camera_transform: &'a mut [f32; 3],
    primary_camera: &'a mut bool,
    second_camera_ortho_size: &'a mut f32,
}

impl egui_dock::TabViewer for EditorTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Viewport => "Viewport".into(),
            Tab::Properties => "Properties".into(),
            Tab::Settings => "Settings".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Viewport => {
                let available = ui.available_size();
                // Guard against negative or zero sizes (egui can report
                // negative available regions during minimize/Win+D).
                if available.x > 0.0 && available.y > 0.0 {
                    *self.viewport_size = (available.x as u32, available.y as u32);
                }

                // Hovered: mouse is over the viewport panel right now.
                *self.viewport_hovered = ui.ui_contains_pointer();

                // Focused: click-based, persists until another panel is clicked.
                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && *self.viewport_hovered {
                    *self.viewport_focused = true;
                }

                if let Some(tex_id) = self.fb_tex_id {
                    let size = egui::vec2(available.x, available.y);
                    ui.image(egui::load::SizedTexture::new(tex_id, size));
                }
            }
            Tab::Properties => {
                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && ui.ui_contains_pointer() {
                    *self.viewport_focused = false;
                }

                // -- Square entity --
                ui.heading(self.entity_tag);
                ui.separator();

                let mut color = egui::Color32::from_rgba_unmultiplied(
                    (self.square_color[0] * 255.0) as u8,
                    (self.square_color[1] * 255.0) as u8,
                    (self.square_color[2] * 255.0) as u8,
                    (self.square_color[3] * 255.0) as u8,
                );
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut color,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .changed()
                {
                    let [r, g, b, a] = color.to_srgba_unmultiplied();
                    self.square_color[0] = r as f32 / 255.0;
                    self.square_color[1] = g as f32 / 255.0;
                    self.square_color[2] = b as f32 / 255.0;
                    self.square_color[3] = a as f32 / 255.0;
                }

                ui.add_space(12.0);

                // -- Camera controls --
                ui.heading("Camera");
                ui.separator();
                ui.checkbox(self.primary_camera, "Camera A");
                ui.label(if *self.primary_camera {
                    "Rendering through Camera A"
                } else {
                    "Rendering through Clip Space Camera"
                });

                ui.add_space(8.0);
                ui.label("Camera A Transform:");
                ui.horizontal(|ui| {
                    ui.label("X");
                    ui.add(egui::DragValue::new(&mut self.camera_transform[0]).speed(0.1));
                    ui.label("Y");
                    ui.add(egui::DragValue::new(&mut self.camera_transform[1]).speed(0.1));
                    ui.label("Z");
                    ui.add(egui::DragValue::new(&mut self.camera_transform[2]).speed(0.1));
                });

                ui.add_space(8.0);
                ui.label("Second Camera Ortho Size:");
                ui.add(egui::DragValue::new(self.second_camera_ortho_size).speed(0.1));
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
