use glam::{Mat4, Vec3, Vec4};

use crate::renderer::SceneCamera;

/// Human-readable name for an entity. Every entity created via
/// [`Scene::create_entity`](super::Scene::create_entity) receives a
/// `TagComponent` automatically.
pub struct TagComponent {
    pub tag: String,
}

impl TagComponent {
    pub fn new(tag: impl Into<String>) -> Self {
        Self { tag: tag.into() }
    }
}

impl Default for TagComponent {
    fn default() -> Self {
        Self {
            tag: "Entity".into(),
        }
    }
}

/// Transform expressed as a 4x4 matrix. Starts as a raw `Mat4`;
/// decomposition into position/rotation/scale is a future step.
pub struct TransformComponent {
    pub transform: Mat4,
}

impl TransformComponent {
    pub fn new(transform: Mat4) -> Self {
        Self { transform }
    }

    /// Create from a translation vector (identity rotation/scale).
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            transform: Mat4::from_translation(translation),
        }
    }
}

impl Default for TransformComponent {
    fn default() -> Self {
        Self {
            transform: Mat4::IDENTITY,
        }
    }
}

/// Camera attached to an entity for scene rendering.
///
/// The projection comes from the [`SceneCamera`]; the view matrix is derived
/// from the entity's [`TransformComponent`] at render time.
///
/// Multiple cameras can exist in a scene. Only the one with `primary = true`
/// is used for rendering. If multiple cameras have `primary = true`, the
/// last one found in the query is used.
pub struct CameraComponent {
    pub camera: SceneCamera,
    pub primary: bool,
    /// When `true`, the camera keeps its current aspect ratio regardless of
    /// viewport resizes. When `false` (the default), the projection is
    /// recalculated whenever [`Scene::on_viewport_resize`](super::Scene::on_viewport_resize)
    /// is called.
    pub fixed_aspect_ratio: bool,
}

impl CameraComponent {
    pub fn new(camera: SceneCamera, primary: bool) -> Self {
        Self {
            camera,
            primary,
            fixed_aspect_ratio: false,
        }
    }
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self {
            camera: SceneCamera::default(),
            primary: true,
            fixed_aspect_ratio: false,
        }
    }
}

/// RGBA color attached to an entity for 2D rendering.
///
/// Used by [`Scene::on_update`](super::Scene::on_update) together with
/// [`TransformComponent`] to submit flat-colored quad draw calls.
pub struct SpriteRendererComponent {
    pub color: Vec4,
}

impl SpriteRendererComponent {
    pub fn new(color: Vec4) -> Self {
        Self { color }
    }

    /// Convenience: opaque RGB color (alpha = 1.0).
    pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self {
            color: Vec4::new(r, g, b, 1.0),
        }
    }
}

impl Default for SpriteRendererComponent {
    fn default() -> Self {
        Self { color: Vec4::ONE }
    }
}
