use std::collections::HashMap;
use std::fs;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as Base64Engine};
use glam::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, EngineResult};
use crate::renderer::{ProjectionType, SceneCamera};
use crate::scene::entity::Entity;
#[cfg(feature = "lua-scripting")]
use crate::scene::LuaScriptComponent;
use crate::scene::{
    AmbientLightComponent, AnimationClip, AnimationControllerComponent, AnimationTransition,
    AudioCategory, AudioListenerComponent, AudioSourceComponent, BoxCollider2DComponent,
    CameraComponent, CircleCollider2DComponent, CircleRendererComponent, DirectionalLightComponent,
    EnvironmentComponent, FloatOrdering, IdComponent, InstancedSpriteAnimator, MeshPrimitive,
    MeshRendererComponent, MeshSource, ParticleEmitterComponent, PointLightComponent,
    RelationshipComponent, RigidBody2DComponent, RigidBody2DType, Scene,
    SkeletalAnimationComponent, SpriteAnimatorComponent, SpriteRendererComponent, TagComponent,
    TextComponent, TilemapComponent, TransformComponent, TransitionCondition, UIAnchorComponent,
    UIImageComponent, UIInteractableComponent, UILayoutAlignment, UILayoutComponent,
    UILayoutDirection, UIRectComponent,
};
#[cfg(feature = "physics-3d")]
use crate::scene::{
    BoxCollider3DComponent, CapsuleCollider3DComponent, RigidBody3DComponent, RigidBody3DType,
    SphereCollider3DComponent,
};

/// Default value for collision layer/mask fields — all bits set (collides with everything).
fn default_collision_bits() -> u32 {
    u32::MAX
}
use crate::uuid::Uuid;

// ---------------------------------------------------------------------------
// Serialization data types (intermediate representation)
// ---------------------------------------------------------------------------

const SCENE_VERSION: u32 = 2;

fn default_scene_version() -> u32 {
    1
}

/// Apply version migrations to a scene YAML string, upgrading from `from_version` to `SCENE_VERSION`.
///
/// Each migration transforms the YAML string in place. Returns the (possibly modified) YAML.
fn migrate_scene_yaml(mut yaml: String, from_version: u32) -> String {
    if from_version >= SCENE_VERSION {
        return yaml;
    }

    // v1 → v2: Tilemap tiles format changed from YAML array to base64.
    // No YAML-level migration needed — TilemapData deserializes both formats.

    // Update the version field in the YAML so re-serialization writes v2.
    if from_version < 2 {
        // Replace "Version: 1" at the start of the document.
        if let Some(pos) = yaml.find("Version: 1") {
            yaml.replace_range(pos..pos + "Version: 1".len(), "Version: 2");
        }
        log::info!("Migrated scene from v1 to v2");
    }

    yaml
}

/// Lightweight struct to peek at the version field without parsing the full scene.
#[derive(Deserialize)]
struct VersionPeek {
    #[serde(rename = "Version", default = "default_scene_version")]
    version: u32,
}

#[derive(Serialize, Deserialize)]
struct SceneData {
    #[serde(rename = "Version", default = "default_scene_version")]
    version: u32,
    #[serde(rename = "Scene")]
    name: String,
    #[serde(rename = "Entities")]
    entities: Vec<EntityData>,
}

#[derive(Serialize, Deserialize)]
struct PrefabData {
    #[serde(rename = "Version", default = "default_scene_version")]
    version: u32,
    #[serde(rename = "Prefab")]
    name: String,
    #[serde(rename = "Entities")]
    entities: Vec<EntityData>,
}

#[derive(Serialize, Deserialize)]
struct EntityData {
    #[serde(rename = "Entity")]
    id: u64,
    #[serde(rename = "TagComponent", skip_serializing_if = "Option::is_none")]
    tag: Option<TagData>,
    #[serde(rename = "TransformComponent", skip_serializing_if = "Option::is_none")]
    transform: Option<TransformData>,
    #[serde(rename = "CameraComponent", skip_serializing_if = "Option::is_none")]
    camera: Option<CameraData>,
    #[serde(
        rename = "SpriteRendererComponent",
        skip_serializing_if = "Option::is_none"
    )]
    sprite: Option<SpriteData>,
    #[serde(
        rename = "CircleRendererComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    circle: Option<CircleData>,
    #[serde(
        rename = "TextComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    text: Option<TextData>,
    #[serde(
        rename = "RigidBody2DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    rigidbody_2d: Option<RigidBody2DData>,
    #[serde(
        rename = "BoxCollider2DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    box_collider_2d: Option<BoxCollider2DData>,
    #[serde(
        rename = "CircleCollider2DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    circle_collider_2d: Option<CircleCollider2DData>,
    #[serde(
        rename = "RigidBody3DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    rigidbody_3d: Option<RigidBody3DData>,
    #[serde(
        rename = "BoxCollider3DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    box_collider_3d: Option<BoxCollider3DData>,
    #[serde(
        rename = "SphereCollider3DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    sphere_collider_3d: Option<SphereCollider3DData>,
    #[serde(
        rename = "CapsuleCollider3DComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    capsule_collider_3d: Option<CapsuleCollider3DData>,
    #[serde(
        rename = "LuaScriptComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    lua_script: Option<LuaScriptData>,
    #[serde(
        rename = "SpriteAnimatorComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    sprite_animator: Option<SpriteAnimatorData>,
    #[serde(
        rename = "InstancedSpriteAnimator",
        skip_serializing_if = "Option::is_none",
        default
    )]
    instanced_animator: Option<InstancedSpriteAnimatorData>,
    #[serde(
        rename = "AnimationControllerComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    animation_controller: Option<AnimationControllerData>,
    #[serde(
        rename = "RelationshipComponent",
        skip_serializing_if = "has_no_relationships",
        default
    )]
    relationship: Option<RelationshipData>,
    #[serde(
        rename = "TilemapComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    tilemap: Option<TilemapData>,
    #[serde(
        rename = "AudioSourceComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    audio_source: Option<AudioSourceData>,
    #[serde(
        rename = "AudioListenerComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    audio_listener: Option<AudioListenerData>,
    #[serde(
        rename = "ParticleEmitterComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    particle_emitter: Option<ParticleEmitterData>,
    #[serde(
        rename = "MeshRendererComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    mesh_renderer: Option<MeshRendererData>,
    #[serde(
        rename = "SkeletalAnimationComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    skeletal_animation: Option<SkeletalAnimationData>,
    #[serde(
        rename = "DirectionalLightComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    directional_light: Option<DirectionalLightData>,
    #[serde(
        rename = "PointLightComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    point_light: Option<PointLightData>,
    #[serde(
        rename = "AmbientLightComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ambient_light: Option<AmbientLightData>,
    #[serde(
        rename = "EnvironmentComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    environment: Option<EnvironmentData>,
    #[serde(
        rename = "UIAnchorComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ui_anchor: Option<UIAnchorData>,
    #[serde(
        rename = "UIRectComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ui_rect: Option<UIRectData>,
    #[serde(
        rename = "UIImageComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ui_image: Option<UIImageData>,
    #[serde(
        rename = "UIInteractableComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ui_interactable: Option<UIInteractableData>,
    #[serde(
        rename = "UILayoutComponent",
        skip_serializing_if = "Option::is_none",
        default
    )]
    ui_layout: Option<UILayoutData>,
}

#[derive(Serialize, Deserialize)]
struct TagData {
    #[serde(rename = "Tag")]
    tag: String,
}

#[derive(Serialize, Deserialize)]
struct TransformData {
    #[serde(rename = "Translation")]
    translation: [f32; 3],
    /// Quaternion [x, y, z, w]. Deserializes from either 4 floats (quat) or 3 floats (legacy Euler radians).
    #[serde(rename = "Rotation", deserialize_with = "deserialize_rotation")]
    rotation: [f32; 4],
    #[serde(rename = "Scale")]
    scale: [f32; 3],
}

/// Deserialize rotation from either [x,y,z,w] quaternion (4 elements)
/// or [rx,ry,rz] Euler radians (3 elements, legacy format).
fn deserialize_rotation<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[f32; 4], D::Error> {
    let value: Vec<f32> = Vec::deserialize(deserializer)?;
    match value.len() {
        4 => Ok([value[0], value[1], value[2], value[3]]),
        3 => {
            let q = glam::Quat::from_euler(glam::EulerRot::XYZ, value[0], value[1], value[2]);
            Ok([q.x, q.y, q.z, q.w])
        }
        _ => Err(serde::de::Error::custom(
            "expected 3 or 4 floats for Rotation",
        )),
    }
}

#[derive(Serialize, Deserialize)]
struct SceneCameraData {
    #[serde(rename = "ProjectionType")]
    projection_type: u32,
    #[serde(rename = "PerspectiveFOV")]
    perspective_fov: f32,
    #[serde(rename = "PerspectiveNear")]
    perspective_near: f32,
    #[serde(rename = "PerspectiveFar")]
    perspective_far: f32,
    #[serde(rename = "OrthographicSize")]
    orthographic_size: f32,
    #[serde(rename = "OrthographicNear")]
    orthographic_near: f32,
    #[serde(rename = "OrthographicFar")]
    orthographic_far: f32,
}

#[derive(Serialize, Deserialize)]
struct CameraData {
    #[serde(rename = "Camera")]
    camera: SceneCameraData,
    #[serde(rename = "Primary")]
    primary: bool,
    #[serde(rename = "FixedAspectRatio")]
    fixed_aspect_ratio: bool,
}

#[derive(Serialize, Deserialize)]
struct SpriteData {
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "TilingFactor", default = "default_one_f32")]
    tiling_factor: f32,
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_u64")]
    texture_handle: u64,
    #[serde(rename = "SortingLayer", default)]
    sorting_layer: i32,
    #[serde(rename = "OrderInLayer", default)]
    order_in_layer: i32,
    #[serde(rename = "AtlasMin", default, skip_serializing_if = "is_zero_vec2")]
    atlas_min: [f32; 2],
    #[serde(
        rename = "AtlasMax",
        default = "default_one_vec2",
        skip_serializing_if = "is_one_vec2"
    )]
    atlas_max: [f32; 2],
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn default_one_f32() -> f32 {
    1.0
}

fn is_one_vec2(v: &[f32; 2]) -> bool {
    (v[0] - 1.0).abs() < f32::EPSILON && (v[1] - 1.0).abs() < f32::EPSILON
}

fn default_one_vec2() -> [f32; 2] {
    [1.0, 1.0]
}

#[derive(Serialize, Deserialize)]
struct CircleData {
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "Thickness", default = "default_one_f32")]
    thickness: f32,
    #[serde(rename = "Fade", default = "default_fade")]
    fade: f32,
    #[serde(rename = "SortingLayer", default)]
    sorting_layer: i32,
    #[serde(rename = "OrderInLayer", default)]
    order_in_layer: i32,
}

fn default_fade() -> f32 {
    0.005
}

#[derive(Serialize, Deserialize)]
struct TextData {
    #[serde(rename = "Text")]
    text: String,
    #[serde(rename = "FontPath")]
    font_path: String,
    #[serde(rename = "FontSize", default = "default_one_f32")]
    font_size: f32,
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "LineSpacing", default = "default_one_f32")]
    line_spacing: f32,
    #[serde(rename = "Kerning", default)]
    kerning: f32,
    #[serde(rename = "SortingLayer", default)]
    sorting_layer: i32,
    #[serde(rename = "OrderInLayer", default)]
    order_in_layer: i32,
}

#[derive(Serialize, Deserialize)]
struct RigidBody2DData {
    #[serde(rename = "BodyType")]
    body_type: String,
    #[serde(rename = "FixedRotation")]
    fixed_rotation: bool,
    #[serde(rename = "GravityScale", default = "default_gravity_scale")]
    gravity_scale: f32,
    #[serde(rename = "LinearDamping", default)]
    linear_damping: f32,
    #[serde(rename = "AngularDamping", default)]
    angular_damping: f32,
}

fn default_gravity_scale() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize)]
struct BoxCollider2DData {
    #[serde(rename = "Offset")]
    offset: [f32; 2],
    #[serde(rename = "Size")]
    size: [f32; 2],
    #[serde(rename = "Density")]
    density: f32,
    #[serde(rename = "Friction")]
    friction: f32,
    #[serde(rename = "Restitution")]
    restitution: f32,
    #[serde(rename = "CollisionLayer", default = "default_collision_bits")]
    collision_layer: u32,
    #[serde(rename = "CollisionMask", default = "default_collision_bits")]
    collision_mask: u32,
    #[serde(rename = "IsSensor", default)]
    is_sensor: bool,
    /// Legacy field, ignored on load — rapier2d has no restitution threshold.
    #[serde(rename = "RestitutionThreshold", default, skip_serializing)]
    _restitution_threshold: f32,
}

