use glam::Mat4;

/// A runtime camera that stores orthographic projection parameters and
/// can recalculate its projection matrix when the viewport changes.
///
/// Unlike [`OrthographicCamera`](super::OrthographicCamera), this camera does
/// not store position, rotation, or a view matrix — those come from the
/// entity's [`TransformComponent`](crate::scene::TransformComponent) in the ECS.
///
/// Used inside [`CameraComponent`](crate::scene::CameraComponent) to define
/// how the scene is projected.
pub struct SceneCamera {
    projection: Mat4,
    orthographic_size: f32,
    orthographic_near: f32,
    orthographic_far: f32,
    aspect_ratio: f32,
}

impl SceneCamera {
    /// The raw projection matrix.
    pub fn projection(&self) -> &Mat4 {
        &self.projection
    }

    // -- Orthographic parameters ----------------------------------------------

    /// Set orthographic projection parameters and recalculate the matrix.
    pub fn set_orthographic(&mut self, size: f32, near: f32, far: f32) {
        self.orthographic_size = size;
        self.orthographic_near = near;
        self.orthographic_far = far;
        self.recalculate_projection();
    }

    /// Update the viewport dimensions. Recalculates the projection matrix
    /// to match the new aspect ratio.
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        if height > 0 {
            self.aspect_ratio = width as f32 / height as f32;
            self.recalculate_projection();
        }
    }

    pub fn orthographic_size(&self) -> f32 {
        self.orthographic_size
    }

    pub fn set_orthographic_size(&mut self, size: f32) {
        self.orthographic_size = size;
        self.recalculate_projection();
    }

    pub fn orthographic_near(&self) -> f32 {
        self.orthographic_near
    }

    pub fn orthographic_far(&self) -> f32 {
        self.orthographic_far
    }

    // -- Internal -------------------------------------------------------------

    fn recalculate_projection(&mut self) {
        let half_height = self.orthographic_size * 0.5;
        let half_width = half_height * self.aspect_ratio;

        self.projection = Mat4::orthographic_lh(
            -half_width,
            half_width,
            -half_height,
            half_height,
            self.orthographic_near,
            self.orthographic_far,
        );
        self.projection.y_axis.y *= -1.0; // Vulkan Y-flip: NDC Y+ is down, we want Y+ up.
    }
}

impl Default for SceneCamera {
    /// Default: orthographic size 10, near -1, far 1, aspect ratio 1:1.
    fn default() -> Self {
        let mut cam = Self {
            projection: Mat4::IDENTITY,
            orthographic_size: 10.0,
            orthographic_near: -1.0,
            orthographic_far: 1.0,
            aspect_ratio: 1.0,
        };
        cam.recalculate_projection();
        cam
    }
}
