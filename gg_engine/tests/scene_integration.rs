//! Integration tests for Scene — hierarchy, physics, serialization, and runtime settings.

use glam::{Quat, Vec2, Vec3};
use gg_engine::prelude::*;

/// Helper: create a dynamic rigid body entity with a box collider.
fn spawn_dynamic_box(scene: &mut Scene, pos: Vec3) -> Entity {
    let e = scene.create_entity();
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
        tc.translation = pos;
    }
    scene.add_component(e, RigidBody2DComponent::default());
    scene.add_component(e, BoxCollider2DComponent::default());
    e
}

/// Helper: create a static rigid body entity with a box collider.
fn spawn_static_box(scene: &mut Scene, pos: Vec3) -> Entity {
    let e = scene.create_entity();
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
        tc.translation = pos;
    }
    let mut rb = RigidBody2DComponent::default();
    rb.body_type = RigidBody2DType::Static;
    scene.add_component(e, rb);
    scene.add_component(e, BoxCollider2DComponent::default());
    e
}

// ---------------------------------------------------------------------------
// Hierarchy & world transforms
// ---------------------------------------------------------------------------

#[test]
fn hierarchy_set_parent_links_both_directions() {
    let mut scene = Scene::new();
    let parent = scene.create_entity_with_tag("Parent");
    let child = scene.create_entity_with_tag("Child");

    let ok = scene.set_parent(child, parent, false);
    assert!(ok);

    let parent_uuid = scene.get_component::<IdComponent>(parent).unwrap().id.raw();
    let child_uuid = scene.get_component::<IdComponent>(child).unwrap().id.raw();

    let rel = scene.get_component::<RelationshipComponent>(child).unwrap();
    assert_eq!(rel.parent, Some(parent_uuid));

    let rel_p = scene.get_component::<RelationshipComponent>(parent).unwrap();
    assert!(rel_p.children.contains(&child_uuid));
}

#[test]
fn hierarchy_cycle_detection() {
    let mut scene = Scene::new();
    let a = scene.create_entity_with_tag("A");
    let b = scene.create_entity_with_tag("B");
    let c = scene.create_entity_with_tag("C");

    assert!(scene.set_parent(b, a, false));
    assert!(scene.set_parent(c, b, false));

    // C -> A would create a cycle.
    assert!(!scene.set_parent(a, c, false));
}

#[test]
fn hierarchy_world_transform_propagation() {
    let mut scene = Scene::new();
    let parent = scene.create_entity();
    let child = scene.create_entity();

    {
        let mut tc = scene.get_component_mut::<TransformComponent>(parent).unwrap();
        tc.translation = Vec3::new(10.0, 0.0, 0.0);
    }
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(child).unwrap();
        tc.translation = Vec3::new(5.0, 0.0, 0.0);
    }
    scene.set_parent(child, parent, false);

    let world = scene.get_world_transform(child);
    let world_pos = world.col(3).truncate();
    assert!((world_pos.x - 15.0).abs() < 0.001);
    assert!(world_pos.y.abs() < 0.001);
}

#[test]
fn hierarchy_detach_preserves_world_transform() {
    let mut scene = Scene::new();
    let parent = scene.create_entity();
    let child = scene.create_entity();

    {
        let mut tc = scene.get_component_mut::<TransformComponent>(parent).unwrap();
        tc.translation = Vec3::new(10.0, 0.0, 0.0);
    }
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(child).unwrap();
        tc.translation = Vec3::new(5.0, 0.0, 0.0);
    }
    scene.set_parent(child, parent, false);

    scene.detach_from_parent(child, true);

    let tc = scene.get_component::<TransformComponent>(child).unwrap();
    assert!((tc.translation.x - 15.0).abs() < 0.001);
}

#[test]
fn hierarchy_deep_chain_world_transform() {
    let mut scene = Scene::new();
    let a = scene.create_entity();
    let b = scene.create_entity();
    let c = scene.create_entity();

    for e in [a, b, c] {
        let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
        tc.translation = Vec3::new(1.0, 0.0, 0.0);
    }

    scene.set_parent(b, a, false);
    scene.set_parent(c, b, false);

    let world = scene.get_world_transform(c);
    let pos = world.col(3).truncate();
    assert!((pos.x - 3.0).abs() < 0.001);
}

