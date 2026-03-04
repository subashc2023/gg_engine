use glam::{Mat4, Quat, Vec2, Vec3};

use crate::events::{Event, KeyCode, MouseButton, MouseEvent};
use crate::input::Input;

/// Maya-style 3D editor camera, independent of the ECS.
///
/// Controls:
/// - **Alt + LMB** — orbit around the focal point
/// - **Alt + MMB** — pan (translate focal point)
/// - **Alt + RMB** / **scroll wheel** — zoom (change distance to focal point)
///
/// The camera maintains a focal point, distance, yaw, and pitch. Position is
/// derived as `focal_point - forward * distance`.
pub struct EditorCamera {
    fov: f32,
    aspect_ratio: f32,
    near_clip: f32,
    far_clip: f32,

    focal_point: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,

    projection: Mat4,
    view_matrix: Mat4,
    view_projection: Mat4,

    initial_mouse_position: Vec2,
    viewport_width: f32,
    viewport_height: f32,
}

impl EditorCamera {
    /// Create a new editor camera.
    ///
    /// * `fov` — vertical field of view in **radians**
    /// * `near` / `far` — clip planes
    pub fn new(fov: f32, near: f32, far: f32) -> Self {
        let aspect = 1.0;
        let mut cam = Self {
            fov,
            aspect_ratio: aspect,
            near_clip: near,
            far_clip: far,

            focal_point: Vec3::ZERO,
            distance: 10.0,
            yaw: 0.0,
            pitch: 0.0,

            projection: Mat4::IDENTITY,
            view_matrix: Mat4::IDENTITY,
            view_projection: Mat4::IDENTITY,

            initial_mouse_position: Vec2::ZERO,
            viewport_width: 1280.0,
            viewport_height: 720.0,
        };
        cam.update_projection();
        cam.update_view();
        cam
    }

    // -- Public API -----------------------------------------------------------

    /// Call each frame. Tracks mouse delta for orbit/pan/zoom.
    pub fn on_update(&mut self, _dt: crate::timestep::Timestep, input: &Input) {
        let mouse = Vec2::new(input.mouse_x() as f32, input.mouse_y() as f32);
        let delta = (mouse - self.initial_mouse_position) * 0.003;

        let alt = input.is_key_pressed(KeyCode::LeftAlt) || input.is_key_pressed(KeyCode::RightAlt);

        if alt {
            if input.is_mouse_button_pressed(MouseButton::Left) {
                self.mouse_orbit(delta);
            } else if input.is_mouse_button_pressed(MouseButton::Middle) {
                self.mouse_pan(delta);
            } else if input.is_mouse_button_pressed(MouseButton::Right) {
                self.mouse_zoom(delta.x + delta.y);
            }
        }

        self.initial_mouse_position = mouse;
        self.update_view();
    }

    /// Handle scroll and other events. Returns `true` if consumed.
    pub fn on_event(&mut self, event: &Event) -> bool {
        if let Event::Mouse(MouseEvent::Scrolled { y_offset, .. }) = event {
            self.mouse_zoom(*y_offset as f32 * 0.1);
            self.update_view();
            return true;
        }
        false
    }

    /// Update viewport dimensions (recalculates projection).
    pub fn set_viewport_size(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.viewport_width = width;
            self.viewport_height = height;
            self.aspect_ratio = width / height;
            self.update_projection();
            self.update_view();
        }
    }

    /// The combined view-projection matrix (for the renderer).
    pub fn view_projection(&self) -> Mat4 {
        self.view_projection
    }

    /// The raw projection matrix.
    pub fn projection(&self) -> &Mat4 {
        &self.projection
    }

    /// The view matrix.
    pub fn view_matrix(&self) -> &Mat4 {
        &self.view_matrix
    }

    /// Camera position in world space.
    pub fn position(&self) -> Vec3 {
        self.focal_point - self.forward() * self.distance
    }

    /// The camera's orientation quaternion.
    pub fn orientation(&self) -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, -self.yaw, -self.pitch, 0.0)
    }

    /// Forward direction (+Z in left-handed coordinates).
    pub fn forward(&self) -> Vec3 {
        self.orientation() * Vec3::Z
    }

    /// Right direction.
    pub fn right(&self) -> Vec3 {
        self.orientation() * Vec3::X
    }

    /// Up direction.
    pub fn up(&self) -> Vec3 {
        self.orientation() * Vec3::Y
    }

    /// Focal point in world space.
    pub fn focal_point(&self) -> Vec3 {
        self.focal_point
    }

    /// Distance from focal point.
    pub fn distance(&self) -> f32 {
        self.distance
    }

    /// Yaw angle (radians).
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Pitch angle (radians).
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Restore camera state from persisted values.
    pub fn restore_state(&mut self, focal_point: Vec3, distance: f32, yaw: f32, pitch: f32) {
        self.focal_point = focal_point;
        self.distance = distance;
        self.yaw = yaw;
        self.pitch = pitch.clamp(-1.5, 1.5);
        self.update_view();
    }

    // -- Internals ------------------------------------------------------------

    fn update_projection(&mut self) {
        self.projection =
            Mat4::perspective_lh(self.fov, self.aspect_ratio, self.near_clip, self.far_clip);
        // Vulkan Y-flip: NDC Y+ is down, we want Y+ up.
        self.projection.y_axis.y *= -1.0;
    }

    fn update_view(&mut self) {
        let position = self.position();
        let rotation = Mat4::from_quat(self.orientation());
        let translation = Mat4::from_translation(position);
        self.view_matrix = (translation * rotation).inverse();
        self.view_projection = self.projection * self.view_matrix;
    }

    fn mouse_orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x;
        self.pitch -= delta.y;
        // Clamp pitch to avoid flipping.
        self.pitch = self.pitch.clamp(-1.5, 1.5);
    }

    fn mouse_pan(&mut self, delta: Vec2) {
        let speed = self.pan_speed();
        self.focal_point += -self.right() * delta.x * speed.0;
        self.focal_point += self.up() * delta.y * speed.1;
    }

    fn mouse_zoom(&mut self, delta: f32) {
        self.distance -= delta * self.zoom_speed();
        // Clamp to a small positive value to prevent camera inversion.
        self.distance = self.distance.max(0.1);
    }

    fn pan_speed(&self) -> (f32, f32) {
        let x = (self.viewport_width / 1000.0).min(2.4);
        let y = (self.viewport_height / 1000.0).min(2.4);
        // Quadratic viewport scaling, proportional to distance from focal point.
        let xs = (0.0366 * x * x - 0.1778 * x + 0.3021) * self.distance;
        let ys = (0.0366 * y * y - 0.1778 * y + 0.3021) * self.distance;
        (xs, ys)
    }

    fn zoom_speed(&self) -> f32 {
        let mut distance = self.distance * 0.2;
        distance = distance.max(0.0);
        let speed = distance * distance;
        speed.min(100.0)
    }
}
