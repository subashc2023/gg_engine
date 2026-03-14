use gg_engine::prelude::*;

const ENTITY_COUNT: usize = 2000;
const HIERARCHY_DEPTH: usize = 3;
const CHILDREN_PER_GROUP: usize = 4;
const ANIMATED_FRACTION: f32 = 0.5; // 50% of entities get animators
const TRACE_CAPTURE_FRAMES: u32 = 300; // ~5 seconds at 60fps

pub struct JobsStress {
    scene: Scene,
    frame_count: u32,
    trace_started: bool,
    trace_done: bool,
    last_dt: f32,
}

impl Application for JobsStress {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("JobsStress: creating {} entities...", ENTITY_COUNT);
        let mut scene = Scene::new();
        build_stress_scene(&mut scene, ENTITY_COUNT);
        info!("JobsStress: scene ready");

        JobsStress {
            scene,
            frame_count: 0,
            trace_started: false,
            trace_done: false,
            last_dt: 0.0,
        }
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "Jobs Stress Test".into(),
            width: 1280,
            height: 720,
            ..Default::default()
        }
    }

    fn present_mode(&self) -> PresentMode {
        PresentMode::Immediate // Uncapped FPS for stress testing
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        if let Event::Window(WindowEvent::Resize { width, height }) = event {
            if *width > 0 && *height > 0 {
                self.scene.on_viewport_resize(*width, *height);
            }
        }
    }

    fn on_update(&mut self, dt: Timestep, _input: &Input) {
        profile_scope!("JobsStress::on_update");
        self.last_dt = dt.seconds();

        // Animation tick — exercises parallel animation update.
        self.scene.on_update_animations(dt.seconds());
    }

    fn on_render(&mut self, renderer: &mut Renderer) {
        profile_scope!("JobsStress::on_render");
        self.frame_count += 1;

        // First frame: set viewport size, log worker count.
        if self.frame_count == 1 {
            self.scene.on_viewport_resize(1280, 720);
            info!(
                "JobsStress: {} worker threads active",
                gg_engine::jobs::worker_count()
            );
        }

        // Start trace capture after a few warm-up frames.
        if self.frame_count == 30 && !self.trace_started {
            info!(
                "JobsStress: starting trace capture for {} frames...",
                TRACE_CAPTURE_FRAMES
            );
            begin_session("jobs_stress", "gg_jobs_stress.json");
            self.trace_started = true;
        }

        // Stop trace capture.
        if self.trace_started && !self.trace_done && self.frame_count >= 30 + TRACE_CAPTURE_FRAMES {
            end_session();
            self.trace_done = true;
            info!(
                "JobsStress: trace capture complete! Open gg_jobs_stress.json in chrome://tracing"
            );
        }

        // Render the scene — exercises parallel world transform cache,
        // parallel frustum culling, and parallel sort.
        self.scene.on_update_runtime(renderer);
    }

    fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        _window: &gg_engine::winit::window::Window,
    ) {
        let dt_ms = self.last_dt * 1000.0;
        let fps = if self.last_dt > 0.0 {
            1.0 / self.last_dt
        } else {
            0.0
        };

        let profile_results = drain_profile_results();
        let culling = self.scene.culling_stats();

        gg_engine::egui::Window::new("Jobs Stress Test").show(ctx, |ui| {
            ui.label(format!("{:.2} ms ({:.0} FPS)", dt_ms, fps));
            ui.label(format!("Frame: {}", self.frame_count));
            ui.label(format!("Workers: {}", gg_engine::jobs::worker_count()));
            ui.separator();

            ui.label(format!("Entities: {}", ENTITY_COUNT));
            ui.label(format!(
                "Culling: {} rendered / {} culled / {} total",
                culling.rendered, culling.culled, culling.total_cullable
            ));

            if self.trace_started && !self.trace_done {
                let remaining = (30 + TRACE_CAPTURE_FRAMES).saturating_sub(self.frame_count);
                ui.colored_label(
                    gg_engine::egui::Color32::RED,
                    format!("RECORDING TRACE... {} frames left", remaining),
                );
            } else if self.trace_done {
                ui.colored_label(
                    gg_engine::egui::Color32::GREEN,
                    "Trace saved to gg_jobs_stress.json",
                );
            } else {
                ui.label("Warming up...");
            }

            ui.separator();
            ui.strong("Profile Timers");
            for result in &profile_results {
                ui.label(format!("  {}: {:.3} ms", result.name, result.time_ms));
            }
        });
    }
}

