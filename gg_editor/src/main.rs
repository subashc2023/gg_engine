mod camera_controller;
mod gizmo;
mod panels;
mod physics_player;
#[cfg(not(target_os = "macos"))]
mod title_bar;

use gg_engine::egui;
use gg_engine::prelude::*;
use transform_gizmo_egui::Gizmo;

use gizmo::GizmoOperation;
use physics_player::PhysicsPlayer;
use panels::content_browser::{render_dnd_ghost, ASSETS_DIR};
use panels::{EditorTabViewer, Tab};

// ---------------------------------------------------------------------------
// Scene state (edit vs play mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum SceneState {
    Edit,
    Play,
    Simulate,
}

// ---------------------------------------------------------------------------
// GGEditor
// ---------------------------------------------------------------------------

struct GGEditor {
    scene_state: SceneState,
    editor_scene: Option<Scene>,
    editor_scene_path: Option<String>,
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
    pending_texture_loads: Vec<(Entity, std::path::PathBuf)>,
    /// Old scenes awaiting GPU-safe destruction (deferred from on_egui to on_render).
    pending_drop_scenes: Vec<Scene>,
    show_physics_colliders: bool,
    should_exit: bool,
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

        // Create scene — scripting + physics demo.
        let mut scene = Scene::new();

        // -- Camera (static — editor camera handles view in edit/simulate,
        //    scene camera stays fixed in play mode) --
        let ortho_cam = scene.create_entity_with_tag("Camera");
        scene.add_component(ortho_cam, CameraComponent::default());

