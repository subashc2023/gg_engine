use std::fs;
use std::path::Path;

use glam::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

use crate::renderer::{ProjectionType, SceneCamera};
use crate::scene::{
    AnimationClip, AudioSourceComponent, BoxCollider2DComponent, CameraComponent,
    CircleCollider2DComponent, CircleRendererComponent, IdComponent, RelationshipComponent,
    RigidBody2DComponent, RigidBody2DType, Scene, SpriteAnimatorComponent,
    SpriteRendererComponent, TagComponent, TextComponent, TilemapComponent, TransformComponent,
};
#[cfg(feature = "lua-scripting")]
use crate::scene::LuaScriptComponent;
use crate::uuid::Uuid;

// ---------------------------------------------------------------------------
// Serialization data types (intermediate representation)
// ---------------------------------------------------------------------------

const SCENE_VERSION: u32 = 1;

fn default_scene_version() -> u32 {
    SCENE_VERSION
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
    #[serde(rename = "Rotation")]
    rotation: [f32; 3],
    #[serde(rename = "Scale")]
    scale: [f32; 3],
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
    #[serde(rename = "TilingFactor", default = "default_tiling_factor")]
    tiling_factor: f32,
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_handle")]
    texture_handle: u64,
}

fn is_zero_handle(v: &u64) -> bool {
    *v == 0
}

fn default_tiling_factor() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize)]
struct CircleData {
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "Thickness", default = "default_thickness")]
    thickness: f32,
    #[serde(rename = "Fade", default = "default_fade")]
    fade: f32,
}

fn default_thickness() -> f32 {
    1.0
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
    #[serde(rename = "FontSize", default = "default_font_size")]
    font_size: f32,
    #[serde(rename = "Color")]
    color: [f32; 4],
    #[serde(rename = "LineSpacing", default = "default_line_spacing")]
    line_spacing: f32,
    #[serde(rename = "Kerning", default)]
    kerning: f32,
}

fn default_font_size() -> f32 {
    1.0
}

fn default_line_spacing() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize)]
struct RigidBody2DData {
    #[serde(rename = "BodyType")]
    body_type: String,
    #[serde(rename = "FixedRotation")]
    fixed_rotation: bool,
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
    #[serde(rename = "RestitutionThreshold")]
    restitution_threshold: f32,
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
    #[serde(rename = "RestitutionThreshold")]
    restitution_threshold: f32,
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
    fields: Option<serde_yaml::Value>,
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
}

#[derive(Serialize, Deserialize)]
struct SpriteAnimatorData {
    #[serde(rename = "CellSize")]
    cell_size: [f32; 2],
    #[serde(rename = "Columns")]
    columns: u32,
    #[serde(rename = "Clips", default)]
    clips: Vec<AnimationClipData>,
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
    v[0] == 0.0 && v[1] == 0.0
}

#[derive(Serialize, Deserialize)]
struct TilemapData {
    #[serde(rename = "Width")]
    width: u32,
    #[serde(rename = "Height")]
    height: u32,
    #[serde(rename = "TileSize")]
    tile_size: [f32; 2],
    #[serde(rename = "TextureHandle", default, skip_serializing_if = "is_zero_handle")]
    texture_handle: u64,
    #[serde(rename = "TilesetColumns", default = "default_tileset_columns")]
    tileset_columns: u32,
    #[serde(rename = "CellSize")]
    cell_size: [f32; 2],
    #[serde(rename = "Spacing", default = "default_zero_vec2", skip_serializing_if = "is_zero_vec2")]
    spacing: [f32; 2],
    #[serde(rename = "Margin", default = "default_zero_vec2", skip_serializing_if = "is_zero_vec2")]
    margin: [f32; 2],
    #[serde(rename = "Tiles")]
    tiles: Vec<i32>,
}

#[derive(Serialize, Deserialize)]
struct AudioSourceData {
    #[serde(rename = "AudioHandle", default, skip_serializing_if = "is_zero_handle")]
    audio_handle: u64,
    #[serde(rename = "Volume", default = "default_volume")]
    volume: f32,
    #[serde(rename = "Pitch", default = "default_pitch")]
    pitch: f32,
    #[serde(rename = "Looping", default)]
    looping: bool,
    #[serde(rename = "PlayOnStart", default)]
    play_on_start: bool,
}

fn default_volume() -> f32 {
    1.0
}

fn default_pitch() -> f32 {
    1.0
}