#[test]
fn hierarchy_multiple_children_world_transforms() {
    let mut scene = Scene::new();
    let parent = scene.create_entity();
    let child_a = scene.create_entity();
    let child_b = scene.create_entity();

    {
        let mut tc = scene.get_component_mut::<TransformComponent>(parent).unwrap();
        tc.translation = Vec3::new(10.0, 0.0, 0.0);
    }
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(child_a).unwrap();
        tc.translation = Vec3::new(1.0, 0.0, 0.0);
    }
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(child_b).unwrap();
        tc.translation = Vec3::new(0.0, 5.0, 0.0);
    }
    scene.set_parent(child_a, parent, false);
    scene.set_parent(child_b, parent, false);

    let world_a = scene.get_world_transform(child_a);
    let world_b = scene.get_world_transform(child_b);

    assert!((world_a.col(3).x - 11.0).abs() < 0.001);
    assert!((world_b.col(3).y - 5.0).abs() < 0.001);
    assert!((world_b.col(3).x - 10.0).abs() < 0.001);
}

// ---------------------------------------------------------------------------
// Physics 2D — runtime lifecycle
// ---------------------------------------------------------------------------

#[test]
fn physics_2d_runtime_creates_bodies_and_applies_forces() {
    let mut scene = Scene::new();
    let e = spawn_dynamic_box(&mut scene, Vec3::new(0.0, 10.0, 0.0));

    scene.on_runtime_start();

    let vel = scene.get_linear_velocity(e);
    assert!(vel.is_some(), "body should exist after runtime start");
    assert_eq!(vel.unwrap(), Vec2::ZERO);

    scene.apply_impulse(e, Vec2::new(10.0, 0.0));
    let dt = Timestep::from_seconds(1.0 / 60.0);
    scene.on_update_all_physics(dt, None);

    let vel = scene.get_linear_velocity(e).unwrap();
    assert!(vel.x > 0.0, "impulse should produce positive x velocity");
}

#[test]
fn physics_2d_gravity_affects_dynamic_body() {
    let mut scene = Scene::new();
    let e = spawn_dynamic_box(&mut scene, Vec3::new(0.0, 10.0, 0.0));

    scene.on_runtime_start();

    let dt = Timestep::from_seconds(1.0 / 60.0);
    for _ in 0..10 {
        scene.on_update_all_physics(dt, None);
    }

    let vel = scene.get_linear_velocity(e).unwrap();
    assert!(vel.y < 0.0, "gravity should produce downward velocity, got {}", vel.y);
}

#[test]
fn physics_2d_set_gravity() {
    let mut scene = Scene::new();
    let e = spawn_dynamic_box(&mut scene, Vec3::ZERO);

    scene.on_runtime_start();
    scene.set_gravity(0.0, 0.0);

    let dt = Timestep::from_seconds(1.0 / 60.0);
    for _ in 0..10 {
        scene.on_update_all_physics(dt, None);
    }

    let vel = scene.get_linear_velocity(e).unwrap();
    assert!(vel.y.abs() < 0.001, "zero gravity should produce no velocity, got {}", vel.y);
}

#[test]
fn physics_2d_static_body_unaffected_by_gravity() {
    let mut scene = Scene::new();
    let e = spawn_static_box(&mut scene, Vec3::ZERO);

    scene.on_runtime_start();

    let dt = Timestep::from_seconds(1.0 / 60.0);
    for _ in 0..10 {
        scene.on_update_all_physics(dt, None);
    }

    let vel = scene.get_linear_velocity(e).unwrap();
    assert_eq!(vel, Vec2::ZERO, "static body should not move");
}

#[test]
fn physics_2d_raycast() {
    let mut scene = Scene::new();
    let _e = spawn_static_box(&mut scene, Vec3::ZERO);

    scene.on_runtime_start();

    // Step once so the query pipeline is updated.
    let dt = Timestep::from_seconds(1.0 / 60.0);
    scene.on_update_all_physics(dt, None);

    let hit = scene.raycast(Vec2::new(-5.0, 0.0), Vec2::new(1.0, 0.0), 10.0, None);
    assert!(hit.is_some(), "raycast should hit the box");

    let miss = scene.raycast(Vec2::new(-5.0, 0.0), Vec2::new(0.0, 1.0), 10.0, None);
    assert!(miss.is_none(), "raycast should miss when directed away");
}

#[test]
fn physics_2d_set_velocity() {
    let mut scene = Scene::new();
    let e = spawn_dynamic_box(&mut scene, Vec3::ZERO);

    scene.on_runtime_start();
    scene.set_gravity(0.0, 0.0);
    scene.set_linear_velocity(e, Vec2::new(5.0, -3.0));

    let vel = scene.get_linear_velocity(e).unwrap();
    assert!((vel.x - 5.0).abs() < 0.001);
    assert!((vel.y - (-3.0)).abs() < 0.001);
}

