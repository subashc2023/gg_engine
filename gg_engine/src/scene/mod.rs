mod components;
mod entity;
pub mod native_script;
mod physics_2d;
mod scene_serializer;

pub use components::{
    BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent, CircleRendererComponent,
    IdComponent, NativeScriptComponent, RigidBody2DComponent, RigidBody2DType,
    SpriteRendererComponent, TagComponent, TransformComponent,
};
pub use entity::Entity;
pub use native_script::NativeScript;
pub use scene_serializer::SceneSerializer;

use crate::input::Input;
use crate::renderer::Renderer;
use crate::timestep::Timestep;
use crate::uuid::Uuid;

use std::collections::HashMap;

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
    // Scene / Entity copying
    // -----------------------------------------------------------------

    /// Create a deep copy of the entire scene.
    ///
    /// All entities are recreated with their original UUIDs (via
    /// [`create_entity_with_uuid`](Self::create_entity_with_uuid)) and all
    /// component data is cloned. Runtime-only handles (physics bodies,
    /// colliders) are reset to `None`. Script instances are not copied —
    /// they will be lazily re-instantiated on the first update.
    ///
    /// Used by the editor to create a runtime scene from the editor scene
    /// when entering play mode.
    pub fn copy(source: &Scene) -> Scene {
        let mut new_scene = Scene::new();
        new_scene.viewport_width = source.viewport_width;
        new_scene.viewport_height = source.viewport_height;

        // Phase 1: Create entities with matching UUIDs.
        // Build a map from source hecs handle → destination Entity.
        // Collect and sort by hecs entity ID so the destination world spawns
        // entities in the same relative order (preserves Scene Hierarchy ordering).
        let mut entity_map: HashMap<hecs::Entity, Entity> = HashMap::new();

        let mut source_entities: Vec<_> = source
            .world
            .query::<(hecs::Entity, &IdComponent, &TagComponent)>()
            .iter()
            .map(|(handle, id, tag)| (handle, id.id, tag.tag.clone()))
            .collect();
        source_entities.sort_by_key(|(handle, _, _)| handle.id());

        for (handle, uuid, tag) in &source_entities {
            let new_entity = new_scene.create_entity_with_uuid(*uuid, tag);
            entity_map.insert(*handle, new_entity);
        }

        // Phase 2: Copy components.
        // TransformComponent — already exists on destination, overwrite values.
        copy_component_if_has::<TransformComponent>(&source.world, &mut new_scene, &entity_map);
        // CameraComponent
        copy_component_if_has::<CameraComponent>(&source.world, &mut new_scene, &entity_map);
        // SpriteRendererComponent
        copy_component_if_has::<SpriteRendererComponent>(
            &source.world,
            &mut new_scene,
            &entity_map,
        );
        // CircleRendererComponent
        copy_component_if_has::<CircleRendererComponent>(
            &source.world,
            &mut new_scene,
            &entity_map,
        );
        // RigidBody2DComponent (Clone impl resets runtime_body to None)
        copy_component_if_has::<RigidBody2DComponent>(&source.world, &mut new_scene, &entity_map);
        // BoxCollider2DComponent (Clone impl resets runtime_fixture to None)
        copy_component_if_has::<BoxCollider2DComponent>(
            &source.world,
            &mut new_scene,
            &entity_map,
        );
        // CircleCollider2DComponent (Clone impl resets runtime_fixture to None)
        copy_component_if_has::<CircleCollider2DComponent>(
            &source.world,
            &mut new_scene,
            &entity_map,
        );
        // NativeScriptComponent — manual copy (not Clone-able).
        for (handle, nsc) in source
            .world
            .query::<(hecs::Entity, &NativeScriptComponent)>()
            .iter()
        {
            if let Some(&dst_entity) = entity_map.get(&handle) {
                new_scene.add_component(
                    dst_entity,
                    NativeScriptComponent {
                        instance: None,
                        instantiate_fn: nsc.instantiate_fn,
                        created: false,
                    },
                );
            }
        }

        new_scene
    }

    /// Duplicate an entity within this scene, returning the new entity.
    ///
    /// The duplicate receives a fresh UUID but copies the tag name and all
    /// component data from `entity`. Useful for Ctrl+D in the editor.
    pub fn duplicate_entity(&mut self, entity: Entity) -> Entity {
        // Phase 1: Extract all component data as owned values.
        // Each `.map()` closure accesses fields via Deref on hecs::Ref, then
        // the Ref is dropped — releasing the world borrow before the next get.
        let name = self
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_else(|| "Entity".into());

        let transform_data = self
            .get_component::<TransformComponent>(entity)
            .map(|tc| (tc.translation, tc.rotation, tc.scale));

        let camera_data = self
            .get_component::<CameraComponent>(entity)
            .map(|cam| (cam.camera.clone(), cam.primary, cam.fixed_aspect_ratio));

        let sprite_data = self
            .get_component::<SpriteRendererComponent>(entity)
            .map(|s| (s.color, s.texture.clone(), s.tiling_factor));

        let circle_data = self
            .get_component::<CircleRendererComponent>(entity)
            .map(|c| (c.color, c.thickness, c.fade));

        let rb_data = self
            .get_component::<RigidBody2DComponent>(entity)
            .map(|rb| (rb.body_type, rb.fixed_rotation));

        let bc_data = self
            .get_component::<BoxCollider2DComponent>(entity)
            .map(|bc| {
                (
                    bc.offset,
                    bc.size,
                    bc.density,
                    bc.friction,
                    bc.restitution,
                    bc.restitution_threshold,
                )
            });

        let cc_data = self
            .get_component::<CircleCollider2DComponent>(entity)
            .map(|cc| {
                (
                    cc.offset,
                    cc.radius,
                    cc.density,
                    cc.friction,
                    cc.restitution,
                    cc.restitution_threshold,
                )
            });

        let nsc_data = self
            .world
            .get::<&NativeScriptComponent>(entity.handle())
            .ok()
            .map(|nsc| nsc.instantiate_fn);

        // Phase 2: Create new entity with same name but new UUID.
        let new_entity = self.create_entity_with_tag(&name);

        // Phase 3: Copy component data.
        if let Some((translation, rotation, scale)) = transform_data {
            if let Some(mut tc) = self.get_component_mut::<TransformComponent>(new_entity) {
                tc.translation = translation;
                tc.rotation = rotation;
                tc.scale = scale;
            }
        }

        if let Some((camera, primary, fixed_aspect_ratio)) = camera_data {
            self.add_component(
                new_entity,
                CameraComponent {
                    camera,
                    primary,
                    fixed_aspect_ratio,
                },
            );
        }

        if let Some((color, texture, tiling_factor)) = sprite_data {
            self.add_component(
                new_entity,
                SpriteRendererComponent {
                    color,
                    texture,
                    tiling_factor,
                },
            );
        }

        if let Some((color, thickness, fade)) = circle_data {
            self.add_component(
                new_entity,
                CircleRendererComponent {
                    color,
                    thickness,
                    fade,
                },
            );
        }

        if let Some((body_type, fixed_rotation)) = rb_data {
            self.add_component(
                new_entity,
                RigidBody2DComponent {
                    body_type,
                    fixed_rotation,
                    runtime_body: None,
                },
            );
        }

        if let Some((offset, size, density, friction, restitution, restitution_threshold)) =
            bc_data
        {
            self.add_component(
                new_entity,
                BoxCollider2DComponent {
                    offset,
                    size,
                    density,
                    friction,
                    restitution,
                    restitution_threshold,
                    runtime_fixture: None,
                },
            );
        }

        if let Some((offset, radius, density, friction, restitution, restitution_threshold)) =
            cc_data
        {
            self.add_component(
                new_entity,
                CircleCollider2DComponent {
                    offset,
                    radius,
                    density,
                    friction,
                    restitution,
                    restitution_threshold,
                    runtime_fixture: None,
                },
            );
        }

        if let Some(instantiate_fn) = nsc_data {
            self.add_component(
                new_entity,
                NativeScriptComponent {
                    instance: None,
                    instantiate_fn,
                    created: false,
                },
            );
        }

        new_entity
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
    // Physics (shared helpers)
    // -----------------------------------------------------------------

    /// Create the physics world and populate it with rigid bodies / colliders
    /// from all entities that have physics components.
    ///
    /// Shared by both runtime and simulation start paths.
    fn on_physics_2d_start(&mut self) {
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

            // If entity also has a CircleCollider2DComponent, create a collider.
            if let Ok(mut cc) = self.world.get::<&mut CircleCollider2DComponent>(handle) {
                let scaled_radius = cc.radius * scale.x.abs();

                let collider = rapier2d::geometry::ColliderBuilder::ball(scaled_radius)
                    .density(cc.density)
                    .friction(cc.friction)
                    .restitution(cc.restitution)
                    .translation(na::Vector2::new(cc.offset.x, cc.offset.y))
                    .build();

                let collider_handle =
                    physics
                        .colliders
                        .insert_with_parent(collider, body_handle, &mut physics.bodies);
                cc.runtime_fixture = Some(collider_handle);
            }
        }

        self.physics_world = Some(physics);
    }

    /// Tear down the physics world and clear all runtime handles.
    ///
    /// Shared by both runtime and simulation stop paths.
    fn on_physics_2d_stop(&mut self) {
        self.physics_world = None;

        // Clear runtime handles on all physics components.
        for rb in self.world.query_mut::<&mut RigidBody2DComponent>() {
            rb.runtime_body = None;
        }
        for bc in self.world.query_mut::<&mut BoxCollider2DComponent>() {
            bc.runtime_fixture = None;
        }
        for cc in self.world.query_mut::<&mut CircleCollider2DComponent>() {
            cc.runtime_fixture = None;
        }
    }

    // -----------------------------------------------------------------
    // Runtime lifecycle
    // -----------------------------------------------------------------

    /// Initialize physics for runtime (play mode).
    ///
    /// Call this when entering play mode (before the first physics step).
    pub fn on_runtime_start(&mut self) {
        self.on_physics_2d_start();
    }

    /// Tear down physics for runtime (play mode).
    ///
    /// Call this when exiting play mode (before restoring the snapshot).
    pub fn on_runtime_stop(&mut self) {
        self.on_physics_2d_stop();
    }

    // -----------------------------------------------------------------
    // Simulation lifecycle
    // -----------------------------------------------------------------

    /// Initialize physics for simulation mode.
    ///
    /// Call this when entering simulate mode. Sets up the physics world
    /// without initializing scripts — physics only.
    pub fn on_simulation_start(&mut self) {
        self.on_physics_2d_start();
    }

    /// Tear down physics for simulation mode.
    ///
    /// Call this when exiting simulate mode.
    pub fn on_simulation_stop(&mut self) {
        self.on_physics_2d_stop();
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

    /// Draw all sprite and circle entities.
    ///
    /// Shared rendering code used by editor, simulation, and runtime paths.
    /// The caller is responsible for setting the view-projection matrix on
    /// the renderer before calling this.
    fn render_scene(&self, renderer: &mut Renderer) {
        // Draw sprites.
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

        // Draw circles.
        for (entity, transform, circle) in self
            .world
            .query::<(
                hecs::Entity,
                &TransformComponent,
                &CircleRendererComponent,
            )>()
            .iter()
        {
            renderer.draw_circle_component(
                &transform.get_transform(),
                circle,
                entity.id() as i32,
            );
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
            self.render_scene(renderer);
        }
    }

    /// Render all entities using an externally provided view-projection
    /// matrix (e.g. from an [`EditorCamera`](crate::renderer::EditorCamera)).
    ///
    /// Unlike [`on_update_runtime`](Self::on_update_runtime), this does **not**
    /// look for a primary camera entity — it always renders.
    pub fn on_update_editor(&self, editor_camera_vp: &glam::Mat4, renderer: &mut Renderer) {
        renderer.set_view_projection(*editor_camera_vp);
        self.render_scene(renderer);
    }

    /// Render the scene from the editor camera for simulation mode.
    ///
    /// Like [`on_update_editor`], this uses an external camera matrix.
    /// The physics stepping is handled separately in `on_update_physics`.
    pub fn on_update_simulation(&self, editor_camera_vp: &glam::Mat4, renderer: &mut Renderer) {
        renderer.set_view_projection(*editor_camera_vp);
        self.render_scene(renderer);
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy all instances of a cloneable component from `src` world into `dst` scene,
/// using `entity_map` to translate source hecs handles to destination entities.
fn copy_component_if_has<T: hecs::Component + Clone>(
    src: &hecs::World,
    dst: &mut Scene,
    entity_map: &HashMap<hecs::Entity, Entity>,
) {
    for (handle, comp) in src.query::<(hecs::Entity, &T)>().iter() {
        if let Some(&dst_entity) = entity_map.get(&handle) {
            dst.add_component(dst_entity, comp.clone());
        }
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

    #[test]
    fn copy_preserves_entity_count() {
        let mut scene = Scene::new();
        scene.create_entity_with_tag("A");
        scene.create_entity_with_tag("B");
        scene.create_entity_with_tag("C");

        let copy = Scene::copy(&scene);
        assert_eq!(copy.entity_count(), 3);
    }

    #[test]
    fn copy_preserves_uuids() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Player");
        let original_uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();

        let copy = Scene::copy(&scene);
        // Find the entity in the copy by iterating.
        let mut found = false;
        for id in copy.world().query::<&IdComponent>().iter() {
            if id.id.raw() == original_uuid {
                found = true;
                break;
            }
        }
        assert!(found, "UUID not preserved in copy");
    }

    #[test]
    fn copy_preserves_transform() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Obj");
        {
            let mut t = scene.get_component_mut::<TransformComponent>(e).unwrap();
            t.translation = Vec3::new(1.0, 2.0, 3.0);
            t.rotation = Vec3::new(0.1, 0.2, 0.3);
            t.scale = Vec3::new(4.0, 5.0, 6.0);
        }

        let copy = Scene::copy(&scene);
        let mut query = copy.world().query::<&TransformComponent>();
        let tc = query.iter().next().unwrap();
        assert_eq!(tc.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(tc.rotation, Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(tc.scale, Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn copy_is_independent() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Obj");
        {
            let mut t = scene.get_component_mut::<TransformComponent>(e).unwrap();
            t.translation = Vec3::new(1.0, 0.0, 0.0);
        }

        let mut copy = Scene::copy(&scene);
        // Modify the copy — original should be unaffected.
        for tc in copy.world_mut().query_mut::<&mut TransformComponent>() {
            tc.translation = Vec3::new(99.0, 99.0, 99.0);
        }

        let original_tc = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(original_tc.translation, Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn duplicate_entity_creates_new_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Original");
        assert_eq!(scene.entity_count(), 1);

        let dup = scene.duplicate_entity(e);
        assert_eq!(scene.entity_count(), 2);
        assert_ne!(e, dup);
    }

    #[test]
    fn duplicate_entity_copies_tag() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Player");
        let dup = scene.duplicate_entity(e);

        let tag = scene.get_component::<TagComponent>(dup).unwrap();
        assert_eq!(tag.tag, "Player");
    }

    #[test]
    fn duplicate_entity_new_uuid() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Obj");
        let dup = scene.duplicate_entity(e);

        let uuid_orig = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        let uuid_dup = scene.get_component::<IdComponent>(dup).unwrap().id.raw();
        assert_ne!(uuid_orig, uuid_dup);
    }

    #[test]
    fn duplicate_entity_copies_transform() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Obj");
        {
            let mut t = scene.get_component_mut::<TransformComponent>(e).unwrap();
            t.translation = Vec3::new(5.0, 6.0, 7.0);
        }

        let dup = scene.duplicate_entity(e);
        let t = scene.get_component::<TransformComponent>(dup).unwrap();
        assert_eq!(t.translation, Vec3::new(5.0, 6.0, 7.0));
    }
}
