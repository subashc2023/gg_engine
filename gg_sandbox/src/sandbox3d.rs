use std::sync::Arc;

use gg_engine::prelude::*;
use gg_engine::renderer::Pipeline;

/// 3D test scene: cube, sphere, and ground plane with directional + point lighting,
/// backface culling, depth testing, and material support.
/// Middle-click drag to orbit, scroll to zoom.
pub struct Sandbox3D {
    pipeline: Option<Arc<Pipeline>>,
    cube_va: Option<VertexArray>,
    sphere_va: Option<VertexArray>,
    plane_va: Option<VertexArray>,

    // Material handle for the default material used in lighting.
    material_handle: Option<MaterialHandle>,

    // Smoothed camera values (used for rendering).
    orbit_yaw: f32,
    orbit_pitch: f32,
    orbit_dist: f32,
    // Target values (updated instantly from input, smoothed toward).
    target_yaw: f32,
    target_pitch: f32,
    target_dist: f32,

    window_width: u32,
    window_height: u32,
    last_dt: f32,
    elapsed: f32,
}

impl Sandbox3D {
    pub fn new() -> Self {
        info!("Sandbox3D — mesh primitives + directional/point lighting");
        Self {
            pipeline: None,
            cube_va: None,
            sphere_va: None,
            plane_va: None,
            material_handle: None,
            orbit_yaw: std::f32::consts::PI,
            orbit_pitch: 0.4,
            orbit_dist: 5.0,
            target_yaw: std::f32::consts::PI,
            target_pitch: 0.4,
            target_dist: 5.0,
            window_width: 1280,
            window_height: 720,
            last_dt: 0.0,
            elapsed: 0.0,
        }
    }

    pub fn on_attach(&mut self, renderer: &mut Renderer) {
        let shader = renderer
            .create_shader(
                "mesh3d",
                gg_engine::shaders::MESH3D_VERT_SPV,
                gg_engine::shaders::MESH3D_FRAG_SPV,
            )
            .expect("Failed to create mesh3d shader");

        let vertex_layout = Mesh::vertex_layout();

        let pipeline = renderer
            .create_3d_pipeline(
                &shader,
                &vertex_layout,
                CullMode::Back,
                DepthConfig::STANDARD_3D,
                BlendMode::Opaque,
                1,
                MsaaSamples::S1,
            )
            .expect("Failed to create 3D pipeline");
        self.pipeline = Some(pipeline);

        // Upload built-in primitives with neutral vertex colors (lighting provides color).
        let cube = Mesh::cube([1.0, 1.0, 1.0, 1.0]);
        self.cube_va = Some(cube.upload(renderer).expect("cube upload"));

        let sphere = Mesh::sphere(32, 16, [1.0, 1.0, 1.0, 1.0]);
        self.sphere_va = Some(sphere.upload(renderer).expect("sphere upload"));

        let plane = Mesh::plane([1.0, 1.0, 1.0, 1.0]);
        self.plane_va = Some(plane.upload(renderer).expect("plane upload"));

        // Create a default material for lit rendering.
        let handle = renderer.material_library().default_handle();
        self.material_handle = Some(handle);
    }

    pub fn clear_color(&self) -> [f32; 4] {
        [0.05, 0.05, 0.08, 1.0]
    }

    pub fn on_event(&mut self, event: &Event, _input: &Input) {
        if let Event::Window(WindowEvent::Resize { width, height }) = event {
            if *width > 0 && *height > 0 {
                self.window_width = *width;
                self.window_height = *height;
            }
        }
        if let Event::Mouse(MouseEvent::Scrolled { y_offset, .. }) = event {
            self.target_dist = (self.target_dist - *y_offset as f32 * 0.5).clamp(1.0, 20.0);
        }
    }