// ---------------------------------------------------------------------------
// Serialization round-trips
// ---------------------------------------------------------------------------

#[test]
fn serialize_deserialize_complex_scene() {
    let mut scene = Scene::new();

    // Entity with sprite + physics.
    let e1 = scene.create_entity_with_tag("PhysicsSprite");
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(e1).unwrap();
        tc.translation = Vec3::new(1.0, 2.0, 3.0);
        tc.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        tc.scale = Vec3::new(2.0, 2.0, 1.0);
    }
    scene.add_component(e1, SpriteRendererComponent {
        color: glam::Vec4::new(1.0, 0.0, 0.0, 1.0),
        tiling_factor: 2.5,
        ..Default::default()
    });
    let mut rb = RigidBody2DComponent::default();
    rb.body_type = RigidBody2DType::Dynamic;
    rb.fixed_rotation = true;
    rb.gravity_scale = 0.5;
    rb.linear_damping = 0.1;
    rb.angular_damping = 0.2;
    scene.add_component(e1, rb);

    let mut col = BoxCollider2DComponent::default();
    col.density = 2.0;
    col.friction = 0.3;
    col.restitution = 0.7;
    col.is_sensor = true;
    scene.add_component(e1, col);

    // Entity with circle + camera.
    let e2 = scene.create_entity_with_tag("CircleCamera");
    scene.add_component(e2, CircleRendererComponent {
        color: glam::Vec4::new(0.0, 1.0, 0.0, 0.5),
        thickness: 0.8,
        fade: 0.05,
        ..Default::default()
    });
    scene.add_component(e2, CameraComponent::default());

    // Entity with UI anchor.
    let e3 = scene.create_entity_with_tag("UIElement");
    scene.add_component(e3, UIAnchorComponent {
        anchor: Vec2::new(0.5, 0.0),
        offset: Vec2::new(0.0, 50.0),
    });

    // Parent-child relationship.
    scene.set_parent(e2, e1, false);

    let e1_uuid = scene.get_component::<IdComponent>(e1).unwrap().id.raw();
    let e2_uuid = scene.get_component::<IdComponent>(e2).unwrap().id.raw();
    let e3_uuid = scene.get_component::<IdComponent>(e3).unwrap().id.raw();

    let yaml = SceneSerializer::serialize_to_string(&scene).expect("serialize");

    let mut loaded = Scene::new();
    SceneSerializer::deserialize_from_string(&mut loaded, &yaml).expect("deserialize");

    assert_eq!(loaded.entity_count(), scene.entity_count());

    // Verify entity 1.
    let le1 = loaded.find_entity_by_uuid(e1_uuid).expect("e1 exists");
    let tc = loaded.get_component::<TransformComponent>(le1).unwrap();
    assert!((tc.translation - Vec3::new(1.0, 2.0, 3.0)).length() < 0.001);
    assert!((tc.scale - Vec3::new(2.0, 2.0, 1.0)).length() < 0.001);

    let sprite = loaded.get_component::<SpriteRendererComponent>(le1).unwrap();
    assert!((sprite.tiling_factor - 2.5).abs() < 0.001);

    let rb = loaded.get_component::<RigidBody2DComponent>(le1).unwrap();
    assert!(rb.fixed_rotation);
    assert!((rb.gravity_scale - 0.5).abs() < 0.001);

    let col = loaded.get_component::<BoxCollider2DComponent>(le1).unwrap();
    assert!(col.is_sensor);
    assert!((col.restitution - 0.7).abs() < 0.001);

    // Verify entity 2 — hierarchy preserved.
    let le2 = loaded.find_entity_by_uuid(e2_uuid).expect("e2 exists");
    let rel = loaded.get_component::<RelationshipComponent>(le2).unwrap();
    assert_eq!(rel.parent, Some(e1_uuid));

    assert!(loaded.has_component::<CircleRendererComponent>(le2));
    assert!(loaded.has_component::<CameraComponent>(le2));

    // Verify entity 3 — UI anchor.
    let le3 = loaded.find_entity_by_uuid(e3_uuid).expect("e3 exists");
    let anchor = loaded.get_component::<UIAnchorComponent>(le3).unwrap();
    assert!((anchor.anchor - Vec2::new(0.5, 0.0)).length() < 0.001);
}

