mod camera_controller;
mod gizmo;
mod panels;

use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::Gizmo;

use camera_controller::CameraController;
use gizmo::GizmoOperation;
use panels::content_browser::{render_dnd_ghost, ASSETS_DIR};
use panels::{EditorTabViewer, Tab};

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
    selection_context: Option<Entity>,
    gizmo: Gizmo,
    gizmo_operation: GizmoOperation,
    editor_camera: EditorCamera,
    hovered_entity: i32,
    current_directory: std::path::PathBuf,
    pending_open_path: Option<std::path::PathBuf>,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // Layout:
        //  ┌──────────┬──────────────┬─────────────────┐
        //  │          │              │ Scene Hierarchy  │
        //  │ Settings │   Viewport   ├─────────────────┤
        //  │          │              │   Properties    │
        //  ├──────────┴──────────────┤                  │
        //  │     Content Browser     │                  │
        //  └─────────────────────────┴─────────────────┘
        let mut dock_state = egui_dock::DockState::new(vec![Tab::Settings]);
        let surface = dock_state.main_surface_mut();
        let root = egui_dock::NodeIndex::root();
        // Right sidebar (20%) — hierarchy + properties, full height.
        let [left, right] = surface.split_right(root, 0.77, vec![Tab::SceneHierarchy]);
        surface.split_below(right, 0.5, vec![Tab::Properties]);
        // Content browser at bottom of left column (30%).
        let [top_left, _bottom_left] = surface.split_below(left, 0.7, vec![Tab::ContentBrowser]);
        // Viewport takes 80% right of the top-left area; Settings stays left (20%).
        surface.split_right(top_left, 0.20, vec![Tab::Viewport]);

        // Create scene.
        let mut scene = Scene::new();

        // Three squares for perspective vs orthographic testing.
        let left_square = scene.create_entity_with_tag("Left Square");
        scene.add_component(
            left_square,
            SpriteRendererComponent::new(Vec4::new(0.2, 0.8, 0.3, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(left_square) {
            t.translation = Vec3::new(-2.0, 0.0, 0.0);
        }

        let middle_square = scene.create_entity_with_tag("Middle Square");
        scene.add_component(
            middle_square,
            SpriteRendererComponent::new(Vec4::new(0.8, 0.2, 0.2, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(middle_square) {
            t.translation = Vec3::new(0.0, 0.0, -5.0);
            t.scale = Vec3::new(3.0, 3.0, 1.0);
        }

        let right_square = scene.create_entity_with_tag("Right Square");
        scene.add_component(
            right_square,
            SpriteRendererComponent::new(Vec4::new(0.2, 0.3, 0.8, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(right_square) {
            t.translation = Vec3::new(2.0, 0.0, 0.0);
        }

        // Orthographic Camera — primary, default ortho (size 10).
        let ortho_cam = scene.create_entity_with_tag("Orthographic Camera");
        scene.add_component(ortho_cam, CameraComponent::default());
        scene.add_component(ortho_cam, NativeScriptComponent::bind::<CameraController>());

        // Perspective Camera — secondary, pulled back on Z so all squares are visible.
        let persp_cam = scene.create_entity_with_tag("Perspective Camera");
        let mut persp_scene_camera = SceneCamera::default();
        persp_scene_camera.set_perspective(45.0_f32.to_radians(), 0.01, 1000.0);
        scene.add_component(persp_cam, CameraComponent::new(persp_scene_camera, false));
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(persp_cam) {
            t.translation = Vec3::new(0.0, 0.0, -5.0);
        }
        scene.add_component(persp_cam, NativeScriptComponent::bind::<CameraController>());

        GGEditor {
            dock_state,
            scene_fb: None,
            viewport_size: (0, 0),
            viewport_focused: false,
            viewport_hovered: false,
            vsync: true,
            frame_time_ms: 0.0,
            scene,
            selection_context: None,
            gizmo: Gizmo::default(),
            gizmo_operation: GizmoOperation::Translate,
            editor_camera: EditorCamera::new(45.0_f32.to_radians(), 0.1, 1000.0),
            hovered_entity: -1,
            current_directory: std::path::PathBuf::from(ASSETS_DIR),
            pending_open_path: None,
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
            attachments: vec![
                FramebufferTextureFormat::RGBA8.into(),
                FramebufferTextureFormat::RedInteger.into(),
                FramebufferTextureFormat::Depth.into(),
            ],
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

    fn on_event(&mut self, event: &Event, input: &Input) {
        self.editor_camera.on_event(event);

        if let Event::Key(KeyEvent::Pressed {
            key_code,
            repeat: false,
        }) = event
        {
            let ctrl =
                input.is_key_pressed(KeyCode::LeftCtrl) || input.is_key_pressed(KeyCode::RightCtrl);
            let shift = input.is_key_pressed(KeyCode::LeftShift)
                || input.is_key_pressed(KeyCode::RightShift);

            match key_code {
                // File commands.
                KeyCode::N if ctrl => self.new_scene(),
                KeyCode::O if ctrl => self.open_scene(),
                KeyCode::S if ctrl && shift => self.save_scene_as(),

                // Gizmo shortcuts (Q/W/E/R) — only when no modifier is held.
                KeyCode::Q if !ctrl && !shift => {
                    self.gizmo_operation = GizmoOperation::None;
                }
                KeyCode::W if !ctrl && !shift => {
                    self.gizmo_operation = GizmoOperation::Translate;
                }
                KeyCode::E if !ctrl && !shift => {
                    self.gizmo_operation = GizmoOperation::Rotate;
                }
                KeyCode::R if !ctrl && !shift => {
                    self.gizmo_operation = GizmoOperation::Scale;
                }
                _ => {}
            }
        }
    }

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        // Exponential moving average for stable frame time display.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        // Notify scene cameras of viewport resize.
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
            self.editor_camera.set_viewport_size(w as f32, h as f32);
        }

        // Update editor camera (orbit/pan/zoom via Alt+mouse).
        self.editor_camera.on_update(dt, input);

        // Run native scripts (e.g. CameraController on Camera A).
        self.scene.on_update_scripts(dt, input);

        // Read latest pixel readback result.
        self.hovered_entity = self
            .scene_fb
            .as_ref()
            .map(|fb| fb.hovered_entity())
            .unwrap_or(-1);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        self.scene
            .on_update_editor(&self.editor_camera.view_projection(), renderer);
    }

    fn on_egui(&mut self, ctx: &egui::Context) {
        // -- Menu bar --
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                        .clicked()
                    {
                        self.new_scene();
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                        .clicked()
                    {
                        self.open_scene();
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                        .clicked()
                    {
                        self.save_scene_as();
                        ui.close();
                    }
                });
            });
        });

        let fb_tex_id = self.scene_fb.as_ref().and_then(|fb| fb.egui_texture_id());

        let mut dock_style = egui_dock::Style::from_egui(ctx.style().as_ref());

        // Tab bar background and separator line.
        dock_style.tab_bar.bg_fill = egui::Color32::from_rgb(0x18, 0x18, 0x18);
        dock_style.tab_bar.hline_color = egui::Color32::from_rgb(0x3C, 0x3C, 0x3C);

        // Active tab — matches panel background, white text.
        dock_style.tab.active.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
        dock_style.tab.active.text_color = egui::Color32::WHITE;

        // Inactive tab — dark, dimmed text.
        dock_style.tab.inactive.bg_fill = egui::Color32::from_rgb(0x18, 0x18, 0x18);
        dock_style.tab.inactive.text_color = egui::Color32::from_rgb(0x96, 0x96, 0x96);

        // Focused tab — same as active.
        dock_style.tab.focused.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
        dock_style.tab.focused.text_color = egui::Color32::WHITE;

        // Hovered tab.
        dock_style.tab.hovered.bg_fill = egui::Color32::from_rgb(0x25, 0x25, 0x26);
        dock_style.tab.hovered.text_color = egui::Color32::WHITE;

        // Blue underline on active tab.
        dock_style.tab.hline_below_active_tab_name = true;

        // Separator colors.
        dock_style.separator.color_idle = egui::Color32::from_rgb(0x28, 0x28, 0x28);
        dock_style.separator.color_hovered = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);
        dock_style.separator.color_dragged = egui::Color32::from_rgb(0x00, 0x7A, 0xCC);

        // Tab body matches panel.
        dock_style.tab.tab_body.bg_fill = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);

        // Scope the viewer so its borrows are released before we handle
        // pending actions and paint the DnD ghost overlay.
        {
            let mut viewer = EditorTabViewer {
                scene: &mut self.scene,
                selection_context: &mut self.selection_context,
                viewport_size: &mut self.viewport_size,
                viewport_focused: &mut self.viewport_focused,
                viewport_hovered: &mut self.viewport_hovered,
                fb_tex_id,
                vsync: &mut self.vsync,
                frame_time_ms: self.frame_time_ms,
                gizmo: &mut self.gizmo,
                gizmo_operation: self.gizmo_operation,
                editor_camera: &self.editor_camera,
                scene_fb: &mut self.scene_fb,
                hovered_entity: self.hovered_entity,
                current_directory: &mut self.current_directory,
                pending_open_path: &mut self.pending_open_path,
            };

            egui_dock::DockArea::new(&mut self.dock_state)
                .style(dock_style)
                .show(ctx, &mut viewer);
        }

        // DnD ghost overlay — painted on a tooltip layer so it floats above
        // all panels and follows the cursor.
        render_dnd_ghost(ctx);

        // Handle pending scene open from content browser drag-drop.
        if let Some(path) = self.pending_open_path.take() {
            self.open_scene_from_path(&path);
        }
    }
}

// ---------------------------------------------------------------------------
// File commands (New / Open / Save As)
// ---------------------------------------------------------------------------

impl GGEditor {
    fn new_scene(&mut self) {
        self.scene = Scene::new();
        self.selection_context = None;

        // Ensure cameras get the correct viewport on the next frame.
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }
    }

    fn open_scene(&mut self) {
        if let Some(path) = FileDialogs::open_file("GGScene files", &["ggscene"]) {
            let mut new_scene = Scene::new();
            if SceneSerializer::deserialize(&mut new_scene, &path) {
                self.scene = new_scene;
                self.selection_context = None;

                let (w, h) = self.viewport_size;
                if w > 0 && h > 0 {
                    self.scene.on_viewport_resize(w, h);
                }
            }
        }
    }

    fn save_scene_as(&self) {
        if let Some(path) = FileDialogs::save_file("GGScene files", &["ggscene"]) {
            SceneSerializer::serialize(&self.scene, &path);
        }
    }

    fn open_scene_from_path(&mut self, path: &std::path::Path) {
        let path_str = path.to_string_lossy();
        let mut new_scene = Scene::new();
        if SceneSerializer::deserialize(&mut new_scene, &path_str) {
            self.scene = new_scene;
            self.selection_context = None;
            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
        }
    }
}

fn main() {
    run::<GGEditor>();
}
