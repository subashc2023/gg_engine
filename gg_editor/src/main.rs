use gg_engine::egui;
use gg_engine::glam::EulerRot;
use gg_engine::prelude::*;
use transform_gizmo_egui::math::{DQuat, DVec3, Transform as GizmoTransform};
use transform_gizmo_egui::{EnumSet, Gizmo, GizmoConfig, GizmoExt, GizmoMode, GizmoOrientation};

// ---------------------------------------------------------------------------
// CameraController native script — WASD movement demo
// ---------------------------------------------------------------------------

struct CameraController {
    speed: f32,
}

impl Default for CameraController {
    fn default() -> Self {
        Self { speed: 5.0 }
    }
}

impl NativeScript for CameraController {
    fn on_create(&mut self, entity: Entity, _scene: &mut Scene) {
        info!("CameraController created (entity {})", entity.id());
    }

    fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {
        if let Some(mut transform) = scene.get_component_mut::<TransformComponent>(entity) {
            let speed = self.speed * dt.seconds();
            if input.is_key_pressed(KeyCode::A) {
                transform.translation.x -= speed;
            }
            if input.is_key_pressed(KeyCode::D) {
                transform.translation.x += speed;
            }
            if input.is_key_pressed(KeyCode::W) {
                transform.translation.y += speed;
            }
            if input.is_key_pressed(KeyCode::S) {
                transform.translation.y -= speed;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tab {
    SceneHierarchy,
    Viewport,
    Properties,
    Settings,
}

// ---------------------------------------------------------------------------
// Gizmo operation modes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum GizmoOperation {
    None,      // Q — select mode, no gizmo
    Translate, // W
    Rotate,    // E
    Scale,     // R
}

fn gizmo_modes_for(op: GizmoOperation) -> EnumSet<GizmoMode> {
    match op {
        GizmoOperation::None => EnumSet::empty(),
        GizmoOperation::Translate => {
            GizmoMode::TranslateX
                | GizmoMode::TranslateY
                | GizmoMode::TranslateZ
                | GizmoMode::TranslateXY
                | GizmoMode::TranslateXZ
                | GizmoMode::TranslateYZ
        }
        GizmoOperation::Rotate => GizmoMode::RotateX | GizmoMode::RotateY | GizmoMode::RotateZ,
        GizmoOperation::Scale => {
            GizmoMode::ScaleX | GizmoMode::ScaleY | GizmoMode::ScaleZ | GizmoMode::ScaleUniform
        }
    }
}

/// Convert a glam Mat4 (f32) to a row-major f64 array for the gizmo library.
///
/// GizmoConfig stores matrices as `mint::RowMatrix4<f64>`.  The `From<[[f64;4];4]>`
/// impl for RowMatrix4 treats the outer arrays as **rows**, so we must supply
/// rows, not columns.  `transpose().to_cols_array_2d()` gives us exactly that
/// (columns of M^T = rows of M).
fn mat4_to_f64(m: &Mat4) -> [[f64; 4]; 4] {
    m.transpose()
        .to_cols_array_2d()
        .map(|row| row.map(|v| v as f64))
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
    selection_context: Option<Entity>,
    gizmo: Gizmo,
    gizmo_operation: GizmoOperation,
    editor_camera: EditorCamera,
    hovered_entity: i32,
}

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");

        // Layout:
        //  ┌──────────────────┬──────────────────┐
        //  │                  │  Scene Hierarchy  │
        //  │     Viewport     ├──────────────────┤
        //  ├──────────────────┤    Properties     │
        //  │     Settings     │                   │
        //  └──────────────────┴──────────────────┘
        let mut dock_state = egui_dock::DockState::new(vec![Tab::Viewport]);
        let surface = dock_state.main_surface_mut();
        let root = egui_dock::NodeIndex::root();
        // Right sidebar (25%) for hierarchy + properties.
        let [left, right] = surface.split_right(root, 0.75, vec![Tab::SceneHierarchy]);
        // Right sidebar: hierarchy top (50%), properties bottom (50%).
        surface.split_below(right, 0.5, vec![Tab::Properties]);
        // Left column: viewport top (80%), settings bottom (20%).
        surface.split_below(left, 0.8, vec![Tab::Settings]);

        // Create scene.
        let mut scene = Scene::new();

        // Three squares for perspective vs orthographic testing.
        // Left: small, close (z=0). Middle: bigger, further away (z=-5). Right: small, close (z=0).
        // In orthographic they'll look different sizes; in perspective the middle one
        // should appear roughly the same size as the others due to distance.
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
        };

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

        egui_dock::DockArea::new(&mut self.dock_state)
            .style(dock_style)
            .show(ctx, &mut viewer);
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
}

// ---------------------------------------------------------------------------
// TabViewer implementation
// ---------------------------------------------------------------------------

struct EditorTabViewer<'a> {
    scene: &'a mut Scene,
    selection_context: &'a mut Option<Entity>,
    viewport_size: &'a mut (u32, u32),
    viewport_focused: &'a mut bool,
    viewport_hovered: &'a mut bool,
    fb_tex_id: Option<egui::TextureId>,
    vsync: &'a mut bool,
    frame_time_ms: f32,
    gizmo: &'a mut Gizmo,
    gizmo_operation: GizmoOperation,
    editor_camera: &'a EditorCamera,
    scene_fb: &'a mut Option<Framebuffer>,
    hovered_entity: i32,
}