#[test]
fn serialize_preserves_hierarchy_tree() {
    let mut scene = Scene::new();
    let root = scene.create_entity_with_tag("Root");
    let child_a = scene.create_entity_with_tag("ChildA");
    let child_b = scene.create_entity_with_tag("ChildB");
    let grandchild = scene.create_entity_with_tag("Grandchild");

    scene.set_parent(child_a, root, false);
    scene.set_parent(child_b, root, false);
    scene.set_parent(grandchild, child_a, false);

    let root_uuid = scene.get_component::<IdComponent>(root).unwrap().id.raw();
    let gc_uuid = scene.get_component::<IdComponent>(grandchild).unwrap().id.raw();

    let yaml = SceneSerializer::serialize_to_string(&scene).unwrap();
    let mut loaded = Scene::new();
    SceneSerializer::deserialize_from_string(&mut loaded, &yaml).unwrap();

    let lr = loaded.find_entity_by_uuid(root_uuid).unwrap();
    let rel = loaded.get_component::<RelationshipComponent>(lr).unwrap();
    assert_eq!(rel.children.len(), 2);

    assert!(loaded.is_ancestor_of(root_uuid, gc_uuid));
}

#[test]
fn serialize_deserialize_idempotent() {
    let mut scene = Scene::new();
    let e = scene.create_entity_with_tag("TestEntity");
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
        tc.translation = Vec3::new(7.0, 8.0, 9.0);
    }

    let yaml1 = SceneSerializer::serialize_to_string(&scene).unwrap();

    let mut loaded = Scene::new();
    SceneSerializer::deserialize_from_string(&mut loaded, &yaml1).unwrap();

    let yaml2 = SceneSerializer::serialize_to_string(&loaded).unwrap();
    assert_eq!(yaml1, yaml2, "double round-trip should produce identical YAML");
}

// ---------------------------------------------------------------------------
// Scene::copy() (play/stop workflow)
// ---------------------------------------------------------------------------

#[test]
fn scene_copy_preserves_data_but_physics_inactive() {
    let mut scene = Scene::new();
    let e = scene.create_entity();
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
        tc.translation = Vec3::new(42.0, 0.0, 0.0);
    }
    scene.add_component(e, RigidBody2DComponent::default());
    scene.add_component(e, BoxCollider2DComponent::default());

    scene.on_runtime_start();
    assert!(scene.get_linear_velocity(e).is_some());

    let copy = Scene::copy(&scene);
    let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
    let ce = copy.find_entity_by_uuid(uuid).unwrap();

    let tc = copy.get_component::<TransformComponent>(ce).unwrap();
    assert!((tc.translation.x - 42.0).abs() < 0.001);
    assert!(copy.has_component::<RigidBody2DComponent>(ce));
    assert!(copy.has_component::<BoxCollider2DComponent>(ce));

    // Physics not initialized in the copy.
    assert!(copy.get_linear_velocity(ce).is_none());
}

#[test]
fn scene_copy_hierarchy_preserved() {
    let mut scene = Scene::new();
    let parent = scene.create_entity_with_tag("Parent");
    let child = scene.create_entity_with_tag("Child");
    scene.set_parent(child, parent, false);

    let parent_uuid = scene.get_component::<IdComponent>(parent).unwrap().id.raw();
    let child_uuid = scene.get_component::<IdComponent>(child).unwrap().id.raw();

    let copy = Scene::copy(&scene);
    let cc = copy.find_entity_by_uuid(child_uuid).unwrap();
    let rel = copy.get_component::<RelationshipComponent>(cc).unwrap();
    assert_eq!(rel.parent, Some(parent_uuid));
}

// ---------------------------------------------------------------------------
// Command buffer (deferred structural changes)
// ---------------------------------------------------------------------------

#[test]
fn command_buffer_spawn_and_destroy() {
    let mut scene = Scene::new();
    let e = scene.create_entity_with_tag("Victim");
    let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();

    assert_eq!(scene.entity_count(), 1);

    let mut cmd = gg_engine::jobs::command_buffer::CommandBuffer::new();
    cmd.destroy_entity(uuid);
    cmd.spawn(|s: &mut Scene| {
        s.create_entity_with_tag("Spawned");
    });

    assert!(!cmd.is_empty());
    cmd.flush(&mut scene);

    assert_eq!(scene.entity_count(), 1);
    assert!(scene.find_entity_by_name("Spawned").is_some());
}

#[test]
fn command_buffer_insert_component() {
    let mut scene = Scene::new();
    let e = scene.create_entity();
    assert!(!scene.has_component::<CameraComponent>(e));

    let mut cmd = gg_engine::jobs::command_buffer::CommandBuffer::new();
    cmd.insert_component(e.handle(), CameraComponent::default());
    cmd.flush(&mut scene);

    assert!(scene.has_component::<CameraComponent>(e));
}