/// Build a scene with many entities arranged in a grid with hierarchy.
fn build_stress_scene(scene: &mut Scene, count: usize) {
    // Create a camera entity with a large orthographic view.
    let cam = scene.create_entity_with_tag("Camera");
    let mut cam_comp = CameraComponent::default();
    cam_comp.camera.set_orthographic_size(60.0); // Large enough to see many entities
    scene.add_component(cam, cam_comp);

    // Compute grid dimensions.
    let cols = (count as f32).sqrt().ceil() as usize;
    let spacing = 1.2_f32;

    let mut entity_index = 0usize;
    let mut group_roots: Vec<Entity> = Vec::new();

    while entity_index < count {
        // Create a group root.
        let root = scene.create_entity_with_tag("Root");
        let col = entity_index % cols;
        let row = entity_index / cols;
        let x = col as f32 * spacing * (CHILDREN_PER_GROUP as f32 + 1.0)
            - (cols as f32 * spacing * 0.5);
        let y = row as f32 * spacing * 2.0 - (count as f32 / cols as f32 * spacing);

        if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(root) {
            tc.translation = Vec3::new(x, y, 0.0);
            tc.scale = Vec3::splat(0.4);
        }

        // Colored quad.
        let hue = (entity_index as f32 / count as f32) * 360.0;
        let color = hue_to_rgb(hue);
        scene.add_component(
            root,
            SpriteRendererComponent {
                color,
                ..Default::default()
            },
        );

        // Optionally add animator.
        if should_animate(entity_index) {
            add_animator(scene, root, entity_index);
        }

        entity_index += 1;

        // Create children in a small hierarchy.
        let mut parent = root;
        for depth in 0..HIERARCHY_DEPTH {
            let children_count = CHILDREN_PER_GROUP.min(count - entity_index);
            if children_count == 0 {
                break;
            }

            for c in 0..children_count {
                let child = scene.create_entity_with_tag("Child");

                // Position children around their parent.
                let angle = (c as f32 / children_count as f32) * std::f32::consts::TAU;
                let radius = 0.8 + depth as f32 * 0.3;
                let cx = angle.cos() * radius;
                let cy = angle.sin() * radius;
                let z = (depth as f32 + 1.0) * 0.01; // slight z offset per depth

                if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(child) {
                    tc.translation = Vec3::new(cx, cy, z);
                    tc.scale = Vec3::splat(0.7); // Each depth level slightly smaller
                }

                let child_hue = ((entity_index as f32 / count as f32) * 360.0 + 60.0) % 360.0;
                scene.add_component(
                    child,
                    SpriteRendererComponent {
                        color: hue_to_rgb(child_hue),
                        ..Default::default()
                    },
                );

                if should_animate(entity_index) {
                    add_animator(scene, child, entity_index);
                }

                scene.set_parent(child, parent, false);
                entity_index += 1;

                if entity_index >= count {
                    break;
                }
            }

            // Next depth: first child becomes parent.
            if let Some(first_child_uuid) = scene.get_children(parent).first().copied() {
                if let Some(first_child) = scene.find_entity_by_uuid(first_child_uuid) {
                    parent = first_child;
                }
            } else {
                break;
            }
        }

        group_roots.push(root);
    }

    info!(
        "Built stress scene: {} entities, {} groups, {} hierarchy depth",
        entity_index,
        group_roots.len(),
        HIERARCHY_DEPTH
    );
}

fn should_animate(index: usize) -> bool {
    (index as f32 / ENTITY_COUNT as f32) < ANIMATED_FRACTION
}

fn add_animator(scene: &mut Scene, entity: Entity, index: usize) {
    let fps = 8.0 + (index % 12) as f32 * 2.0;
    let frame_count = 4 + (index % 8) as u32;
    let looping = !index.is_multiple_of(5); // 80% looping, 20% non-looping

    let mut anim = SpriteAnimatorComponent::default();
    anim.cell_size = Vec2::new(32.0, 32.0);
    anim.columns = frame_count;
    anim.clips = vec![AnimationClip {
        name: "anim".to_string(),
        start_frame: 0,
        end_frame: frame_count - 1,
        fps,
        looping,
        texture_handle: gg_engine::uuid::Uuid::from_raw(0),
        texture: None,
        events: Vec::new(),
    }];
    anim.default_clip = if looping {
        "anim".to_string()
    } else {
        String::new()
    };
    anim.speed_scale = 0.5 + (index % 10) as f32 * 0.2;

    scene.add_component(entity, anim);

    // Start playing.
    if let Some(mut anim) = scene.get_component_mut::<SpriteAnimatorComponent>(entity) {
        anim.play("anim");
    }
}

/// Simple HSV to RGB (s=0.7, v=0.9).
fn hue_to_rgb(hue: f32) -> Vec4 {
    let h = hue % 360.0;
    let s = 0.7f32;
    let v = 0.9f32;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    Vec4::new(r + m, g + m, b + m, 1.0)
}
