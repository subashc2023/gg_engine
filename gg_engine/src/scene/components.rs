use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::renderer::Font;
use crate::renderer::SceneCamera;
use crate::renderer::Texture2D;
use crate::scene::native_script::NativeScript;
use crate::uuid::Uuid;
use crate::Ref;

// ---------------------------------------------------------------------------
// Relationship Component (parent-child hierarchy)
// ---------------------------------------------------------------------------

/// Tracks parent-child relationships between entities.
///
/// Every entity gets a default `RelationshipComponent` on creation.
/// Parent and children are stored as UUIDs (from [`IdComponent`]) so
/// relationships survive scene copy and serialization.
#[derive(Clone, Default)]
pub struct RelationshipComponent {
    /// Parent entity UUID. `None` = root entity.
    pub parent: Option<u64>,
    /// Ordered list of child entity UUIDs.
    pub children: Vec<u64>,
}

impl RelationshipComponent {
    /// Returns `true` if this entity has a parent or children.
    pub fn has_relationships(&self) -> bool {
        self.parent.is_some() || !self.children.is_empty()
    }
}

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
/// Rotation is stored as a **quaternion** (`Quat`) for gimbal-lock-free 3D rotation.
/// Use [`get_transform()`](TransformComponent::get_transform) to build
/// the combined 4×4 matrix for rendering, or [`euler_angles()`](TransformComponent::euler_angles)
/// for a human-readable Euler decomposition.
#[derive(Clone, Copy)]
pub struct TransformComponent {
    pub translation: Vec3,
    /// Rotation quaternion (normalized).
    pub rotation: glam::Quat,
    pub scale: Vec3,
}

impl TransformComponent {
    pub fn new(translation: Vec3) -> Self {
        Self {
            translation,
            ..Default::default()
        }
    }

    /// Build the combined transform matrix (Translation × Rotation × Scale).
    pub fn get_transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Returns Euler angles (XYZ order, radians) for UI display.
    pub fn euler_angles(&self) -> Vec3 {
        let (rx, ry, rz) = self.rotation.to_euler(glam::EulerRot::XYZ);
        Vec3::new(rx, ry, rz)
    }

    /// Sets rotation from Euler angles (XYZ order, radians).
    pub fn set_euler_angles(&mut self, euler: Vec3) {
        self.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, euler.x, euler.y, euler.z);
    }

    /// Returns the Z-axis rotation in radians (useful for 2D).
    pub fn rotation_z(&self) -> f32 {
        self.euler_angles().z
    }

    /// Sets only the Z-axis rotation, zeroing X/Y (useful for 2D physics write-back).
    pub fn set_rotation_z(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_z(angle);
    }
}

impl Default for TransformComponent {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
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
    /// Runtime-only loaded texture. Not serialized.
    pub texture: Option<Ref<Texture2D>>,
    /// Asset handle referencing a texture in the asset registry.
    /// 0 = no texture assigned.
    pub texture_handle: Uuid,
    pub tiling_factor: f32,
    /// Sorting layer for draw order. Lower layers render first (behind).
    pub sorting_layer: i32,
    /// Order within the same sorting layer. Lower values render first.
    pub order_in_layer: i32,
    /// UV min corner for atlas sub-region. (0, 0) = top-left of full texture.
    pub atlas_min: Vec2,
    /// UV max corner for atlas sub-region. (1, 1) = bottom-right of full texture.
    pub atlas_max: Vec2,
}

impl SpriteRendererComponent {
    pub fn new(color: Vec4) -> Self {
        Self {
            color,
            texture: None,
            texture_handle: Uuid::from_raw(0),
            tiling_factor: 1.0,
            sorting_layer: 0,
            order_in_layer: 0,
            atlas_min: Vec2::ZERO,
            atlas_max: Vec2::ONE,
        }
    }

    /// Convenience: opaque RGB color (alpha = 1.0).
    pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self {
            color: Vec4::new(r, g, b, 1.0),
            texture: None,
            texture_handle: Uuid::from_raw(0),
            tiling_factor: 1.0,
            sorting_layer: 0,
            order_in_layer: 0,
            atlas_min: Vec2::ZERO,
            atlas_max: Vec2::ONE,
        }
    }

    /// Returns true if this sprite uses a sub-region of its texture (atlas mode).
    pub fn is_atlas(&self) -> bool {
        self.atlas_min != Vec2::ZERO || self.atlas_max != Vec2::ONE
    }
}