// ---------------------------------------------------------------------------
// Runtime settings pipeline
// ---------------------------------------------------------------------------

#[test]
fn settings_request_take_roundtrip() {
    let scene = Scene::new();

    // VSync
    assert!(scene.take_requested_vsync().is_none());
    scene.request_vsync(true);
    assert_eq!(scene.take_requested_vsync(), Some(true));
    assert!(scene.take_requested_vsync().is_none());

    // Fullscreen
    scene.request_fullscreen(FullscreenMode::Borderless);
    assert_eq!(scene.take_requested_fullscreen(), Some(FullscreenMode::Borderless));
    assert!(scene.take_requested_fullscreen().is_none());

    // Shadow quality
    scene.request_shadow_quality(2);
    assert_eq!(scene.take_requested_shadow_quality(), Some(2));
    assert!(scene.take_requested_shadow_quality().is_none());

    // Window size
    scene.request_window_size(1920, 1080);
    assert_eq!(scene.take_requested_window_size(), Some((1920, 1080)));

    // Quit
    assert!(!scene.take_requested_quit());
    scene.request_quit();
    assert!(scene.take_requested_quit());
    assert!(!scene.take_requested_quit());

    // Scene load
    scene.request_load_scene("Level2.ggscene".to_string());
    assert_eq!(scene.take_requested_load_scene(), Some("Level2.ggscene".to_string()));
    assert!(scene.take_requested_load_scene().is_none());
}

#[test]
fn gui_scale_clamped() {
    let scene = Scene::new();
    scene.set_gui_scale(0.1);
    assert!(scene.gui_scale() >= 0.5);

    scene.set_gui_scale(5.0);
    assert!(scene.gui_scale() <= 2.0);

    scene.set_gui_scale(1.5);
    assert!((scene.gui_scale() - 1.5).abs() < 0.001);
}

// ---------------------------------------------------------------------------
// Spatial queries
// ---------------------------------------------------------------------------

#[test]
fn spatial_grid_2d_query() {
    let mut scene = Scene::new();

    let inside = scene.create_entity_with_tag("Inside");
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(inside).unwrap();
        tc.translation = Vec3::new(5.0, 5.0, 0.0);
    }
    scene.add_component(inside, SpriteRendererComponent::default());

    let outside = scene.create_entity_with_tag("Outside");
    {
        let mut tc = scene.get_component_mut::<TransformComponent>(outside).unwrap();
        tc.translation = Vec3::new(100.0, 100.0, 0.0);
    }
    scene.add_component(outside, SpriteRendererComponent::default());

    scene.rebuild_spatial_grid(16.0);

    let results = scene.query_entities_in_region(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0));
    let inside_id = inside.id();
    let outside_id = outside.id();

    assert!(results.iter().any(|e| e.id() == inside_id));
    assert!(!results.iter().any(|e| e.id() == outside_id));
}

// ---------------------------------------------------------------------------
// Deferred entity destruction
// ---------------------------------------------------------------------------

#[test]
fn deferred_destruction_flushes_correctly() {
    let mut scene = Scene::new();
    let e1 = scene.create_entity_with_tag("A");
    let e2 = scene.create_entity_with_tag("B");
    let e3 = scene.create_entity_with_tag("C");

    let uuid1 = scene.get_component::<IdComponent>(e1).unwrap().id.raw();
    let uuid3 = scene.get_component::<IdComponent>(e3).unwrap().id.raw();

    scene.queue_entity_destroy(uuid1);
    scene.queue_entity_destroy(uuid3);

    assert_eq!(scene.entity_count(), 3);

    scene.flush_pending_destroys();

    assert_eq!(scene.entity_count(), 1);
    assert!(scene.is_alive(e2));
    assert!(!scene.is_alive(e1));
    assert!(!scene.is_alive(e3));
}

// ---------------------------------------------------------------------------
// Entity lookup methods
// ---------------------------------------------------------------------------

#[test]
fn find_entity_by_name_and_uuid() {
    let mut scene = Scene::new();
    let e = scene.create_entity_with_tag("Player");
    let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();

    let (found_entity, found_uuid) = scene.find_entity_by_name("Player").unwrap();
    assert_eq!(found_uuid, uuid);
    assert_eq!(found_entity.id(), e.id());

    let found = scene.find_entity_by_uuid(uuid).unwrap();
    assert_eq!(found.id(), e.id());

    assert!(scene.find_entity_by_name("NonExistent").is_none());
    assert!(scene.find_entity_by_uuid(999999).is_none());
}
