use glam::{Mat4, Vec3};

/// An orthographic camera storing projection, view, and cached view-projection
/// matrices. Designed for 2D rendering — rotation is along the Z axis only.
///
/// Create with [`OrthographicCamera::new`] specifying the visible bounds, then
/// optionally adjust [`set_position`] / [`set_rotation`] each frame.
pub struct OrthographicCamera {
    projection_matrix: Mat4,
    view_matrix: Mat4,
    view_projection_matrix: Mat4,

    position: Vec3,
    /// Z-axis rotation in **radians**.
    rotation: f32,
}

impl OrthographicCamera {
    /// Create an orthographic camera with the given bounds.
    ///
    /// Near/far default to -1.0 / 1.0 which is fine for 2D rendering.
    pub fn new(left: f32, right: f32, bottom: f32, top: f32) -> Self {
        let mut projection_matrix = Mat4::orthographic_lh(left, right, bottom, top, -1.0, 1.0);
        projection_matrix.y_axis.y *= -1.0; // Vulkan Y-flip: NDC Y+ is down, we want Y+ up.
        let view_matrix = Mat4::IDENTITY;
        let view_projection_matrix = projection_matrix * view_matrix;

        Self {
            projection_matrix,
            view_matrix,
            view_projection_matrix,
            position: Vec3::ZERO,
            rotation: 0.0,
        }
    }

    // -- Setters ---------------------------------------------------------------

    /// Set the camera position in world space.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
        self.recalculate_view_matrix();
    }

    /// Set the Z-axis rotation in **radians**.
    pub fn set_rotation(&mut self, rotation: f32) {
        self.rotation = rotation;
        self.recalculate_view_matrix();
    }

    /// Recalculate the projection matrix (e.g. after a window resize).
    pub fn set_projection(&mut self, left: f32, right: f32, bottom: f32, top: f32) {
        self.projection_matrix = Mat4::orthographic_lh(left, right, bottom, top, -1.0, 1.0);
        self.projection_matrix.y_axis.y *= -1.0; // Vulkan Y-flip.
        self.view_projection_matrix = self.projection_matrix * self.view_matrix;
    }

    // -- Getters ---------------------------------------------------------------

    pub fn projection_matrix(&self) -> &Mat4 {
        &self.projection_matrix
    }

    pub fn view_matrix(&self) -> &Mat4 {
        &self.view_matrix
    }

    pub fn view_projection_matrix(&self) -> &Mat4 {
        &self.view_projection_matrix
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn rotation(&self) -> f32 {
        self.rotation
    }

    // -- Internal --------------------------------------------------------------

    fn recalculate_view_matrix(&mut self) {
        let transform =
            Mat4::from_translation(self.position) * Mat4::from_rotation_z(self.rotation);

        self.view_matrix = transform.inverse();
        self.view_projection_matrix = self.projection_matrix * self.view_matrix;
    }
}