impl Default for SpriteRendererComponent {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            texture: None,
            texture_handle: Uuid::from_raw(0),
            tiling_factor: 1.0,
            sorting_layer: 0,
            order_in_layer: 0,
            atlas_min: Vec2::ZERO,
            atlas_max: Vec2::ONE,
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
    /// Sorting layer for draw order. Lower layers render first (behind).
    pub sorting_layer: i32,
    /// Order within the same sorting layer. Lower values render first.
    pub order_in_layer: i32,
}

impl CircleRendererComponent {
    pub fn new(color: Vec4) -> Self {
        Self {
            color,
            thickness: 1.0,
            fade: 0.005,
            sorting_layer: 0,
            order_in_layer: 0,
        }
    }
}

impl Default for CircleRendererComponent {
    fn default() -> Self {
        Self {
            color: Vec4::ONE,
            thickness: 1.0,
            fade: 0.005,
            sorting_layer: 0,
            order_in_layer: 0,
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
// Text Component
// ---------------------------------------------------------------------------

/// Text rendered using an MSDF font atlas.
///
/// The `font_path` points to a `.ttf` file. At runtime the font is loaded
/// into a [`Font`] (MSDF atlas + glyph metrics). The `font` field is
/// runtime-only and not serialized.
pub struct TextComponent {
    pub text: String,
    pub font_path: String,
    /// Runtime-only loaded font. Not serialized.
    pub font: Option<Ref<Font>>,
    pub font_size: f32,
    pub color: Vec4,
    pub line_spacing: f32,
    pub kerning: f32,
    /// Sorting layer for draw order. Lower layers render first (behind).
    pub sorting_layer: i32,
    /// Order within the same sorting layer. Lower values render first.
    pub order_in_layer: i32,
}

impl Clone for TextComponent {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            font_path: self.font_path.clone(),
            font: self.font.clone(),
            font_size: self.font_size,
            color: self.color,
            line_spacing: self.line_spacing,
            kerning: self.kerning,
            sorting_layer: self.sorting_layer,
            order_in_layer: self.order_in_layer,
        }
    }
}

impl Default for TextComponent {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_path: String::new(),
            font: None,
            font_size: 1.0,
            color: Vec4::ONE,
            line_spacing: 1.0,
            kerning: 0.0,
            sorting_layer: 0,
            order_in_layer: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 2D Physics Components
// ---------------------------------------------------------------------------

/// Body type for a 2D rigid body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RigidBody2DType {
    #[default]
    Static,
    Dynamic,
    Kinematic,
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
    /// Per-body gravity multiplier (0.0 = no gravity, 1.0 = normal, 2.0 = double).
    pub gravity_scale: f32,
    /// Velocity damping (drag). Higher = more resistance to linear motion.
    pub linear_damping: f32,
    /// Angular velocity damping. Higher = more resistance to rotation.
    pub angular_damping: f32,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_body: Option<rapier2d::dynamics::RigidBodyHandle>,
}

impl RigidBody2DComponent {
    pub fn new(body_type: RigidBody2DType) -> Self {
        Self {
            body_type,
            fixed_rotation: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            runtime_body: None,
        }
    }
}

impl Clone for RigidBody2DComponent {
    fn clone(&self) -> Self {
        Self {
            body_type: self.body_type,
            fixed_rotation: self.fixed_rotation,
            gravity_scale: self.gravity_scale,
            linear_damping: self.linear_damping,
            angular_damping: self.angular_damping,
            runtime_body: None, // Runtime-only, not copied.
        }
    }
}

impl Default for RigidBody2DComponent {
    fn default() -> Self {
        Self {
            body_type: RigidBody2DType::Dynamic,
            fixed_rotation: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
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
    /// Collision group membership bitmask (which groups this collider belongs to).
    /// Default: `u32::MAX` (all groups).
    pub collision_layer: u32,
    /// Collision group filter bitmask (which groups this collider interacts with).
    /// Default: `u32::MAX` (interacts with all groups).
    pub collision_mask: u32,
    /// If true, this collider acts as a trigger/sensor: detects overlaps but
    /// does not generate contact forces.
    pub is_sensor: bool,
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
            collision_layer: self.collision_layer,
            collision_mask: self.collision_mask,
            is_sensor: self.is_sensor,
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
            collision_layer: u32::MAX,
            collision_mask: u32::MAX,
            is_sensor: false,
            runtime_fixture: None,
        }
    }
}

/// 2D circle collider attached to an entity for collision detection.
///
/// Requires a [`RigidBody2DComponent`] on the same entity. The collider
/// is created as a ball whose radius is `radius * max(scale.x, scale.y)`.
pub struct CircleCollider2DComponent {
    pub offset: Vec2,
    /// Radius of the circle (default 0.5 to match a 1×1 unit sprite).
    pub radius: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    /// Collision group membership bitmask (which groups this collider belongs to).
    /// Default: `u32::MAX` (all groups).
    pub collision_layer: u32,
    /// Collision group filter bitmask (which groups this collider interacts with).
    /// Default: `u32::MAX` (interacts with all groups).
    pub collision_mask: u32,
    /// If true, this collider acts as a trigger/sensor: detects overlaps but
    /// does not generate contact forces.
    pub is_sensor: bool,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_fixture: Option<rapier2d::geometry::ColliderHandle>,
}

impl Clone for CircleCollider2DComponent {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            radius: self.radius,
            density: self.density,
            friction: self.friction,
            restitution: self.restitution,
            collision_layer: self.collision_layer,
            collision_mask: self.collision_mask,
            is_sensor: self.is_sensor,
            runtime_fixture: None, // Runtime-only, not copied.
        }
    }
}

