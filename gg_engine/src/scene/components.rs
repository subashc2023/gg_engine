use glam::{Mat4, Vec3, Vec4};

use crate::renderer::SceneCamera;
use crate::scene::native_script::NativeScript;

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

/// Transform decomposed into translation, rotation, and scale.
///
/// Rotation is stored in **radians** (Euler angles, XYZ order).
/// Use [`get_transform()`](TransformComponent::get_transform) to build
/// the combined 4×4 matrix for rendering.
pub struct TransformComponent {
    pub translation: Vec3,
    /// Euler rotation in radians (X, Y, Z).
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl TransformComponent {
    pub fn new(translation: Vec3) -> Self {
        Self {
            translation,
            ..Default::default()
        }
    }

    /// Build the combined transform matrix: Translation × Rotation(Z) × Rotation(Y) × Rotation(X) × Scale.
    pub fn get_transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            self.scale,
            glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                self.rotation.x,
                self.rotation.y,
                self.rotation.z,
            ),
            self.translation,
        )
    }
}

impl Default for TransformComponent {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
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

/// Attaches a [`NativeScript`] to an entity for per-frame behavior.
///
/// Use [`bind::<T>()`](NativeScriptComponent::bind) to create an instance.
/// The script is lazily instantiated on the first [`Scene::on_update_scripts`](super::Scene::on_update_scripts)
/// call, and receives lifecycle callbacks (`on_create`, `on_update`, `on_destroy`).
pub struct NativeScriptComponent {
    pub(crate) instance: Option<Box<dyn NativeScript>>,
    pub(crate) instantiate_fn: fn() -> Box<dyn NativeScript>,
    pub(crate) created: bool,
}

impl NativeScriptComponent {
    /// Create a `NativeScriptComponent` bound to a concrete script type.
    ///
    /// `T` must implement [`NativeScript`] and [`Default`]. The script is
    /// not instantiated immediately — it will be created lazily by the scene.
    pub fn bind<T: NativeScript + Default>() -> Self {
        fn instantiate<T: NativeScript + Default>() -> Box<dyn NativeScript> {
            Box::new(T::default())
        }
        Self {
            instance: None,
            instantiate_fn: instantiate::<T>,
            created: false,
        }
    }
}