impl EditorTabViewer<'_> {
    fn unfocus_viewport_on_click(&mut self, ui: &egui::Ui) {
        let clicked = ui.input(|i| i.pointer.any_pressed());
        if clicked && ui.ui_contains_pointer() {
            *self.viewport_focused = false;
        }
    }
}

impl egui_dock::TabViewer for EditorTabViewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::SceneHierarchy => "Scene Hierarchy".into(),
            Tab::Viewport => "Viewport".into(),
            Tab::Properties => "Properties".into(),
            Tab::Settings => "Settings".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            // ---------------------------------------------------------
            // Scene Hierarchy — list all entities, support selection
            // ---------------------------------------------------------
            Tab::SceneHierarchy => {
                self.unfocus_viewport_on_click(ui);

                let entities = self.scene.each_entity_with_tag();
                let mut entity_to_delete = None;

                for (entity, tag) in &entities {
                    let selected = self.selection_context.is_some_and(|sel| sel == *entity);
                    let response = ui.selectable_label(selected, tag);
                    if response.clicked() {
                        *self.selection_context = Some(*entity);
                    }
                    // Right-click on entity → delete.
                    response.context_menu(|ui| {
                        if ui.button("Delete Entity").clicked() {
                            entity_to_delete = Some(*entity);
                            ui.close();
                        }
                    });
                }

                // Click on blank space to deselect.
                let remaining = ui.available_rect_before_wrap();
                if remaining.width() > 0.0 && remaining.height() > 0.0 {
                    let response = ui.allocate_rect(remaining, egui::Sense::click());
                    if response.clicked() {
                        *self.selection_context = None;
                    }
                    // Right-click on blank space → create entity.
                    response.context_menu(|ui| {
                        if ui.button("Create Empty Entity").clicked() {
                            self.scene.create_entity_with_tag("Empty Entity");
                            ui.close();
                        }
                    });
                }

                // Deferred entity deletion.
                if let Some(entity) = entity_to_delete {
                    if *self.selection_context == Some(entity) {
                        *self.selection_context = None;
                    }
                    let _ = self.scene.destroy_entity(entity);
                }
            }

            // ---------------------------------------------------------
            // Viewport
            // ---------------------------------------------------------
            Tab::Viewport => {
                let available = ui.available_size();
                if available.x > 0.0 && available.y > 0.0 {
                    // Scale by DPI so the framebuffer renders at physical
                    // pixel resolution (crisp on high-DPI displays).
                    let ppp = ui.ctx().pixels_per_point();
                    *self.viewport_size = ((available.x * ppp) as u32, (available.y * ppp) as u32);
                }

                *self.viewport_hovered = ui.ui_contains_pointer();

                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && *self.viewport_hovered {
                    *self.viewport_focused = true;

                    // Mouse picking — select entity on left click.
                    let left_click =
                        ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
                    let alt_held = ui.input(|i| i.modifiers.alt);
                    if left_click && !self.gizmo.is_focused() && !alt_held {
                        if self.hovered_entity >= 0 {
                            *self.selection_context = self
                                .scene
                                .find_entity_by_id(self.hovered_entity as u32)
                                .filter(|e| self.scene.is_alive(*e));
                        } else {
                            *self.selection_context = None;
                        }
                    }
                }

                let viewport_rect = if let Some(tex_id) = self.fb_tex_id {
                    let size = egui::vec2(available.x, available.y);
                    let response = ui.image(egui::load::SizedTexture::new(tex_id, size));
                    Some(response.rect)
                } else {
                    None
                };

                // -- Mouse picking: schedule pixel readback --
                if *self.viewport_hovered {
                    if let Some(viewport_rect) = viewport_rect {
                        if let Some(pos) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                            let ppp = ui.ctx().pixels_per_point();
                            let mx = ((pos.x - viewport_rect.min.x) * ppp) as i32;
                            let my = ((pos.y - viewport_rect.min.y) * ppp) as i32;

                            if mx >= 0
                                && my >= 0
                                && mx < self.viewport_size.0 as i32
                                && my < self.viewport_size.1 as i32
                            {
                                if let Some(fb) = self.scene_fb.as_mut() {
                                    fb.schedule_pixel_readback(1, mx, my);
                                }
                            }
                        }
                    }
                }

                // -- Gizmos --
                if let Some(viewport_rect) = viewport_rect {
                    if let Some(entity) = *self.selection_context {
                        if self.scene.is_alive(entity)
                            && self.gizmo_operation != GizmoOperation::None
                        {
                            // Use the editor camera for gizmo view/projection.
                            let camera_view = *self.editor_camera.view_matrix();
                            // Undo Vulkan Y-flip for the gizmo library.
                            let mut camera_projection = *self.editor_camera.projection();
                            camera_projection.y_axis.y *= -1.0;

                            // Read entity transform.
                            let entity_transform = {
                                let tc = self.scene.get_component::<TransformComponent>(entity);
                                tc.map(|tc| {
                                    let original_rotation = tc.rotation;
                                    let quat = Quat::from_euler(
                                        EulerRot::XYZ,
                                        tc.rotation.x,
                                        tc.rotation.y,
                                        tc.rotation.z,
                                    );
                                    (tc.translation, quat, tc.scale, original_rotation)
                                })
                            };

                            if let Some((translation, quat, scale, original_rotation)) =
                                entity_transform
                            {
                                // Snapping: Ctrl held enables snap.
                                let snapping = ui.input(|i| i.modifiers.ctrl);

                                // Configure the gizmo.
                                self.gizmo.update_config(GizmoConfig {
                                    view_matrix: mat4_to_f64(&camera_view).into(),
                                    projection_matrix: mat4_to_f64(&camera_projection).into(),
                                    viewport: viewport_rect,
                                    modes: gizmo_modes_for(self.gizmo_operation),
                                    orientation: GizmoOrientation::Local,
                                    snapping,
                                    snap_angle: std::f32::consts::FRAC_PI_4, // 45 degrees
                                    snap_distance: 0.5_f32,
                                    snap_scale: 0.5_f32,
                                    ..Default::default()
                                });

                                // Build gizmo Transform from entity data.
                                let gizmo_transform =
                                    GizmoTransform::from_scale_rotation_translation(
                                        DVec3::new(scale.x as f64, scale.y as f64, scale.z as f64),
                                        DQuat::from_xyzw(
                                            quat.x as f64,
                                            quat.y as f64,
                                            quat.z as f64,
                                            quat.w as f64,
                                        ),
                                        DVec3::new(
                                            translation.x as f64,
                                            translation.y as f64,
                                            translation.z as f64,
                                        ),
                                    );

                                // Interact (renders gizmo + returns new transforms).
                                if let Some((_result, new_transforms)) =
                                    self.gizmo.interact(ui, &[gizmo_transform])
                                {
                                    if let Some(new_t) = new_transforms.first() {
                                        // Read back translation & scale from mint types.
                                        let new_translation = Vec3::new(
                                            new_t.translation.x as f32,
                                            new_t.translation.y as f32,
                                            new_t.translation.z as f32,
                                        );
                                        let new_scale = Vec3::new(
                                            new_t.scale.x as f32,
                                            new_t.scale.y as f32,
                                            new_t.scale.z as f32,
                                        );

                                        // Rotation: use delta approach to avoid
                                        // gimbal lock snapping.
                                        let new_quat = Quat::from_xyzw(
                                            new_t.rotation.v.x as f32,
                                            new_t.rotation.v.y as f32,
                                            new_t.rotation.v.z as f32,
                                            new_t.rotation.s as f32,
                                        );
                                        let (nx, ny, nz) = new_quat.to_euler(EulerRot::XYZ);
                                        let (ox, oy, oz) = quat.to_euler(EulerRot::XYZ);
                                        let delta_rotation = Vec3::new(nx - ox, ny - oy, nz - oz);
                                        let new_rotation = original_rotation + delta_rotation;

                                        // Write back to component.
                                        if let Some(mut tc) = self
                                            .scene
                                            .get_component_mut::<TransformComponent>(entity)
                                        {
                                            tc.translation = new_translation;
                                            tc.rotation = new_rotation;
                                            tc.scale = new_scale;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---------------------------------------------------------
            // Properties — component inspector for selected entity
            // ---------------------------------------------------------
            Tab::Properties => {
                self.unfocus_viewport_on_click(ui);

                if let Some(entity) = *self.selection_context {
                    if self.scene.is_alive(entity) {
                        draw_components(ui, self.scene, entity);
                    } else {
                        *self.selection_context = None;
                    }
                }
            }

            // ---------------------------------------------------------
            // Settings
            // ---------------------------------------------------------
            Tab::Settings => {
                self.unfocus_viewport_on_click(ui);

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

                ui.add_space(8.0);
                ui.heading("Mouse Picking");
                ui.separator();
                let hovered_name = if self.hovered_entity >= 0 {
                    self.scene
                        .find_entity_by_id(self.hovered_entity as u32)
                        .and_then(|e| {
                            self.scene
                                .get_component::<TagComponent>(e)
                                .map(|tag| tag.tag.clone())
                        })
                        .unwrap_or_else(|| format!("Entity({})", self.hovered_entity))
                } else {
                    "None".to_string()
                };
                ui.label(format!("Hovered Entity: {}", hovered_name));
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

// ---------------------------------------------------------------------------
// Vec3 control (colored XYZ drag values with reset buttons)
// ---------------------------------------------------------------------------

/// Draw a labeled Vec3 control with colored X/Y/Z buttons that reset to
/// `reset_value` on click. `column_width` sets the label column width.
/// Returns `true` if any value changed.
fn draw_vec3_control(
    ui: &mut egui::Ui,
    label: &str,
    values: &mut Vec3,
    reset_value: f32,
    column_width: f32,
) -> bool {
    let mut changed = false;

    ui.push_id(label, |ui| {
        // Compute sizes based on current line height.
        let line_height =
            ui.text_style_height(&egui::TextStyle::Body) + 2.0 * ui.spacing().button_padding.y;
        let button_size = egui::vec2(line_height + 3.0, line_height);

        ui.horizontal(|ui| {
            // Fixed-width label — takes exactly column_width, left-aligned.
            let (_, label_resp) =
                ui.allocate_exact_size(egui::vec2(column_width, line_height), egui::Sense::hover());
            ui.painter().text(
                label_resp.rect.left_center(),
                egui::Align2::LEFT_CENTER,
                label,
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().text_color(),
            );

            ui.spacing_mut().item_spacing.x = 0.0;

            let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

            // Compute a fixed width for each DragValue so all 3 groups fit.
            let spacing = 4.0 * 2.0; // two 4px gaps between XYZ groups
            let available = ui.available_width() - spacing - 3.0 * button_size.x;
            let drag_width = (available / 3.0).max(20.0);

            // --- X (red) ---
            let x_color = egui::Color32::from_rgba_unmultiplied(204, 26, 38, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("X")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family.clone())),
                    )
                    .fill(x_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.x = reset_value;
                changed = true;
            }

            // Drag value for X.
            let drag_x = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.x)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_x.changed() {
                changed = true;
            }

            ui.add_space(4.0);

            // --- Y (green) ---
            let y_color = egui::Color32::from_rgba_unmultiplied(47, 153, 47, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Y")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family.clone())),
                    )
                    .fill(y_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.y = reset_value;
                changed = true;
            }

            let drag_y = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.y)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_y.changed() {
                changed = true;
            }

            ui.add_space(4.0);

            // --- Z (blue) ---
            let z_color = egui::Color32::from_rgba_unmultiplied(20, 64, 204, 255);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Z")
                            .color(egui::Color32::WHITE)
                            .font(egui::FontId::new(14.0, bold_family)),
                    )
                    .fill(z_color)
                    .min_size(button_size)
                    .corner_radius(egui::CornerRadius::same(2)),
                )
                .clicked()
            {
                values.z = reset_value;
                changed = true;
            }

            let drag_z = ui.add_sized(
                [drag_width, button_size.y],
                egui::DragValue::new(&mut values.z)
                    .speed(0.1)
                    .custom_formatter(|n, _| format!("{n:.2}"))
                    .update_while_editing(false),
            );
            if drag_z.changed() {
                changed = true;
            }
        });
    });

    changed
}

// ---------------------------------------------------------------------------
// Component inspector
// ---------------------------------------------------------------------------

fn draw_components(ui: &mut egui::Ui, scene: &mut Scene, entity: Entity) {
    let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

    // -- Tag Component + Add Component button (inline) --
    if scene.has_component::<TagComponent>(entity) {
        let mut tag = scene
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_default();

        ui.horizontal(|ui| {
            if ui.text_edit_singleline(&mut tag).changed() {
                if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
                    tc.tag = tag;
                }
            }

            let add_btn = ui.add(
                egui::Button::new(
                    egui::RichText::new("Add")
                        .color(egui::Color32::WHITE)
                        .font(egui::FontId::new(12.0, bold_family.clone())),
                )
                .fill(egui::Color32::from_rgb(0x00, 0x7A, 0xCC))
                .corner_radius(egui::CornerRadius::same(2)),
            );

            egui::Popup::from_toggle_button_response(&add_btn).show(|ui| {
                if !scene.has_component::<CameraComponent>(entity) && ui.button("Camera").clicked()
                {
                    scene.add_component(entity, CameraComponent::default());
                }
                if !scene.has_component::<SpriteRendererComponent>(entity)
                    && ui.button("Sprite Renderer").clicked()
                {
                    scene.add_component(entity, SpriteRendererComponent::default());
                }
            });
        });
        ui.separator();
    }

    // -- Transform Component (not removable) --
    if scene.has_component::<TransformComponent>(entity) {
        egui::CollapsingHeader::new(
            egui::RichText::new("Transform").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("transform", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let (mut translation, mut rotation_deg, mut scale) = {
                let tc = scene.get_component::<TransformComponent>(entity).unwrap();
                (
                    tc.translation,
                    Vec3::new(
                        tc.rotation.x.to_degrees(),
                        tc.rotation.y.to_degrees(),
                        tc.rotation.z.to_degrees(),
                    ),
                    tc.scale,
                )
            };

            let mut changed = false;
            changed |= draw_vec3_control(ui, "Translate", &mut translation, 0.0, 70.0);
            changed |= draw_vec3_control(ui, "Rotate", &mut rotation_deg, 0.0, 70.0);
            changed |= draw_vec3_control(ui, "Scale", &mut scale, 1.0, 70.0);

            if changed {
                if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(entity) {
                    tc.translation = translation;
                    tc.rotation = Vec3::new(
                        rotation_deg.x.to_radians(),
                        rotation_deg.y.to_radians(),
                        rotation_deg.z.to_radians(),
                    );
                    tc.scale = scale;
                }
            }
        });
    }

    // -- Camera Component (removable) --
    let mut remove_camera = false;
    if scene.has_component::<CameraComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Camera").font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("camera", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            // Read all camera state up front.
            let (
                mut primary,
                mut fixed_aspect,
                mut proj_type,
                mut ortho_size,
                mut ortho_near,
                mut ortho_far,
                mut persp_fov_deg,
                mut persp_near,
                mut persp_far,
            ) = {
                let cam = scene.get_component::<CameraComponent>(entity).unwrap();
                (
                    cam.primary,
                    cam.fixed_aspect_ratio,
                    cam.camera.projection_type(),
                    cam.camera.orthographic_size(),
                    cam.camera.orthographic_near(),
                    cam.camera.orthographic_far(),
                    cam.camera.perspective_vertical_fov().to_degrees(),
                    cam.camera.perspective_near(),
                    cam.camera.perspective_far(),
                )
            };

            let mut changed = false;

            // Primary camera toggle — uses set_primary_camera to ensure
            // only one camera is primary at a time.
            if ui.checkbox(&mut primary, "Primary").changed() {
                if primary {
                    scene.set_primary_camera(entity);
                } else if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.primary = false;
                }
            }

            // Projection type combo box.
            let proj_type_strings = ["Perspective", "Orthographic"];
            let current_label = proj_type_strings[proj_type as usize];
            egui::ComboBox::from_label("Projection")
                .selected_text(current_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut proj_type,
                            ProjectionType::Perspective,
                            proj_type_strings[0],
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    if ui
                        .selectable_value(
                            &mut proj_type,
                            ProjectionType::Orthographic,
                            proj_type_strings[1],
                        )
                        .changed()
                    {
                        changed = true;
                    }
                });

            // Projection-type-specific controls.
            match proj_type {
                ProjectionType::Perspective => {
                    ui.horizontal(|ui| {
                        ui.label("Vertical FOV");
                        if ui
                            .add(
                                egui::DragValue::new(&mut persp_fov_deg)
                                    .speed(0.1)
                                    .range(1.0..=179.0)
                                    .suffix("°"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near");
                        if ui
                            .add(
                                egui::DragValue::new(&mut persp_near)
                                    .speed(0.01)
                                    .range(0.001..=f32::MAX),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far");
                        if ui
                            .add(egui::DragValue::new(&mut persp_far).speed(1.0))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                }
                ProjectionType::Orthographic => {
                    ui.horizontal(|ui| {
                        ui.label("Size");
                        if ui
                            .add(
                                egui::DragValue::new(&mut ortho_size)
                                    .speed(0.1)
                                    .range(0.1..=1000.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Near");
                        if ui
                            .add(egui::DragValue::new(&mut ortho_near).speed(0.1))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Far");
                        if ui
                            .add(egui::DragValue::new(&mut ortho_far).speed(0.1))
                            .changed()
                        {
                            changed = true;
                        }
                    });
                }
            }

            // Fixed aspect ratio (applies to both projection types).
            changed |= ui
                .checkbox(&mut fixed_aspect, "Fixed Aspect Ratio")
                .changed();

            // Write back all changes.
            if changed {
                if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity) {
                    cam.fixed_aspect_ratio = fixed_aspect;

                    if cam.camera.projection_type() != proj_type {
                        cam.camera.set_projection_type(proj_type);
                    }

                    // Perspective parameters.
                    let new_fov_rad = persp_fov_deg.to_radians();
                    if (cam.camera.perspective_vertical_fov() - new_fov_rad).abs() > f32::EPSILON {
                        cam.camera.set_perspective_vertical_fov(new_fov_rad);
                    }
                    if (cam.camera.perspective_near() - persp_near).abs() > f32::EPSILON {
                        cam.camera.set_perspective_near(persp_near);
                    }
                    if (cam.camera.perspective_far() - persp_far).abs() > f32::EPSILON {
                        cam.camera.set_perspective_far(persp_far);
                    }

                    // Orthographic parameters.
                    if (cam.camera.orthographic_size() - ortho_size).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_size(ortho_size);
                    }
                    if (cam.camera.orthographic_near() - ortho_near).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_near(ortho_near);
                    }
                    if (cam.camera.orthographic_far() - ortho_far).abs() > f32::EPSILON {
                        cam.camera.set_orthographic_far(ortho_far);
                    }
                }
            }
        });

        // Right-click header to remove.
        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_camera = true;
                ui.close();
            }
        });
    }
    if remove_camera {
        scene.remove_component::<CameraComponent>(entity);
    }

    // -- Sprite Renderer Component (removable) --
    let mut remove_sprite = false;
    if scene.has_component::<SpriteRendererComponent>(entity) {
        let cr = egui::CollapsingHeader::new(
            egui::RichText::new("Sprite Renderer")
                .font(egui::FontId::new(14.0, bold_family.clone())),
        )
        .id_salt(("sprite_renderer", entity.id()))
        .default_open(true)
        .show(ui, |ui| {
            let mut color_arr = {
                let sprite = scene
                    .get_component::<SpriteRendererComponent>(entity)
                    .unwrap();
                [
                    sprite.color.x,
                    sprite.color.y,
                    sprite.color.z,
                    sprite.color.w,
                ]
            };

            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                (color_arr[0] * 255.0) as u8,
                (color_arr[1] * 255.0) as u8,
                (color_arr[2] * 255.0) as u8,
                (color_arr[3] * 255.0) as u8,
            );

            ui.horizontal(|ui| {
                ui.label("Color");
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut egui_color,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .changed()
                {
                    let [r, g, b, a] = egui_color.to_srgba_unmultiplied();
                    color_arr = [
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                        a as f32 / 255.0,
                    ];
                    if let Some(mut sprite) =
                        scene.get_component_mut::<SpriteRendererComponent>(entity)
                    {
                        sprite.color = Vec4::from(color_arr);
                    }
                }
            });
        });

        // Right-click header to remove.
        cr.header_response.context_menu(|ui| {
            if ui.button("Remove Component").clicked() {
                remove_sprite = true;
                ui.close();
            }
        });
    }
    if remove_sprite {
        scene.remove_component::<SpriteRendererComponent>(entity);
    }
}

fn main() {
    run::<GGEditor>();
}