impl Default for CircleCollider2DComponent {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            radius: 0.5,
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            collision_layer: u32::MAX,
            collision_mask: u32::MAX,
            is_sensor: false,
            runtime_fixture: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 3D Physics Components
// ---------------------------------------------------------------------------

/// Body type for a 3D rigid body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RigidBody3DType {
    #[default]
    Static,
    Dynamic,
    Kinematic,
}

impl RigidBody3DType {
    pub(crate) fn to_rapier(self) -> rapier3d::dynamics::RigidBodyType {
        match self {
            Self::Static => rapier3d::dynamics::RigidBodyType::Fixed,
            Self::Dynamic => rapier3d::dynamics::RigidBodyType::Dynamic,
            Self::Kinematic => rapier3d::dynamics::RigidBodyType::KinematicPositionBased,
        }
    }
}

/// 3D rigid body attached to an entity for physics simulation.
///
/// Requires a [`TransformComponent`] on the same entity. At runtime start
/// the scene creates a rapier3d rigid body from this component's settings and
/// the entity's transform.
pub struct RigidBody3DComponent {
    pub body_type: RigidBody3DType,
    /// Lock rotation around individual axes.
    pub lock_rotation_x: bool,
    pub lock_rotation_y: bool,
    pub lock_rotation_z: bool,
    /// Per-body gravity multiplier (0.0 = no gravity, 1.0 = normal, 2.0 = double).
    pub gravity_scale: f32,
    /// Velocity damping (drag). Higher = more resistance to linear motion.
    pub linear_damping: f32,
    /// Angular velocity damping. Higher = more resistance to rotation.
    pub angular_damping: f32,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_body: Option<rapier3d::dynamics::RigidBodyHandle>,
}

impl RigidBody3DComponent {
    pub fn new(body_type: RigidBody3DType) -> Self {
        Self {
            body_type,
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            runtime_body: None,
        }
    }
}

impl Clone for RigidBody3DComponent {
    fn clone(&self) -> Self {
        Self {
            body_type: self.body_type,
            lock_rotation_x: self.lock_rotation_x,
            lock_rotation_y: self.lock_rotation_y,
            lock_rotation_z: self.lock_rotation_z,
            gravity_scale: self.gravity_scale,
            linear_damping: self.linear_damping,
            angular_damping: self.angular_damping,
            runtime_body: None, // Runtime-only, not copied.
        }
    }
}

impl Default for RigidBody3DComponent {
    fn default() -> Self {
        Self {
            body_type: RigidBody3DType::Dynamic,
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            runtime_body: None,
        }
    }
}

