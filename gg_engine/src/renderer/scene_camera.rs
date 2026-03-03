use glam::Mat4;

/// Whether the camera uses perspective or orthographic projection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionType {
    Perspective = 0,
    #[default]
    Orthographic = 1,
}

/// A runtime camera that stores both orthographic and perspective projection
/// parameters and can recalculate its projection matrix when the viewport
/// changes or the projection type is switched.
///
/// Unlike [`OrthographicCamera`](super::OrthographicCamera), this camera does
/// not store position, rotation, or a view matrix — those come from the
/// entity's [`TransformComponent`](crate::scene::TransformComponent) in the ECS.
///
/// Both parameter sets (orthographic and perspective) are always stored so that
/// switching projection type preserves user-edited values.
///
/// Used inside [`CameraComponent`](crate::scene::CameraComponent) to define
/// how the scene is projected.
#[derive(Clone)]
pub struct SceneCamera {
    projection: Mat4,
    projection_type: ProjectionType,

    // Perspective parameters
    perspective_fov: f32, // radians
    perspective_near: f32,
    perspective_far: f32,

    // Orthographic parameters
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

    // -- Projection type ------------------------------------------------------

    pub fn projection_type(&self) -> ProjectionType {
        self.projection_type
    }

    pub fn set_projection_type(&mut self, projection_type: ProjectionType) {
        self.projection_type = projection_type;
        self.recalculate_projection();
    }

    // -- Perspective parameters -----------------------------------------------

    /// Set all perspective projection parameters and recalculate the matrix.
    pub fn set_perspective(&mut self, vertical_fov: f32, near: f32, far: f32) {
        self.projection_type = ProjectionType::Perspective;
        self.perspective_fov = vertical_fov;
        self.perspective_near = near;
        self.perspective_far = far;
        self.recalculate_projection();
    }

    pub fn perspective_vertical_fov(&self) -> f32 {
        self.perspective_fov
    }

    pub fn set_perspective_vertical_fov(&mut self, fov: f32) {
        self.perspective_fov = fov;
        self.recalculate_projection();
    }

    pub fn perspective_near(&self) -> f32 {
        self.perspective_near
    }

    pub fn set_perspective_near(&mut self, near: f32) {
        self.perspective_near = near;
        self.recalculate_projection();
    }

    pub fn perspective_far(&self) -> f32 {
        self.perspective_far
    }

    pub fn set_perspective_far(&mut self, far: f32) {
        self.perspective_far = far;
        self.recalculate_projection();
    }

    // -- Orthographic parameters ----------------------------------------------

    /// Set all orthographic projection parameters and recalculate the matrix.
    pub fn set_orthographic(&mut self, size: f32, near: f32, far: f32) {
        self.projection_type = ProjectionType::Orthographic;
        self.orthographic_size = size;
        self.orthographic_near = near;
        self.orthographic_far = far;
        self.recalculate_projection();
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

    pub fn set_orthographic_near(&mut self, near: f32) {
        self.orthographic_near = near;
        self.recalculate_projection();
    }

    pub fn orthographic_far(&self) -> f32 {
        self.orthographic_far
    }

    pub fn set_orthographic_far(&mut self, far: f32) {
        self.orthographic_far = far;
        self.recalculate_projection();
    }

    // -- Viewport -------------------------------------------------------------

    /// Update the viewport dimensions. Recalculates the projection matrix
    /// to match the new aspect ratio.
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        if height > 0 {
            self.aspect_ratio = width as f32 / height as f32;
            self.recalculate_projection();
        }
    }

    // -- Internal -------------------------------------------------------------

    fn recalculate_projection(&mut self) {
        match self.projection_type {
            ProjectionType::Perspective => {
                self.projection = Mat4::perspective_lh(
                    self.perspective_fov,
                    self.aspect_ratio,
                    self.perspective_near,
                    self.perspective_far,
                );
                // Vulkan Y-flip: NDC Y+ is down, we want Y+ up.
                self.projection.y_axis.y *= -1.0;
            }
            ProjectionType::Orthographic => {
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
                // Vulkan Y-flip: NDC Y+ is down, we want Y+ up.
                self.projection.y_axis.y *= -1.0;
            }
        }
    }
}

impl Default for SceneCamera {
    /// Default: orthographic projection, size 10, near -1, far 1, aspect ratio 1:1.
    /// Perspective defaults: FOV 45deg, near 0.01, far 1000.
    fn default() -> Self {
        let mut cam = Self {
            projection: Mat4::IDENTITY,
            projection_type: ProjectionType::Orthographic,

            perspective_fov: 45.0_f32.to_radians(),
            perspective_near: 0.01,
            perspective_far: 1000.0,

            orthographic_size: 10.0,
            orthographic_near: -1.0,
            orthographic_far: 1.0,

            aspect_ratio: 1.0,
        };
        cam.recalculate_projection();
        cam
    }
}