#[derive(Serialize, Deserialize)]
struct CircleCollider2DData {
    #[serde(rename = "Offset")]
    offset: [f32; 2],
    #[serde(rename = "Radius")]
    radius: f32,
    #[serde(rename = "Density")]
    density: f32,
    #[serde(rename = "Friction")]
    friction: f32,
    #[serde(rename = "Restitution")]
    restitution: f32,
    #[serde(rename = "CollisionLayer", default = "default_collision_bits")]
    collision_layer: u32,
    #[serde(rename = "CollisionMask", default = "default_collision_bits")]
    collision_mask: u32,
    #[serde(rename = "IsSensor", default)]
    is_sensor: bool,
    /// Legacy field, ignored on load — rapier2d has no restitution threshold.
    #[serde(rename = "RestitutionThreshold", default, skip_serializing)]
    _restitution_threshold: f32,
}

#[derive(Serialize, Deserialize)]
struct RigidBody3DData {
    #[serde(rename = "BodyType")]
    body_type: String,
    #[serde(rename = "LockRotationX", default)]
    lock_rotation_x: bool,
    #[serde(rename = "LockRotationY", default)]
    lock_rotation_y: bool,
    #[serde(rename = "LockRotationZ", default)]
    lock_rotation_z: bool,
    #[serde(rename = "GravityScale", default = "default_gravity_scale")]
    gravity_scale: f32,
    #[serde(rename = "LinearDamping", default)]
    linear_damping: f32,
    #[serde(rename = "AngularDamping", default)]
    angular_damping: f32,
}

#[derive(Serialize, Deserialize)]
struct BoxCollider3DData {
    #[serde(rename = "Offset")]
    offset: [f32; 3],
    #[serde(rename = "Size")]
    size: [f32; 3],
    #[serde(rename = "Density")]
    density: f32,
    #[serde(rename = "Friction")]
    friction: f32,
    #[serde(rename = "Restitution")]
    restitution: f32,
    #[serde(rename = "CollisionLayer", default = "default_collision_bits")]
    collision_layer: u32,
    #[serde(rename = "CollisionMask", default = "default_collision_bits")]
    collision_mask: u32,
    #[serde(rename = "IsSensor", default)]
    is_sensor: bool,
}

#[derive(Serialize, Deserialize)]
struct SphereCollider3DData {
    #[serde(rename = "Offset")]
    offset: [f32; 3],
    #[serde(rename = "Radius")]
    radius: f32,
    #[serde(rename = "Density")]
    density: f32,
    #[serde(rename = "Friction")]
    friction: f32,
    #[serde(rename = "Restitution")]
    restitution: f32,
    #[serde(rename = "CollisionLayer", default = "default_collision_bits")]
    collision_layer: u32,
    #[serde(rename = "CollisionMask", default = "default_collision_bits")]
    collision_mask: u32,
    #[serde(rename = "IsSensor", default)]
    is_sensor: bool,
}

#[derive(Serialize, Deserialize)]
struct CapsuleCollider3DData {
    #[serde(rename = "Offset")]
    offset: [f32; 3],
    #[serde(rename = "HalfHeight")]
    half_height: f32,
    #[serde(rename = "Radius")]
    radius: f32,
    #[serde(rename = "Density")]
    density: f32,
    #[serde(rename = "Friction")]
    friction: f32,
    #[serde(rename = "Restitution")]
    restitution: f32,
    #[serde(rename = "CollisionLayer", default = "default_collision_bits")]
    collision_layer: u32,
    #[serde(rename = "CollisionMask", default = "default_collision_bits")]
    collision_mask: u32,
    #[serde(rename = "IsSensor", default)]
    is_sensor: bool,
}

#[derive(Serialize, Deserialize)]
struct LuaScriptData {
    #[serde(rename = "ScriptPath")]
    script_path: String,
    #[cfg(feature = "lua-scripting")]
    #[serde(rename = "Fields", default, skip_serializing_if = "Option::is_none")]
    fields: Option<std::collections::HashMap<String, super::script_engine::ScriptFieldValue>>,
    #[cfg(not(feature = "lua-scripting"))]
    #[serde(rename = "Fields", default, skip_serializing_if = "Option::is_none")]
    fields: Option<serde_yaml_ng::Value>,
}

#[derive(Serialize, Deserialize)]
struct RelationshipData {
    #[serde(rename = "Parent", skip_serializing_if = "Option::is_none")]
    parent: Option<u64>,
    #[serde(rename = "Children", skip_serializing_if = "Vec::is_empty", default)]
    children: Vec<u64>,
}

#[derive(Serialize, Deserialize)]
struct AnimationClipData {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "StartFrame")]
    start_frame: u32,
    #[serde(rename = "EndFrame")]
    end_frame: u32,
    #[serde(rename = "FPS", default = "default_animation_fps")]
    fps: f32,
    #[serde(rename = "Looping", default = "default_true")]
    looping: bool,
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_u64")]
    texture_handle: u64,
}

fn convert_clips(clips: &[AnimationClip]) -> Vec<AnimationClipData> {
    clips
        .iter()
        .map(|c| AnimationClipData {
            name: c.name.clone(),
            start_frame: c.start_frame,
            end_frame: c.end_frame,
            fps: c.fps,
            looping: c.looping,
            texture_handle: c.texture_handle.raw(),
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
struct SpriteAnimatorData {
    #[serde(rename = "CellSize")]
    cell_size: [f32; 2],
    #[serde(rename = "Columns")]
    columns: u32,
    #[serde(rename = "Clips", default)]
    clips: Vec<AnimationClipData>,
    #[serde(
        rename = "DefaultClip",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    default_clip: String,
    #[serde(
        rename = "SpeedScale",
        default = "default_one_f32",
        skip_serializing_if = "is_one_f32"
    )]
    speed_scale: f32,
}

/// Serialization struct for [`InstancedSpriteAnimator`].
#[derive(Serialize, Deserialize)]
struct InstancedSpriteAnimatorData {
    #[serde(rename = "CellSize")]
    cell_size: [f32; 2],
    #[serde(rename = "Columns")]
    columns: u32,
    #[serde(rename = "Clips", default)]
    clips: Vec<AnimationClipData>,
    #[serde(
        rename = "DefaultClip",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    default_clip: String,
    #[serde(
        rename = "SpeedScale",
        default = "default_one_f32",
        skip_serializing_if = "is_one_f32"
    )]
    speed_scale: f32,
}

/// Serialization struct for a single [`AnimationTransition`].
#[derive(Serialize, Deserialize)]
struct AnimationTransitionData {
    #[serde(rename = "From", default, skip_serializing_if = "String::is_empty")]
    from: String,
    #[serde(rename = "To")]
    to: String,
    #[serde(rename = "ConditionType")]
    condition_type: String,
    #[serde(
        rename = "ParamName",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    param_name: String,
    #[serde(rename = "BoolValue", default, skip_serializing_if = "is_false")]
    bool_value: bool,
    #[serde(
        rename = "FloatOrdering",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    float_ordering: String,
    #[serde(
        rename = "FloatThreshold",
        default,
        skip_serializing_if = "is_zero_f32"
    )]
    float_threshold: f32,
}

/// Serialization struct for [`AnimationControllerComponent`].
#[derive(Serialize, Deserialize)]
struct AnimationControllerData {
    #[serde(rename = "Transitions", default)]
    transitions: Vec<AnimationTransitionData>,
    #[serde(
        rename = "BoolParams",
        default,
        skip_serializing_if = "HashMap::is_empty"
    )]
    bool_params: HashMap<String, bool>,
    #[serde(
        rename = "FloatParams",
        default,
        skip_serializing_if = "HashMap::is_empty"
    )]
    float_params: HashMap<String, f32>,
}

fn is_false(v: &bool) -> bool {
    !v
}

fn is_zero_f32(v: &f32) -> bool {
    v.abs() < f32::EPSILON
}

fn is_one_f32(v: &f32) -> bool {
    (*v - 1.0).abs() < f32::EPSILON
}

fn default_animation_fps() -> f32 {
    12.0
}

fn default_true() -> bool {
    true
}

fn default_zero_vec2() -> [f32; 2] {
    [0.0, 0.0]
}

fn is_zero_vec2(v: &[f32; 2]) -> bool {
    v[0].abs() < f32::EPSILON && v[1].abs() < f32::EPSILON
}

#[derive(Serialize, Deserialize)]
struct TilemapData {
    #[serde(rename = "Width")]
    width: u32,
    #[serde(rename = "Height")]
    height: u32,
    #[serde(rename = "TileSize")]
    tile_size: [f32; 2],
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_u64")]
    texture_handle: u64,
    #[serde(rename = "TilesetColumns", default = "default_tileset_columns")]
    tileset_columns: u32,
    #[serde(rename = "CellSize")]
    cell_size: [f32; 2],
    #[serde(
        rename = "Spacing",
        default = "default_zero_vec2",
        skip_serializing_if = "is_zero_vec2"
    )]
    spacing: [f32; 2],
    #[serde(
        rename = "Margin",
        default = "default_zero_vec2",
        skip_serializing_if = "is_zero_vec2"
    )]
    margin: [f32; 2],
    /// Legacy per-element tile array (v1 format). Kept for backward compatibility.
    #[serde(rename = "Tiles", default, skip_serializing)]
    tiles_legacy: Vec<i32>,
    /// Compact base64-encoded tile data (v2+). Little-endian i32 bytes.
    #[serde(rename = "TilesB64", default, skip_serializing_if = "String::is_empty")]
    tiles_b64: String,
    #[serde(rename = "SortingLayer", default)]
    sorting_layer: i32,
    #[serde(rename = "OrderInLayer", default)]
    order_in_layer: i32,
}

impl TilemapData {
    /// Encode tiles as base64 (little-endian i32 bytes).
    fn encode_tiles(tiles: &[i32]) -> String {
        let bytes: Vec<u8> = tiles.iter().flat_map(|t| t.to_le_bytes()).collect();
        BASE64.encode(&bytes)
    }

    /// Decode tiles, preferring base64 (v2+) and falling back to legacy Vec (v1).
    fn decode_tiles(&self) -> Vec<i32> {
        if !self.tiles_b64.is_empty() {
            if let Ok(bytes) = BASE64.decode(&self.tiles_b64) {
                return bytes
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
            }
            log::warn!("Failed to decode TilesB64, falling back to legacy Tiles");
        }
        self.tiles_legacy.clone()
    }
}

#[derive(Serialize, Deserialize)]
struct AudioSourceData {
    #[serde(rename = "AudioHandle", default, skip_serializing_if = "is_zero_u64")]
    audio_handle: u64,
    #[serde(rename = "Volume", default = "default_one_f32")]
    volume: f32,
    #[serde(rename = "Pitch", default = "default_one_f32")]
    pitch: f32,
    #[serde(rename = "Looping", default)]
    looping: bool,
    #[serde(rename = "PlayOnStart", default)]
    play_on_start: bool,
    #[serde(rename = "Streaming", default)]
    streaming: bool,
    #[serde(rename = "Spatial", default)]
    spatial: bool,
    #[serde(rename = "HRTF", default)]
    hrtf: bool,
    #[serde(
        rename = "MinDistance",
        default = "default_one_f32",
        skip_serializing_if = "is_one_f32"
    )]
    min_distance: f32,
    #[serde(
        rename = "MaxDistance",
        default = "default_max_distance",
        skip_serializing_if = "is_default_max_distance"
    )]
    max_distance: f32,
    #[serde(
        rename = "Category",
        default,
        skip_serializing_if = "is_default_audio_category"
    )]
    category: String,
}

fn default_max_distance() -> f32 {
    50.0
}

fn is_default_max_distance(v: &f32) -> bool {
    (*v - 50.0).abs() < f32::EPSILON
}

fn is_default_audio_category(v: &str) -> bool {
    v.is_empty() || v == "SFX"
}

#[derive(Serialize, Deserialize)]
struct AudioListenerData {
    #[serde(rename = "Active", default = "default_true")]
    active: bool,
}

#[derive(Serialize, Deserialize)]
struct ParticleEmitterData {
    #[serde(rename = "EmitRate", default = "default_emit_rate")]
    emit_rate: u32,
    #[serde(rename = "MaxParticles", default = "default_max_particles")]
    max_particles: u32,
    #[serde(rename = "Playing", default = "default_true")]
    playing: bool,
    #[serde(rename = "Velocity", default = "default_zero_vec2")]
    velocity: [f32; 2],
    #[serde(rename = "VelocityVariation", default = "default_velocity_variation")]
    velocity_variation: [f32; 2],
    #[serde(rename = "ColorBegin")]
    color_begin: [f32; 4],
    #[serde(rename = "ColorEnd")]
    color_end: [f32; 4],
    #[serde(rename = "SizeBegin", default = "default_size_begin")]
    size_begin: f32,
    #[serde(rename = "SizeEnd", default)]
    size_end: f32,
    #[serde(rename = "SizeVariation", default = "default_size_variation")]
    size_variation: f32,
    #[serde(rename = "Lifetime", default = "default_lifetime")]
    lifetime: f32,
}

