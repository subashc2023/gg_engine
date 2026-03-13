use std::sync::Arc;

use gg_engine::prelude::*;
use gg_engine::renderer::skeleton::{Skeleton, SkeletalAnimationClip};

/// 3D test scene: cube, sphere, and ground plane with directional + point lighting,
/// backface culling, depth testing, material support, and directional shadow mapping.
/// Includes a skinned Fox.glb model with skeletal animation.
/// Middle-click drag to orbit, scroll to zoom.
pub struct Sandbox3D {
    cube_va: Option<VertexArray>,
    sphere_va: Option<VertexArray>,
    plane_va: Option<VertexArray>,
    cylinder_va: Option<VertexArray>,
    cone_va: Option<VertexArray>,
    torus_va: Option<VertexArray>,
    capsule_va: Option<VertexArray>,

    // Fox skeletal animation.
    fox_skeleton: Option<Arc<Skeleton>>,
    fox_clips: Vec<SkeletalAnimationClip>,
    fox_va: Option<VertexArray>,
    fox_current_clip: usize,
    fox_playback_time: f32,
    fox_speed: f32,

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

    // Shadow mapping.
    shadows_enabled: bool,
    shadow_cascade_vps: Option<[Mat4; 4]>,
    shadow_split_depths: [f32; 3],
    shadow_texel_sizes: [f32; 4],
}

impl Sandbox3D {
    pub fn new() -> Self {
        info!("Sandbox3D — mesh primitives + directional/point lighting + Fox skeletal animation");
        Self {
            cube_va: None,
            sphere_va: None,
            plane_va: None,
            cylinder_va: None,
            cone_va: None,
            torus_va: None,
            capsule_va: None,
            fox_skeleton: None,
            fox_clips: Vec::new(),
            fox_va: None,
            fox_current_clip: 0,
            fox_playback_time: 0.0,
            fox_speed: 1.0,
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
            shadows_enabled: true,
            shadow_cascade_vps: None,
            shadow_split_depths: [0.0; 3],
            shadow_texel_sizes: [1.0; 4],
        }
    }

    pub fn on_attach(&mut self, renderer: &mut Renderer) {
        // Upload built-in primitives with neutral vertex colors (lighting provides color).
        let cube = Mesh::cube([1.0, 1.0, 1.0, 1.0]);
        self.cube_va = Some(cube.upload(renderer).expect("cube upload"));

        let sphere = Mesh::sphere(32, 16, [1.0, 1.0, 1.0, 1.0]);
        self.sphere_va = Some(sphere.upload(renderer).expect("sphere upload"));

        let plane = Mesh::plane([1.0, 1.0, 1.0, 1.0]);
        self.plane_va = Some(plane.upload(renderer).expect("plane upload"));

        let cylinder = Mesh::cylinder(32, [1.0, 1.0, 1.0, 1.0]);
        self.cylinder_va = Some(cylinder.upload(renderer).expect("cylinder upload"));

        let cone = Mesh::cone(32, [1.0, 1.0, 1.0, 1.0]);
        self.cone_va = Some(cone.upload(renderer).expect("cone upload"));

        let torus = Mesh::torus(32, 16, [1.0, 1.0, 1.0, 1.0]);
        self.torus_va = Some(torus.upload(renderer).expect("torus upload"));

        let capsule = Mesh::capsule(32, 16, [1.0, 1.0, 1.0, 1.0]);
        self.capsule_va = Some(capsule.upload(renderer).expect("capsule upload"));

        // Create a default material for lit rendering.
        let handle = renderer.material_library().default_handle();
        self.material_handle = Some(handle);

        // Load the Fox.glb skinned mesh.
        let fox_path = std::path::Path::new("test_assets/Fox.glb");
        if fox_path.exists() {
            match load_gltf_skinned(fox_path) {
                Ok(skin_data) => {
                    info!(
                        "Fox loaded: {} vertices, {} bones, {} clips",
                        skin_data.mesh.vertices.len(),
                        skin_data.skeleton.joint_count(),
                        skin_data.clips.len(),
                    );
                    for (i, clip) in skin_data.clips.iter().enumerate() {
                        info!("  clip {}: \"{}\" ({:.2}s)", i, clip.name, clip.duration);
                    }
                    match skin_data.mesh.upload(renderer) {
                        Ok(va) => {
                            self.fox_va = Some(va);
                            self.fox_skeleton = Some(Arc::new(skin_data.skeleton));
                            self.fox_clips = skin_data.clips;
                        }
                        Err(e) => error!("Failed to upload Fox mesh: {e}"),
                    }
                }
                Err(e) => error!("Failed to load Fox.glb: {e}"),
            }
        } else {
            warn!("test_assets/Fox.glb not found, skipping Fox model");
        }
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

        // Advance Fox animation.
        if !self.fox_clips.is_empty() {
            let clip = &self.fox_clips[self.fox_current_clip];
            self.fox_playback_time += dt.seconds() * self.fox_speed;
            if self.fox_playback_time >= clip.duration {
                self.fox_playback_time %= clip.duration;
            }
        }
    }

