mod components;
mod entity;
pub mod native_script;
mod physics_2d;
mod scene_serializer;

pub use components::{
    BoxCollider2DComponent, CameraComponent, IdComponent, NativeScriptComponent,
    RigidBody2DComponent, RigidBody2DType, SpriteRendererComponent, TagComponent,
    TransformComponent,
};
pub use entity::Entity;
pub use native_script::NativeScript;
pub use scene_serializer::SceneSerializer;

use crate::input::Input;
use crate::renderer::Renderer;
use crate::timestep::Timestep;
use crate::uuid::Uuid;

use physics_2d::PhysicsWorld2D;
use rapier2d::na;

/// A scene is a container for entities and their components.
///
/// Internally wraps a [`hecs::World`], providing a focused API surface.
/// The scene owns all entity data and provides methods for entity
/// creation, destruction, and component access.
pub struct Scene {
    world: hecs::World,
    viewport_width: u32,
    viewport_height: u32,
    physics_world: Option<PhysicsWorld2D>,
}

impl Scene {
    /// Create an empty scene.
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            viewport_width: 0,
            viewport_height: 0,
            physics_world: None,
        }
    }

    // -----------------------------------------------------------------
    // Entity lifecycle
    // -----------------------------------------------------------------

    /// Create a new entity with a default [`TagComponent`] (`"Entity"`)
    /// and a default [`TransformComponent`] (identity matrix).
    /// A random [`Uuid`] is generated automatically.
    pub fn create_entity(&mut self) -> Entity {
        self.create_entity_with_uuid(Uuid::new(), "Entity")
    }

    /// Create a new entity with the given tag name and a default
    /// [`TransformComponent`] (identity matrix).
    /// A random [`Uuid`] is generated automatically.
    pub fn create_entity_with_tag(&mut self, name: &str) -> Entity {
        self.create_entity_with_uuid(Uuid::new(), name)
    }

    /// Create a new entity with a specific [`Uuid`] (e.g. from deserialization).
    pub fn create_entity_with_uuid(&mut self, uuid: Uuid, name: &str) -> Entity {
        let handle = self.world.spawn((
            IdComponent::new(uuid),
            TagComponent::new(name),
            TransformComponent::default(),
        ));
        Entity::new(handle)
    }

    /// Remove an entity and all its components from the scene.
    pub fn destroy_entity(&mut self, entity: Entity) -> Result<(), hecs::NoSuchEntity> {
        self.world.despawn(entity.handle())
    }

    // -----------------------------------------------------------------
    // Component access
    // -----------------------------------------------------------------

    /// Add a component to an entity. If the entity already has a
    /// component of this type, it is replaced.
    ///
    /// Automatically handles component-specific initialization (e.g.
    /// setting the viewport size on a newly added [`CameraComponent`]).
    ///
    /// # Panics
    ///
    /// Panics if the entity does not exist.
    pub fn add_component<T: hecs::Component>(&mut self, entity: Entity, component: T) {
        self.world
            .insert_one(entity.handle(), component)
            .expect("Entity does not exist");
        self.on_component_added::<T>(entity);
    }

    /// Component-specific initialization after insertion.
    fn on_component_added<T: 'static>(&mut self, entity: Entity) {
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<CameraComponent>() {
            let (w, h) = (self.viewport_width, self.viewport_height);
            if w > 0 && h > 0 {
                if let Ok(mut cam) = self.world.get::<&mut CameraComponent>(entity.handle()) {
                    if !cam.fixed_aspect_ratio {
                        cam.camera.set_viewport_size(w, h);
                    }
                }
            }
        }
    }

    /// Get an immutable reference to a component on an entity.
    ///
    /// Returns `None` if the entity does not have the component or
    /// the entity does not exist.
    pub fn get_component<T: hecs::Component>(&self, entity: Entity) -> Option<hecs::Ref<'_, T>> {
        self.world.get::<&T>(entity.handle()).ok()
    }

    /// Get a mutable reference to a component on an entity.
    ///
    /// Returns `None` if the entity does not have the component or
    /// the entity does not exist.
    pub fn get_component_mut<T: hecs::Component>(
        &mut self,
        entity: Entity,
    ) -> Option<hecs::RefMut<'_, T>> {
        self.world.get::<&mut T>(entity.handle()).ok()
    }

    /// Check whether an entity has a component of the given type.
    pub fn has_component<T: hecs::Component>(&self, entity: Entity) -> bool {
        self.world.get::<&T>(entity.handle()).is_ok()
    }

    /// Remove a component from an entity, returning the removed value.
    ///
    /// Returns `None` if the entity did not have a component of this type.
    pub fn remove_component<T: hecs::Component>(&mut self, entity: Entity) -> Option<T> {
        self.world.remove_one::<T>(entity.handle()).ok()
    }

    // -----------------------------------------------------------------
    // Query access (pass-through to hecs)
    // -----------------------------------------------------------------

    /// Borrow the underlying [`hecs::World`] for advanced queries.
    ///
    /// Use this for multi-component iteration:
    /// ```ignore
    /// for (transform, sprite) in scene.world().query::<(&TransformComponent, &SpriteRendererComponent)>().iter() {
    ///     // ...
    /// }
    /// ```
    pub fn world(&self) -> &hecs::World {
        &self.world
    }

    /// Mutable borrow of the underlying [`hecs::World`].
    pub fn world_mut(&mut self) -> &mut hecs::World {
        &mut self.world
    }

    // -----------------------------------------------------------------
    // Utility
    // -----------------------------------------------------------------

    /// Iterate all entities that have a [`TagComponent`], returning each
    /// entity handle paired with a clone of its tag string.
    ///
    /// Results are sorted by entity ID for stable display ordering.
    pub fn each_entity_with_tag(&self) -> Vec<(Entity, String)> {
        let mut entities: Vec<(Entity, String)> = self
            .world
            .query::<(hecs::Entity, &TagComponent)>()
            .iter()
            .map(|(handle, tag)| (Entity::new(handle), tag.tag.clone()))
            .collect();
        entities.sort_by_key(|(e, _)| e.id());
        entities
    }

    /// Find an entity by its raw integer ID (e.g. from pixel readback).
    ///
    /// Returns `None` if no living entity has the given ID.
    pub fn find_entity_by_id(&self, id: u32) -> Option<Entity> {
        for handle in self.world.query::<hecs::Entity>().iter() {
            if handle.id() == id {
                return Some(Entity::new(handle));
            }
        }
        None
    }

    /// Number of living entities in the scene.
    pub fn entity_count(&self) -> u32 {
        self.world.len()
    }

    /// Returns `true` if the entity handle is still valid (alive).
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.world.contains(entity.handle())
    }

    /// Return the entity that has a [`CameraComponent`] with `primary = true`.
    ///
    /// Returns `None` if no primary camera exists in the scene.
    pub fn get_primary_camera_entity(&self) -> Option<Entity> {
        for (handle, camera) in self
            .world
            .query::<(hecs::Entity, &CameraComponent)>()
            .iter()
        {
            if camera.primary {
                return Some(Entity::new(handle));
            }
        }
        None
    }

    /// Set `entity` as the primary camera, clearing the `primary` flag on
    /// all other [`CameraComponent`]s. If `entity` does not have a
    /// `CameraComponent`, this is a no-op.
    pub fn set_primary_camera(&mut self, entity: Entity) {
        for (handle, camera) in self
            .world
            .query_mut::<(hecs::Entity, &mut CameraComponent)>()
        {
            camera.primary = handle == entity.handle();
        }
    }

    // -----------------------------------------------------------------
    // Viewport
    // -----------------------------------------------------------------

    /// Notify the scene that the viewport (or framebuffer) dimensions changed.
    ///
    /// Iterates all [`CameraComponent`]s whose `fixed_aspect_ratio` is `false`
    /// and updates their [`SceneCamera`](crate::renderer::SceneCamera) projection
    /// to match the new aspect ratio.
    pub fn on_viewport_resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.viewport_width == width && self.viewport_height == height {
            return;
        }
        self.viewport_width = width;
        self.viewport_height = height;

        for camera in self.world.query_mut::<&mut CameraComponent>() {
            if !camera.fixed_aspect_ratio {
                camera.camera.set_viewport_size(width, height);
            }
        }
    }

    // -----------------------------------------------------------------
    // Physics (runtime lifecycle)
    // -----------------------------------------------------------------

    /// Initialize the physics world and create rigid bodies / colliders
    /// from all entities that have physics components.
    ///
    /// Call this when entering play mode (before the first physics step).
    pub fn on_runtime_start(&mut self) {
        let mut physics = PhysicsWorld2D::new(0.0, -9.81);

        // Snapshot entities with RigidBody2DComponent to avoid borrow conflicts.
        let body_entities: Vec<(hecs::Entity, glam::Vec3, glam::Vec3, glam::Vec3, RigidBody2DType, bool)> = self
            .world
            .query::<(
                hecs::Entity,
                &TransformComponent,
                &RigidBody2DComponent,
            )>()
            .iter()
            .map(|(handle, transform, rb)| {
                (
                    handle,
                    transform.translation,
                    transform.rotation,
                    transform.scale,
                    rb.body_type,
                    rb.fixed_rotation,
                )
            })
            .collect();

        for (handle, translation, rotation, scale, body_type, fixed_rotation) in body_entities {
            // Create rapier rigid body.
            let mut body_builder = rapier2d::dynamics::RigidBodyBuilder::new(body_type.to_rapier())
                .translation(na::Vector2::new(translation.x, translation.y))
                .rotation(rotation.z);

            if fixed_rotation {
                body_builder = body_builder.lock_rotations();
            }

            let body_handle = physics.bodies.insert(body_builder.build());

            // Store the handle back on the component.
            if let Ok(mut rb) = self.world.get::<&mut RigidBody2DComponent>(handle) {
                rb.runtime_body = Some(body_handle);
            }

            // If entity also has a BoxCollider2DComponent, create a collider.
            if let Ok(mut bc) = self.world.get::<&mut BoxCollider2DComponent>(handle) {
                let half_x = bc.size.x * scale.x.abs();
                let half_y = bc.size.y * scale.y.abs();

                let collider = rapier2d::geometry::ColliderBuilder::cuboid(half_x, half_y)
                    .density(bc.density)
                    .friction(bc.friction)
                    .restitution(bc.restitution)
                    .translation(na::Vector2::new(bc.offset.x, bc.offset.y))
                    .build();

                let collider_handle =
                    physics
                        .colliders
                        .insert_with_parent(collider, body_handle, &mut physics.bodies);
                bc.runtime_fixture = Some(collider_handle);
            }
        }

        self.physics_world = Some(physics);
    }

    /// Tear down the physics world and clear all runtime handles.
    ///
    /// Call this when exiting play mode (before restoring the snapshot).
    pub fn on_runtime_stop(&mut self) {
        self.physics_world = None;

        // Clear runtime handles on all physics components.
        for rb in self.world.query_mut::<&mut RigidBody2DComponent>() {
            rb.runtime_body = None;
        }
        for bc in self.world.query_mut::<&mut BoxCollider2DComponent>() {
            bc.runtime_fixture = None;
        }
    }

    /// Step the physics simulation and write body transforms back to entities.
    ///
    /// Call this each frame during play mode, after `on_update_scripts`.
    pub fn on_update_physics(&mut self, _dt: Timestep) {
        if let Some(ref mut physics) = self.physics_world {
            physics.step();

            // Write physics body positions back to transforms.
            for (transform, rb) in self
                .world
                .query_mut::<(&mut TransformComponent, &RigidBody2DComponent)>()
            {
                if let Some(body_handle) = rb.runtime_body {
                    if let Some(body) = physics.bodies.get(body_handle) {
                        let pos = body.translation();
                        transform.translation.x = pos.x;
                        transform.translation.y = pos.y;
                        transform.rotation.z = body.rotation().angle();
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Per-frame update / render
    // -----------------------------------------------------------------

    /// Run all [`NativeScriptComponent`] scripts for this frame.
    ///
    /// Scripts are lazily instantiated on their first update. The update order
    /// follows hecs iteration order (not guaranteed to be stable across
    /// entity additions/removals).
    ///
    /// Call this from [`Application::on_update`] each frame, **before** rendering.
    pub fn on_update_scripts(&mut self, dt: Timestep, input: &Input) {
        // Collect entity handles that have a NativeScriptComponent.
        // We snapshot first because we need &mut self inside the loop.
        let script_entities: Vec<(hecs::Entity, bool)> = self
            .world
            .query::<(hecs::Entity, &NativeScriptComponent)>()
            .iter()
            .map(|(e, nsc)| (e, nsc.instance.is_some()))
            .collect();

        for (handle, had_instance) in script_entities {
            let entity = Entity::new(handle);

            // Lazy instantiation.
            if !had_instance {
                if let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) {
                    nsc.instance = Some((nsc.instantiate_fn)());
                }
            }

            // Take the instance out to release the hecs borrow, allowing
            // script methods to access &mut self (Scene) freely.
            let (mut instance, needs_create) = {
                let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) else {
                    continue;
                };
                let Some(inst) = nsc.instance.take() else {
                    continue;
                };
                let needs_create = !nsc.created;
                nsc.created = true;
                (inst, needs_create)
            };

            if needs_create {
                instance.on_create(entity, self);
            }
            instance.on_update(entity, self, dt, input);

            // Put the instance back.
            if let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) {
                nsc.instance = Some(instance);
            }
        }
    }

    /// Find the primary camera, set the view-projection matrix on the
    /// renderer, and draw all entities with sprites.
    ///
    /// If no entity has a [`CameraComponent`] with `primary = true`, nothing
    /// is rendered.
    ///
    /// Use this for **runtime** rendering where the scene's own ECS camera
    /// drives the view. For editor rendering with an external camera, use
    /// [`on_update_editor`](Self::on_update_editor).
    pub fn on_update_runtime(&self, renderer: &mut Renderer) {
        // Find the primary camera entity.
        let mut main_camera_vp: Option<glam::Mat4> = None;
        for (transform, camera) in self
            .world
            .query::<(&TransformComponent, &CameraComponent)>()
            .iter()
        {
            if camera.primary {
                // VP = projection * inverse(camera_transform)
                main_camera_vp =
                    Some(*camera.camera.projection() * transform.get_transform().inverse());
                break;
            }
        }

        if let Some(vp) = main_camera_vp {
            renderer.set_view_projection(vp);

            // Draw all sprite entities.
            for (entity, transform, sprite) in self
                .world
                .query::<(
                    hecs::Entity,
                    &TransformComponent,
                    &SpriteRendererComponent,
                )>()
                .iter()
            {
                renderer.draw_sprite(
                    &transform.get_transform(),
                    sprite,
                    entity.id() as i32,
                );
            }
        }
    }

    /// Render all sprite entities using an externally provided view-projection
    /// matrix (e.g. from an [`EditorCamera`](crate::renderer::EditorCamera)).
    ///
    /// Unlike [`on_update_runtime`](Self::on_update_runtime), this does **not**
    /// look for a primary camera entity — it always renders.
    pub fn on_update_editor(&self, editor_camera_vp: &glam::Mat4, renderer: &mut Renderer) {
        renderer.set_view_projection(*editor_camera_vp);

        for (entity, transform, sprite) in self
            .world
            .query::<(
                hecs::Entity,
                &TransformComponent,
                &SpriteRendererComponent,
            )>()
            .iter()
        {
            renderer.draw_sprite(
                &transform.get_transform(),
                sprite,
                entity.id() as i32,
            );
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    #[test]
    fn create_entity_has_default_components() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        assert!(scene.has_component::<TagComponent>(e));
        assert!(scene.has_component::<TransformComponent>(e));
    }

    #[test]
    fn create_entity_with_tag() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Player");
        let tag = scene.get_component::<TagComponent>(e).unwrap();
        assert_eq!(tag.tag, "Player");
    }

    #[test]
    fn default_tag_is_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let tag = scene.get_component::<TagComponent>(e).unwrap();
        assert_eq!(tag.tag, "Entity");
    }

    #[test]
    fn default_transform_is_identity() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let t = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(t.get_transform(), Mat4::IDENTITY);
    }

    #[test]
    fn destroy_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        assert_eq!(scene.entity_count(), 1);
        scene.destroy_entity(e).unwrap();
        assert_eq!(scene.entity_count(), 0);
        assert!(!scene.is_alive(e));
    }

    #[test]
    fn add_and_get_component() {
        struct Health(f32);

        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, Health(100.0));
        let h = scene.get_component::<Health>(e).unwrap();
        assert_eq!(h.0, 100.0);
    }

    #[test]
    fn get_component_mut() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        {
            let mut t = scene.get_component_mut::<TransformComponent>(e).unwrap();
            t.translation = Vec3::new(1.0, 2.0, 3.0);
        }
        let t = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn remove_component() {
        #[allow(dead_code)]
        struct Velocity(f32);

        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, Velocity(5.0));
        assert!(scene.has_component::<Velocity>(e));
        let removed = scene.remove_component::<Velocity>(e);
        assert!(removed.is_some());
        assert!(!scene.has_component::<Velocity>(e));
    }

    #[test]
    fn get_nonexistent_component_returns_none() {
        struct Missing;

        let mut scene = Scene::new();
        let e = scene.create_entity();
        assert!(scene.get_component::<Missing>(e).is_none());
    }

    #[test]
    fn entity_is_copy() {
        let mut scene = Scene::new();
        let e1 = scene.create_entity();
        let e2 = e1; // Copy
        assert_eq!(e1, e2);
    }

    #[test]
    fn multiple_entities() {
        let mut scene = Scene::new();
        let e1 = scene.create_entity_with_tag("A");
        let e2 = scene.create_entity_with_tag("B");
        let e3 = scene.create_entity_with_tag("C");
        assert_eq!(scene.entity_count(), 3);
        assert_ne!(e1, e2);
        assert_ne!(e2, e3);
        scene.destroy_entity(e2).unwrap();
        assert_eq!(scene.entity_count(), 2);
        assert!(scene.is_alive(e1));
        assert!(!scene.is_alive(e2));
        assert!(scene.is_alive(e3));
    }

    #[test]
    fn query_world_directly() {
        let mut scene = Scene::new();
        scene.create_entity_with_tag("A");
        scene.create_entity_with_tag("B");

        let count = scene.world().query::<&TagComponent>().iter().count();
        assert_eq!(count, 2);
    }
}