/// 3D box collider attached to an entity for collision detection.
///
/// Requires a [`RigidBody3DComponent`] on the same entity. The collider
/// is created as a cuboid whose half-extents are `size * entity_scale`.
pub struct BoxCollider3DComponent {
    pub offset: Vec3,
    /// Half-extents of the box (default 0.5 × 0.5 × 0.5 to match a unit cube).
    pub size: Vec3,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    /// Collision group membership bitmask.
    pub collision_layer: u32,
    /// Collision group filter bitmask.
    pub collision_mask: u32,
    /// If true, this collider acts as a trigger/sensor.
    pub is_sensor: bool,
    /// Runtime-only handle into the physics world. Not serialized.
    pub(crate) runtime_fixture: Option<rapier3d::geometry::ColliderHandle>,
}

impl Clone for BoxCollider3DComponent {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            size: self.size,
            density: self.density,
            friction: self.friction,
            restitution: self.restitution,
            collision_layer: self.collision_layer,
            collision_mask: self.collision_mask,
            is_sensor: self.is_sensor,
            runtime_fixture: None,
        }
    }
}

impl Default for BoxCollider3DComponent {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            size: Vec3::new(0.5, 0.5, 0.5),
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            collision_layer: u32::MAX,
            collision_mask: u32::MAX,
            is_sensor: false,
            runtime_fixture: None,
        }
    }
}

/// 3D sphere collider attached to an entity for collision detection.
///
/// Requires a [`RigidBody3DComponent`] on the same entity.
pub struct SphereCollider3DComponent {
    pub offset: Vec3,
    /// Radius of the sphere (default 0.5 to match a unit sphere).
    pub radius: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub collision_layer: u32,
    pub collision_mask: u32,
    pub is_sensor: bool,
    pub(crate) runtime_fixture: Option<rapier3d::geometry::ColliderHandle>,
}

impl Clone for SphereCollider3DComponent {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            radius: self.radius,
            density: self.density,
            friction: self.friction,
            restitution: self.restitution,
            collision_layer: self.collision_layer,
            collision_mask: self.collision_mask,
            is_sensor: self.is_sensor,
            runtime_fixture: None,
        }
    }
}

impl Default for SphereCollider3DComponent {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            radius: 0.5,
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            collision_layer: u32::MAX,
            collision_mask: u32::MAX,
            is_sensor: false,
            runtime_fixture: None,
        }
    }
}

/// 3D capsule collider attached to an entity for collision detection.
///
/// Requires a [`RigidBody3DComponent`] on the same entity. The capsule
/// is aligned along the Y axis by default (half_height along Y + hemisphere caps).
pub struct CapsuleCollider3DComponent {
    pub offset: Vec3,
    /// Half the height of the cylindrical segment (excluding hemisphere caps).
    pub half_height: f32,
    /// Radius of the hemisphere caps.
    pub radius: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub collision_layer: u32,
    pub collision_mask: u32,
    pub is_sensor: bool,
    pub(crate) runtime_fixture: Option<rapier3d::geometry::ColliderHandle>,
}

impl Clone for CapsuleCollider3DComponent {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            half_height: self.half_height,
            radius: self.radius,
            density: self.density,
            friction: self.friction,
            restitution: self.restitution,
            collision_layer: self.collision_layer,
            collision_mask: self.collision_mask,
            is_sensor: self.is_sensor,
            runtime_fixture: None,
        }
    }
}

