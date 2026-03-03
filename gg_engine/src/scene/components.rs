use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::renderer::SceneCamera;
use crate::renderer::Texture2D;
use crate::scene::native_script::NativeScript;
use crate::uuid::Uuid;
use crate::Ref;

/// Globally unique identifier for an entity, persisted across
/// save/load and scene copies. Every entity receives an `IdComponent`
/// automatically on creation.
#[derive(Default)]
pub struct IdComponent {
    pub id: Uuid,
}

impl IdComponent {
    pub fn new(id: Uuid) -> Self {
        Self { id }
    }
}

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
#[derive(Clone)]
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
#[derive(Clone)]
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

/// Sprite attached to an entity for 2D rendering.
///
/// Used by [`Scene::on_update`](super::Scene::on_update) together with
/// [`TransformComponent`] to submit quad draw calls. When `texture` is
/// `Some`, the texture is sampled and multiplied by `color` (tint). When
/// `None`, a white texture is used and the quad is flat-colored.
#[derive(Clone)]
pub struct SpriteRendererComponent {
    pub color: Vec4,
    pub texture: Option<Ref<Texture2D>>,
    pub tiling_factor: f32,
}

impl SpriteRendererComponent {
    pub fn new(color: Vec4) -> Self {
        Self {
            color,
            texture: None,
            tiling_factor: 1.0,
        }
    }

    /// Convenience: opaque RGB color (alpha = 1.0).
    pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self {
            color: Vec4::new(r, g, b, 1.0),
            texture: None,
            tiling_factor: 1.0,
        }
    }
}

impl Default for SpriteRendererComponent {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            texture: None,
            tiling_factor: 1.0,
        }
    }
}

/// Circle renderer attached to an entity for 2D circle rendering.
///
/// Renders a circle using a fragment shader SDF approach on a quad.
/// The circle's size is controlled by the entity's [`TransformComponent`] scale
/// (diameter = scale). No separate radius field — use scale to control size.
///
/// - `thickness`: 1.0 = filled circle, lower values create a ring/outline.
/// - `fade`: controls edge softness (higher = blurrier edges).
#[derive(Clone)]
pub struct CircleRendererComponent {
    pub color: Vec4,
    /// Thickness of the circle. 1.0 = fully filled, 0.01 = thin outline.
    pub thickness: f32,
    /// Edge fade/softness. Higher values = softer edges.
    pub fade: f32,
}

impl CircleRendererComponent {
    pub fn new(color: Vec4) -> Self {
        Self {
            color,
            thickness: 1.0,
            fade: 0.005,
        }
    }
}

impl Default for CircleRendererComponent {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            thickness: 1.0,
            fade: 0.005,
        }
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

// ---------------------------------------------------------------------------
// 2D Physics Components
// ---------------------------------------------------------------------------

/// Body type for a 2D rigid body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RigidBody2DType {
    Static,
    Dynamic,
    Kinematic,
}

impl Default for RigidBody2DType {
    fn default() -> Self {
        Self::Static
    }
}

impl RigidBody2DType {
    pub(crate) fn to_rapier(self) -> rapier2d::dynamics::RigidBodyType {
        match self {
            Self::Static => rapier2d::dynamics::RigidBodyType::Fixed,
            Self::Dynamic => rapier2d::dynamics::RigidBodyType::Dynamic,
            Self::Kinematic => rapier2d::dynamics::RigidBodyType::KinematicPositionBased,
        }
    }
}

/// 2D rigid body attached to an entity for physics simulation.
///
/// Requires a [`TransformComponent`] on the same entity. At runtime start
/// the scene creates a rapier rigid body from this component's settings and
/// the entity's transform position/rotation.
pub struct RigidBody2DComponent {
    pub body_type: RigidBody2DType,
    pub fixed_rotation: bool,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_body: Option<rapier2d::dynamics::RigidBodyHandle>,
}

impl RigidBody2DComponent {
    pub fn new(body_type: RigidBody2DType) -> Self {
        Self {
            body_type,
            fixed_rotation: false,
            runtime_body: None,
        }
    }
}

impl Clone for RigidBody2DComponent {
    fn clone(&self) -> Self {
        Self {
            body_type: self.body_type,
            fixed_rotation: self.fixed_rotation,
            runtime_body: None, // Runtime-only, not copied.
        }
    }
}

impl Default for RigidBody2DComponent {
    fn default() -> Self {
        Self {
            body_type: RigidBody2DType::Dynamic,
            fixed_rotation: false,
            runtime_body: None,
        }
    }
}

/// 2D box collider attached to an entity for collision detection.
///
/// Requires a [`RigidBody2DComponent`] on the same entity. The collider
/// is created as a cuboid whose half-extents are `size * entity_scale`.
pub struct BoxCollider2DComponent {
    pub offset: Vec2,
    /// Half-extents of the box (default 0.5 × 0.5 to match a 1×1 unit sprite).
    pub size: Vec2,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub restitution_threshold: f32,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_fixture: Option<rapier2d::geometry::ColliderHandle>,
}

impl Clone for BoxCollider2DComponent {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            size: self.size,
            density: self.density,
            friction: self.friction,
            restitution: self.restitution,
            restitution_threshold: self.restitution_threshold,
            runtime_fixture: None, // Runtime-only, not copied.
        }
    }
}

impl Default for BoxCollider2DComponent {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            size: Vec2::new(0.5, 0.5),
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            restitution_threshold: 0.5,
            runtime_fixture: None,
        }
    }
}
