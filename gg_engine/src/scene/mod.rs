mod components;
mod entity;
pub mod native_script;

pub use components::{
    CameraComponent, NativeScriptComponent, SpriteRendererComponent, TagComponent,
    TransformComponent,
};
pub use entity::Entity;
pub use native_script::NativeScript;

use crate::input::Input;
use crate::renderer::Renderer;
use crate::timestep::Timestep;

/// A scene is a container for entities and their components.
///
/// Internally wraps a [`hecs::World`], providing a focused API surface.
/// The scene owns all entity data and provides methods for entity
/// creation, destruction, and component access.
pub struct Scene {
    world: hecs::World,
    viewport_width: u32,
    viewport_height: u32,
}

impl Scene {
    /// Create an empty scene.
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            viewport_width: 0,
            viewport_height: 0,
        }
    }

    // -----------------------------------------------------------------
    // Entity lifecycle
    // -----------------------------------------------------------------

    /// Create a new entity with a default [`TagComponent`] (`"Entity"`)
    /// and a default [`TransformComponent`] (identity matrix).
    pub fn create_entity(&mut self) -> Entity {
        self.create_entity_with_tag("Entity")
    }

    /// Create a new entity with the given tag name and a default
    /// [`TransformComponent`] (identity matrix).
    pub fn create_entity_with_tag(&mut self, name: &str) -> Entity {
        let handle = self.world.spawn((
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
    /// # Panics
    ///
    /// Panics if the entity does not exist.
    pub fn add_component<T: hecs::Component>(&mut self, entity: Entity, component: T) {
        self.world
            .insert_one(entity.handle(), component)
            .expect("Entity does not exist");
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

    /// Number of living entities in the scene.
    pub fn entity_count(&self) -> u32 {
        self.world.len()
    }

    /// Returns `true` if the entity handle is still valid (alive).
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.world.contains(entity.handle())
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
    pub fn on_update(&self, renderer: &mut Renderer) {
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
                    Some(*camera.camera.projection() * transform.transform.inverse());
                break;
            }
        }

        if let Some(vp) = main_camera_vp {
            renderer.set_view_projection(vp);

            // Draw all sprite entities.
            for (transform, sprite) in self
                .world
                .query::<(&TransformComponent, &SpriteRendererComponent)>()
                .iter()
            {
                renderer.draw_quad_transform(&transform.transform, sprite.color);
            }
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
        assert_eq!(t.transform, Mat4::IDENTITY);
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
            t.transform = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        }
        let t = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(
            t.transform,
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))
        );
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