    pub fn on_render_shadows(
        &mut self,
        renderer: &mut Renderer,
        cmd_buf: gg_engine::ash::vk::CommandBuffer,
        current_frame: usize,
    ) {
        if !self.shadows_enabled {
            self.shadow_cascade_vps = None;
            return;
        }

        // Initialize shadow pipeline lazily.
        if !renderer.has_shadow_pipeline() {
            if let Err(e) = renderer.init_shadow_pipeline() {
                gg_engine::log::error!("Failed to create shadow pipeline: {e}");
                self.shadow_cascade_vps = None;
                return;
            }
        }

        let light_dir = Vec3::new(-0.3, -1.0, -0.5).normalize();

        // Scene AABB covering the sandbox geometry.
        let scene_min = Vec3::new(-5.0, -1.0, -5.0);
        let scene_max = Vec3::new(5.0, 3.0, 5.0);

        // Compute per-cascade VPs fitted to the camera frustum.
        let aspect = self.window_width as f32 / self.window_height.max(1) as f32;
        let mut cam_proj = Mat4::perspective_lh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
        cam_proj.z_axis.z = 0.1 / (0.1 - 100.0);
        cam_proj.w_axis.z = 0.1 * 100.0 / (100.0 - 0.1);
        cam_proj.y_axis.y *= -1.0;
        let eye = Vec3::new(
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.sin(),
            self.orbit_dist * self.orbit_pitch.sin(),
            self.orbit_dist * self.orbit_pitch.cos() * self.orbit_yaw.cos(),
        );
        let cam_view = Mat4::look_at_lh(eye, Vec3::ZERO, Vec3::Y);
        let camera_info = gg_engine::renderer::ShadowCameraInfo {
            view_projection: cam_proj * cam_view,
            near: 0.1,
            far: 100.0,
            camera_position: eye,
            shadow_distance: 100.0,
        };
        let (cascade_vps, split_depths, _shadow_far, texel_sizes) =
            gg_engine::renderer::shadow_map::compute_cascade_vps(
                &camera_info,
                light_dir,
                scene_min,
                scene_max,
            );
        self.shadow_cascade_vps = Some(cascade_vps);
        self.shadow_split_depths = split_depths;
        self.shadow_texel_sizes = texel_sizes;

        // Mesh transforms for shadow submission.
        let mesh_models: Vec<Mat4> = vec![
            // Ground plane.
            Mat4::from_scale_rotation_translation(
                Vec3::new(10.0, 1.0, 10.0),
                Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            ),
            // Cube.
            Mat4::from_translation(Vec3::ZERO),
            // Sphere.
            Mat4::from_scale_rotation_translation(
                Vec3::splat(1.5),
                Quat::IDENTITY,
                Vec3::new(2.0, 0.25, 0.0),
            ),
            // Cylinder.
            Mat4::from_scale_rotation_translation(
                Vec3::new(0.8, 1.2, 0.8),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.1, 2.0),
            ),
            // Cone.
            Mat4::from_translation(Vec3::new(0.0, 0.0, 2.0)),
            // Torus.
            Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                Quat::IDENTITY,
                Vec3::new(2.0, 0.3, 2.0),
            ),
            // Capsule.
            Mat4::from_scale_rotation_translation(
                Vec3::new(1.5, 2.0, 1.5),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.5, -2.0),
            ),
        ];
        let mesh_vas: Vec<Option<&VertexArray>> = vec![
            self.plane_va.as_ref(),
            self.cube_va.as_ref(),
            self.sphere_va.as_ref(),
            self.cylinder_va.as_ref(),
            self.cone_va.as_ref(),
            self.torus_va.as_ref(),
            self.capsule_va.as_ref(),
        ];

        // Pre-compute Fox bone pose + upload for shadow pass.
        let fox_bone_offset = if let (Some(ref skeleton), Some(ref _va)) =
            (&self.fox_skeleton, &self.fox_va)
        {
            if let Err(e) = renderer.ensure_bone_palette() {
                gg_engine::log::error!("Failed to init bone palette for Fox shadow: {e}");
                None
            } else {
                let clip = &self.fox_clips[self.fox_current_clip];
                let pose = skeleton.compute_pose(clip, self.fox_playback_time);
                renderer.write_bone_matrices(&pose.matrices)
            }
        } else {
            None
        };

        // Initialize skinned shadow pipeline if Fox is present.
        if fox_bone_offset.is_some() {
            if let Err(e) = renderer.init_skinned_shadow_pipeline() {
                gg_engine::log::error!("Failed to create skinned shadow pipeline: {e}");
            }
        }

        let fox_model = Mat4::from_scale_rotation_translation(
            Vec3::splat(0.02),
            Quat::IDENTITY,
            Vec3::new(-2.0, -0.5, 0.0),
        );

        for (cascade, cascade_vp) in cascade_vps.iter().enumerate() {
            renderer.begin_shadow_pass(cascade_vp, cascade, cmd_buf, current_frame, 0, false);
            for (va_opt, model) in mesh_vas.iter().zip(&mesh_models) {
                if let Some(va) = va_opt {
                    renderer.submit_shadow(va, model, cmd_buf);
                }
            }

            // Skinned Fox shadow.
            if let (Some(bone_offset), Some(ref fox_va)) = (fox_bone_offset, &self.fox_va) {
                renderer.bind_skinned_shadow_pipeline(cmd_buf);
                renderer.submit_skinned_shadow_with_pipeline(
                    fox_va,
                    cascade_vp,
                    &fox_model,
                    bone_offset,
                    cmd_buf,
                );
            }

            renderer.end_shadow_pass(cmd_buf);
        }
    }

    pub fn on_render(&mut self, renderer: &mut Renderer) {
        let pipeline = match renderer.mesh3d_pipeline() {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to get mesh3d pipeline: {e}");
                return;
            }
        };

        let aspect = self.window_width as f32 / self.window_height.max(1) as f32;
        let mut proj = Mat4::perspective_lh(45.0_f32.to_radians(), aspect, 0.1, 100.0);
        // Reverse-Z: near→1, far→0 for better depth precision at distance.
        proj.z_axis.z = 0.1 / (0.1 - 100.0);
        proj.w_axis.z = 0.1 * 100.0 / (100.0 - 0.1);
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
            shadow_cascade_vps: self.shadow_cascade_vps,
            cascade_split_depths: self.shadow_split_depths,
            shadow_distance: 100.0,
            cascade_texel_sizes: self.shadow_texel_sizes,
        };
        renderer.upload_lights(&light_env);

        let mat_handle = self.material_handle.as_ref();

        // Bind shared descriptor sets once before all 3D draws.
        renderer.bind_3d_shared_sets(&pipeline);

        // Ground plane (scaled up).
        if let Some(va) = &self.plane_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(10.0, 1.0, 10.0),
                Quat::IDENTITY,
                Vec3::new(0.0, -0.5, 0.0),
            );
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Cube.
        if let Some(va) = &self.cube_va {
            let model = Mat4::from_translation(Vec3::ZERO);
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Sphere.
        if let Some(va) = &self.sphere_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(1.5),
                Quat::IDENTITY,
                Vec3::new(2.0, 0.25, 0.0),
            );
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Cylinder.
        if let Some(va) = &self.cylinder_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(0.8, 1.2, 0.8),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.1, 2.0),
            );
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Cone.
        if let Some(va) = &self.cone_va {
            let model = Mat4::from_translation(Vec3::new(0.0, 0.0, 2.0));
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Torus.
        if let Some(va) = &self.torus_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::splat(2.0),
                Quat::IDENTITY,
                Vec3::new(2.0, 0.3, 2.0),
            );
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Capsule.
        if let Some(va) = &self.capsule_va {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(1.5, 2.0, 1.5),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.5, -2.0),
            );
            renderer.submit_3d(&pipeline, va, &model, mat_handle, -1);
        }

        // Skinned Fox.
        if let (Some(ref skeleton), Some(ref fox_va)) = (&self.fox_skeleton, &self.fox_va) {
            // Compute bone pose.
            let clip = &self.fox_clips[self.fox_current_clip];
            let pose = skeleton.compute_pose(clip, self.fox_playback_time);

            // Ensure bone palette is ready.
            if let Err(e) = renderer.ensure_bone_palette() {
                error!("Failed to init bone palette: {e}");
                return;
            }
            if let Some(bone_offset) = renderer.write_bone_matrices(&pose.matrices) {
                let skinned_pipeline = match renderer.skinned_mesh3d_pipeline() {
                    Ok(p) => p,
                    Err(e) => {
                        error!("Failed to create skinned pipeline: {e}");
                        return;
                    }
                };
                renderer.bind_skinned_3d_shared_sets(&skinned_pipeline);

                // Fox model: scale down (Fox.glb is ~100 units tall) and place it.
                let fox_model = Mat4::from_scale_rotation_translation(
                    Vec3::splat(0.02),
                    Quat::IDENTITY,
                    Vec3::new(-2.0, -0.5, 0.0),
                );
                renderer.submit_skinned_3d(
                    &skinned_pipeline,
                    fox_va,
                    &fox_model,
                    mat_handle,
                    -1,
                    bone_offset,
                );
            }
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
            ui.checkbox(&mut self.shadows_enabled, "Shadows");
            ui.separator();
            ui.label(format!(
                "Yaw {:.1}\u{00b0}  Pitch {:.1}\u{00b0}  Dist {:.1}",
                self.orbit_yaw.to_degrees(),
                self.orbit_pitch.to_degrees(),
                self.orbit_dist,
            ));
            ui.label(format!("Eye: ({:.2}, {:.2}, {:.2})", eye.x, eye.y, eye.z));

            // Fox animation controls.
            if !self.fox_clips.is_empty() {
                ui.separator();
                ui.label("Fox Animation");
                let clip_names: Vec<String> = self
                    .fox_clips
                    .iter()
                    .enumerate()
                    .map(|(i, c)| format!("{}: {} ({:.1}s)", i, c.name, c.duration))
                    .collect();
                gg_engine::egui::ComboBox::from_label("Clip")
                    .selected_text(&clip_names[self.fox_current_clip])
                    .show_ui(ui, |ui| {
                        for (i, name) in clip_names.iter().enumerate() {
                            if ui
                                .selectable_value(&mut self.fox_current_clip, i, name)
                                .changed()
                            {
                                self.fox_playback_time = 0.0;
                            }
                        }
                    });
                ui.add(
                    gg_engine::egui::Slider::new(&mut self.fox_speed, 0.0..=3.0).text("Speed"),
                );
                let clip = &self.fox_clips[self.fox_current_clip];
                ui.label(format!(
                    "Time: {:.2} / {:.2}s",
                    self.fox_playback_time, clip.duration
                ));
                ui.label(format!(
                    "Bones: {}",
                    self.fox_skeleton.as_ref().map(|s| s.joint_count()).unwrap_or(0)
                ));
            } else if self.fox_va.is_none() {
                ui.separator();
                ui.label("Fox.glb not loaded (missing test_assets/Fox.glb)");
            }
        });
    }
}
