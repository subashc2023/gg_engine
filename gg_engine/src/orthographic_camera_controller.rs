use glam::Vec3;

use crate::events::{Event, KeyCode, MouseEvent, WindowEvent};
use crate::input::Input;
use crate::profiling::ProfileTimer;
use crate::renderer::OrthographicCamera;
use crate::timestep::Timestep;

/// Wraps an [`OrthographicCamera`] with WASD movement, Q/E rotation, mouse
/// scroll zooming, and automatic window-resize handling.
///
/// This is a convenience controller for development and editor use — real games
/// will likely implement their own camera behaviour. The translation speed
/// automatically scales with the zoom level so movement feels natural at any
/// zoom.
pub struct OrthographicCameraController {
    aspect_ratio: f32,
    zoom_level: f32,
    camera: OrthographicCamera,

    rotation_enabled: bool,
    camera_position: Vec3,
    camera_rotation: f32,

    camera_translation_speed: f32,
    camera_rotation_speed: f32,
}

impl OrthographicCameraController {
    /// Create a new controller.
    ///
    /// `aspect_ratio` — width / height (e.g. 1280.0 / 720.0).
    /// `rotation` — enable Q/E rotation controls.
    pub fn new(aspect_ratio: f32, rotation: bool) -> Self {
        let zoom_level = 1.0;
        let camera = OrthographicCamera::new(
            -aspect_ratio * zoom_level,
            aspect_ratio * zoom_level,
            -zoom_level,
            zoom_level,
        );

        Self {
            aspect_ratio,
            zoom_level,
            camera,
            rotation_enabled: rotation,
            camera_position: Vec3::ZERO,
            camera_rotation: 0.0,
            camera_translation_speed: 5.0,
            camera_rotation_speed: 180.0,
        }
    }

    // -- Per-frame update (input polling) --------------------------------------

    /// Call each frame from `Application::on_update`. Polls WASD/QE for
    /// movement and rotation. Translation speed scales with the zoom level.
    pub fn on_update(&mut self, dt: Timestep, input: &Input) {
        let _timer = ProfileTimer::new("CameraController::on_update");
        let mut changed = false;

        // Translation speed proportional to zoom so it feels consistent.
        let speed = self.camera_translation_speed * self.zoom_level;

        if input.is_key_pressed(KeyCode::A) {
            self.camera_position.x -= speed * dt;
            changed = true;
        } else if input.is_key_pressed(KeyCode::D) {
            self.camera_position.x += speed * dt;
            changed = true;
        }

        if input.is_key_pressed(KeyCode::W) {
            self.camera_position.y += speed * dt;
            changed = true;
        } else if input.is_key_pressed(KeyCode::S) {
            self.camera_position.y -= speed * dt;
            changed = true;
        }

        if self.rotation_enabled {
            if input.is_key_pressed(KeyCode::Q) {
                self.camera_rotation += (self.camera_rotation_speed * dt).to_radians();
                changed = true;
            } else if input.is_key_pressed(KeyCode::E) {
                self.camera_rotation -= (self.camera_rotation_speed * dt).to_radians();
                changed = true;
            }
        }

        if changed {
            self.camera.set_position(self.camera_position);
            self.camera.set_rotation(self.camera_rotation);
        }
    }

    // -- Event handling (scroll, resize) ---------------------------------------

    /// Call from `Application::on_event`. Handles mouse scroll (zoom) and
    /// window resize (aspect ratio).
    pub fn on_event(&mut self, event: &Event) {
        let _timer = ProfileTimer::new("CameraController::on_event");
        match event {
            Event::Mouse(MouseEvent::Scrolled { y_offset, .. }) => {
                self.on_mouse_scrolled(*y_offset);
            }
            Event::Window(WindowEvent::Resize { width, height }) => {
                self.on_window_resized(*width, *height);
            }
            _ => {}
        }
    }

    // -- Getters ---------------------------------------------------------------

    pub fn camera(&self) -> &OrthographicCamera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut OrthographicCamera {
        &mut self.camera
    }

    pub fn zoom_level(&self) -> f32 {
        self.zoom_level
    }

    pub fn position(&self) -> Vec3 {
        self.camera_position
    }

    pub fn rotation(&self) -> f32 {
        self.camera_rotation
    }

    /// Returns the visible world bounds as `(left, right, bottom, top)`.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let hw = self.aspect_ratio * self.zoom_level;
        let hh = self.zoom_level;
        (
            -hw + self.camera_position.x,
             hw + self.camera_position.x,
            -hh + self.camera_position.y,
             hh + self.camera_position.y,
        )
    }

    /// Returns the total visible `(width, height)` in world units.
    pub fn bounds_size(&self) -> (f32, f32) {
        (
            self.aspect_ratio * self.zoom_level * 2.0,
            self.zoom_level * 2.0,
        )
    }

    /// Convert screen-space pixel coordinates to world-space coordinates.
    ///
    /// Accounts for camera position and zoom but **not** camera rotation.
    pub fn screen_to_world(
        &self,
        screen_x: f64,
        screen_y: f64,
        window_width: u32,
        window_height: u32,
    ) -> glam::Vec2 {
        let (bw, bh) = self.bounds_size();
        let x = (screen_x as f32 / window_width as f32 - 0.5) * bw + self.camera_position.x;
        let y = (0.5 - screen_y as f32 / window_height as f32) * bh + self.camera_position.y;
        glam::Vec2::new(x, y)
    }

    // -- Setters ---------------------------------------------------------------

    pub fn set_zoom_level(&mut self, level: f32) {
        self.zoom_level = level.max(0.25);
        self.update_projection();
    }

    pub fn set_position(&mut self, position: Vec3) {
        self.camera_position = position;
        self.camera.set_position(position);
    }

    pub fn set_rotation(&mut self, rotation: f32) {
        self.camera_rotation = rotation;
        self.camera.set_rotation(rotation);
    }

    // -- Internal --------------------------------------------------------------

    fn on_mouse_scrolled(&mut self, y_offset: f64) {
        self.zoom_level -= y_offset as f32 * 0.25;
        self.zoom_level = self.zoom_level.max(0.25);
        self.update_projection();
    }

    fn on_window_resized(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.aspect_ratio = width as f32 / height as f32;
        self.update_projection();
    }

    fn update_projection(&mut self) {
        self.camera.set_projection(
            -self.aspect_ratio * self.zoom_level,
            self.aspect_ratio * self.zoom_level,
            -self.zoom_level,
            self.zoom_level,
        );
    }
}