    pub fn on_update(&mut self, dt: Timestep, input: &Input) {
        self.last_dt = dt.seconds();
        self.elapsed += dt.seconds();

        if input.is_mouse_button_pressed(MouseButton::Middle) {
            let (dx, dy) = input.mouse_delta();
            let sensitivity = 0.005;
            self.target_yaw += dx as f32 * sensitivity;
            self.target_pitch = (self.target_pitch + dy as f32 * sensitivity).clamp(-1.5, 1.5);
        }

        // Frame-rate independent exponential smoothing.
        let t = 1.0 - (-dt.seconds() * 30.0).exp();
        self.orbit_yaw += (self.target_yaw - self.orbit_yaw) * t;
        self.orbit_pitch += (self.target_pitch - self.orbit_pitch) * t;
        self.orbit_dist += (self.target_dist - self.orbit_dist) * t;
    }

    pub fn on_render(&mut self, renderer: &mut Renderer) {
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => return,
        };

        let aspect = self.window_width as f32 / self.window_height.max(1) as f32;
        let mut proj = Mat4::perspective_lh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
        proj.y_axis.y *= -1.0;

        let eye = Vec3::new(
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.sin(),
            self.orbit_dist * self.orbit_pitch.sin(),
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.cos(),
        );
        let view = Mat4::look_at_lh(eye, Vec3::ZERO, Vec3::Y);
        renderer.set_view_projection(proj * view);
        renderer.set_camera_position(eye);

        // Set up lighting: directional sun + orbiting point light.
        let point_light_pos = Vec3::new(3.0 * self.elapsed.sin(), 1.5, 3.0 * self.elapsed.cos());

        let light_env = LightEnvironment {
            directional: Some((
                Vec3::new(-0.3, -1.0, -0.5), // direction
                Vec3::ONE,                   // white color
                0.8,                         // intensity
            )),
            point_lights: vec![(
                point_light_pos,          // position
                Vec3::new(1.0, 0.4, 0.1), // warm orange
                3.0,                      // intensity
                8.0,                      // radius
            )],
            ambient_color: Vec3::new(0.05, 0.05, 0.08),
            ambient_intensity: 1.0,
            camera_position: eye,
        };
        renderer.upload_lights(&light_env);

        let mat_handle = self.material_handle.as_ref();

        // Ground plane (scaled up).
        if let Some(va) = &self.plane_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(6.0, 1.0, 6.0),
                Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            );
            renderer.submit_3d(pipeline, va, &model, mat_handle, -1);
        }

        // Cube.
        if let Some(va) = &self.cube_va {
            let model = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0));
            renderer.submit_3d(pipeline, va, &model, mat_handle, -1);
        }

        // Sphere.
        if let Some(va) = &self.sphere_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(1.5),
                Quat::IDENTITY,
                Vec3::new(2.0, 0.25, 0.0),
            );
            renderer.submit_3d(pipeline, va, &model, mat_handle, -1);
        }
    }

    pub fn on_egui(
        &mut self,
        ctx: &gg_engine::egui::Context,
        _window: &gg_engine::winit::window::Window,
    ) {
        let fps = if self.last_dt > 0.0 {
            1.0 / self.last_dt
        } else {
            0.0
        };
        let eye = Vec3::new(
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.sin(),
            self.orbit_dist * self.orbit_pitch.sin(),
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.cos(),
        );

        gg_engine::egui::Window::new("Sandbox 3D").show(ctx, |ui| {
            ui.label(format!("{:.1} FPS", fps));
            ui.separator();
            ui.label("Middle-click drag: orbit  |  Scroll: zoom");
            ui.separator();
            ui.label("Directional light (sun) + orbiting point light (warm)");
            ui.label("Blinn-Phong shading with material UBO");
            ui.separator();
            ui.label(format!(
                "Yaw {:.1}\u{00b0}  Pitch {:.1}\u{00b0}  Dist {:.1}",
                self.orbit_yaw.to_degrees(),
                self.orbit_pitch.to_degrees(),
                self.orbit_dist,
            ));
            ui.label(format!("Eye: ({:.2}, {:.2}, {:.2})", eye.x, eye.y, eye.z));
        });
    }
}