        // -- Native Physics Player (green, left side) --
        // Demonstrates the Rust NativeScript physics API: apply_impulse,
        // get/set_linear_velocity, has_component check.
        let native_player = scene.create_entity_with_tag("Native Player");
        scene.add_component(
            native_player,
            SpriteRendererComponent::new(Vec4::new(0.2, 0.8, 0.3, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(native_player) {
            t.translation = Vec3::new(-3.0, 0.0, 0.0);
        }
        scene.add_component(native_player, RigidBody2DComponent::default());
        scene.add_component(native_player, BoxCollider2DComponent::default());
        scene.add_component(native_player, NativeScriptComponent::bind::<PhysicsPlayer>());

        // -- Lua Physics Player (blue, right side) --
        // Demonstrates the Lua physics API: apply_impulse, get/set_linear_velocity,
        // has_component. Equivalent to the native player above.
        let lua_player = scene.create_entity_with_tag("Lua Player");
        scene.add_component(
            lua_player,
            SpriteRendererComponent::new(Vec4::new(0.2, 0.3, 0.8, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(lua_player) {
            t.translation = Vec3::new(3.0, 0.0, 0.0);
        }
        scene.add_component(lua_player, RigidBody2DComponent::default());
        scene.add_component(lua_player, BoxCollider2DComponent::default());
        #[cfg(feature = "lua-scripting")]
        scene.add_component(
            lua_player,
            LuaScriptComponent::new("assets/scripts/physics_player.lua"),
        );

        // -- Force Block (red, center) --
        // Demonstrates: apply_force, apply_impulse_at_point, get/set_angular_velocity,
        // get/set_scale. Controlled by force_block.lua.
        let force_block = scene.create_entity_with_tag("Force Block");
        scene.add_component(
            force_block,
            SpriteRendererComponent::new(Vec4::new(0.8, 0.2, 0.2, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(force_block) {
            t.translation = Vec3::new(0.0, 0.0, 0.0);
        }
        scene.add_component(force_block, RigidBody2DComponent::default());
        scene.add_component(force_block, BoxCollider2DComponent::default());
        #[cfg(feature = "lua-scripting")]
        scene.add_component(
            force_block,
            LuaScriptComponent::new("assets/scripts/force_block.lua"),
        );

        // -- Spinner (sprite, top-center) --
        // Demonstrates: get_rotation, set_rotation. No physics, pure script rotation.
        // Uses a sprite (not a circle) so rotation is visually apparent.
        let spinner = scene.create_entity_with_tag("Spinner");
        scene.add_component(
            spinner,
            SpriteRendererComponent::new(Vec4::new(0.9, 0.6, 0.1, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(spinner) {
            t.translation = Vec3::new(0.0, 3.0, 0.0);
            t.scale = Vec3::new(1.5, 0.3, 1.0); // Flat bar — rotation clearly visible
        }
        #[cfg(feature = "lua-scripting")]
        scene.add_component(
            spinner,
            LuaScriptComponent::new("assets/scripts/spinner.lua"),
        );

        // -- Bouncy Ball (orange circle, dynamic + high restitution) --
        let bouncy_ball = scene.create_entity_with_tag("Bouncy Ball");
        scene.add_component(
            bouncy_ball,
            CircleRendererComponent::new(Vec4::new(1.0, 0.5, 0.0, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(bouncy_ball) {
            t.translation = Vec3::new(-1.0, 3.0, 0.0);
            t.scale = Vec3::new(0.6, 0.6, 1.0);
        }
        scene.add_component(bouncy_ball, RigidBody2DComponent::default());
        scene.add_component(bouncy_ball, {
            let mut cc = CircleCollider2DComponent::default();
            cc.restitution = 0.9;
            cc
        });

        // -- Ground (static platform) --
        let ground = scene.create_entity_with_tag("Ground");
        scene.add_component(
            ground,
            SpriteRendererComponent::new(Vec4::new(0.4, 0.4, 0.4, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(ground) {
            t.translation = Vec3::new(0.0, -3.0, 0.0);
            t.scale = Vec3::new(20.0, 0.5, 1.0);
        }
        scene.add_component(
            ground,
            RigidBody2DComponent::new(RigidBody2DType::Static),
        );
        scene.add_component(ground, BoxCollider2DComponent::default());

        // -- Left Wall --
        let left_wall = scene.create_entity_with_tag("Left Wall");
        scene.add_component(
            left_wall,
            SpriteRendererComponent::new(Vec4::new(0.35, 0.35, 0.35, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(left_wall) {
            t.translation = Vec3::new(-8.0, 1.0, 0.0);
            t.scale = Vec3::new(0.5, 8.0, 1.0);
        }
        scene.add_component(
            left_wall,
            RigidBody2DComponent::new(RigidBody2DType::Static),
        );
        scene.add_component(left_wall, BoxCollider2DComponent::default());

        // -- Right Wall --
        let right_wall = scene.create_entity_with_tag("Right Wall");
        scene.add_component(
            right_wall,
            SpriteRendererComponent::new(Vec4::new(0.35, 0.35, 0.35, 1.0)),
        );
        if let Some(mut t) = scene.get_component_mut::<TransformComponent>(right_wall) {
            t.translation = Vec3::new(8.0, 1.0, 0.0);
            t.scale = Vec3::new(0.5, 8.0, 1.0);
        }
        scene.add_component(
            right_wall,
            RigidBody2DComponent::new(RigidBody2DType::Static),
        );
        scene.add_component(right_wall, BoxCollider2DComponent::default());

        GGEditor {
            scene_state: SceneState::Edit,
            editor_scene: None,
            editor_scene_path: None,
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
            pending_texture_loads: Vec::new(),
            pending_drop_scenes: Vec::new(),
            show_physics_colliders: false,
            should_exit: false,
        }
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEditor".into(),
            width: 1600,
            height: 900,
            decorations: cfg!(target_os = "macos"),
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

    fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn on_event(&mut self, event: &Event, input: &Input) {
        // Editor camera responds in edit and simulate modes.
        if self.scene_state != SceneState::Play {
            self.editor_camera.on_event(event);
        }

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
                // File commands — always available; stop playback/simulation first.
                KeyCode::N if ctrl => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.new_scene();
                }
                KeyCode::O if ctrl => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.open_scene();
                }
                KeyCode::S if ctrl && shift => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene_as();
                }
                KeyCode::S if ctrl && !shift => {
                    if self.scene_state != SceneState::Edit {
                        self.on_scene_stop();
                    }
                    self.save_scene();
                }

                // Entity duplication — edit mode only.
                KeyCode::D if ctrl && self.scene_state == SceneState::Edit => {
                    self.on_duplicate_entity();
                }

                // Gizmo shortcuts (Q/W/E/R) — edit mode only, no modifiers.
                KeyCode::Q if !ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.gizmo_operation = GizmoOperation::None;
                }
                KeyCode::W if !ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.gizmo_operation = GizmoOperation::Translate;
                }
                KeyCode::E if !ctrl && !shift && self.scene_state == SceneState::Edit => {
                    self.gizmo_operation = GizmoOperation::Rotate;
                }
                KeyCode::R if !ctrl && !shift && self.scene_state == SceneState::Edit => {
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

        match self.scene_state {
            SceneState::Edit => {
                // Update editor camera (orbit/pan/zoom via Alt+mouse).
                self.editor_camera.on_update(dt, input);
            }
            SceneState::Simulate => {
                // Update editor camera — simulation renders from the editor
                // camera, not the scene camera.
                self.editor_camera.on_update(dt, input);
                // Step physics (no scripts).
                self.scene.on_update_physics(dt);
            }
            SceneState::Play => {
                // Run native scripts (e.g. CameraController).
                self.scene.on_update_scripts(dt, input);
                // Run Lua scripts.
                #[cfg(feature = "lua-scripting")]
                self.scene.on_update_lua_scripts(dt, input);
                // Step physics simulation.
                self.scene.on_update_physics(dt);
            }
        }

        // Read latest pixel readback result.
        self.hovered_entity = self
            .scene_fb
            .as_ref()
            .map(|fb| fb.hovered_entity())
            .unwrap_or(-1);
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        // Drop old scenes that may hold GPU resources (textures). We must
        // wait for all in-flight GPU work to finish before destroying them,
        // since previous frames' command buffers may still reference them.
        if !self.pending_drop_scenes.is_empty() {
            renderer.wait_gpu_idle();
            self.pending_drop_scenes.clear();
        }

        // Process deferred texture loads from the properties panel.
        for (entity, path) in self.pending_texture_loads.drain(..) {
            if self.scene.is_alive(entity) {
                let texture = Ref::new(renderer.create_texture_from_file(&path));
                if let Some(mut sprite) =
                    self.scene.get_component_mut::<SpriteRendererComponent>(entity)
                {
                    sprite.texture = Some(texture);
                    sprite.texture_path = Some(path.to_string_lossy().to_string());
                }
            }
        }

        match self.scene_state {
            SceneState::Edit => {
                self.scene
                    .on_update_editor(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Simulate => {
                self.scene
                    .on_update_simulation(&self.editor_camera.view_projection(), renderer);
            }
            SceneState::Play => {
                self.scene.on_update_runtime(renderer);
            }
        }

        // -- Overlay rendering (collider visualization) --
        self.on_overlay_render(renderer);
    }

    fn on_egui(&mut self, ctx: &egui::Context, window: &Window) {
        // Sync window title with active scene name.
        let title = match &self.editor_scene_path {
            Some(path) => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                format!("GGEditor - {}", name)
            }
            None => "GGEditor".into(),
        };
        window.set_title(&title);

        // -- Title bar / Menu bar --
        #[cfg(not(target_os = "macos"))]
        {
            let play_state = match self.scene_state {
                SceneState::Edit => title_bar::PlayState::Edit,
                SceneState::Play => title_bar::PlayState::Play,
                SceneState::Simulate => title_bar::PlayState::Simulate,
            };
            let response = title_bar::title_bar_ui(ctx, window, play_state, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                        .clicked()
                    {
                        if self.scene_state != SceneState::Edit {
                            self.on_scene_stop();
                        }
                        self.new_scene();
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                        .clicked()
                    {
                        if self.scene_state != SceneState::Edit {
                            self.on_scene_stop();
                        }
                        self.open_scene();
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                        .clicked()
                    {
                        if self.scene_state != SceneState::Edit {
                            self.on_scene_stop();
                        }
                        self.save_scene();
                        ui.close();
                    }
                    if ui
                        .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                        .clicked()
                    {
                        if self.scene_state != SceneState::Edit {
                            self.on_scene_stop();
                        }
                        self.save_scene_as();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut self.show_physics_colliders, "Show Physics Colliders")
                        .clicked()
                    {
                        ui.close();
                    }
                });
            });
            if response.close_requested {
                self.should_exit = true;
            }
            if response.play_toggled {
                match self.scene_state {
                    SceneState::Edit => self.on_scene_play(),
                    SceneState::Simulate => {
                        self.on_scene_stop();
                        self.on_scene_play();
                    }
                    SceneState::Play => self.on_scene_stop(),
                }
            }
            if response.simulate_toggled {
                match self.scene_state {
                    SceneState::Edit => self.on_scene_simulate(),
                    SceneState::Play => {
                        self.on_scene_stop();
                        self.on_scene_simulate();
                    }
                    SceneState::Simulate => self.on_scene_stop(),
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let _ = window;
            egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui
                            .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                            .clicked()
                        {
                            if self.scene_state != SceneState::Edit {
                                self.on_scene_stop();
                            }
                            self.new_scene();
                            ui.close();
                        }
                        if ui
                            .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                            .clicked()
                        {
                            if self.scene_state != SceneState::Edit {
                                self.on_scene_stop();
                            }
                            self.open_scene();
                            ui.close();
                        }
                        if ui
                            .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                            .clicked()
                        {
                            if self.scene_state != SceneState::Edit {
                                self.on_scene_stop();
                            }
                            self.save_scene();
                            ui.close();
                        }
                        if ui
                            .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
                            .clicked()
                        {
                            if self.scene_state != SceneState::Edit {
                                self.on_scene_stop();
                            }
                            self.save_scene_as();
                            ui.close();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui
                            .checkbox(&mut self.show_physics_colliders, "Show Physics Colliders")
                            .clicked()
                        {
                            ui.close();
                        }
                    });
                });
            });
            // Toolbar (Play / Stop) — macOS only (Windows has it in the title bar).
            self.toolbar_ui(ctx);
        }

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
                pending_texture_loads: &mut self.pending_texture_loads,
                is_playing: self.scene_state == SceneState::Play,  // Simulate still uses editor camera + gizmos
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
// Overlay rendering (collider visualization, debug shapes)
// ---------------------------------------------------------------------------

impl GGEditor {
    fn on_overlay_render(&self, renderer: &mut Renderer) {
        // Set the appropriate camera for the overlay pass.
        match self.scene_state {
            SceneState::Play => {
                if let Some(cam_entity) = self.scene.get_primary_camera_entity() {
                    let cam = self.scene.get_component::<CameraComponent>(cam_entity);
                    let tc = self.scene.get_component::<TransformComponent>(cam_entity);
                    if let (Some(cam), Some(tc)) = (cam, tc) {
                        let vp = *cam.camera.projection() * tc.get_transform().inverse();
                        renderer.set_view_projection(vp);
                    }
                }
            }
            SceneState::Edit | SceneState::Simulate => {
                renderer.set_view_projection(self.editor_camera.view_projection());
            }
        }

        // Physics collider visualization.
        if self.show_physics_colliders {
            let collider_color = Vec4::new(0.0, 1.0, 0.0, 1.0);

            // Circle colliders.
            for (transform, cc) in self
                .scene
                .world()
                .query::<(&TransformComponent, &CircleCollider2DComponent)>()
                .iter()
            {
                let translation = Vec3::new(
                    transform.translation.x + cc.offset.x,
                    transform.translation.y + cc.offset.y,
                    transform.translation.z - 0.001,
                );
                let scale = transform.scale * cc.radius * 2.0;
                let collider_transform = Mat4::from_scale_rotation_translation(
                    Vec3::new(scale.x, scale.y, 1.0),
                    Quat::IDENTITY,
                    translation,
                );

                renderer.draw_circle(&collider_transform, collider_color, 0.01, 0.005, -1);
            }

            // Box colliders.
            for (transform, bc) in self
                .scene
                .world()
                .query::<(&TransformComponent, &BoxCollider2DComponent)>()
                .iter()
            {
                let translation = Vec3::new(
                    transform.translation.x + bc.offset.x,
                    transform.translation.y + bc.offset.y,
                    transform.translation.z - 0.001,
                );
                let scale = Vec3::new(
                    transform.scale.x * bc.size.x * 2.0,
                    transform.scale.y * bc.size.y * 2.0,
                    1.0,
                );
                let collider_transform = Mat4::from_scale_rotation_translation(
                    scale,
                    Quat::from_rotation_z(transform.rotation.z),
                    translation,
                );

                renderer.draw_rect_transform(&collider_transform, collider_color, -1);
            }
        }

        // Selected entity outline.
        if let Some(selected) = self.selection_context {
            if let Some(transform) = self.scene.get_component::<TransformComponent>(selected) {
                let outline_color = Vec4::new(1.0, 0.5, 0.0, 1.0);
                renderer.draw_rect_transform(&transform.get_transform(), outline_color, -1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Play / Stop
// ---------------------------------------------------------------------------

impl GGEditor {
    #[cfg(target_os = "macos")]
    fn toolbar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(34.0)
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0x25, 0x25, 0x26))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                // 1px bottom border line.
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.min.x, rect.max.y),
                        egui::pos2(rect.max.x, rect.max.y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3C, 0x3C, 0x3C)),
                );

                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center)
                        .with_main_justify(true),
                    |ui| {
                        ui.add_space(3.0);

                        let btn_size = egui::vec2(28.0, 28.0);
                        let spacing = 4.0;
                        let total_width = btn_size.x * 2.0 + spacing;
                        let avail = ui.available_width();
                        ui.add_space((avail - total_width) / 2.0);

                        // Play/Stop button.
                        let (play_rect, play_resp) = ui.allocate_exact_size(
                            btn_size,
                            egui::Sense::click(),
                        );
                        ui.add_space(spacing);
                        // Simulate button.
                        let (sim_rect, sim_resp) = ui.allocate_exact_size(
                            btn_size,
                            egui::Sense::click(),
                        );

                        // Paint play button.
                        if play_resp.hovered() {
                            ui.painter().rect_filled(
                                play_rect,
                                egui::CornerRadius::same(3),
                                egui::Color32::from_rgb(0x40, 0x40, 0x40),
                            );
                        }

                        let play_center = play_rect.center();
                        match self.scene_state {
                            SceneState::Edit | SceneState::Simulate => {
                                // Green play triangle.
                                let half = 7.0;
                                let points = vec![
                                    egui::pos2(play_center.x - half * 0.7, play_center.y - half),
                                    egui::pos2(play_center.x + half, play_center.y),
                                    egui::pos2(play_center.x - half * 0.7, play_center.y + half),
                                ];
                                ui.painter().add(egui::Shape::convex_polygon(
                                    points,
                                    egui::Color32::from_rgb(0x4E, 0xC9, 0x4E),
                                    egui::Stroke::NONE,
                                ));
                            }
                            SceneState::Play => {
                                // Blue stop square.
                                let half = 6.0;
                                let stop_rect = egui::Rect::from_center_size(
                                    play_center,
                                    egui::vec2(half * 2.0, half * 2.0),
                                );
                                ui.painter().rect_filled(
                                    stop_rect,
                                    egui::CornerRadius::same(2),
                                    egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
                                );
                            }
                        }

                        // Paint simulate button.
                        if sim_resp.hovered() {
                            ui.painter().rect_filled(
                                sim_rect,
                                egui::CornerRadius::same(3),
                                egui::Color32::from_rgb(0x40, 0x40, 0x40),
                            );
                        }

                        let sim_center = sim_rect.center();
                        match self.scene_state {
                            SceneState::Simulate => {
                                // Blue stop square.
                                let half = 6.0;
                                let stop_rect = egui::Rect::from_center_size(
                                    sim_center,
                                    egui::vec2(half * 2.0, half * 2.0),
                                );
                                ui.painter().rect_filled(
                                    stop_rect,
                                    egui::CornerRadius::same(2),
                                    egui::Color32::from_rgb(0x3B, 0x9C, 0xE9),
                                );
                            }
                            _ => {
                                // Gear icon.
                                paint_gear_icon(ui.painter(), sim_center, 8.0);
                            }
                        }

                        if play_resp.clicked() {
                            match self.scene_state {
                                SceneState::Edit => self.on_scene_play(),
                                SceneState::Simulate => {
                                    self.on_scene_stop();
                                    self.on_scene_play();
                                }
                                SceneState::Play => self.on_scene_stop(),
                            }
                        }
                        if sim_resp.clicked() {
                            match self.scene_state {
                                SceneState::Edit => self.on_scene_simulate(),
                                SceneState::Play => {
                                    self.on_scene_stop();
                                    self.on_scene_simulate();
                                }
                                SceneState::Simulate => self.on_scene_stop(),
                            }
                        }
                    },
                );
            });
    }

    fn on_scene_play(&mut self) {
        self.scene_state = SceneState::Play;
        let runtime_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, runtime_scene);
        self.editor_scene = Some(editor_scene);
        self.scene.on_runtime_start();
    }

    fn on_scene_simulate(&mut self) {
        self.scene_state = SceneState::Simulate;
        let sim_scene = Scene::copy(&self.scene);
        let editor_scene = std::mem::replace(&mut self.scene, sim_scene);
        self.editor_scene = Some(editor_scene);
        self.scene.on_simulation_start();
    }

    fn on_scene_stop(&mut self) {
        match self.scene_state {
            SceneState::Play => self.scene.on_runtime_stop(),
            SceneState::Simulate => self.scene.on_simulation_stop(),
            SceneState::Edit => return,
        }

        self.scene_state = SceneState::Edit;

        if let Some(editor_scene) = self.editor_scene.take() {
            let old = std::mem::replace(&mut self.scene, editor_scene);
            self.pending_drop_scenes.push(old);
            self.selection_context = None;

            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File commands (New / Open / Save As)
// ---------------------------------------------------------------------------

impl GGEditor {
    fn new_scene(&mut self) {
        let old = std::mem::replace(&mut self.scene, Scene::new());
        self.pending_drop_scenes.push(old);
        self.selection_context = None;
        self.editor_scene_path = None;

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
                let old = std::mem::replace(&mut self.scene, new_scene);
                self.pending_drop_scenes.push(old);
                self.selection_context = None;
                self.editor_scene_path = Some(path);

                let (w, h) = self.viewport_size;
                if w > 0 && h > 0 {
                    self.scene.on_viewport_resize(w, h);
                }
                self.queue_texture_loads_from_scene();
            }
        }
    }

    fn save_scene(&mut self) {
        if let Some(ref path) = self.editor_scene_path {
            SceneSerializer::serialize(&self.scene, path);
        } else {
            self.save_scene_as();
        }
    }

    fn save_scene_as(&mut self) {
        if let Some(path) = FileDialogs::save_file("GGScene files", &["ggscene"]) {
            SceneSerializer::serialize(&self.scene, &path);
            self.editor_scene_path = Some(path);
        }
    }

    fn on_duplicate_entity(&mut self) {
        if let Some(selected) = self.selection_context {
            if self.scene.is_alive(selected) {
                self.scene.duplicate_entity(selected);
            }
        }
    }

    fn open_scene_from_path(&mut self, path: &std::path::Path) {
        let path_str = path.to_string_lossy().to_string();
        let mut new_scene = Scene::new();
        if SceneSerializer::deserialize(&mut new_scene, &path_str) {
            let old = std::mem::replace(&mut self.scene, new_scene);
            self.pending_drop_scenes.push(old);
            self.selection_context = None;
            self.editor_scene_path = Some(path_str);
            let (w, h) = self.viewport_size;
            if w > 0 && h > 0 {
                self.scene.on_viewport_resize(w, h);
            }
            self.queue_texture_loads_from_scene();
        }
    }

    /// Scan the current scene for entities with `texture_path` set on their
    /// [`SpriteRendererComponent`] and queue them for deferred GPU loading.
    fn queue_texture_loads_from_scene(&mut self) {
        let entities = self.scene.each_entity_with_tag();
        for (entity, _tag) in &entities {
            if let Some(sprite) = self.scene.get_component::<SpriteRendererComponent>(*entity) {
                if let Some(ref path_str) = sprite.texture_path {
                    let path = std::path::PathBuf::from(path_str);
                    if path.exists() {
                        self.pending_texture_loads.push((*entity, path));
                    } else {
                        warn!("Texture not found: {}", path_str);
                    }
                }
            }
        }
    }
}

/// Procedural gear icon for the simulate button (macOS toolbar).
#[cfg(target_os = "macos")]
fn paint_gear_icon(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    let color = egui::Color32::from_rgb(0xCC, 0xCC, 0xCC);
    let bg = egui::Color32::from_rgb(0x25, 0x25, 0x26);
    let teeth = 6;
    let inner_r = radius * 0.55;
    let outer_r = radius;
    let tooth_width = std::f32::consts::PI / (teeth as f32 * 2.0);

    let mut points = Vec::new();
    for i in 0..teeth {
        let angle = (i as f32 / teeth as f32) * std::f32::consts::TAU;
        let a1 = angle - tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a1.cos(),
            center.y + inner_r * a1.sin(),
        ));
        let a2 = angle - tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a2.cos(),
            center.y + outer_r * a2.sin(),
        ));
        let a3 = angle + tooth_width * 0.7;
        points.push(egui::pos2(
            center.x + outer_r * a3.cos(),
            center.y + outer_r * a3.sin(),
        ));
        let a4 = angle + tooth_width * 1.5;
        points.push(egui::pos2(
            center.x + inner_r * a4.cos(),
            center.y + inner_r * a4.sin(),
        ));
    }

    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
    painter.circle_filled(center, radius * 0.25, bg);
}

fn main() {
    run::<GGEditor>();
}