fn default_tileset_columns() -> u32 {
    1
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
    /// Creates parent directories if they don't exist. Returns `true` on
    /// success, `false` on failure (errors are logged).
    pub fn serialize(scene: &Scene, file_path: &str, scene_name: Option<&str>) -> bool {
        let scene_data = Self::scene_to_data(scene, scene_name);

        // Ensure parent directories exist.
        if let Some(parent) = Path::new(file_path).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = fs::create_dir_all(parent) {
                    log::error!("Failed to create directories for '{}': {}", file_path, e);
                    return false;
                }
            }
        }

        match serde_yaml::to_string(&scene_data) {
            Ok(yaml) => {
                if let Err(e) = crate::platform_utils::atomic_write(file_path, &yaml) {
                    log::error!("Failed to write scene file '{}': {}", file_path, e);
                    false
                } else {
                    log::info!("Scene serialized to '{}'", file_path);
                    true
                }
            }
            Err(e) => {
                log::error!("Failed to serialize scene: {}", e);
                false
            }
        }
    }

    /// Deserialize a scene from a YAML file.
    ///
    /// Entities are created in the provided `scene`. If the scene is not empty,
    /// deserialized entities are added to existing ones — callers should provide
    /// a fresh scene if a clean load is desired.
    ///
    /// Returns `true` on success, `false` on failure (errors are logged).
    pub fn deserialize(scene: &mut Scene, file_path: &str) -> bool {
        let contents = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to read scene file '{}': {}", file_path, e);
                return false;
            }
        };

        let scene_data: SceneData = match serde_yaml::from_str(&contents) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to parse scene file '{}': {}", file_path, e);
                return false;
            }
        };

        if scene_data.version > SCENE_VERSION {
            log::warn!(
                "Scene '{}' was saved with version {} (current: {}). Some data may not load correctly.",
                file_path, scene_data.version, SCENE_VERSION
            );
        }

        log::info!(
            "Deserializing scene '{}' (version {}, {} entities)",
            scene_data.name,
            scene_data.version,
            scene_data.entities.len()
        );

        Self::data_to_scene(scene, &scene_data);
        true
    }

    /// Serialize a scene to a YAML string (in-memory snapshot).
    pub fn serialize_to_string(scene: &Scene) -> Option<String> {
        let scene_data = Self::scene_to_data(scene, None);
        match serde_yaml::to_string(&scene_data) {
            Ok(yaml) => Some(yaml),
            Err(e) => {
                log::error!("Failed to serialize scene to string: {}", e);
                None
            }
        }
    }

    /// Deserialize a scene from a YAML string (in-memory snapshot restore).
    ///
    /// Entities are created in the provided `scene`. Callers should provide
    /// a fresh scene if a clean restore is desired.
    pub fn deserialize_from_string(scene: &mut Scene, yaml: &str) -> bool {
        let scene_data: SceneData = match serde_yaml::from_str(yaml) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to parse scene from string: {}", e);
                return false;
            }
        };

        Self::data_to_scene(scene, &scene_data);
        true
    }

    // -- Shared helpers -------------------------------------------------------

    fn scene_to_data(scene: &Scene, scene_name: Option<&str>) -> SceneData {
        let mut entities_data = Vec::new();

        for (entity, _name) in scene.each_entity_with_tag() {
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
                        rotation: tc.rotation.into(),
                        scale: tc.scale.into(),
                    });

            let camera_data =
                scene
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

            let sprite_data =
                scene
                    .get_component::<SpriteRendererComponent>(entity)
                    .map(|sprite| SpriteData {
                        color: sprite.color.into(),
                        tiling_factor: sprite.tiling_factor,
                        texture_handle: sprite.texture_handle.raw(),
                    });

            let circle_data =
                scene
                    .get_component::<CircleRendererComponent>(entity)
                    .map(|circle| CircleData {
                        color: circle.color.into(),
                        thickness: circle.thickness,
                        fade: circle.fade,
                    });

            let text_data =
                scene
                    .get_component::<TextComponent>(entity)
                    .map(|tc| TextData {
                        text: tc.text.clone(),
                        font_path: tc.font_path.clone(),
                        font_size: tc.font_size,
                        color: tc.color.into(),
                        line_spacing: tc.line_spacing,
                        kerning: tc.kerning,
                    });

            let rigidbody_2d_data =
                scene
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
                        restitution_threshold: bc.restitution_threshold,
                    });

            let circle_collider_2d_data =
                scene
                    .get_component::<CircleCollider2DComponent>(entity)
                    .map(|cc| CircleCollider2DData {
                        offset: cc.offset.into(),
                        radius: cc.radius,
                        density: cc.density,
                        friction: cc.friction,
                        restitution: cc.restitution,
                        restitution_threshold: cc.restitution_threshold,
                    });

            #[cfg(feature = "lua-scripting")]
            let lua_script_data =
                scene
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
            let sprite_animator_data = scene
                .get_component::<SpriteAnimatorComponent>(entity)
                .map(|sa| SpriteAnimatorData {
                    cell_size: sa.cell_size.into(),
                    columns: sa.columns,
                    clips: sa
                        .clips
                        .iter()
                        .map(|c| AnimationClipData {
                            name: c.name.clone(),
                            start_frame: c.start_frame,
                            end_frame: c.end_frame,
                            fps: c.fps,
                            looping: c.looping,
                        })
                        .collect(),
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
                });

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
                    tiles: tm.tiles.clone(),
                });

            let uuid = scene
                .get_component::<IdComponent>(entity)
                .map(|id| id.id.raw())
                .unwrap_or(0);

            entities_data.push(EntityData {
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
                lua_script: lua_script_data,
                sprite_animator: sprite_animator_data,
                relationship: relationship_data,
                tilemap: tilemap_data,
                audio_source: audio_source_data,
            });
        }

        SceneData {
            version: SCENE_VERSION,
            name: scene_name.unwrap_or("Untitled").to_string(),
            entities: entities_data,
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

            // TransformComponent — always present on newly created entities,
            // so we just update the values.
            if let Some(ref td) = entity_data.transform {
                if let Some(mut tc) = scene.get_component_mut::<TransformComponent>(entity) {
                    tc.translation = Vec3::from(td.translation);
                    tc.rotation = Vec3::from(td.rotation);
                    tc.scale = Vec3::from(td.scale);
                }
            }

            // CameraComponent — added only if present in the file.
            if let Some(ref cd) = entity_data.camera {
                let mut cam = SceneCamera::default();

                let proj_type = match cd.camera.projection_type {
                    0 => ProjectionType::Perspective,
                    _ => ProjectionType::Orthographic,
                };

                // Set both parameter sets so switching projection type preserves values.
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
                // Final projection type (recalculates the active projection).
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

            // SpriteRendererComponent — added only if present in the file.
            if let Some(ref sd) = entity_data.sprite {
                let mut sprite = SpriteRendererComponent::new(Vec4::from(sd.color));
                sprite.tiling_factor = sd.tiling_factor;
                sprite.texture_handle = Uuid::from_raw(sd.texture_handle);
                scene.add_component(entity, sprite);
            }

            // CircleRendererComponent — added only if present in the file.
            if let Some(ref cd) = entity_data.circle {
                scene.add_component(
                    entity,
                    CircleRendererComponent {
                        color: Vec4::from(cd.color),
                        thickness: cd.thickness,
                        fade: cd.fade,
                    },
                );
            }

            // TextComponent — added only if present in the file.
            if let Some(ref td) = entity_data.text {
                scene.add_component(
                    entity,
                    TextComponent {
                        text: td.text.clone(),
                        font_path: td.font_path.clone(),
                        font: None, // Loaded at runtime.
                        font_size: td.font_size,
                        color: Vec4::from(td.color),
                        line_spacing: td.line_spacing,
                        kerning: td.kerning,
                    },
                );
            }

            // RigidBody2DComponent — added only if present in the file.
            if let Some(ref rbd) = entity_data.rigidbody_2d {
                let body_type = match rbd.body_type.as_str() {
                    "Dynamic" => RigidBody2DType::Dynamic,
                    "Kinematic" => RigidBody2DType::Kinematic,
                    _ => RigidBody2DType::Static,
                };
                let mut rb = RigidBody2DComponent::new(body_type);
                rb.fixed_rotation = rbd.fixed_rotation;
                scene.add_component(entity, rb);
            }

            // BoxCollider2DComponent — added only if present in the file.
            if let Some(ref bcd) = entity_data.box_collider_2d {
                scene.add_component(
                    entity,
                    BoxCollider2DComponent {
                        offset: Vec2::from(bcd.offset),
                        size: Vec2::from(bcd.size),
                        density: bcd.density,
                        friction: bcd.friction,
                        restitution: bcd.restitution,
                        restitution_threshold: bcd.restitution_threshold,
                        runtime_fixture: None,
                    },
                );
            }

            // CircleCollider2DComponent — added only if present in the file.
            if let Some(ref ccd) = entity_data.circle_collider_2d {
                scene.add_component(
                    entity,
                    CircleCollider2DComponent {
                        offset: Vec2::from(ccd.offset),
                        radius: ccd.radius,
                        density: ccd.density,
                        friction: ccd.friction,
                        restitution: ccd.restitution,
                        restitution_threshold: ccd.restitution_threshold,
                        runtime_fixture: None,
                    },
                );
            }

            // LuaScriptComponent — added only if present in the file.
            #[cfg(feature = "lua-scripting")]
            if let Some(ref lsd) = entity_data.lua_script {
                let mut lsc = LuaScriptComponent::new(&lsd.script_path);
                if let Some(ref fields) = lsd.fields {
                    lsc.field_overrides = fields.clone();
                }
                scene.add_component(entity, lsc);
            }

            // SpriteAnimatorComponent — added only if present in the file.
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
                    })
                    .collect();
                scene.add_component(
                    entity,
                    SpriteAnimatorComponent {
                        cell_size: Vec2::from(sad.cell_size),
                        columns: sad.columns,
                        clips,
                        ..Default::default()
                    },
                );
            }

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

            // AudioSourceComponent — added only if present in the file.
            if let Some(ref asd) = entity_data.audio_source {
                scene.add_component(
                    entity,
                    AudioSourceComponent {
                        audio_handle: Uuid::from_raw(asd.audio_handle),
                        volume: asd.volume,
                        pitch: asd.pitch,
                        looping: asd.looping,
                        play_on_start: asd.play_on_start,
                        resolved_path: None,
                    },
                );
            }

            // TilemapComponent — added only if present in the file.
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
                        tiles: td.tiles.clone(),
                    },
                );
            }
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
            tc.rotation = Vec3::new(0.1, 0.2, 0.3);
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
        assert!(SceneSerializer::serialize(&scene, &path, Some("gg_test_scene")));

        // Deserialize into a fresh scene.
        let mut loaded = Scene::new();
        assert!(SceneSerializer::deserialize(&mut loaded, &path));
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
        assert_eq!(tc.rotation, Vec3::new(0.1, 0.2, 0.3));
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
        let mut tilemap = crate::scene::TilemapComponent::default();
        tilemap.width = 3;
        tilemap.height = 2;
        tilemap.tile_size = Vec2::new(1.5, 1.5);
        tilemap.tileset_columns = 4;
        tilemap.cell_size = Vec2::new(16.0, 16.0);
        tilemap.spacing = Vec2::new(2.0, 2.0);
        tilemap.margin = Vec2::new(1.0, 1.0);
        tilemap.tiles = vec![0, 1, -1, 3, -1, 2];
        tilemap.texture_handle = crate::uuid::Uuid::from_raw(99999);
        scene.add_component(e, tilemap);

        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        let mut loaded = Scene::new();
        assert!(SceneSerializer::deserialize_from_string(&mut loaded, &yaml));
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
            resolved_path: None,
        };
        scene.add_component(e, audio);

        let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
        let mut loaded = Scene::new();
        assert!(SceneSerializer::deserialize_from_string(&mut loaded, &yaml));
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
        assert!(ac.resolved_path.is_none());
    }

    #[test]
    fn demo_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/lua_camera_follow.ggscene");
        let mut scene = Scene::new();
        assert!(
            SceneSerializer::deserialize_from_string(&mut scene, yaml),
            "Failed to deserialize demo scene"
        );
        assert_eq!(scene.entity_count(), 6);

        let entities = scene.each_entity_with_tag();
        let names: Vec<&str> = entities.iter().map(|(_, name)| name.as_str()).collect();
        assert!(names.contains(&"Camera"));
        assert!(names.contains(&"Player"));
        assert!(names.contains(&"Ground"));

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
        assert!(
            SceneSerializer::deserialize_from_string(&mut scene, yaml),
            "Failed to deserialize tilemap_test scene"
        );
        assert_eq!(scene.entity_count(), 3);

        let entities = scene.each_entity_with_tag();
        let (tm_ent, _) = entities.iter().find(|(_, n)| n == "Tilemap").unwrap();
        let tm = scene
            .get_component::<crate::scene::TilemapComponent>(*tm_ent)
            .unwrap();
        assert_eq!(tm.width, 10);
        assert_eq!(tm.height, 10);
        assert_eq!(tm.tiles.len(), 100);
        assert_eq!(tm.texture_handle.raw(), 2001);
    }

    #[test]
    fn audio_test_scene_deserializes() {
        let yaml = include_str!("../../../assets/scenes/audio_test.ggscene");
        let mut scene = Scene::new();
        assert!(
            SceneSerializer::deserialize_from_string(&mut scene, yaml),
            "Failed to deserialize audio_test scene"
        );
        assert_eq!(scene.entity_count(), 7);

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
    }
}
