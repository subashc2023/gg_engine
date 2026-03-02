use gg_engine::egui;
use gg_engine::prelude::*;

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

    fn on_update(&mut self, dt: Timestep, input: &Input) {
        // Exponential moving average for stable frame time display.
        self.frame_time_ms = self.frame_time_ms * 0.95 + dt.millis() * 0.05;

        // Notify scene cameras of viewport resize.
        let (w, h) = self.viewport_size;
        if w > 0 && h > 0 {
            self.scene.on_viewport_resize(w, h);
        }

        // Run native scripts (e.g. CameraController on Camera A).
        self.scene.on_update_scripts(dt, input);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        self.scene.on_update(renderer);
    }

    fn on_egui(&mut self, ctx: &egui::Context) {
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
                    *self.viewport_size = (available.x as u32, available.y as u32);
                }

                *self.viewport_hovered = ui.ui_contains_pointer();

                let clicked = ui.input(|i| i.pointer.any_pressed());
                if clicked && *self.viewport_hovered {
                    *self.viewport_focused = true;
                }

                if let Some(tex_id) = self.fb_tex_id {
                    let size = egui::vec2(available.x, available.y);
                    ui.image(egui::load::SizedTexture::new(tex_id, size));
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
        ui.columns(2, |columns| {
            columns[0].set_width(column_width);
            columns[0].label(label);

            let col = &mut columns[1];

            // Compute sizes based on current line height.
            let line_height = col.text_style_height(&egui::TextStyle::Body)
                + 2.0 * col.spacing().button_padding.y;
            let button_size = egui::vec2(line_height + 3.0, line_height);

            col.spacing_mut().item_spacing.x = 0.0;

            col.horizontal(|ui| {
                let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

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
                let drag_x = ui.add(
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

                let drag_y = ui.add(
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

                let drag_z = ui.add(
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
    });

    changed
}

// ---------------------------------------------------------------------------
// Component inspector
// ---------------------------------------------------------------------------

fn draw_components(ui: &mut egui::Ui, scene: &mut Scene, entity: Entity) {
    // -- Tag Component (editable entity name) --
    if scene.has_component::<TagComponent>(entity) {
        let mut tag = scene
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_default();
        if ui.text_edit_singleline(&mut tag).changed() {
            if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
                tc.tag = tag;
            }
        }
        ui.separator();
    }

    let bold_family = egui::FontFamily::Name(BOLD_FONT.into());

    // -- Transform Component (not removable) --
    if scene.has_component::<TransformComponent>(entity) {
        egui::CollapsingHeader::new(
            egui::RichText::new("Transform")
                .font(egui::FontId::new(14.0, bold_family.clone())),
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
                changed |= draw_vec3_control(ui, "Translation", &mut translation, 0.0, 100.0);
                changed |= draw_vec3_control(ui, "Rotation", &mut rotation_deg, 0.0, 100.0);
                changed |= draw_vec3_control(ui, "Scale", &mut scale, 1.0, 100.0);

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
            egui::RichText::new("Camera")
                .font(egui::FontId::new(14.0, bold_family.clone())),
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
                    } else if let Some(mut cam) = scene.get_component_mut::<CameraComponent>(entity)
                    {
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
                        if (cam.camera.perspective_vertical_fov() - new_fov_rad).abs()
                            > f32::EPSILON
                        {
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

        // Settings button (right-aligned on the header line).
        draw_component_settings_button(ui, &cr.header_response, || remove_camera = true);
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

        // Settings button (right-aligned on the header line).
        draw_component_settings_button(ui, &cr.header_response, || remove_sprite = true);
    }
    if remove_sprite {
        scene.remove_component::<SpriteRendererComponent>(entity);
    }

    // -- Add Component button (full-width, blue accent) --
    ui.add_space(8.0);
    let popup_id = ui.make_persistent_id("add_component_popup");
    let add_btn = ui.add_sized(
        [ui.available_width(), 0.0],
        egui::Button::new(
            egui::RichText::new("Add Component")
                .color(egui::Color32::WHITE)
                .font(egui::FontId::new(14.0, bold_family)),
        )
        .fill(egui::Color32::from_rgb(0x00, 0x7A, 0xCC)),
    );
    if add_btn.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    if egui::Popup::is_id_open(ui.ctx(), popup_id) {
        let area_response = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .default_pos(add_btn.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if !scene.has_component::<CameraComponent>(entity)
                        && ui.button("Camera").clicked()
                    {
                        scene.add_component(entity, CameraComponent::default());
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                    if !scene.has_component::<SpriteRendererComponent>(entity)
                        && ui.button("Sprite Renderer").clicked()
                    {
                        scene.add_component(entity, SpriteRendererComponent::default());
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                });
            });
        if area_response.response.clicked_elsewhere() {
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
    }
}

/// Draw a small "+" button right-aligned on a component header line.
/// Clicking it opens a popup with "Remove Component".
fn draw_component_settings_button(
    ui: &mut egui::Ui,
    header_response: &egui::Response,
    mut on_remove: impl FnMut(),
) {
    let header_rect = header_response.rect;
    let btn_size = egui::vec2(20.0, 20.0);
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(
            ui.max_rect().right() - btn_size.x - 4.0,
            header_rect.min.y + (header_rect.height() - btn_size.y) / 2.0,
        ),
        btn_size,
    );
    let settings_btn = ui.put(btn_rect, egui::Button::new("+"));

    let popup_id = header_response.id.with("component_settings");
    if settings_btn.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }
    if egui::Popup::is_id_open(ui.ctx(), popup_id) {
        let area_response = egui::Area::new(popup_id)
            .order(egui::Order::Foreground)
            .default_pos(settings_btn.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if ui.button("Remove Component").clicked() {
                        on_remove();
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                });
            });
        if area_response.response.clicked_elsewhere() {
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
    }
}

fn main() {
    run::<GGEditor>();
}