impl Default for CapsuleCollider3DComponent {
    fn default() -> Self {
        Self {
            offset: Vec3::ZERO,
            half_height: 0.5,
            radius: 0.25,
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            collision_layer: u32::MAX,
            collision_mask: u32::MAX,
            is_sensor: false,
            runtime_fixture: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Audio Source Component
// ---------------------------------------------------------------------------

/// Audio source attached to an entity for sound playback.
///
/// The `audio_handle` references an audio asset (wav/ogg/mp3/flac) in the
/// asset registry. At runtime, the resolved file path is stored in
/// `resolved_path` (runtime-only, not serialized).
#[derive(Clone)]
pub struct AudioSourceComponent {
    /// Asset handle referencing an audio file in the asset registry.
    /// 0 = no audio assigned.
    pub audio_handle: crate::uuid::Uuid,
    /// Playback volume (0.0–1.0).
    pub volume: f32,
    /// Playback rate/pitch (1.0 = normal speed).
    pub pitch: f32,
    /// Whether the sound loops.
    pub looping: bool,
    /// If true, the sound plays automatically when entering play mode.
    pub play_on_start: bool,
    /// If true, use streaming playback (decode from disk gradually).
    /// Better for long music tracks. Worse for short SFX (higher CPU, startup delay).
    pub streaming: bool,
    /// If true, panning and volume are computed from entity position
    /// relative to the listener (primary camera).
    pub spatial: bool,
    /// Distance below which spatial volume is at full strength (default 1.0).
    pub min_distance: f32,
    /// Distance above which spatial volume is zero (default 50.0).
    pub max_distance: f32,
    /// Runtime-only: resolved file path from asset manager. Not serialized.
    pub(crate) resolved_path: Option<String>,
}

impl Default for AudioSourceComponent {
    fn default() -> Self {
        Self {
            audio_handle: crate::uuid::Uuid::from_raw(0),
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            play_on_start: false,
            streaming: false,
            spatial: false,
            min_distance: 1.0,
            max_distance: 50.0,
            resolved_path: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Particle Emitter Component
// ---------------------------------------------------------------------------

/// GPU particle emitter attached to an entity.
///
/// Particles are emitted from the entity's [`TransformComponent`] position
/// each frame while `playing` is true. The GPU particle system must be
/// initialized on the renderer first (done automatically by the scene when
/// a `ParticleEmitterComponent` is encountered).
///
/// All particles share a single GPU particle pool on the renderer.
#[derive(Clone)]
pub struct ParticleEmitterComponent {
    /// Number of particles emitted per frame.
    pub emit_rate: u32,
    /// Maximum particles this emitter contributes to the shared pool.
    /// Used when auto-creating the GPU particle system (first emitter wins).
    pub max_particles: u32,
    /// Whether the emitter is actively emitting.
    pub playing: bool,
    /// Base velocity for emitted particles.
    pub velocity: Vec2,
    /// Random spread added to velocity.
    pub velocity_variation: Vec2,
    /// Start color (interpolated to `color_end` over lifetime).
    pub color_begin: Vec4,
    /// End color.
    pub color_end: Vec4,
    /// Start size.
    pub size_begin: f32,
    /// End size.
    pub size_end: f32,
    /// Random variation added to size_begin.
    pub size_variation: f32,
    /// Particle lifetime in seconds.
    pub lifetime: f32,
}

impl Default for ParticleEmitterComponent {
    fn default() -> Self {
        Self {
            emit_rate: 5,
            max_particles: 100_000,
            playing: true,
            velocity: Vec2::ZERO,
            velocity_variation: Vec2::new(3.0, 3.0),
            color_begin: Vec4::new(0.98, 0.33, 0.16, 1.0),
            color_end: Vec4::new(0.98, 0.84, 0.16, 0.0),
            size_begin: 0.1,
            size_end: 0.0,
            size_variation: 0.05,
            lifetime: 5.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Audio Listener Component
// ---------------------------------------------------------------------------

/// Marks an entity as the spatial audio listener.
///
/// By default, spatial audio uses the primary camera's position as the
/// listener. Adding an `AudioListenerComponent` to an entity overrides this
/// — the entity's [`TransformComponent`] position will be used instead.
/// Useful when the listener should follow a player character rather than
/// the camera.
///
/// If multiple entities have this component, the last one found is used.
#[derive(Clone)]
pub struct AudioListenerComponent {
    /// Whether this listener is active. Allows disabling without removing.
    pub active: bool,
}

impl Default for AudioListenerComponent {
    fn default() -> Self {
        Self { active: true }
    }
}

// ---------------------------------------------------------------------------
// Tilemap Component
// ---------------------------------------------------------------------------

/// Bit flag for horizontal tile flip (bit 30). Combine with tile ID via bitwise OR.
pub const TILE_FLIP_H: i32 = 0x4000_0000;
/// Bit flag for vertical tile flip (bit 29). Combine with tile ID via bitwise OR.
pub const TILE_FLIP_V: i32 = 0x2000_0000;
/// Mask to extract the raw tile ID (lower 29 bits) from a tile value with flip flags.
pub const TILE_ID_MASK: i32 = 0x1FFF_FFFF;

/// Tile-based map renderer for 2D grid levels.
///
/// Each entity with a `TilemapComponent` renders a grid of tiles using a
/// tileset texture. Tile IDs map to sub-regions of the tileset image.
/// A tile ID of `-1` means "empty" (not rendered).
///
/// Tile values may include flip flags in the high bits:
/// - Bit 30 ([`TILE_FLIP_H`]): horizontal flip
/// - Bit 29 ([`TILE_FLIP_V`]): vertical flip
/// - Lower 29 bits ([`TILE_ID_MASK`]): actual tile ID
///
/// The tilemap's world position comes from the entity's [`TransformComponent`].
/// Each tile is `tile_size` in world-space units, laid out in a row-major grid.
#[derive(Clone)]
pub struct TilemapComponent {
    /// Number of columns in the grid.
    pub width: u32,
    /// Number of rows in the grid.
    pub height: u32,
    /// World-space size per tile.
    pub tile_size: Vec2,
    /// Asset handle referencing the tileset texture. 0 = no texture assigned.
    pub texture_handle: crate::uuid::Uuid,
    /// Runtime-only loaded texture. Not serialized.
    pub texture: Option<Ref<crate::renderer::Texture2D>>,
    /// Number of columns in the tileset image.
    pub tileset_columns: u32,
    /// Pixel size per cell in the tileset image.
    pub cell_size: Vec2,
    /// Spacing between tiles in the tileset image (pixels). Default: (0, 0).
    pub spacing: Vec2,
    /// Margin from the edge of the tileset image (pixels). Default: (0, 0).
    pub margin: Vec2,
    /// Tile IDs, row-major (width * height). -1 = empty.
    /// High bits encode flip flags (see [`TILE_FLIP_H`], [`TILE_FLIP_V`]).
    pub tiles: Vec<i32>,
    /// Sorting layer for draw order. Lower layers render first (behind).
    pub sorting_layer: i32,
    /// Order within the same sorting layer. Lower values render first.
    pub order_in_layer: i32,
}

impl TilemapComponent {
    /// Get the tile ID at grid position (x, y). Returns -1 if out of bounds.
    pub fn get_tile(&self, x: u32, y: u32) -> i32 {
        if x >= self.width || y >= self.height {
            return -1;
        }
        self.tiles[(y * self.width + x) as usize]
    }

    /// Set the tile ID at grid position (x, y). No-op if out of bounds.
    pub fn set_tile(&mut self, x: u32, y: u32, id: i32) {
        if x < self.width && y < self.height {
            self.tiles[(y * self.width + x) as usize] = id;
        }
    }

    /// Resize the grid, preserving existing tile data where possible.
    /// New cells are filled with -1 (empty).
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == self.width && new_height == self.height {
            return;
        }
        let mut new_tiles = vec![-1i32; (new_width * new_height) as usize];
        let copy_w = self.width.min(new_width);
        let copy_h = self.height.min(new_height);
        for y in 0..copy_h {
            for x in 0..copy_w {
                new_tiles[(y * new_width + x) as usize] = self.tiles[(y * self.width + x) as usize];
            }
        }
        self.width = new_width;
        self.height = new_height;
        self.tiles = new_tiles;
    }
}

impl Default for TilemapComponent {
    fn default() -> Self {
        Self {
            width: 10,
            height: 10,
            tile_size: Vec2::new(1.0, 1.0),
            texture_handle: crate::uuid::Uuid::from_raw(0),
            texture: None,
            tileset_columns: 1,
            cell_size: Vec2::new(32.0, 32.0),
            spacing: Vec2::ZERO,
            margin: Vec2::ZERO,
            tiles: vec![-1; 100],
            sorting_layer: 0,
            order_in_layer: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Mesh Renderer Component (3D)
// ---------------------------------------------------------------------------

/// Which built-in mesh primitive to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MeshPrimitive {
    #[default]
    Cube,
    Sphere,
    Plane,
}

impl MeshPrimitive {
    /// Local-space axis-aligned bounding box as `(min, max)` corners.
    pub fn local_bounds(self) -> (Vec3, Vec3) {
        match self {
            Self::Cube | Self::Sphere => (Vec3::splat(-0.5), Vec3::splat(0.5)),
            Self::Plane => {
                // Flat on XZ, Y = 0.
                (Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 0.0, 0.5))
            }
        }
    }
}

/// 3D mesh renderer attached to an entity.
///
/// Uses the entity's [`TransformComponent`] as the model matrix.
/// Currently supports built-in primitives (cube, sphere, plane).
/// The mesh is uploaded to the GPU lazily on first render.
pub struct MeshRendererComponent {
    /// Which built-in primitive to render.
    pub primitive: MeshPrimitive,
    /// Vertex color / tint (multiplied with albedo in shader).
    pub color: Vec4,
    /// 0.0 = dielectric (plastic, wood), 1.0 = metal (gold, steel).
    pub metallic: f32,
    /// 0.0 = mirror-smooth, 1.0 = fully rough/matte.
    pub roughness: f32,
    /// Emissive color (HDR). Black = no emission.
    pub emissive_color: Vec3,
    /// Multiplier on emissive color for HDR bloom intensity.
    pub emissive_strength: f32,
    /// Runtime-only loaded albedo texture. Not serialized.
    pub texture: Option<Ref<Texture2D>>,
    /// Asset handle referencing an albedo texture in the asset registry.
    /// 0 = no texture assigned.
    pub texture_handle: Uuid,
    /// Runtime-only uploaded vertex array. Not serialized.
    pub(crate) vertex_array: Option<crate::renderer::VertexArray>,
}

impl Clone for MeshRendererComponent {
    fn clone(&self) -> Self {
        Self {
            primitive: self.primitive,
            color: self.color,
            metallic: self.metallic,
            roughness: self.roughness,
            emissive_color: self.emissive_color,
            emissive_strength: self.emissive_strength,
            texture: self.texture.clone(),
            texture_handle: self.texture_handle,
            vertex_array: None, // Runtime-only, not copied.
        }
    }
}

impl MeshRendererComponent {
    pub fn new(primitive: MeshPrimitive, color: Vec4) -> Self {
        Self {
            primitive,
            color,
            metallic: 0.0,
            roughness: 0.5,
            emissive_color: Vec3::ZERO,
            emissive_strength: 1.0,
            texture: None,
            texture_handle: Uuid::from_raw(0),
            vertex_array: None,
        }
    }
}

impl Default for MeshRendererComponent {
    fn default() -> Self {
        Self {
            primitive: MeshPrimitive::Cube,
            color: Vec4::ONE,
            metallic: 0.0,
            roughness: 0.5,
            emissive_color: Vec3::ZERO,
            emissive_strength: 1.0,
            texture: None,
            texture_handle: Uuid::from_raw(0),
            vertex_array: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Light Components
// ---------------------------------------------------------------------------

/// Directional light — infinite distance, uniform direction (like the sun).
///
/// Direction is derived from the entity's rotation: `rotation * CANONICAL_FORWARD`.
/// With identity rotation the light points straight down (`-Y`), like a noon sun.
/// Rotate the entity to aim the light.
#[derive(Clone)]
pub struct DirectionalLightComponent {
    /// Light color (linear RGB).
    pub color: Vec3,
    /// Brightness multiplier.
    pub intensity: f32,
    /// Whether this light casts shadows via a shadow map.
    pub cast_shadows: bool,
}

impl DirectionalLightComponent {
    /// The local-space direction vector before rotation is applied.
    /// With identity rotation the light points straight down.
    pub const CANONICAL_FORWARD: Vec3 = Vec3::NEG_Y;

    /// Compute the world-space light direction from the entity's rotation.
    #[inline]
    pub fn direction(rotation: Quat) -> Vec3 {
        rotation * Self::CANONICAL_FORWARD
    }
}

impl Default for DirectionalLightComponent {
    fn default() -> Self {
        Self {
            color: Vec3::ONE,
            intensity: 1.0,
            cast_shadows: false,
        }
    }
}

/// Point light — emits light in all directions from the entity's position.
///
/// Position is taken from the entity's [`TransformComponent`].
/// Uses smooth quadratic attenuation: `max(0, 1 - (d/radius)^2)^2`.
#[derive(Clone)]
pub struct PointLightComponent {
    /// Light color (linear RGB).
    pub color: Vec3,
    /// Brightness multiplier.
    pub intensity: f32,
    /// Maximum influence radius. Light is zero beyond this distance.
    pub radius: f32,
}

impl Default for PointLightComponent {
    fn default() -> Self {
        Self {
            color: Vec3::ONE,
            intensity: 1.0,
            radius: 10.0,
        }
    }
}

/// Ambient light override for a scene. If no entity has this component,
/// a default ambient of (0.03, 0.03, 0.03) is used.
///
/// Only the first entity with this component is used (scene-wide setting).
#[derive(Clone)]
pub struct AmbientLightComponent {
    /// Ambient light color (linear RGB).
    pub color: Vec3,
    /// Intensity multiplier.
    pub intensity: f32,
}

impl Default for AmbientLightComponent {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.03, 0.03, 0.03),
            intensity: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Lua Scripting Component
// ---------------------------------------------------------------------------

/// Lua script attached to an entity for per-frame behavior via LuaJIT.
///
/// The `script_path` points to a `.lua` file relative to the project root.
/// At runtime start, the [`Scene`](super::Scene) loads the script into
/// its [`ScriptEngine`](super::script_engine::ScriptEngine) and sets
/// `loaded = true`. The `loaded` flag is reset on clone (same pattern as
/// physics runtime handles).
///
/// `field_overrides` stores editor-set values for the script's `fields`
/// table. These are applied after loading the script environment and
/// before `on_create()` is called.
#[derive(Default)]
#[cfg(feature = "lua-scripting")]
pub struct LuaScriptComponent {
    pub script_path: String,
    /// Per-field overrides set from the editor. Keyed by field name.
    pub field_overrides: std::collections::HashMap<String, super::script_engine::ScriptFieldValue>,
    /// Runtime-only flag indicating whether the script has been loaded.
    /// Reset on clone (same pattern as physics handles).
    pub(crate) loaded: bool,
    /// Runtime-only flag set when script loading fails (e.g. file not found).
    /// Prevents infinite retry every frame. Reset on clone and hot-reload.
    pub(crate) load_failed: bool,
}

#[cfg(feature = "lua-scripting")]
impl LuaScriptComponent {
    pub fn new(script_path: impl Into<String>) -> Self {
        Self {
            script_path: script_path.into(),
            field_overrides: std::collections::HashMap::new(),
            loaded: false,
            load_failed: false,
        }
    }
}

#[cfg(feature = "lua-scripting")]
impl Clone for LuaScriptComponent {
    fn clone(&self) -> Self {
        Self {
            script_path: self.script_path.clone(),
            field_overrides: self.field_overrides.clone(),
            loaded: false,      // Runtime-only, not copied.
            load_failed: false, // Runtime-only, not copied.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilemap_get_set_tile() {
        let mut tm = TilemapComponent::default();
        assert_eq!(tm.get_tile(0, 0), -1);
        tm.set_tile(2, 3, 5);
        assert_eq!(tm.get_tile(2, 3), 5);
        assert_eq!(tm.get_tile(0, 0), -1);
        // OOB returns -1.
        assert_eq!(tm.get_tile(100, 100), -1);
        // OOB set is a no-op.
        tm.set_tile(100, 100, 42);
    }

    #[test]
    fn tilemap_resize_preserves_data() {
        let mut tm = TilemapComponent::default(); // 10x10
        tm.set_tile(0, 0, 1);
        tm.set_tile(4, 4, 7);
        tm.set_tile(9, 9, 3);

        // Shrink.
        tm.resize(5, 5);
        assert_eq!(tm.width, 5);
        assert_eq!(tm.height, 5);
        assert_eq!(tm.get_tile(0, 0), 1);
        assert_eq!(tm.get_tile(4, 4), 7);
        assert_eq!(tm.get_tile(3, 3), -1); // new cell
        assert_eq!(tm.tiles.len(), 25);

        // Grow.
        tm.resize(8, 8);
        assert_eq!(tm.width, 8);
        assert_eq!(tm.height, 8);
        assert_eq!(tm.get_tile(0, 0), 1);
        assert_eq!(tm.get_tile(4, 4), 7);
        assert_eq!(tm.get_tile(7, 7), -1);
        assert_eq!(tm.tiles.len(), 64);
    }

    #[test]
    fn tilemap_resize_noop() {
        let mut tm = TilemapComponent::default();
        tm.set_tile(0, 0, 42);
        tm.resize(10, 10); // same size
        assert_eq!(tm.get_tile(0, 0), 42);
    }
}