fn default_emit_rate() -> u32 {
    5
}
fn default_max_particles() -> u32 {
    100_000
}
fn default_velocity_variation() -> [f32; 2] {
    [3.0, 3.0]
}
fn default_size_begin() -> f32 {
    0.1
}
fn default_size_variation() -> f32 {
    0.05
}
fn default_lifetime() -> f32 {
    5.0
}

fn default_tileset_columns() -> u32 {
    1
}

#[derive(Serialize, Deserialize)]
struct MeshRendererData {
    #[serde(rename = "Primitive", default = "default_mesh_primitive")]
    primitive: String,
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "Metallic", default)]
    metallic: f32,
    #[serde(rename = "Roughness", default = "default_roughness")]
    roughness: f32,
    #[serde(rename = "EmissiveColor", default)]
    emissive_color: [f32; 3],
    #[serde(rename = "EmissiveStrength", default = "default_emissive_strength")]
    emissive_strength: f32,
    #[serde(rename = "AlbedoTexture", default)]
    albedo_texture: u64,
    /// Mesh asset handle (glTF/GLB). 0 = no asset, use primitive instead.
    #[serde(rename = "MeshAsset", default)]
    mesh_asset: u64,
    #[serde(rename = "NormalTexture", default)]
    normal_texture: u64,
    /// Use alpha-tested shadow pipeline for this mesh.
    #[serde(rename = "CastAlphaShadow", default)]
    cast_alpha_shadow: bool,
}

#[derive(Serialize, Deserialize)]
struct SkeletalAnimationData {
    /// glTF/GLB mesh asset handle (UUID).
    #[serde(rename = "MeshAsset")]
    mesh_asset: u64,
    /// Playback speed multiplier.
    #[serde(rename = "Speed", default = "default_anim_speed")]
    speed: f32,
    /// Whether playback loops.
    #[serde(rename = "Looping", default = "default_true")]
    looping: bool,
    /// Whether to auto-play on load.
    #[serde(rename = "Playing", default = "default_true")]
    playing: bool,
    /// Name of the clip to play on load (empty = first clip).
    #[serde(rename = "DefaultClip", default)]
    default_clip: String,
}

fn default_anim_speed() -> f32 {
    1.0
}

fn default_roughness() -> f32 {
    0.5
}

fn default_emissive_strength() -> f32 {
    1.0
}

fn default_mesh_primitive() -> String {
    "Cube".into()
}

#[derive(Serialize, Deserialize)]
struct DirectionalLightData {
    /// Legacy field — direction is now derived from entity rotation.
    /// Kept for backward compatibility when loading old scenes.
    #[serde(rename = "Direction", default)]
    _direction: Option<[f32; 3]>,
    #[serde(rename = "Color")]
    color: [f32; 3],
    #[serde(rename = "Intensity", default = "default_light_intensity")]
    intensity: f32,
    #[serde(rename = "CastShadows", default)]
    cast_shadows: bool,
    #[serde(rename = "ShadowDistance", default = "default_shadow_distance")]
    shadow_distance: f32,
    #[serde(rename = "ShadowCullFrontFaces", default)]
    shadow_cull_front_faces: bool,
}

fn default_shadow_distance() -> f32 {
    100.0
}

#[derive(Serialize, Deserialize)]
struct PointLightData {
    #[serde(rename = "Color")]
    color: [f32; 3],
    #[serde(rename = "Intensity", default = "default_light_intensity")]
    intensity: f32,
    #[serde(rename = "Radius", default = "default_point_light_radius")]
    radius: f32,
}

#[derive(Serialize, Deserialize)]
struct AmbientLightData {
    #[serde(rename = "Color")]
    color: [f32; 3],
    #[serde(rename = "Intensity", default = "default_light_intensity")]
    intensity: f32,
}

fn default_light_intensity() -> f32 {
    1.0
}
fn default_point_light_radius() -> f32 {
    10.0
}

#[derive(Serialize, Deserialize)]
struct EnvironmentData {
    #[serde(rename = "EnvironmentMap")]
    environment_handle: u64,
    #[serde(rename = "SkyboxExposure", default = "default_light_intensity")]
    skybox_exposure: f32,
    #[serde(rename = "IBLIntensity", default = "default_light_intensity")]
    ibl_intensity: f32,
    #[serde(rename = "SkyboxRotation", default)]
    skybox_rotation: f32,
    #[serde(rename = "ShowSkybox", default = "default_true")]
    show_skybox: bool,
}

#[derive(Serialize, Deserialize)]
struct UIAnchorData {
    #[serde(rename = "Anchor")]
    anchor: [f32; 2],
    #[serde(rename = "Offset", default)]
    offset: [f32; 2],
}

#[derive(Serialize, Deserialize)]
struct UIRectData {
    #[serde(rename = "Size")]
    size: [f32; 2],
    #[serde(rename = "Pivot", default = "default_half_vec2")]
    pivot: [f32; 2],
    #[serde(rename = "RaycastTarget", default = "default_true")]
    raycast_target: bool,
}

fn default_half_vec2() -> [f32; 2] {
    [0.5, 0.5]
}

#[derive(Serialize, Deserialize)]
struct UIImageData {
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_u64")]
    texture_handle: u64,
    #[serde(rename = "Border", default)]
    border: [f32; 4],
    #[serde(rename = "FillCenter", default = "default_true")]
    fill_center: bool,
    #[serde(rename = "SortingLayer", default)]
    sorting_layer: i32,
    #[serde(rename = "OrderInLayer", default)]
    order_in_layer: i32,
}

#[derive(Serialize, Deserialize)]
struct UIInteractableData {
    #[serde(rename = "Interactable", default = "default_true")]
    interactable: bool,
    #[serde(
        rename = "HoverColor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    hover_color: Option<[f32; 4]>,
    #[serde(
        rename = "PressColor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    press_color: Option<[f32; 4]>,
    #[serde(
        rename = "DisabledColor",
        skip_serializing_if = "Option::is_none",
        default
    )]
    disabled_color: Option<[f32; 4]>,
}

#[derive(Serialize, Deserialize)]
struct UILayoutData {
    #[serde(rename = "Direction", default)]
    direction: String,
    #[serde(rename = "Spacing", default)]
    spacing: f32,
    #[serde(rename = "Alignment", default)]
    alignment: String,
    #[serde(rename = "Padding", default)]
    padding: [f32; 4],
}

