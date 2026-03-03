use std::fs;
use std::path::Path;

use glam::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

use crate::renderer::{ProjectionType, SceneCamera};
use crate::scene::{
    BoxCollider2DComponent, CameraComponent, CircleRendererComponent, IdComponent,
    RigidBody2DComponent, RigidBody2DType, Scene, SpriteRendererComponent, TagComponent,
    TransformComponent,
};
use crate::uuid::Uuid;

// ---------------------------------------------------------------------------
// Serialization data types (intermediate representation)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct SceneData {
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
/// SceneSerializer::serialize(&scene, "assets/scenes/example.ggscene");
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
    pub fn serialize(scene: &Scene, file_path: &str) -> bool {
        let scene_data = Self::scene_to_data(scene);

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
                if let Err(e) = fs::write(file_path, &yaml) {
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

        log::info!(
            "Deserializing scene '{}' ({} entities)",
            scene_data.name,
            scene_data.entities.len()
        );

        Self::data_to_scene(scene, &scene_data);
        true
    }

    /// Serialize a scene to a YAML string (in-memory snapshot).
    pub fn serialize_to_string(scene: &Scene) -> Option<String> {
        let scene_data = Self::scene_to_data(scene);
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

    fn scene_to_data(scene: &Scene) -> SceneData {
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
                    });

            let circle_data =
                scene
                    .get_component::<CircleRendererComponent>(entity)
                    .map(|circle| CircleData {
                        color: circle.color.into(),
                        thickness: circle.thickness,
                        fade: circle.fade,
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
                rigidbody_2d: rigidbody_2d_data,
                box_collider_2d: box_collider_2d_data,
            });
        }

        SceneData {
            name: "Untitled".to_string(),
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
        scene.add_component(
            e1,
            SpriteRendererComponent::new(Vec4::new(0.8, 0.2, 0.2, 1.0)),
        );

        let e2 = scene.create_entity_with_tag("Camera");
        scene.add_component(e2, CameraComponent::default());

        // Serialize.
        let path = std::env::temp_dir()
            .join("gg_test_scene.ggscene")
            .to_string_lossy()
            .to_string();
        assert!(SceneSerializer::serialize(&scene, &path));

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

        // Verify sprite.
        let sprite = loaded
            .get_component::<SpriteRendererComponent>(*test_entity)
            .unwrap();
        assert_eq!(sprite.color, Vec4::new(0.8, 0.2, 0.2, 1.0));

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
}