fn has_no_relationships(r: &Option<RelationshipData>) -> bool {
    match r {
        None => true,
        Some(rd) => rd.parent.is_none() && rd.children.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// SceneSerializer
// ---------------------------------------------------------------------------

/// Serialize `data` to YAML and write it to `file_path`, creating parent
/// directories as needed. `label` is used in log messages (e.g. "scene", "prefab").
fn write_yaml_to_file<T: Serialize>(data: &T, file_path: &str, label: &str) -> EngineResult<()> {
    if let Some(parent) = Path::new(file_path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let yaml = serde_yaml_ng::to_string(data)?;
    crate::platform_utils::atomic_write(file_path, &yaml)?;
    log::info!("{} serialized to '{}'", label, file_path);
    Ok(())
}

/// Serializes and deserializes [`Scene`] data to/from YAML files.
///
/// This is an external serializer — scene types themselves have no serialization
/// dependency. The serializer traverses the scene's entities and components,
/// converts them to an intermediate representation, and writes YAML.
///
/// # Example
///
/// ```ignore
/// // Save
/// SceneSerializer::serialize(&scene, "assets/scenes/example.ggscene", Some("example"));
///
/// // Load
/// let mut scene = Scene::new();
/// SceneSerializer::deserialize(&mut scene, "assets/scenes/example.ggscene");
/// ```
pub struct SceneSerializer;

impl SceneSerializer {
    /// Serialize a scene to a YAML file at the given path.
    ///
    /// Creates parent directories if they don't exist.
    pub fn serialize(scene: &Scene, file_path: &str, scene_name: Option<&str>) -> EngineResult<()> {
        let scene_data = Self::scene_to_data(scene, scene_name);
        write_yaml_to_file(&scene_data, file_path, "scene")
    }

    /// Deserialize a scene from a YAML file.
    ///
    /// Entities are created in the provided `scene`. If the scene is not empty,
    /// deserialized entities are added to existing ones — callers should provide
    /// a fresh scene if a clean load is desired.
    pub fn deserialize(scene: &mut Scene, file_path: &str) -> EngineResult<()> {
        let contents = fs::read_to_string(file_path)?;

        // Peek at version for migration.
        let peek: VersionPeek =
            serde_yaml_ng::from_str(&contents).unwrap_or(VersionPeek { version: 1 });

        let contents = if peek.version < SCENE_VERSION {
            log::info!(
                "Scene '{}' is version {} (current: {}), applying migrations",
                file_path,
                peek.version,
                SCENE_VERSION
            );
            migrate_scene_yaml(contents, peek.version)
        } else if peek.version > SCENE_VERSION {
            log::warn!(
                "Scene '{}' was saved with version {} (current: {}). Some data may not load correctly.",
                file_path, peek.version, SCENE_VERSION
            );
            contents
        } else {
            contents
        };

        let scene_data: SceneData = serde_yaml_ng::from_str(&contents)?;

        log::info!(
            "Deserializing scene '{}' (version {}, {} entities)",
            scene_data.name,
            scene_data.version,
            scene_data.entities.len()
        );

        Self::data_to_scene(scene, &scene_data);
        Ok(())
    }

    /// Serialize a scene to a YAML string (in-memory snapshot).
    pub fn serialize_to_string(scene: &Scene) -> EngineResult<String> {
        let scene_data = Self::scene_to_data(scene, None);
        let yaml = serde_yaml_ng::to_string(&scene_data)?;
        Ok(yaml)
    }

    /// Deserialize a scene from a YAML string (in-memory snapshot restore).
    ///
    /// Entities are created in the provided `scene`. Callers should provide
    /// a fresh scene if a clean restore is desired.
    pub fn deserialize_from_string(scene: &mut Scene, yaml: &str) -> EngineResult<()> {
        let scene_data: SceneData = serde_yaml_ng::from_str(yaml)?;
        Self::data_to_scene(scene, &scene_data);
        Ok(())
    }

    // -- JSON snapshot API (fast undo/redo) ------------------------------------

    /// Serialize a scene to a JSON string. ~5-10× faster than YAML.
    pub fn serialize_scene_to_json(scene: &Scene) -> EngineResult<String> {
        let scene_data = Self::scene_to_data(scene, None);
        let json = serde_json::to_string(&scene_data)?;
        Ok(json)
    }

    /// Deserialize a scene from a JSON string.
    pub fn deserialize_scene_from_json(scene: &mut Scene, json: &str) -> EngineResult<()> {
        let scene_data: SceneData = serde_json::from_str(json)?;
        Self::data_to_scene(scene, &scene_data);
        Ok(())
    }

    /// Serialize a single entity to a JSON string.
    pub fn serialize_entity_to_json(scene: &Scene, entity: Entity) -> EngineResult<String> {
        let entity_data = Self::entity_to_data(scene, entity);
        let json = serde_json::to_string(&entity_data)?;
        Ok(json)
    }

    /// Restore a single entity's components from a JSON snapshot.
    ///
    /// Finds the entity by UUID, strips all serializable components, and
    /// re-applies from the snapshot. Returns an error if the entity is not
    /// found. Tag, transform, and all optional components are restored;
    /// relationships and IdComponent are preserved as-is.
    pub fn restore_entity_from_json(scene: &mut Scene, uuid: u64, json: &str) -> EngineResult<()> {
        let entity_data: EntityData = serde_json::from_str(json)?;
        let entity = scene
            .find_entity_by_uuid(uuid)
            .ok_or_else(|| crate::error::EngineError::Asset(
                format!("Entity with UUID {} not found for undo restore", uuid),
            ))?;

        // Strip all optional serializable components so that removed components
        // are not left behind (e.g. undoing "Add SpriteRenderer").
        Self::strip_serializable_components(scene, entity);

        // Update tag if present.
        if let Some(ref td) = entity_data.tag {
            if let Some(mut tc) = scene.get_component_mut::<TagComponent>(entity) {
                tc.tag = td.tag.clone();
            }
        }

        // Re-apply all components from the snapshot.
        Self::apply_entity_data(scene, entity, &entity_data);

        // Restore relationship if present in the snapshot.
        if let Some(ref rd) = entity_data.relationship {
            scene.add_component(
                entity,
                RelationshipComponent {
                    parent: rd.parent,
                    children: rd.children.clone(),
                },
            );
        }

        Ok(())
    }

    /// Remove all optional serializable components from an entity.
    ///
    /// Preserves IdComponent, TagComponent, TransformComponent, and
    /// RelationshipComponent (which are always present or handled separately).
    fn strip_serializable_components(scene: &mut Scene, entity: Entity) {
        scene.remove_component::<CameraComponent>(entity);
        scene.remove_component::<SpriteRendererComponent>(entity);
        scene.remove_component::<CircleRendererComponent>(entity);
        scene.remove_component::<TextComponent>(entity);
        scene.remove_component::<RigidBody2DComponent>(entity);
        scene.remove_component::<BoxCollider2DComponent>(entity);
        scene.remove_component::<CircleCollider2DComponent>(entity);
        #[cfg(feature = "physics-3d")]
        {
            scene.remove_component::<RigidBody3DComponent>(entity);
            scene.remove_component::<BoxCollider3DComponent>(entity);
            scene.remove_component::<SphereCollider3DComponent>(entity);
            scene.remove_component::<CapsuleCollider3DComponent>(entity);
        }
        #[cfg(feature = "lua-scripting")]
        scene.remove_component::<LuaScriptComponent>(entity);
        scene.remove_component::<SpriteAnimatorComponent>(entity);
        scene.remove_component::<InstancedSpriteAnimator>(entity);
        scene.remove_component::<AnimationControllerComponent>(entity);
        scene.remove_component::<AudioSourceComponent>(entity);
        scene.remove_component::<AudioListenerComponent>(entity);
        scene.remove_component::<TilemapComponent>(entity);
        scene.remove_component::<ParticleEmitterComponent>(entity);
        scene.remove_component::<MeshRendererComponent>(entity);
        scene.remove_component::<SkeletalAnimationComponent>(entity);
        scene.remove_component::<DirectionalLightComponent>(entity);
        scene.remove_component::<PointLightComponent>(entity);
        scene.remove_component::<AmbientLightComponent>(entity);
        scene.remove_component::<EnvironmentComponent>(entity);
        scene.remove_component::<UIAnchorComponent>(entity);
        scene.remove_component::<UIRectComponent>(entity);
        scene.remove_component::<UIImageComponent>(entity);
        scene.remove_component::<UIInteractableComponent>(entity);
        scene.remove_component::<UILayoutComponent>(entity);
    }

    // -- Prefab serialization -------------------------------------------------

    /// Serialize an entity (and its children) to a `.ggprefab` YAML file.
    ///
    /// The root entity's parent reference is stripped — the prefab root is
    /// always a root-level entity when saved.
    pub fn serialize_prefab(scene: &Scene, root: Entity, file_path: &str) -> EngineResult<()> {
        let name = scene
            .get_component::<TagComponent>(root)
            .map(|t| t.tag.clone())
            .unwrap_or_else(|| "Prefab".into());

        let entities = Self::collect_hierarchy(scene, root);
        let mut entities_data: Vec<EntityData> = entities
            .iter()
            .map(|&e| Self::entity_to_data(scene, e))
            .collect();

        // Strip the root entity's parent reference.
        if let Some(first) = entities_data.first_mut() {
            if let Some(ref mut rel) = first.relationship {
                rel.parent = None;
            }
        }

        let prefab_data = PrefabData {
            version: SCENE_VERSION,
            name,
            entities: entities_data,
        };

        write_yaml_to_file(&prefab_data, file_path, "prefab")
    }

    /// Instantiate a prefab from a `.ggprefab` file, creating entities with
    /// fresh UUIDs. Returns the root entity on success.
    pub fn instantiate_prefab(scene: &mut Scene, file_path: &str) -> EngineResult<Entity> {
        let contents = fs::read_to_string(file_path)?;
        Self::instantiate_prefab_from_string(scene, &contents)
    }

    /// Instantiate a prefab from a YAML string, creating entities with
    /// fresh UUIDs. Returns the root entity on success.
    pub fn instantiate_prefab_from_string(scene: &mut Scene, yaml: &str) -> EngineResult<Entity> {
        let prefab_data: PrefabData = serde_yaml_ng::from_str(yaml)?;

        if prefab_data.entities.is_empty() {
            return Err(EngineError::Asset("Prefab contains no entities".into()));
        }

        Self::instantiate_prefab_entities(scene, &prefab_data.entities).ok_or_else(|| {
            EngineError::Asset("Prefab instantiation produced no root entity".into())
        })
    }

    /// Core prefab instantiation: creates entities from `EntityData` with fresh
    /// UUIDs, remapping all internal references (parent/children).
    fn instantiate_prefab_entities(
        scene: &mut Scene,
        entities_data: &[EntityData],
    ) -> Option<Entity> {
        // Build UUID remap: old → new.
        let mut uuid_remap: HashMap<u64, u64> = HashMap::new();
        for ed in entities_data {
            uuid_remap.insert(ed.id, Uuid::new().raw());
        }

        let mut root_entity = None;

        for entity_data in entities_data {
            let new_uuid = uuid_remap[&entity_data.id];
            let name = entity_data
                .tag
                .as_ref()
                .map(|t| t.tag.as_str())
                .unwrap_or("Entity");

            let entity = scene.create_entity_with_uuid(Uuid::from_raw(new_uuid), name);
            if root_entity.is_none() {
                root_entity = Some(entity);
            }

            Self::apply_entity_data(scene, entity, entity_data);

            // Remap relationship UUIDs.
            if let Some(ref rd) = entity_data.relationship {
                let remapped_parent = rd.parent.and_then(|p| uuid_remap.get(&p).copied());
                let remapped_children: Vec<u64> = rd
                    .children
                    .iter()
                    .filter_map(|c| uuid_remap.get(c).copied())
                    .collect();
                if remapped_parent.is_some() || !remapped_children.is_empty() {
                    scene.add_component(
                        entity,
                        RelationshipComponent {
                            parent: remapped_parent,
                            children: remapped_children,
                        },
                    );
                }
            }
        }

        root_entity
    }

    /// Collect an entity and all its descendants in hierarchy order (BFS).
    fn collect_hierarchy(scene: &Scene, root: Entity) -> Vec<Entity> {
        let mut result = vec![root];
        let mut i = 0;
        while i < result.len() {
            let entity = result[i];
            for child_uuid in scene.get_children(entity) {
                if let Some(child) = scene.find_entity_by_uuid(child_uuid) {
                    result.push(child);
                }
            }
            i += 1;
        }
        result
    }

    // -- Shared helpers -------------------------------------------------------

    fn scene_to_data(scene: &Scene, scene_name: Option<&str>) -> SceneData {
        let entities_data: Vec<EntityData> = scene
            .each_entity_with_tag()
            .iter()
            .map(|(entity, _name)| Self::entity_to_data(scene, *entity))
            .collect();

        SceneData {
            version: SCENE_VERSION,
            name: scene_name.unwrap_or("Untitled").to_string(),
            entities: entities_data,
        }
    }

    /// Convert a single entity's components into serializable `EntityData`.
    fn entity_to_data(scene: &Scene, entity: Entity) -> EntityData {
        let tag_data = scene
            .get_component::<TagComponent>(entity)
            .map(|tag| TagData {
                tag: tag.tag.clone(),
            });

        let transform_data =
            scene
                .get_component::<TransformComponent>(entity)
                .map(|tc| TransformData {
                    translation: tc.translation.into(),
                    rotation: [tc.rotation.x, tc.rotation.y, tc.rotation.z, tc.rotation.w],
                    scale: tc.scale.into(),
                });

        let camera_data = scene
            .get_component::<CameraComponent>(entity)
            .map(|cam| CameraData {
                camera: SceneCameraData {
                    projection_type: cam.camera.projection_type() as u32,
                    perspective_fov: cam.camera.perspective_vertical_fov(),
                    perspective_near: cam.camera.perspective_near(),
                    perspective_far: cam.camera.perspective_far(),
                    orthographic_size: cam.camera.orthographic_size(),
                    orthographic_near: cam.camera.orthographic_near(),
                    orthographic_far: cam.camera.orthographic_far(),
                },
                primary: cam.primary,
                fixed_aspect_ratio: cam.fixed_aspect_ratio,
            });

        let sprite_data = scene
            .get_component::<SpriteRendererComponent>(entity)
            .map(|sprite| SpriteData {
                color: sprite.color.into(),
                tiling_factor: sprite.tiling_factor,
                texture_handle: sprite.texture_handle.raw(),
                sorting_layer: sprite.sorting_layer,
                order_in_layer: sprite.order_in_layer,
                atlas_min: sprite.atlas_min.into(),
                atlas_max: sprite.atlas_max.into(),
            });

        let circle_data = scene
            .get_component::<CircleRendererComponent>(entity)
            .map(|circle| CircleData {
                color: circle.color.into(),
                thickness: circle.thickness,
                fade: circle.fade,
                sorting_layer: circle.sorting_layer,
                order_in_layer: circle.order_in_layer,
            });

        let text_data = scene
            .get_component::<TextComponent>(entity)
            .map(|tc| TextData {
                text: tc.text.clone(),
                font_path: tc.font_path.clone(),
                font_size: tc.font_size,
                color: tc.color.into(),
                line_spacing: tc.line_spacing,
                kerning: tc.kerning,
                sorting_layer: tc.sorting_layer,
                order_in_layer: tc.order_in_layer,
            });

        let rigidbody_2d_data = scene
            .get_component::<RigidBody2DComponent>(entity)
            .map(|rb| {
                let body_type_str = match rb.body_type {
                    RigidBody2DType::Static => "Static",
                    RigidBody2DType::Dynamic => "Dynamic",
                    RigidBody2DType::Kinematic => "Kinematic",
                };
                RigidBody2DData {
                    body_type: body_type_str.to_string(),
                    fixed_rotation: rb.fixed_rotation,
                    gravity_scale: rb.gravity_scale,
                    linear_damping: rb.linear_damping,
                    angular_damping: rb.angular_damping,
                }
            });

        let box_collider_2d_data =
            scene
                .get_component::<BoxCollider2DComponent>(entity)
                .map(|bc| BoxCollider2DData {
                    offset: bc.offset.into(),
                    size: bc.size.into(),
                    density: bc.density,
                    friction: bc.friction,
                    restitution: bc.restitution,
                    collision_layer: bc.collision_layer,
                    collision_mask: bc.collision_mask,
                    is_sensor: bc.is_sensor,
                    _restitution_threshold: 0.0,
                });

        let circle_collider_2d_data = scene
            .get_component::<CircleCollider2DComponent>(entity)
            .map(|cc| CircleCollider2DData {
                offset: cc.offset.into(),
                radius: cc.radius,
                density: cc.density,
                friction: cc.friction,
                restitution: cc.restitution,
                collision_layer: cc.collision_layer,
                collision_mask: cc.collision_mask,
                is_sensor: cc.is_sensor,
                _restitution_threshold: 0.0,
            });

        #[cfg(feature = "physics-3d")]
        let rigidbody_3d_data = scene
            .get_component::<RigidBody3DComponent>(entity)
            .map(|rb| {
                let body_type_str = match rb.body_type {
                    RigidBody3DType::Static => "Static",
                    RigidBody3DType::Dynamic => "Dynamic",
                    RigidBody3DType::Kinematic => "Kinematic",
                };
                RigidBody3DData {
                    body_type: body_type_str.to_string(),
                    lock_rotation_x: rb.lock_rotation_x,
                    lock_rotation_y: rb.lock_rotation_y,
                    lock_rotation_z: rb.lock_rotation_z,
                    gravity_scale: rb.gravity_scale,
                    linear_damping: rb.linear_damping,
                    angular_damping: rb.angular_damping,
                }
            });
        #[cfg(not(feature = "physics-3d"))]
        let rigidbody_3d_data: Option<RigidBody3DData> = None;

        #[cfg(feature = "physics-3d")]
        let box_collider_3d_data =
            scene
                .get_component::<BoxCollider3DComponent>(entity)
                .map(|bc| BoxCollider3DData {
                    offset: bc.offset.into(),
                    size: bc.size.into(),
                    density: bc.density,
                    friction: bc.friction,
                    restitution: bc.restitution,
                    collision_layer: bc.collision_layer,
                    collision_mask: bc.collision_mask,
                    is_sensor: bc.is_sensor,
                });
        #[cfg(not(feature = "physics-3d"))]
        let box_collider_3d_data: Option<BoxCollider3DData> = None;

        #[cfg(feature = "physics-3d")]
        let sphere_collider_3d_data = scene
            .get_component::<SphereCollider3DComponent>(entity)
            .map(|sc| SphereCollider3DData {
                offset: sc.offset.into(),
                radius: sc.radius,
                density: sc.density,
                friction: sc.friction,
                restitution: sc.restitution,
                collision_layer: sc.collision_layer,
                collision_mask: sc.collision_mask,
                is_sensor: sc.is_sensor,
            });
        #[cfg(not(feature = "physics-3d"))]
        let sphere_collider_3d_data: Option<SphereCollider3DData> = None;

        #[cfg(feature = "physics-3d")]
        let capsule_collider_3d_data = scene
            .get_component::<CapsuleCollider3DComponent>(entity)
            .map(|cc| CapsuleCollider3DData {
                offset: cc.offset.into(),
                half_height: cc.half_height,
                radius: cc.radius,
                density: cc.density,
                friction: cc.friction,
                restitution: cc.restitution,
                collision_layer: cc.collision_layer,
                collision_mask: cc.collision_mask,
                is_sensor: cc.is_sensor,
            });
        #[cfg(not(feature = "physics-3d"))]
        let capsule_collider_3d_data: Option<CapsuleCollider3DData> = None;

        #[cfg(feature = "lua-scripting")]
        let lua_script_data = scene
            .get_component::<LuaScriptComponent>(entity)
            .map(|lsc| {
                let fields = if lsc.field_overrides.is_empty() {
                    None
                } else {
                    Some(lsc.field_overrides.clone())
                };
                LuaScriptData {
                    script_path: lsc.script_path.clone(),
                    fields,
                }
            });
        #[cfg(not(feature = "lua-scripting"))]
        let lua_script_data: Option<LuaScriptData> = None;
        let sprite_animator_data =
            scene
                .get_component::<SpriteAnimatorComponent>(entity)
                .map(|sa| SpriteAnimatorData {
                    cell_size: sa.cell_size.into(),
                    columns: sa.columns,
                    clips: convert_clips(&sa.clips),
                    default_clip: sa.default_clip.clone(),
                    speed_scale: sa.speed_scale,
                });

        let instanced_animator_data =
            scene
                .get_component::<InstancedSpriteAnimator>(entity)
                .map(|ia| InstancedSpriteAnimatorData {
                    cell_size: ia.cell_size.into(),
                    columns: ia.columns,
                    clips: convert_clips(&ia.clips),
                    default_clip: ia.default_clip.clone(),
                    speed_scale: ia.speed_scale,
                });

        let animation_controller_data = scene
            .get_component::<AnimationControllerComponent>(entity)
            .map(|ctrl| AnimationControllerData {
                transitions: ctrl
                    .transitions
                    .iter()
                    .map(|t| {
                        let (cond_type, param_name, bool_value, float_ordering, float_threshold) =
                            match &t.condition {
                                TransitionCondition::OnFinished => (
                                    "OnFinished".into(),
                                    String::new(),
                                    false,
                                    String::new(),
                                    0.0,
                                ),
                                TransitionCondition::ParamBool(name, val) => {
                                    ("ParamBool".into(), name.clone(), *val, String::new(), 0.0)
                                }
                                TransitionCondition::ParamFloat(name, ord, thresh) => {
                                    let ord_str = match ord {
                                        FloatOrdering::Greater => "Greater",
                                        FloatOrdering::Less => "Less",
                                        FloatOrdering::GreaterOrEqual => "GreaterOrEqual",
                                        FloatOrdering::LessOrEqual => "LessOrEqual",
                                    };
                                    (
                                        "ParamFloat".into(),
                                        name.clone(),
                                        false,
                                        ord_str.into(),
                                        *thresh,
                                    )
                                }
                            };
                        AnimationTransitionData {
                            from: t.from.clone(),
                            to: t.to.clone(),
                            condition_type: cond_type,
                            param_name,
                            bool_value,
                            float_ordering,
                            float_threshold,
                        }
                    })
                    .collect(),
                bool_params: ctrl.bool_params.clone(),
                float_params: ctrl.float_params.clone(),
            });

        let relationship_data = scene
            .get_component::<RelationshipComponent>(entity)
            .filter(|r| r.has_relationships())
            .map(|r| RelationshipData {
                parent: r.parent,
                children: r.children.clone(),
            });

        let audio_source_data = scene
            .get_component::<AudioSourceComponent>(entity)
            .map(|asc| AudioSourceData {
                audio_handle: asc.audio_handle.raw(),
                volume: asc.volume,
                pitch: asc.pitch,
                looping: asc.looping,
                play_on_start: asc.play_on_start,
                streaming: asc.streaming,
                spatial: asc.spatial,
                hrtf: asc.hrtf,
                min_distance: asc.min_distance,
                max_distance: asc.max_distance,
                category: match asc.category {
                    AudioCategory::SFX => String::new(),
                    other => other.label().to_string(),
                },
            });

        let audio_listener_data = scene
            .get_component::<AudioListenerComponent>(entity)
            .map(|al| AudioListenerData { active: al.active });

        let tilemap_data = scene
            .get_component::<TilemapComponent>(entity)
            .map(|tm| TilemapData {
                width: tm.width,
                height: tm.height,
                tile_size: tm.tile_size.into(),
                texture_handle: tm.texture_handle.raw(),
                tileset_columns: tm.tileset_columns,
                cell_size: tm.cell_size.into(),
                spacing: tm.spacing.into(),
                margin: tm.margin.into(),
                tiles_legacy: Vec::new(),
                tiles_b64: TilemapData::encode_tiles(&tm.tiles),
                sorting_layer: tm.sorting_layer,
                order_in_layer: tm.order_in_layer,
            });

        let uuid = scene
            .get_component::<IdComponent>(entity)
            .map(|id| id.id.raw())
            .unwrap_or(0);

        EntityData {
            id: uuid,
            tag: tag_data,
            transform: transform_data,
            camera: camera_data,
            sprite: sprite_data,
            circle: circle_data,
            text: text_data,
            rigidbody_2d: rigidbody_2d_data,
            box_collider_2d: box_collider_2d_data,
            circle_collider_2d: circle_collider_2d_data,
            rigidbody_3d: rigidbody_3d_data,
            box_collider_3d: box_collider_3d_data,
            sphere_collider_3d: sphere_collider_3d_data,
            capsule_collider_3d: capsule_collider_3d_data,
            lua_script: lua_script_data,
            sprite_animator: sprite_animator_data,
            instanced_animator: instanced_animator_data,
            animation_controller: animation_controller_data,
            relationship: relationship_data,
            tilemap: tilemap_data,
            audio_source: audio_source_data,
            audio_listener: audio_listener_data,
            particle_emitter: scene
                .get_component::<ParticleEmitterComponent>(entity)
                .map(|pe| ParticleEmitterData {
                    emit_rate: pe.emit_rate,
                    max_particles: pe.max_particles,
                    playing: pe.playing,
                    velocity: pe.velocity.into(),
                    velocity_variation: pe.velocity_variation.into(),
                    color_begin: pe.color_begin.into(),
                    color_end: pe.color_end.into(),
                    size_begin: pe.size_begin,
                    size_end: pe.size_end,
                    size_variation: pe.size_variation,
                    lifetime: pe.lifetime,
                }),
            mesh_renderer: scene
                .get_component::<MeshRendererComponent>(entity)
                .map(|mc| {
                    let (prim_str, mesh_asset) = match &mc.mesh_source {
                        MeshSource::Primitive(p) => {
                            let s = match p {
                                MeshPrimitive::Cube => "Cube",
                                MeshPrimitive::Sphere => "Sphere",
                                MeshPrimitive::Plane => "Plane",
                                MeshPrimitive::Cylinder => "Cylinder",
                                MeshPrimitive::Cone => "Cone",
                                MeshPrimitive::Torus => "Torus",
                                MeshPrimitive::Capsule => "Capsule",
                            };
                            (s.to_string(), 0)
                        }
                        MeshSource::Asset(uuid) => (String::new(), uuid.raw()),
                    };
                    MeshRendererData {
                        primitive: prim_str,
                        color: mc.color.into(),
                        metallic: mc.metallic,
                        roughness: mc.roughness,
                        emissive_color: mc.emissive_color.into(),
                        emissive_strength: mc.emissive_strength,
                        albedo_texture: mc.texture_handle.raw(),
                        mesh_asset,
                        normal_texture: mc.normal_texture_handle.raw(),
                        cast_alpha_shadow: mc.cast_alpha_shadow,
                    }
                }),
            skeletal_animation: scene
                .get_component::<SkeletalAnimationComponent>(entity)
                .map(|sac| SkeletalAnimationData {
                    mesh_asset: sac.mesh_asset.raw(),
                    speed: sac.speed,
                    looping: sac.looping,
                    playing: sac.playing,
                    default_clip: sac.current_clip_name().unwrap_or("").to_string(),
                }),
            directional_light: scene
                .get_component::<DirectionalLightComponent>(entity)
                .map(|dl| DirectionalLightData {
                    _direction: None,
                    color: dl.color.into(),
                    intensity: dl.intensity,
                    cast_shadows: dl.cast_shadows,
                    shadow_distance: dl.shadow_distance,
                    shadow_cull_front_faces: dl.shadow_cull_front_faces,
                }),
            point_light: scene
                .get_component::<PointLightComponent>(entity)
                .map(|pl| PointLightData {
                    color: pl.color.into(),
                    intensity: pl.intensity,
                    radius: pl.radius,
                }),
            ambient_light: scene
                .get_component::<AmbientLightComponent>(entity)
                .map(|al| AmbientLightData {
                    color: al.color.into(),
                    intensity: al.intensity,
                }),
            environment: scene
                .get_component::<EnvironmentComponent>(entity)
                .map(|ec| EnvironmentData {
                    environment_handle: ec.environment_handle,
                    skybox_exposure: ec.skybox_exposure,
                    ibl_intensity: ec.ibl_intensity,
                    skybox_rotation: ec.skybox_rotation,
                    show_skybox: ec.show_skybox,
                }),
            ui_anchor: scene
                .get_component::<UIAnchorComponent>(entity)
                .map(|ua| UIAnchorData {
                    anchor: ua.anchor.into(),
                    offset: ua.offset.into(),
                }),
            ui_rect: scene
                .get_component::<UIRectComponent>(entity)
                .map(|r| UIRectData {
                    size: r.size.into(),
                    pivot: r.pivot.into(),
                    raycast_target: r.raycast_target,
                }),
            ui_image: scene
                .get_component::<UIImageComponent>(entity)
                .map(|img| UIImageData {
                    color: img.color.into(),
                    texture_handle: img.texture_handle.raw(),
                    border: img.border,
                    fill_center: img.fill_center,
                    sorting_layer: img.sorting_layer,
                    order_in_layer: img.order_in_layer,
                }),
            ui_interactable: scene
                .get_component::<UIInteractableComponent>(entity)
                .map(|inter| UIInteractableData {
                    interactable: inter.interactable,
                    hover_color: inter.hover_color.map(|c| c.into()),
                    press_color: inter.press_color.map(|c| c.into()),
                    disabled_color: inter.disabled_color.map(|c| c.into()),
                }),
            ui_layout: scene
                .get_component::<UILayoutComponent>(entity)
                .map(|layout| UILayoutData {
                    direction: match layout.direction {
                        UILayoutDirection::Vertical => "Vertical".to_string(),
                        UILayoutDirection::Horizontal => "Horizontal".to_string(),
                    },
                    spacing: layout.spacing,
                    alignment: match layout.alignment {
                        UILayoutAlignment::Start => "Start".to_string(),
                        UILayoutAlignment::Center => "Center".to_string(),
                        UILayoutAlignment::End => "End".to_string(),
                    },
                    padding: layout.padding,
                }),
        }
    }

    fn data_to_scene(scene: &mut Scene, scene_data: &SceneData) {
        for entity_data in &scene_data.entities {
            let name = entity_data
                .tag
                .as_ref()
                .map(|t| t.tag.as_str())
                .unwrap_or("Entity");

            let uuid = Uuid::from_raw(entity_data.id);
            let entity = scene.create_entity_with_uuid(uuid, name);

            Self::apply_entity_data(scene, entity, entity_data);

            // RelationshipComponent — applied only if present in the file.
            if let Some(ref rd) = entity_data.relationship {
                scene.add_component(
                    entity,
                    RelationshipComponent {
                        parent: rd.parent,
                        children: rd.children.clone(),
                    },
                );
            }
        }
    }

    /// Apply component data from `EntityData` onto an already-created entity.
    ///
    /// Does NOT apply `RelationshipComponent` — callers handle relationships
    /// separately (scene deserialization uses raw UUIDs, prefab instantiation
    /// remaps them).
    fn apply_entity_data(scene: &mut Scene, entity: Entity, entity_data: &EntityData) {
        // TransformComponent — always present on newly created entities,
        // so we just update the values.
        if let Some(ref td) = entity_data.transform {
            if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(entity) {
                tc.translation = Vec3::from(td.translation);
                tc.set_rotation_quat(glam::Quat::from_xyzw(
                    td.rotation[0],
                    td.rotation[1],
                    td.rotation[2],
                    td.rotation[3],
                ));
                tc.scale = Vec3::from(td.scale);
            }
        }

        // CameraComponent
        if let Some(ref cd) = entity_data.camera {
            let mut cam = SceneCamera::default();
            let proj_type = match cd.camera.projection_type {
                0 => ProjectionType::Perspective,
                _ => ProjectionType::Orthographic,
            };
            cam.set_orthographic(
                cd.camera.orthographic_size,
                cd.camera.orthographic_near,
                cd.camera.orthographic_far,
            );
            cam.set_perspective(
                cd.camera.perspective_fov,
                cd.camera.perspective_near,
                cd.camera.perspective_far,
            );
            cam.set_projection_type(proj_type);
            scene.add_component(
                entity,
                CameraComponent {
                    camera: cam,
                    primary: cd.primary,
                    fixed_aspect_ratio: cd.fixed_aspect_ratio,
                },
            );
        }

        // SpriteRendererComponent
        if let Some(ref sd) = entity_data.sprite {
            let mut sprite = SpriteRendererComponent::new(Vec4::from(sd.color));
            sprite.tiling_factor = sd.tiling_factor;
            sprite.texture_handle = Uuid::from_raw(sd.texture_handle);
            sprite.sorting_layer = sd.sorting_layer;
            sprite.order_in_layer = sd.order_in_layer;
            sprite.atlas_min = Vec2::from(sd.atlas_min);
            sprite.atlas_max = Vec2::from(sd.atlas_max);
            scene.add_component(entity, sprite);
        }

        // CircleRendererComponent
        if let Some(ref cd) = entity_data.circle {
            scene.add_component(
                entity,
                CircleRendererComponent {
                    color: Vec4::from(cd.color),
                    thickness: cd.thickness,
                    fade: cd.fade,
                    sorting_layer: cd.sorting_layer,
                    order_in_layer: cd.order_in_layer,
                },
            );
        }

        // TextComponent
        if let Some(ref td) = entity_data.text {
            scene.add_component(
                entity,
                TextComponent {
                    text: td.text.clone(),
                    font_path: td.font_path.clone(),
                    font: None,
                    font_size: td.font_size,
                    color: Vec4::from(td.color),
                    line_spacing: td.line_spacing,
                    kerning: td.kerning,
                    sorting_layer: td.sorting_layer,
                    order_in_layer: td.order_in_layer,
                },
            );
        }

        // RigidBody2DComponent
        if let Some(ref rbd) = entity_data.rigidbody_2d {
            let body_type = match rbd.body_type.as_str() {
                "Dynamic" => RigidBody2DType::Dynamic,
                "Kinematic" => RigidBody2DType::Kinematic,
                _ => RigidBody2DType::Static,
            };
            let mut rb = RigidBody2DComponent::new(body_type);
            rb.fixed_rotation = rbd.fixed_rotation;
            rb.gravity_scale = rbd.gravity_scale;
            rb.linear_damping = rbd.linear_damping;
            rb.angular_damping = rbd.angular_damping;
            scene.add_component(entity, rb);
        }

        // BoxCollider2DComponent
        if let Some(ref bcd) = entity_data.box_collider_2d {
            scene.add_component(
                entity,
                BoxCollider2DComponent {
                    offset: Vec2::from(bcd.offset),
                    size: Vec2::from(bcd.size),
                    density: bcd.density,
                    friction: bcd.friction,
                    restitution: bcd.restitution,
                    collision_layer: bcd.collision_layer,
                    collision_mask: bcd.collision_mask,
                    is_sensor: bcd.is_sensor,
                    runtime_fixture: None,
                },
            );
        }

        // CircleCollider2DComponent
        if let Some(ref ccd) = entity_data.circle_collider_2d {
            scene.add_component(
                entity,
                CircleCollider2DComponent {
                    offset: Vec2::from(ccd.offset),
                    radius: ccd.radius,
                    density: ccd.density,
                    friction: ccd.friction,
                    restitution: ccd.restitution,
                    collision_layer: ccd.collision_layer,
                    collision_mask: ccd.collision_mask,
                    is_sensor: ccd.is_sensor,
                    runtime_fixture: None,
                },
            );
        }

        // RigidBody3DComponent
        #[cfg(feature = "physics-3d")]
        if let Some(ref rbd) = entity_data.rigidbody_3d {
            let body_type = match rbd.body_type.as_str() {
                "Dynamic" => RigidBody3DType::Dynamic,
                "Kinematic" => RigidBody3DType::Kinematic,
                _ => RigidBody3DType::Static,
            };
            let mut rb = RigidBody3DComponent::new(body_type);
            rb.lock_rotation_x = rbd.lock_rotation_x;
            rb.lock_rotation_y = rbd.lock_rotation_y;
            rb.lock_rotation_z = rbd.lock_rotation_z;
            rb.gravity_scale = rbd.gravity_scale;
            rb.linear_damping = rbd.linear_damping;
            rb.angular_damping = rbd.angular_damping;
            scene.add_component(entity, rb);
        }

        // BoxCollider3DComponent
        #[cfg(feature = "physics-3d")]
        if let Some(ref bcd) = entity_data.box_collider_3d {
            scene.add_component(
                entity,
                BoxCollider3DComponent {
                    offset: Vec3::from(bcd.offset),
                    size: Vec3::from(bcd.size),
                    density: bcd.density,
                    friction: bcd.friction,
                    restitution: bcd.restitution,
                    collision_layer: bcd.collision_layer,
                    collision_mask: bcd.collision_mask,
                    is_sensor: bcd.is_sensor,
                    runtime_fixture: None,
                },
            );
        }

        // SphereCollider3DComponent
        #[cfg(feature = "physics-3d")]
        if let Some(ref scd) = entity_data.sphere_collider_3d {
            scene.add_component(
                entity,
                SphereCollider3DComponent {
                    offset: Vec3::from(scd.offset),
                    radius: scd.radius,
                    density: scd.density,
                    friction: scd.friction,
                    restitution: scd.restitution,
                    collision_layer: scd.collision_layer,
                    collision_mask: scd.collision_mask,
                    is_sensor: scd.is_sensor,
                    runtime_fixture: None,
                },
            );
        }

        // CapsuleCollider3DComponent
        #[cfg(feature = "physics-3d")]
        if let Some(ref ccd) = entity_data.capsule_collider_3d {
            scene.add_component(
                entity,
                CapsuleCollider3DComponent {
                    offset: Vec3::from(ccd.offset),
                    half_height: ccd.half_height,
                    radius: ccd.radius,
                    density: ccd.density,
                    friction: ccd.friction,
                    restitution: ccd.restitution,
                    collision_layer: ccd.collision_layer,
                    collision_mask: ccd.collision_mask,
                    is_sensor: ccd.is_sensor,
                    runtime_fixture: None,
                },
            );
        }

        // LuaScriptComponent
        #[cfg(feature = "lua-scripting")]
        if let Some(ref lsd) = entity_data.lua_script {
            let mut lsc = LuaScriptComponent::new(&lsd.script_path);
            if let Some(ref fields) = lsd.fields {
                lsc.field_overrides = fields.clone();
            }
            scene.add_component(entity, lsc);
        }

        // SpriteAnimatorComponent
        if let Some(ref sad) = entity_data.sprite_animator {
            let clips = sad
                .clips
                .iter()
                .map(|c| AnimationClip {
                    name: c.name.clone(),
                    start_frame: c.start_frame,
                    end_frame: c.end_frame,
                    fps: c.fps,
                    looping: c.looping,
                    texture_handle: Uuid::from_raw(c.texture_handle),
                    texture: None,
                })
                .collect();
            scene.add_component(
                entity,
                SpriteAnimatorComponent {
                    cell_size: Vec2::from(sad.cell_size),
                    columns: sad.columns,
                    clips,
                    default_clip: sad.default_clip.clone(),
                    speed_scale: sad.speed_scale,
                    ..Default::default()
                },
            );
        }

        // InstancedSpriteAnimator
        if let Some(ref iad) = entity_data.instanced_animator {
            let clips = iad
                .clips
                .iter()
                .map(|c| AnimationClip {
                    name: c.name.clone(),
                    start_frame: c.start_frame,
                    end_frame: c.end_frame,
                    fps: c.fps,
                    looping: c.looping,
                    texture_handle: Uuid::from_raw(c.texture_handle),
                    texture: None,
                })
                .collect();
            scene.add_component(
                entity,
                InstancedSpriteAnimator {
                    cell_size: Vec2::from(iad.cell_size),
                    columns: iad.columns,
                    clips,
                    default_clip: iad.default_clip.clone(),
                    speed_scale: iad.speed_scale,
                    ..Default::default()
                },
            );
        }

        // AnimationControllerComponent
        if let Some(ref acd) = entity_data.animation_controller {
            let transitions = acd
                .transitions
                .iter()
                .filter_map(|t| {
                    let condition = match t.condition_type.as_str() {
                        "OnFinished" => TransitionCondition::OnFinished,
                        "ParamBool" => {
                            TransitionCondition::ParamBool(t.param_name.clone(), t.bool_value)
                        }
                        "ParamFloat" => {
                            let ordering = match t.float_ordering.as_str() {
                                "Greater" => FloatOrdering::Greater,
                                "Less" => FloatOrdering::Less,
                                "GreaterOrEqual" => FloatOrdering::GreaterOrEqual,
                                "LessOrEqual" => FloatOrdering::LessOrEqual,
                                _ => {
                                    log::warn!(
                                        "Unknown float ordering '{}', defaulting to Greater",
                                        t.float_ordering
                                    );
                                    FloatOrdering::Greater
                                }
                            };
                            TransitionCondition::ParamFloat(
                                t.param_name.clone(),
                                ordering,
                                t.float_threshold,
                            )
                        }
                        other => {
                            log::warn!("Unknown transition condition type '{}'", other);
                            return None;
                        }
                    };
                    Some(AnimationTransition {
                        from: t.from.clone(),
                        to: t.to.clone(),
                        condition,
                    })
                })
                .collect();
            scene.add_component(
                entity,
                AnimationControllerComponent {
                    transitions,
                    bool_params: acd.bool_params.clone(),
                    float_params: acd.float_params.clone(),
                },
            );
        }

        // AudioSourceComponent
        if let Some(ref asd) = entity_data.audio_source {
            scene.add_component(
                entity,
                AudioSourceComponent {
                    audio_handle: Uuid::from_raw(asd.audio_handle),
                    volume: asd.volume,
                    pitch: asd.pitch,
                    looping: asd.looping,
                    play_on_start: asd.play_on_start,
                    streaming: asd.streaming,
                    spatial: asd.spatial,
                    hrtf: asd.hrtf,
                    min_distance: asd.min_distance,
                    max_distance: asd.max_distance,
                    category: AudioCategory::from_str_loose(&asd.category).unwrap_or_default(),
                    resolved_path: None,
                },
            );
        }

        // TilemapComponent
        if let Some(ref td) = entity_data.tilemap {
            scene.add_component(
                entity,
                TilemapComponent {
                    width: td.width,
                    height: td.height,
                    tile_size: Vec2::from(td.tile_size),
                    texture_handle: Uuid::from_raw(td.texture_handle),
                    texture: None,
                    tileset_columns: td.tileset_columns,
                    cell_size: Vec2::from(td.cell_size),
                    spacing: Vec2::from(td.spacing),
                    margin: Vec2::from(td.margin),
                    tiles: td.decode_tiles(),
                    sorting_layer: td.sorting_layer,
                    order_in_layer: td.order_in_layer,
                },
            );
        }

        // AudioListenerComponent
        if let Some(ref ald) = entity_data.audio_listener {
            scene.add_component(entity, AudioListenerComponent { active: ald.active });
        }

        // ParticleEmitterComponent
        if let Some(ref ped) = entity_data.particle_emitter {
            scene.add_component(
                entity,
                ParticleEmitterComponent {
                    emit_rate: ped.emit_rate,
                    max_particles: ped.max_particles,
                    playing: ped.playing,
                    velocity: Vec2::from(ped.velocity),
                    velocity_variation: Vec2::from(ped.velocity_variation),
                    color_begin: Vec4::from(ped.color_begin),
                    color_end: Vec4::from(ped.color_end),
                    size_begin: ped.size_begin,
                    size_end: ped.size_end,
                    size_variation: ped.size_variation,
                    lifetime: ped.lifetime,
                },
            );
        }

        // MeshRendererComponent
        if let Some(ref mrd) = entity_data.mesh_renderer {
            let mesh_source = if mrd.mesh_asset != 0 {
                MeshSource::Asset(Uuid::from_raw(mrd.mesh_asset))
            } else {
                let primitive = match mrd.primitive.as_str() {
                    "Sphere" => MeshPrimitive::Sphere,
                    "Plane" => MeshPrimitive::Plane,
                    "Cylinder" => MeshPrimitive::Cylinder,
                    "Cone" => MeshPrimitive::Cone,
                    "Torus" => MeshPrimitive::Torus,
                    "Capsule" => MeshPrimitive::Capsule,
                    _ => MeshPrimitive::Cube,
                };
                MeshSource::Primitive(primitive)
            };
            scene.add_component(
                entity,
                MeshRendererComponent {
                    mesh_source,
                    color: Vec4::from(mrd.color),
                    metallic: mrd.metallic,
                    roughness: mrd.roughness,
                    emissive_color: Vec3::from(mrd.emissive_color),
                    emissive_strength: mrd.emissive_strength,
                    texture: None,
                    texture_handle: Uuid::from_raw(mrd.albedo_texture),
                    normal_texture: None,
                    normal_texture_handle: Uuid::from_raw(mrd.normal_texture),
                    loaded_mesh: None,
                    local_bounds: None,
                    cast_alpha_shadow: mrd.cast_alpha_shadow,
                    vertex_array: None,
                },
            );
        }

        // SkeletalAnimationComponent
        if let Some(ref sad) = entity_data.skeletal_animation {
            if sad.mesh_asset != 0 {
                let mut sac =
                    SkeletalAnimationComponent::from_asset(Uuid::from_raw(sad.mesh_asset));
                sac.speed = sad.speed;
                sac.looping = sad.looping;
                sac.playing = sad.playing;
                // default_clip is resolved after asset loads (clip names not available yet).
                scene.add_component(entity, sac);
            }
        }

        // DirectionalLightComponent
        if let Some(ref dl) = entity_data.directional_light {
            scene.add_component(
                entity,
                DirectionalLightComponent {
                    color: Vec3::from(dl.color),
                    intensity: dl.intensity,
                    cast_shadows: dl.cast_shadows,
                    shadow_distance: dl.shadow_distance,
                    shadow_cull_front_faces: dl.shadow_cull_front_faces,
                },
            );
        }

        // PointLightComponent
        if let Some(ref pl) = entity_data.point_light {
            scene.add_component(
                entity,
                PointLightComponent {
                    color: Vec3::from(pl.color),
                    intensity: pl.intensity,
                    radius: pl.radius,
                },
            );
        }

        // AmbientLightComponent
        if let Some(ref al) = entity_data.ambient_light {
            scene.add_component(
                entity,
                AmbientLightComponent {
                    color: Vec3::from(al.color),
                    intensity: al.intensity,
                },
            );
        }

        // EnvironmentComponent
        if let Some(ref ec) = entity_data.environment {
            scene.add_component(
                entity,
                EnvironmentComponent {
                    environment_handle: ec.environment_handle,
                    loaded: false,
                    skybox_exposure: ec.skybox_exposure,
                    ibl_intensity: ec.ibl_intensity,
                    skybox_rotation: ec.skybox_rotation,
                    show_skybox: ec.show_skybox,
                },
            );
        }

        // UIAnchorComponent
        if let Some(ref ua) = entity_data.ui_anchor {
            scene.add_component(
                entity,
                UIAnchorComponent {
                    anchor: Vec2::from(ua.anchor),
                    offset: Vec2::from(ua.offset),
                },
            );
        }

        // UIRectComponent
        if let Some(ref r) = entity_data.ui_rect {
            scene.add_component(
                entity,
                UIRectComponent {
                    size: Vec2::from(r.size),
                    pivot: Vec2::from(r.pivot),
                    raycast_target: r.raycast_target,
                },
            );
        }

        // UIImageComponent
        if let Some(ref img) = entity_data.ui_image {
            scene.add_component(
                entity,
                UIImageComponent {
                    color: Vec4::from(img.color),
                    texture: None,
                    texture_handle: Uuid::from_raw(img.texture_handle),
                    border: img.border,
                    fill_center: img.fill_center,
                    sorting_layer: img.sorting_layer,
                    order_in_layer: img.order_in_layer,
                },
            );
        }

        // UIInteractableComponent
        if let Some(ref inter) = entity_data.ui_interactable {
            scene.add_component(
                entity,
                UIInteractableComponent {
                    interactable: inter.interactable,
                    hover_color: inter.hover_color.map(Vec4::from),
                    press_color: inter.press_color.map(Vec4::from),
                    disabled_color: inter.disabled_color.map(Vec4::from),
                    state: Default::default(),
                },
            );
        }

        // UILayoutComponent
        if let Some(ref layout) = entity_data.ui_layout {
            scene.add_component(
                entity,
                UILayoutComponent {
                    direction: match layout.direction.as_str() {
                        "Horizontal" => UILayoutDirection::Horizontal,
                        _ => UILayoutDirection::Vertical,
                    },
                    spacing: layout.spacing,
                    alignment: match layout.alignment.as_str() {
                        "Start" => UILayoutAlignment::Start,
                        "End" => UILayoutAlignment::End,
                        _ => UILayoutAlignment::Center,
                    },
                    padding: layout.padding,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;

    #[test]
    fn round_trip_serialize_deserialize() {
        // Build a test scene.
        let mut scene = Scene::new();

        let e1 = scene.create_entity_with_tag("Test Entity");
        if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(e1) {
            tc.translation = Vec3::new(1.0, 2.0, 3.0);
            tc.set_euler_angles(Vec3::new(0.1, 0.2, 0.3));
            tc.scale = Vec3::new(2.0, 2.0, 2.0);
        }
        let mut sprite = SpriteRendererComponent::new(Vec4::new(0.8, 0.2, 0.2, 1.0));
        sprite.texture_handle = crate::uuid::Uuid::from_raw(12345);
        scene.add_component(e1, sprite);

        let e2 = scene.create_entity_with_tag("Camera");
        scene.add_component(e2, CameraComponent::default());

        // Serialize.
        let path = std::env::temp_dir()
            .join("gg_test_scene.ggscene")
            .to_string_lossy()
            .to_string();
        SceneSerializer::serialize(&scene, &path, Some("gg_test_scene")).unwrap();

        // Deserialize into a fresh scene.
        let mut loaded = Scene::new();
        SceneSerializer::deserialize(&mut loaded, &path).unwrap();
        assert_eq!(loaded.entity_count(), 2);

        // Verify entities by tag.
        let entities = loaded.each_entity_with_tag();
        let names: Vec<&str> = entities.iter().map(|(_, name)| name.as_str()).collect();
        assert!(names.contains(&"Test Entity"));
        assert!(names.contains(&"Camera"));

        // Verify transform values on "Test Entity".
        let (test_entity, _) = entities.iter().find(|(_, n)| n == "Test Entity").unwrap();
        let tc = loaded
            .get_component::<TransformComponent>(*test_entity)
            .unwrap();
        assert_eq!(tc.translation, Vec3::new(1.0, 2.0, 3.0));
        let expected_quat = glam::Quat::from_euler(glam::EulerRot::XYZ, 0.1, 0.2, 0.3);
        assert!(
            tc.rotation.abs_diff_eq(expected_quat, 1e-6),
            "rotation mismatch: {:?} vs {:?}",
            tc.rotation,
            expected_quat
        );
        assert_eq!(tc.scale, Vec3::new(2.0, 2.0, 2.0));

        // Verify sprite (color + texture_handle round-trip).
        let sprite = loaded
            .get_component::<SpriteRendererComponent>(*test_entity)
            .unwrap();
        assert_eq!(sprite.color, Vec4::new(0.8, 0.2, 0.2, 1.0));
        assert_eq!(sprite.texture_handle.raw(), 12345);

        // Verify camera entity.
        let (cam_entity, _) = entities.iter().find(|(_, n)| n == "Camera").unwrap();
        assert!(loaded.has_component::<CameraComponent>(*cam_entity));
        let cam = loaded
            .get_component::<CameraComponent>(*cam_entity)
            .unwrap();
        assert!(cam.primary);

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tilemap_round_trip() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Tilemap");
        let tilemap = crate::scene::TilemapComponent {
            width: 3,
            height: 2,
            tile_size: Vec2::new(1.5, 1.5),
            tileset_columns: 4,
            cell_size: Vec2::new(16.0, 16.0),
            spacing: Vec2::new(2.0, 2.0),
            margin: Vec2::new(1.0, 1.0),
            tiles: vec![0, 1, -1, 3, -1, 2],
            texture_handle: crate::uuid::Uuid::from_raw(99999),
            ..Default::default()
        };
        scene.add_component(e, tilemap);

        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        let mut loaded = Scene::new();
        SceneSerializer::deserialize_from_string(&mut loaded, &yaml).unwrap();
        assert_eq!(loaded.entity_count(), 1);

        let entities = loaded.each_entity_with_tag();
        let (ent, _) = entities.iter().find(|(_, n)| n == "Tilemap").unwrap();
        let tm = loaded
            .get_component::<crate::scene::TilemapComponent>(*ent)
            .unwrap();
        assert_eq!(tm.width, 3);
        assert_eq!(tm.height, 2);
        assert_eq!(tm.tile_size, Vec2::new(1.5, 1.5));
        assert_eq!(tm.tileset_columns, 4);
        assert_eq!(tm.cell_size, Vec2::new(16.0, 16.0));
        assert_eq!(tm.spacing, Vec2::new(2.0, 2.0));
        assert_eq!(tm.margin, Vec2::new(1.0, 1.0));
        assert_eq!(tm.tiles, vec![0, 1, -1, 3, -1, 2]);
        assert_eq!(tm.texture_handle.raw(), 99999);
        assert!(tm.texture.is_none());
    }

    #[test]
    fn audio_source_round_trip() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("AudioEntity");
        let audio = crate::scene::AudioSourceComponent {
            audio_handle: crate::uuid::Uuid::from_raw(77777),
            volume: 0.75,
            pitch: 1.2,
            looping: true,
            play_on_start: true,
            streaming: true,
            spatial: true,
            hrtf: true,
            min_distance: 2.0,
            max_distance: 30.0,
            category: crate::scene::AudioCategory::Music,
            resolved_path: None,
        };
        scene.add_component(e, audio);

        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        let mut loaded = Scene::new();
        SceneSerializer::deserialize_from_string(&mut loaded, &yaml).unwrap();
        assert_eq!(loaded.entity_count(), 1);

        let entities = loaded.each_entity_with_tag();
        let (ent, _) = entities.iter().find(|(_, n)| n == "AudioEntity").unwrap();
        let ac = loaded
            .get_component::<crate::scene::AudioSourceComponent>(*ent)
            .unwrap();
        assert_eq!(ac.audio_handle.raw(), 77777);
        assert!((ac.volume - 0.75).abs() < 0.001);
        assert!((ac.pitch - 1.2).abs() < 0.001);
        assert!(ac.looping);
        assert!(ac.play_on_start);
        assert!(ac.streaming);
        assert!(ac.spatial);
        assert!((ac.min_distance - 2.0).abs() < 0.001);
        assert!((ac.max_distance - 30.0).abs() < 0.001);
        assert_eq!(ac.category, crate::scene::AudioCategory::Music);
        assert!(ac.resolved_path.is_none());
    }

    #[test]
    fn demo_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/lua_camera_follow.ggscene");
        let mut scene = Scene::new();
        SceneSerializer::deserialize_from_string(&mut scene, yaml)
            .expect("Failed to deserialize demo scene");
        assert_eq!(scene.entity_count(), 7);

        let entities = scene.each_entity_with_tag();
        let names: Vec<&str> = entities.iter().map(|(_, name)| name.as_str()).collect();
        assert!(names.contains(&"Camera"));
        assert!(names.contains(&"Player"));
        assert!(names.contains(&"Ground"));
        assert!(names.contains(&"GameSession"));

        // Verify Lua scripts were loaded.
        let (player, _) = entities.iter().find(|(_, n)| n == "Player").unwrap();
        assert!(scene.has_component::<LuaScriptComponent>(*player));

        // Verify physics components.
        assert!(scene.has_component::<RigidBody2DComponent>(*player));
        assert!(scene.has_component::<BoxCollider2DComponent>(*player));
    }

    #[test]
    fn tilemap_test_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/tilemap_test.ggscene");
        let mut scene = Scene::new();
        SceneSerializer::deserialize_from_string(&mut scene, yaml)
            .expect("Failed to deserialize tilemap_test scene");
        assert_eq!(scene.entity_count(), 2);

        let entities = scene.each_entity_with_tag();
        let (tm_ent, _) = entities.iter().find(|(_, n)| n == "Empty Entity").unwrap();
        let tm = scene
            .get_component::<crate::scene::TilemapComponent>(*tm_ent)
            .unwrap();
        assert_eq!(tm.width, 16);
        assert_eq!(tm.height, 10);
        assert_eq!(tm.tiles.len(), 160);
        assert_eq!(tm.texture_handle.raw(), 2841034490373146);
    }

    #[test]
    fn audio_test_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/audio_test.ggscene");
        let mut scene = Scene::new();
        SceneSerializer::deserialize_from_string(&mut scene, yaml)
            .expect("Failed to deserialize audio_test scene");
        assert_eq!(scene.entity_count(), 12);

        let entities = scene.each_entity_with_tag();

        // Verify Lua-controlled audio entity.
        let (audio_ent, _) = entities.iter().find(|(_, n)| n == "Audio Player").unwrap();
        let ac = scene
            .get_component::<crate::scene::AudioSourceComponent>(*audio_ent)
            .unwrap();
        assert_eq!(ac.audio_handle.raw(), 1001);
        assert!(!ac.play_on_start);
        assert!(ac.looping);

        // Verify auto-play entity.
        let (auto_ent, _) = entities.iter().find(|(_, n)| n == "Auto Player").unwrap();
        let ac2 = scene
            .get_component::<crate::scene::AudioSourceComponent>(*auto_ent)
            .unwrap();
        assert!(ac2.play_on_start);
        assert!(!ac2.looping);
        assert!((ac2.pitch - 1.5).abs() < 0.001);
        assert!((ac2.volume - 0.3).abs() < 0.001);

        // Verify spatial audio entity.
        let (spatial_ent, _) = entities
            .iter()
            .find(|(_, n)| n == "Spatial Source")
            .unwrap();
        let ac3 = scene
            .get_component::<crate::scene::AudioSourceComponent>(*spatial_ent)
            .unwrap();
        assert!(ac3.spatial);
        assert!(ac3.play_on_start);
        assert!(ac3.looping);
        assert!((ac3.min_distance - 2.0).abs() < 0.001);
        assert!((ac3.max_distance - 15.0).abs() < 0.001);
        assert!((ac3.pitch - 0.8).abs() < 0.001);

        // Verify streaming audio entity.
        let (stream_ent, _) = entities
            .iter()
            .find(|(_, n)| n == "Streaming Source")
            .unwrap();
        let ac4 = scene
            .get_component::<crate::scene::AudioSourceComponent>(*stream_ent)
            .unwrap();
        assert!(ac4.streaming);
        assert!(ac4.play_on_start);
        assert!(ac4.looping);
        assert!((ac4.volume - 0.5).abs() < 0.001);

        // Verify audio listener on camera.
        let (cam_ent, _) = entities.iter().find(|(_, n)| n == "Camera").unwrap();
        let listener = scene
            .get_component::<crate::scene::AudioListenerComponent>(*cam_ent)
            .unwrap();
        assert!(listener.active);
    }

    #[test]
    fn prefab_round_trip() {
        // Build a scene with a parent + child hierarchy.
        let mut scene = Scene::new();
        let parent = scene.create_entity_with_tag("Parent");
        if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(parent) {
            tc.translation = Vec3::new(5.0, 10.0, 0.0);
        }
        let mut sprite = SpriteRendererComponent::new(Vec4::new(1.0, 0.0, 0.0, 1.0));
        sprite.texture_handle = crate::uuid::Uuid::from_raw(42);
        scene.add_component(parent, sprite);

        let child = scene.create_entity_with_tag("Child");
        if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(child) {
            tc.translation = Vec3::new(1.0, 2.0, 0.0);
        }
        scene.add_component(
            child,
            CircleRendererComponent {
                color: Vec4::new(0.0, 1.0, 0.0, 1.0),
                thickness: 0.5,
                fade: 0.01,
                sorting_layer: 0,
                order_in_layer: 0,
            },
        );
        scene.set_parent(child, parent, false);

        let parent_uuid = scene.get_component::<IdComponent>(parent).unwrap().id.raw();
        let child_uuid = scene.get_component::<IdComponent>(child).unwrap().id.raw();

        // Serialize to prefab file.
        let path = std::env::temp_dir()
            .join("gg_test.ggprefab")
            .to_string_lossy()
            .to_string();
        SceneSerializer::serialize_prefab(&scene, parent, &path).unwrap();

        // Instantiate prefab in a fresh scene.
        let mut loaded = Scene::new();
        let root = SceneSerializer::instantiate_prefab(&mut loaded, &path).unwrap();
        assert_eq!(loaded.entity_count(), 2);

        // Root entity should have fresh UUID (not the original).
        let new_root_uuid = loaded.get_component::<IdComponent>(root).unwrap().id.raw();
        assert_ne!(new_root_uuid, parent_uuid);

        // Verify root tag and transform.
        let root_tag = loaded.get_component::<TagComponent>(root).unwrap();
        assert_eq!(root_tag.tag, "Parent");
        let root_tc = loaded.get_component::<TransformComponent>(root).unwrap();
        assert_eq!(root_tc.translation, Vec3::new(5.0, 10.0, 0.0));

        // Verify sprite on root.
        let root_sprite = loaded
            .get_component::<SpriteRendererComponent>(root)
            .unwrap();
        assert_eq!(root_sprite.texture_handle.raw(), 42);

        // Verify child exists with remapped UUID.
        let children = loaded.get_children(root);
        assert_eq!(children.len(), 1);
        let new_child_uuid = children[0];
        assert_ne!(new_child_uuid, child_uuid);

        let child_ent = loaded.find_entity_by_uuid(new_child_uuid).unwrap();
        let child_tag = loaded.get_component::<TagComponent>(child_ent).unwrap();
        assert_eq!(child_tag.tag, "Child");

        // Verify child has circle renderer.
        let circle = loaded
            .get_component::<CircleRendererComponent>(child_ent)
            .unwrap();
        assert_eq!(circle.color, Vec4::new(0.0, 1.0, 0.0, 1.0));
        assert!((circle.thickness - 0.5).abs() < f32::EPSILON);

        // Verify root has no parent (prefab root is always root-level).
        assert!(loaded.get_parent(root).is_none());

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tilemap_b64_encode_decode_round_trip() {
        let tiles = vec![0, 1, -1, 3, -1, 2, 100, i32::MAX, i32::MIN];
        let encoded = TilemapData::encode_tiles(&tiles);
        assert!(!encoded.is_empty());

        let data = TilemapData {
            width: 3,
            height: 3,
            tile_size: [1.0, 1.0],
            texture_handle: 0,
            tileset_columns: 1,
            cell_size: [16.0, 16.0],
            spacing: [0.0, 0.0],
            margin: [0.0, 0.0],
            tiles_legacy: Vec::new(),
            tiles_b64: encoded,
            sorting_layer: 0,
            order_in_layer: 0,
        };

        assert_eq!(data.decode_tiles(), tiles);
    }

    #[test]
    fn tilemap_legacy_fallback_when_no_b64() {
        let legacy = vec![5, 10, -1, 3];
        let data = TilemapData {
            width: 2,
            height: 2,
            tile_size: [1.0, 1.0],
            texture_handle: 0,
            tileset_columns: 1,
            cell_size: [16.0, 16.0],
            spacing: [0.0, 0.0],
            margin: [0.0, 0.0],
            tiles_legacy: legacy.clone(),
            tiles_b64: String::new(),
            sorting_layer: 0,
            order_in_layer: 0,
        };

        assert_eq!(data.decode_tiles(), legacy);
    }

    #[test]
    fn tilemap_b64_preferred_over_legacy() {
        let tiles = vec![42, 99];
        let encoded = TilemapData::encode_tiles(&tiles);

        let data = TilemapData {
            width: 1,
            height: 2,
            tile_size: [1.0, 1.0],
            texture_handle: 0,
            tileset_columns: 1,
            cell_size: [16.0, 16.0],
            spacing: [0.0, 0.0],
            margin: [0.0, 0.0],
            tiles_legacy: vec![0, 0], // different from b64 data
            tiles_b64: encoded,
            sorting_layer: 0,
            order_in_layer: 0,
        };

        // b64 should take priority.
        assert_eq!(data.decode_tiles(), tiles);
    }

    #[test]
    fn tilemap_v1_scene_backward_compat() {
        // Simulate a v1 scene with plain Tiles: array.
        let yaml = r#"
Version: 1
Scene: compat_test
Entities:
- Entity: 100
  TagComponent:
    Tag: TileMap
  TransformComponent:
    Translation: [0.0, 0.0, 0.0]
    Rotation: [0.0, 0.0, 0.0]
    Scale: [1.0, 1.0, 1.0]
  TilemapComponent:
    Width: 2
    Height: 2
    TileSize: [1.0, 1.0]
    TilesetColumns: 4
    CellSize: [16.0, 16.0]
    Tiles:
    - 5
    - 10
    - -1
    - 3
"#;
        let mut scene = Scene::new();
        SceneSerializer::deserialize_from_string(&mut scene, yaml)
            .expect("Failed to deserialize v1 tilemap scene");

        let entities = scene.each_entity_with_tag();
        let (ent, _) = entities.iter().find(|(_, n)| n == "TileMap").unwrap();
        let tm = scene
            .get_component::<crate::scene::TilemapComponent>(*ent)
            .unwrap();
        assert_eq!(tm.tiles, vec![5, 10, -1, 3]);
    }

    #[test]
    fn tilemap_serializes_as_b64() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("TM");
        scene.add_component(
            e,
            crate::scene::TilemapComponent {
                width: 2,
                height: 2,
                tile_size: Vec2::ONE,
                tileset_columns: 1,
                cell_size: Vec2::new(16.0, 16.0),
                tiles: vec![1, 2, 3, 4],
                ..Default::default()
            },
        );

        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        // New format should have TilesB64 and NOT have Tiles:
        assert!(yaml.contains("TilesB64:"), "Should contain TilesB64");
        assert!(
            !yaml.contains("\n    Tiles:"),
            "Should NOT contain legacy Tiles array"
        );

        // Verify it round-trips correctly.
        let mut loaded = Scene::new();
        SceneSerializer::deserialize_from_string(&mut loaded, &yaml).unwrap();
        let entities = loaded.each_entity_with_tag();
        let (ent, _) = entities.iter().find(|(_, n)| n == "TM").unwrap();
        let tm = loaded
            .get_component::<crate::scene::TilemapComponent>(*ent)
            .unwrap();
        assert_eq!(tm.tiles, vec![1, 2, 3, 4]);
    }

    #[test]
    fn ui_test_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/ui_test.ggscene");
        let mut scene = Scene::new();
        SceneSerializer::deserialize_from_string(&mut scene, yaml)
            .expect("Failed to deserialize ui_test scene");
        // Camera, GameManager, Title, AccentBar, Panel,
        // PlayBtnBg, ResetBtnBg, QuitBtnBg, ClickCounter, StatusText = 10
        assert_eq!(scene.entity_count(), 10);

        let entities = scene.each_entity_with_tag();
        let names: Vec<&str> = entities.iter().map(|(_, name)| name.as_str()).collect();
        assert!(names.contains(&"Camera"));
        assert!(names.contains(&"PlayBtnBg"));
        assert!(names.contains(&"Panel"));
        assert!(names.contains(&"ClickCounter"));

        // Verify button has UIRect + UIImage + UIInteractable + Text (single entity).
        let (btn, _) = entities.iter().find(|(_, n)| n == "PlayBtnBg").unwrap();
        let rect = scene
            .get_component::<crate::scene::UIRectComponent>(*btn)
            .unwrap();
        assert!((rect.size.x - 320.0).abs() < 0.01);
        assert!((rect.size.y - 60.0).abs() < 0.01);
        assert!(rect.raycast_target);
        assert!(scene.has_component::<crate::scene::UIImageComponent>(*btn));
        assert!(scene.has_component::<crate::scene::UIInteractableComponent>(*btn));
        assert!(scene.has_component::<crate::scene::TextComponent>(*btn));

        // Panel has UIRect + UIImage + UILayout but no UIInteractable.
        let (panel, _) = entities.iter().find(|(_, n)| n == "Panel").unwrap();
        assert!(scene.has_component::<crate::scene::UIRectComponent>(*panel));
        assert!(scene.has_component::<crate::scene::UIImageComponent>(*panel));
        assert!(scene.has_component::<crate::scene::UILayoutComponent>(*panel));
        assert!(!scene.has_component::<crate::scene::UIInteractableComponent>(*panel));

        // Button is a child of Panel.
        let rel = scene
            .get_component::<crate::scene::RelationshipComponent>(*btn)
            .unwrap();
        assert!(rel.parent.is_some());

        // GameManager has the Lua script.
        let (gm, _) = entities.iter().find(|(_, n)| n == "GameManager").unwrap();
        assert!(scene.has_component::<LuaScriptComponent>(*gm));
    }

    #[test]
    fn scene_version_is_v2() {
        let mut scene = Scene::new();
        scene.create_entity_with_tag("Test");
        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        assert!(yaml.contains("Version: 2"));
    }
}
