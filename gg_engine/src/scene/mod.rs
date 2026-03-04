pub mod animation;
pub(crate) mod audio;
mod components;
mod entity;
pub mod native_script;
mod physics_2d;
mod scene_serializer;
#[cfg(feature = "lua-scripting")]
mod script_glue;
#[cfg(feature = "lua-scripting")]
pub(crate) mod script_engine;

use crate::renderer::Font;

pub use components::{
    AudioSourceComponent, BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent,
    CircleRendererComponent, IdComponent, NativeScriptComponent, RelationshipComponent,
    RigidBody2DComponent, RigidBody2DType, SpriteRendererComponent, TagComponent, TextComponent,
    TilemapComponent, TransformComponent,
    TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
#[cfg(feature = "lua-scripting")]
pub use components::LuaScriptComponent;
pub use animation::{AnimationClip, SpriteAnimatorComponent};
pub use entity::Entity;
pub use native_script::NativeScript;
pub use scene_serializer::SceneSerializer;
#[cfg(feature = "lua-scripting")]
pub use script_engine::{ScriptEngine, ScriptFieldValue};

use crate::input::Input;
use crate::renderer::{Renderer, SubTexture2D};
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
    #[cfg(feature = "lua-scripting")]
    script_engine: Option<ScriptEngine>,
    audio_engine: Option<audio::AudioEngine>,
    /// O(1) UUID → hecs::Entity lookup cache, maintained on create/destroy.
    uuid_cache: HashMap<u64, hecs::Entity>,
    /// Deferred entity destruction queue (UUIDs). Flushed after script callbacks.
    pending_destroy: Vec<u64>,
}

/// Invokes `$callback!` with every cloneable component type.
///
/// When adding a new component type, add it here and it will automatically
/// be included in scene copy, entity duplication, etc.
/// `NativeScriptComponent` is excluded (not `Clone` — handled manually).
macro_rules! for_each_cloneable_component {
    ($callback:ident) => {
        $callback!(
            TransformComponent,
            CameraComponent,
            SpriteRendererComponent,
            CircleRendererComponent,
            TextComponent,
            RigidBody2DComponent,
            BoxCollider2DComponent,
            CircleCollider2DComponent,
            RelationshipComponent,
            SpriteAnimatorComponent,
            TilemapComponent,
            AudioSourceComponent,
        );
    };
}

impl Scene {
    /// Create an empty scene.
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            viewport_width: 0,
            viewport_height: 0,
            physics_world: None,
            #[cfg(feature = "lua-scripting")]
            script_engine: None,
            audio_engine: None,
            uuid_cache: HashMap::new(),
            pending_destroy: Vec::new(),
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
            RelationshipComponent::default(),
        ));
        self.uuid_cache.insert(uuid.raw(), handle);
        Entity::new(handle)
    }

    /// Remove an entity and all its components from the scene.
    ///
    /// Recursively destroys all children. Detaches from parent if parented.
    pub fn destroy_entity(&mut self, entity: Entity) -> Result<(), hecs::NoSuchEntity> {
        // Collect relationship info before despawning.
        let (uuid, parent_uuid, child_uuids) = {
            let uuid = self
                .world
                .get::<&IdComponent>(entity.handle())
                .ok()
                .map(|id| id.id.raw());
            let rel = self
                .world
                .get::<&RelationshipComponent>(entity.handle())
                .ok();
            let parent = rel.as_deref().and_then(|r| r.parent);
            let children = rel
                .as_deref()
                .map(|r| r.children.clone())
                .unwrap_or_default();
            (uuid, parent, children)
        };

        // Detach from parent.
        if let (Some(my_uuid), Some(parent_uuid)) = (uuid, parent_uuid) {
            if let Some(parent_entity) = self.find_entity_by_uuid(parent_uuid) {
                if let Ok(mut rel) = self.world.get::<&mut RelationshipComponent>(parent_entity.handle()) {
                    rel.children.retain(|&c| c != my_uuid);
                }
            }
        }

        // Remove from UUID cache.
        if let Some(u) = uuid {
            self.uuid_cache.remove(&u);
        }

        // Despawn self.
        self.world.despawn(entity.handle())?;

        // Recursively destroy children.
        for child_uuid in child_uuids {
            if let Some(child_entity) = self.find_entity_by_uuid(child_uuid) {
                let _ = self.destroy_entity(child_entity);
            }
        }

        Ok(())
    }

    /// Queue an entity for deferred destruction (by UUID).
    ///
    /// The entity is not destroyed immediately — call
    /// [`flush_pending_destroys`](Self::flush_pending_destroys) after all
    /// script callbacks complete. Duplicates are ignored during flush.
    pub fn queue_entity_destroy(&mut self, uuid: u64) {
        self.pending_destroy.push(uuid);
    }

    /// Destroy all entities queued via [`queue_entity_destroy`](Self::queue_entity_destroy).
    ///
    /// Cleans up physics bodies/colliders for each destroyed entity.
    /// Safe to call even if the queue is empty.
    pub fn flush_pending_destroys(&mut self) {
        if self.pending_destroy.is_empty() {
            return;
        }

        // Deduplicate.
        let uuids: Vec<u64> = {
            let mut v = std::mem::take(&mut self.pending_destroy);
            v.sort_unstable();
            v.dedup();
            v
        };

        for uuid in uuids {
            if let Some(entity) = self.find_entity_by_uuid(uuid) {
                // Extract physics body handle before borrowing physics_world mutably.
                let body_handle = self
                    .get_component::<RigidBody2DComponent>(entity)
                    .and_then(|rb| rb.runtime_body);

                if let (Some(handle), Some(ref mut physics)) =
                    (body_handle, &mut self.physics_world)
                {
                    physics.bodies.remove(
                        handle,
                        &mut physics.island_manager,
                        &mut physics.colliders,
                        &mut physics.impulse_joints,
                        &mut physics.multibody_joints,
                        true,
                    );
                }
                let _ = self.destroy_entity(entity);
            }
        }
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

        // Phase 2: Copy cloneable components.
        macro_rules! copy_all {
            ($($comp:ty),* $(,)?) => {
                $(copy_component_if_has::<$comp>(&source.world, &mut new_scene, &entity_map);)*
            };
        }
        for_each_cloneable_component!(copy_all);
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

        // LuaScriptComponent — Clone-able, copy via helper.
        #[cfg(feature = "lua-scripting")]
        copy_component_if_has::<LuaScriptComponent>(&source.world, &mut new_scene, &entity_map);

        new_scene
    }

    /// Duplicate an entity within this scene, returning the new entity.
    ///
    /// The duplicate receives a fresh UUID but copies the tag name and all
    /// component data from `entity`. Useful for Ctrl+D in the editor.
    pub fn duplicate_entity(&mut self, entity: Entity) -> Entity {
        let name = self
            .get_component::<TagComponent>(entity)
            .map(|t| t.tag.clone())
            .unwrap_or_else(|| "Entity".into());

        let nsc_data = self
            .world
            .get::<&NativeScriptComponent>(entity.handle())
            .ok()
            .map(|nsc| nsc.instantiate_fn);

        let new_entity = self.create_entity_with_tag(&name);

        // Copy all cloneable components.
        macro_rules! duplicate_all {
            ($($comp:ty),* $(,)?) => {
                $(duplicate_component_if_has::<$comp>(self, entity, new_entity);)*
            };
        }
        for_each_cloneable_component!(duplicate_all);

        // NativeScriptComponent — manual (not Clone).
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

        // LuaScriptComponent — Clone-able, duplicate via helper.
        #[cfg(feature = "lua-scripting")]
        duplicate_component_if_has::<LuaScriptComponent>(self, entity, new_entity);

        // Reset relationship — duplicate is a root entity (no parent, no children).
        self.add_component(new_entity, RelationshipComponent::default());

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

    /// Find the first entity whose [`TagComponent`] name matches `name`.
    ///
    /// O(n) scan — intended for one-off lookups (e.g. `on_create`), not
    /// per-frame use in hot loops. Returns the entity and its UUID.
    pub fn find_entity_by_name(&self, name: &str) -> Option<(Entity, u64)> {
        for (handle, tag, id) in self
            .world
            .query::<(hecs::Entity, &TagComponent, &IdComponent)>()
            .iter()
        {
            if tag.tag == name {
                return Some((Entity::new(handle), id.id.raw()));
            }
        }
        None
    }

    /// Find an entity by its UUID (from [`IdComponent`]).
    ///
    /// O(1) lookup via internal cache maintained on entity create/destroy.
    pub fn find_entity_by_uuid(&self, uuid: u64) -> Option<Entity> {
        self.uuid_cache
            .get(&uuid)
            .copied()
            .map(Entity::new)
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
    // Hierarchy (parent-child relationships)
    // -----------------------------------------------------------------

    /// Set `child` as a child of `parent`. Detaches from current parent if any.
    ///
    /// If `preserve_world_transform` is `true`, the child's local transform is
    /// adjusted so its world position stays the same.
    ///
    /// Returns `false` if the operation would create a cycle.
    pub fn set_parent(
        &mut self,
        child: Entity,
        parent: Entity,
        preserve_world_transform: bool,
    ) -> bool {
        let child_uuid = match self.get_component::<IdComponent>(child) {
            Some(id) => id.id.raw(),
            None => return false,
        };
        let parent_uuid = match self.get_component::<IdComponent>(parent) {
            Some(id) => id.id.raw(),
            None => return false,
        };

        // Prevent self-parenting.
        if child_uuid == parent_uuid {
            return false;
        }

        // Prevent cycles.
        if self.is_ancestor_of(child_uuid, parent_uuid) {
            return false;
        }

        // Compute world transform before reparenting.
        let world_mat = if preserve_world_transform {
            Some(self.get_world_transform(child))
        } else {
            None
        };

        // Detach from current parent.
        self.detach_from_parent_impl(child, child_uuid, false);

        // Add to new parent.
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent) {
            rel.children.push(child_uuid);
        }
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(child) {
            rel.parent = Some(parent_uuid);
        }

        // Adjust local transform to preserve world position.
        if let Some(world_mat) = world_mat {
            let parent_world = self.get_world_transform(parent);
            let local = parent_world.inverse() * world_mat;
            self.decompose_and_set_local_transform(child, local);
        }

        // Warn if physics entity is parented.
        if self.has_component::<RigidBody2DComponent>(child) {
            log::warn!(
                "Entity with RigidBody2D parented — physics may not behave correctly"
            );
        }

        true
    }

    /// Remove an entity from its parent, making it a root entity.
    ///
    /// If `preserve_world_transform` is `true`, the local transform is adjusted
    /// so the entity's world position stays the same.
    pub fn detach_from_parent(&mut self, entity: Entity, preserve_world_transform: bool) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        self.detach_from_parent_impl(entity, uuid, preserve_world_transform);
    }

    fn detach_from_parent_impl(
        &mut self,
        entity: Entity,
        entity_uuid: u64,
        preserve_world_transform: bool,
    ) {
        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        let Some(parent_uuid) = parent_uuid else {
            return;
        };

        // Compute world transform before detaching.
        let world_mat = if preserve_world_transform {
            Some(self.get_world_transform(entity))
        } else {
            None
        };

        // Remove from parent's children list.
        if let Some(parent_entity) = self.find_entity_by_uuid(parent_uuid) {
            if let Some(mut rel) =
                self.get_component_mut::<RelationshipComponent>(parent_entity)
            {
                rel.children.retain(|&c| c != entity_uuid);
            }
        }

        // Clear parent reference.
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(entity) {
            rel.parent = None;
        }

        // Restore world transform.
        if let Some(world_mat) = world_mat {
            self.decompose_and_set_local_transform(entity, world_mat);
        }
    }

    /// Compute the world-space transform for an entity by walking the parent chain.
    ///
    /// No caching — walks up from entity to root each call. Fine for scenes with
    /// O(100s) of entities and hierarchy depth ~3.
    pub fn get_world_transform(&self, entity: Entity) -> glam::Mat4 {
        let local = self
            .get_component::<TransformComponent>(entity)
            .map(|tc| tc.get_transform())
            .unwrap_or(glam::Mat4::IDENTITY);

        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        match parent_uuid {
            Some(puuid) => {
                if let Some(parent_entity) = self.find_entity_by_uuid(puuid) {
                    self.get_world_transform(parent_entity) * local
                } else {
                    local
                }
            }
            None => local,
        }
    }

    /// Get the children UUIDs of an entity.
    pub fn get_children(&self, entity: Entity) -> Vec<u64> {
        self.get_component::<RelationshipComponent>(entity)
            .map(|r| r.children.clone())
            .unwrap_or_default()
    }

    /// Get the parent UUID of an entity.
    pub fn get_parent(&self, entity: Entity) -> Option<u64> {
        self.get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent)
    }

    /// Return all root entities (entities without a parent), sorted by entity ID.
    pub fn root_entities(&self) -> Vec<(Entity, String)> {
        let mut entities: Vec<(Entity, String)> = self
            .world
            .query::<(hecs::Entity, &TagComponent, &RelationshipComponent)>()
            .iter()
            .filter(|(_, _, rel)| rel.parent.is_none())
            .map(|(handle, tag, _)| (Entity::new(handle), tag.tag.clone()))
            .collect();
        entities.sort_by_key(|(e, _)| e.id());
        entities
    }

    /// Check if `ancestor_uuid` is an ancestor of `entity_uuid`.
    ///
    /// Used for cycle detection in [`set_parent`](Self::set_parent).
    pub fn is_ancestor_of(&self, ancestor_uuid: u64, entity_uuid: u64) -> bool {
        let mut current = entity_uuid;
        for _ in 0..100 {
            // depth limit prevents infinite loops
            if let Some(entity) = self.find_entity_by_uuid(current) {
                if let Some(parent) = self.get_parent(entity) {
                    if parent == ancestor_uuid {
                        return true;
                    }
                    current = parent;
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
        false
    }

    /// Decompose a 4x4 matrix into translation/rotation/scale and set on the entity.
    fn decompose_and_set_local_transform(&mut self, entity: Entity, mat: glam::Mat4) {
        let (scale, rotation, translation) = mat.to_scale_rotation_translation();
        let (rx, ry, rz) = rotation.to_euler(glam::EulerRot::XYZ);
        if let Some(mut tc) = self.get_component_mut::<TransformComponent>(entity) {
            tc.translation = translation;
            tc.rotation = glam::Vec3::new(rx, ry, rz);
            tc.scale = scale;
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
    // Animation
    // -----------------------------------------------------------------

    /// Advance all [`SpriteAnimatorComponent`] timers by `dt`.
    ///
    /// Call this each frame before rendering (in both play mode and editor
    /// preview). This only updates the animator state — rendering uses the
    /// current frame to compute UV coordinates.
    pub fn on_update_animations(&mut self, dt: f32) {
        for animator in self.world.query_mut::<&mut SpriteAnimatorComponent>() {
            animator.update(dt);
        }
    }

    // -----------------------------------------------------------------
    // Texture loading
    // -----------------------------------------------------------------

    /// Resolve texture handles for all sprite entities.
    ///
    /// Scans every [`SpriteRendererComponent`] with a non-zero `texture_handle`
    /// and no loaded texture. For each, ensures the asset is loaded via the
    /// asset manager and assigns the GPU texture to the component.
    ///
    /// Call this after deserializing a scene and before the first render.
    pub fn resolve_texture_handles(
        &mut self,
        asset_manager: &mut crate::asset::EditorAssetManager,
        renderer: &Renderer,
    ) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::resolve_texture_handles");

        // Phase 1: collect entities that need texture resolution.
        let needs_resolve: Vec<(hecs::Entity, crate::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &SpriteRendererComponent)>()
            .iter()
            .filter_map(|(handle, sprite)| {
                if sprite.texture_handle.raw() != 0 && sprite.texture.is_none() {
                    Some((handle, sprite.texture_handle))
                } else {
                    None
                }
            })
            .collect();

        // Phase 2: load assets and assign textures.
        for (handle, asset_handle) in needs_resolve {
            asset_manager.load_asset(&asset_handle, renderer);
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut sprite) = self.world.get::<&mut SpriteRendererComponent>(handle) {
                    sprite.texture = Some(texture);
                }
            }
        }

        // Phase 3: resolve tilemap textures.
        let tilemap_needs: Vec<(hecs::Entity, crate::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &TilemapComponent)>()
            .iter()
            .filter_map(|(handle, tilemap)| {
                if tilemap.texture_handle.raw() != 0 && tilemap.texture.is_none() {
                    Some((handle, tilemap.texture_handle))
                } else {
                    None
                }
            })
            .collect();

        for (handle, asset_handle) in tilemap_needs {
            asset_manager.load_asset(&asset_handle, renderer);
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut tilemap) = self.world.get::<&mut TilemapComponent>(handle) {
                    tilemap.texture = Some(texture);
                }
            }
        }
    }

    /// Load fonts for all [`TextComponent`]s that have a `font_path` set
    /// but no loaded font. Similar to [`resolve_texture_handles`](Self::resolve_texture_handles).
    pub fn load_fonts(&mut self, renderer: &Renderer) {
        use std::collections::HashMap;
        use std::path::PathBuf;
        let _timer = crate::profiling::ProfileTimer::new("Scene::load_fonts");

        let loads: Vec<(hecs::Entity, PathBuf)> = self
            .world
            .query::<(hecs::Entity, &TextComponent)>()
            .iter()
            .filter_map(|(handle, tc)| {
                if tc.font.is_none() && !tc.font_path.is_empty() {
                    let path = PathBuf::from(&tc.font_path);
                    if path.exists() {
                        Some((handle, path))
                    } else {
                        log::warn!("Font not found: {}", tc.font_path);
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        let mut cache: HashMap<PathBuf, crate::Ref<Font>> = HashMap::new();
        for (handle, path) in loads {
            let font = cache.entry(path.clone())
                .or_insert_with(|| crate::Ref::new(renderer.create_font(&path)))
                .clone();
            if let Ok(mut tc) = self.world.get::<&mut TextComponent>(handle) {
                tc.font = Some(font);
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_physics_2d_start");
        let mut physics = PhysicsWorld2D::new(0.0, -9.81);

        // Snapshot entities with RigidBody2DComponent to avoid borrow conflicts.
        let body_entities: Vec<(hecs::Entity, u64, glam::Vec3, glam::Vec3, glam::Vec3, RigidBody2DType, bool)> = self
            .world
            .query::<(
                hecs::Entity,
                &IdComponent,
                &TransformComponent,
                &RigidBody2DComponent,
            )>()
            .iter()
            .map(|(handle, id, transform, rb)| {
                (
                    handle,
                    id.id.raw(),
                    transform.translation,
                    transform.rotation,
                    transform.scale,
                    rb.body_type,
                    rb.fixed_rotation,
                )
            })
            .collect();

        for (handle, entity_uuid, translation, rotation, scale, body_type, fixed_rotation) in body_entities {
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
                    .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS)
                    .build();

                let collider_handle =
                    physics
                        .colliders
                        .insert_with_parent(collider, body_handle, &mut physics.bodies);
                bc.runtime_fixture = Some(collider_handle);
                physics.register_collider(collider_handle, entity_uuid);
            }

            // If entity also has a CircleCollider2DComponent, create a collider.
            if let Ok(mut cc) = self.world.get::<&mut CircleCollider2DComponent>(handle) {
                let scaled_radius = cc.radius * scale.x.abs();

                let collider = rapier2d::geometry::ColliderBuilder::ball(scaled_radius)
                    .density(cc.density)
                    .friction(cc.friction)
                    .restitution(cc.restitution)
                    .translation(na::Vector2::new(cc.offset.x, cc.offset.y))
                    .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS)
                    .build();

                let collider_handle =
                    physics
                        .colliders
                        .insert_with_parent(collider, body_handle, &mut physics.bodies);
                cc.runtime_fixture = Some(collider_handle);
                physics.register_collider(collider_handle, entity_uuid);
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
    // Audio lifecycle
    // -----------------------------------------------------------------

    /// Create the audio engine and play sounds with `play_on_start`.
    fn on_audio_start(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_audio_start");
        let engine = match audio::AudioEngine::new() {
            Some(e) => e,
            None => return,
        };
        self.audio_engine = Some(engine);

        // Collect entities that should auto-play.
        let auto_play: Vec<(u64, String, f32, f32, bool)> = self
            .world
            .query::<(hecs::Entity, &IdComponent, &AudioSourceComponent)>()
            .iter()
            .filter(|(_, _, asc)| asc.play_on_start && asc.resolved_path.is_some())
            .map(|(_, id, asc)| {
                (
                    id.id.raw(),
                    asc.resolved_path.clone().unwrap(),
                    asc.volume,
                    asc.pitch,
                    asc.looping,
                )
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, path, volume, pitch, looping) in auto_play {
                engine.play_sound(uuid, &path, volume, pitch, looping);
            }
        }
    }

    /// Stop all sounds and drop the audio engine.
    fn on_audio_stop(&mut self) {
        if let Some(ref mut engine) = self.audio_engine {
            engine.stop_all();
        }
        self.audio_engine = None;
    }

    /// Resolve audio handles to file paths via the asset manager.
    pub fn resolve_audio_handles(
        &mut self,
        asset_manager: &mut crate::asset::EditorAssetManager,
    ) {
        let needs_resolve: Vec<(hecs::Entity, crate::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &AudioSourceComponent)>()
            .iter()
            .filter_map(|(handle, asc)| {
                if asc.audio_handle.raw() != 0 && asc.resolved_path.is_none() {
                    Some((handle, asc.audio_handle))
                } else {
                    None
                }
            })
            .collect();

        for (handle, asset_handle) in needs_resolve {
            if let Some(abs_path) = asset_manager.get_absolute_path(&asset_handle) {
                if abs_path.exists() {
                    let path_str = abs_path.to_string_lossy().to_string();
                    if let Ok(mut asc) = self.world.get::<&mut AudioSourceComponent>(handle) {
                        asc.resolved_path = Some(path_str);
                    }
                }
            }
        }
    }

    /// Play audio for an entity (used by Lua scripts).
    pub fn play_entity_sound(&mut self, entity: Entity) {
        let (uuid, path, volume, pitch, looping) = {
            let id = match self.get_component::<IdComponent>(entity) {
                Some(id) => id.id.raw(),
                None => return,
            };
            let asc = match self.get_component::<AudioSourceComponent>(entity) {
                Some(a) => a,
                None => return,
            };
            let path = match &asc.resolved_path {
                Some(p) => p.clone(),
                None => return,
            };
            (id, path, asc.volume, asc.pitch, asc.looping)
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.play_sound(uuid, &path, volume, pitch, looping);
        }
    }

    /// Stop audio for an entity (used by Lua scripts).
    pub fn stop_entity_sound(&mut self, entity: Entity) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.stop_sound(uuid);
        }
    }

    /// Set audio volume for an entity (used by Lua scripts).
    pub fn set_entity_volume(&mut self, entity: Entity, volume: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_volume(uuid, volume);
        }
    }

    // -----------------------------------------------------------------
    // Physics scripting API (used by both native + Lua scripts)
    // -----------------------------------------------------------------

    /// Apply a linear impulse to the entity's rigid body.
    ///
    /// No-op if the physics world is inactive (edit mode) or the entity
    /// lacks a [`RigidBody2DComponent`] with a valid runtime body.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse(na::Vector2::new(impulse.x, impulse.y), true);
            }
        }
    }

    /// Apply a linear impulse at a world-space point on the entity's rigid body.
    ///
    /// This can produce both translational and rotational motion depending on
    /// the point relative to the body's center of mass.
    pub fn apply_impulse_at_point(
        &mut self,
        entity: Entity,
        impulse: glam::Vec2,
        point: glam::Vec2,
    ) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.apply_impulse_at_point(
                    na::Vector2::new(impulse.x, impulse.y),
                    na::Point2::new(point.x, point.y),
                    true,
                );
            }
        }
    }

    /// Apply a continuous force to the entity's rigid body.
    ///
    /// Unlike impulses, forces are accumulated and applied during the next
    /// physics step. Call every frame for sustained acceleration.
    pub fn apply_force(&mut self, entity: Entity, force: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.add_force(na::Vector2::new(force.x, force.y), true);
            }
        }
    }

    /// Get the linear velocity of the entity's rigid body.
    ///
    /// Returns `None` if the physics world is inactive or the entity lacks
    /// a runtime rigid body.
    pub fn get_linear_velocity(&self, entity: Entity) -> Option<glam::Vec2> {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world) {
            if let Some(body) = physics.bodies.get(handle) {
                let v = body.linvel();
                return Some(glam::Vec2::new(v.x, v.y));
            }
        }
        None
    }

    /// Set the linear velocity of the entity's rigid body.
    pub fn set_linear_velocity(&mut self, entity: Entity, vel: glam::Vec2) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_linvel(na::Vector2::new(vel.x, vel.y), true);
            }
        }
    }

    /// Get the angular velocity (radians/sec) of the entity's rigid body.
    ///
    /// Returns `None` if the physics world is inactive or the entity lacks
    /// a runtime rigid body.
    pub fn get_angular_velocity(&self, entity: Entity) -> Option<f32> {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref physics)) = (body_handle, &self.physics_world) {
            if let Some(body) = physics.bodies.get(handle) {
                return Some(body.angvel());
            }
        }
        None
    }

    /// Set the angular velocity (radians/sec) of the entity's rigid body.
    pub fn set_angular_velocity(&mut self, entity: Entity, omega: f32) {
        let body_handle = self
            .get_component::<RigidBody2DComponent>(entity)
            .and_then(|rb| rb.runtime_body);
        if let (Some(handle), Some(ref mut physics)) = (body_handle, &mut self.physics_world) {
            if let Some(body) = physics.bodies.get_mut(handle) {
                body.set_angvel(omega, true);
            }
        }
    }

    // -----------------------------------------------------------------
    // Runtime lifecycle
    // -----------------------------------------------------------------

    /// Initialize physics and scripting for runtime (play mode).
    ///
    /// Call this when entering play mode (before the first physics step).
    pub fn on_runtime_start(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_runtime_start");
        self.on_physics_2d_start();
        #[cfg(feature = "lua-scripting")]
        self.on_lua_scripting_start();
        self.on_audio_start();
    }

    /// Tear down physics, scripting, and audio for runtime (play mode).
    ///
    /// Call this when exiting play mode (before restoring the snapshot).
    pub fn on_runtime_stop(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_runtime_stop");
        self.on_audio_stop();
        self.on_native_scripting_stop();
        #[cfg(feature = "lua-scripting")]
        self.on_lua_scripting_stop();
        self.on_physics_2d_stop();
    }

    /// Call `on_destroy` on all active NativeScript instances and reset them.
    fn on_native_scripting_stop(&mut self) {
        // Collect handles for all entities with a NativeScriptComponent that has an active instance.
        let script_entities: Vec<hecs::Entity> = self
            .world
            .query::<(hecs::Entity, &NativeScriptComponent)>()
            .iter()
            .filter(|(_, nsc)| nsc.instance.is_some())
            .map(|(handle, _)| handle)
            .collect();

        for handle in script_entities {
            let entity = Entity::new(handle);
            // Take the instance out so on_destroy can access &mut self.
            let instance = {
                let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) else {
                    continue;
                };
                let Some(inst) = nsc.instance.take() else {
                    continue;
                };
                nsc.created = false;
                inst
            };

            let mut instance = instance;
            instance.on_destroy(entity, self);
            // Instance is dropped here — not put back.
        }
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
    /// When `input` is `Some`, Lua `on_fixed_update(dt)` callbacks are
    /// interleaved with physics steps (play mode). When `None`, only physics
    /// is stepped (simulate mode).
    ///
    /// Per fixed step: `on_fixed_update(FIXED_DT)` → `snapshot` → `step_once`.
    /// After the loop, interpolated transforms are written back for smooth
    /// rendering.
    pub fn on_update_physics(&mut self, dt: Timestep, input: Option<&Input>) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_physics");

        // Accumulate frame dt.
        let Some(physics) = self.physics_world.as_mut() else {
            return;
        };
        physics.accumulate(dt.seconds());
        let fixed_dt = physics.fixed_timestep();

        // Fixed-step loop: scripts apply impulses → snapshot → rapier step.
        loop {
            if !self.physics_world.as_ref().unwrap().can_step() {
                break;
            }

            // Run Lua fixed-update scripts so impulses/forces are applied
            // at the physics rate, not the render rate.
            #[cfg(feature = "lua-scripting")]
            if let Some(inp) = input {
                self.call_lua_fixed_update(fixed_dt, inp);
            }

            let physics = self.physics_world.as_mut().unwrap();
            physics.snapshot_transforms();
            physics.step_once();

            // Dispatch collision events to Lua scripts.
            #[cfg(feature = "lua-scripting")]
            if input.is_some() {
                self.dispatch_collision_events();
            }
        }

        let physics = self.physics_world.as_ref().unwrap();
        let alpha = physics.alpha();

        // Write back interpolated transforms for smooth rendering.
        for (transform, rb) in self
            .world
            .query_mut::<(&mut TransformComponent, &RigidBody2DComponent)>()
        {
            if let Some(body_handle) = rb.runtime_body {
                if let Some(body) = physics.bodies.get(body_handle) {
                    let cur_pos = body.translation();
                    let cur_angle = body.rotation().angle();

                    if let Some((prev_x, prev_y, prev_angle)) =
                        physics.prev_transform(body_handle)
                    {
                        transform.translation.x =
                            prev_x + (cur_pos.x - prev_x) * alpha;
                        transform.translation.y =
                            prev_y + (cur_pos.y - prev_y) * alpha;
                        // Shortest-path angle interpolation to avoid
                        // flipping through the wrong direction on wrap.
                        let mut angle_diff = cur_angle - prev_angle;
                        angle_diff = angle_diff - (angle_diff / std::f32::consts::TAU).round() * std::f32::consts::TAU;
                        transform.rotation.z = prev_angle + angle_diff * alpha;
                    } else {
                        // First frame — no previous, use current directly.
                        transform.translation.x = cur_pos.x;
                        transform.translation.y = cur_pos.y;
                        transform.rotation.z = cur_angle;
                    }
                }
            }
        }
    }

    /// Call `on_fixed_update(dt)` on all loaded Lua scripts.
    ///
    /// Uses the same take-modify-replace pattern as `on_update_lua_scripts`.
    #[cfg(feature = "lua-scripting")]
    fn call_lua_fixed_update(&mut self, fixed_dt: f32, input: &Input) {
        use script_glue::SceneScriptContext;

        let uuids: Vec<u64> = self
            .world
            .query::<(&IdComponent, &LuaScriptComponent)>()
            .iter()
            .filter(|(_, lsc)| lsc.loaded)
            .map(|(id, _)| id.id.raw())
            .collect();

        if uuids.is_empty() {
            return;
        }

        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let ctx = SceneScriptContext {
            scene: self as *mut Scene,
            input: input as *const Input,
            script_engine: &engine as *const ScriptEngine,
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_fixed_update(*uuid, fixed_dt);
        }

        engine.lua().remove_app_data::<SceneScriptContext>();
        self.script_engine = Some(engine);

        // Flush deferred entity destructions from fixed-update scripts.
        self.flush_pending_destroys();
    }

    /// Drain collision events from the physics world and dispatch to Lua scripts.
    ///
    /// Calls `on_collision_enter(other_uuid)` and `on_collision_exit(other_uuid)`
    /// on both entities in each collision pair.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_collision_events(&mut self) {
        use script_glue::SceneScriptContext;

        // Drain events from physics.
        let events: Vec<(u64, u64, bool)> = match self.physics_world.as_ref() {
            Some(physics) => physics.drain_collision_events(),
            None => return,
        };

        if events.is_empty() {
            return;
        }

        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let ctx = SceneScriptContext {
            scene: self as *mut Scene,
            input: std::ptr::null(),
            script_engine: &engine as *const ScriptEngine,
        };
        engine.lua().set_app_data(ctx);

        for (uuid_a, uuid_b, started) in &events {
            let callback = if *started {
                "on_collision_enter"
            } else {
                "on_collision_exit"
            };
            // Notify both entities.
            engine.call_entity_collision(*uuid_a, callback, *uuid_b);
            engine.call_entity_collision(*uuid_b, callback, *uuid_a);
        }

        engine.lua().remove_app_data::<SceneScriptContext>();
        self.script_engine = Some(engine);
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_scripts");
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::render_scene");
        // Draw sprites (with optional animation).
        for (handle, sprite) in self
            .world
            .query::<(
                hecs::Entity,
                &SpriteRendererComponent,
            )>()
            .iter()
        {
            let world_transform = self.get_world_transform(Entity::new(handle));

            // Check if this entity has an active animator.
            let animated = self
                .world
                .get::<&SpriteAnimatorComponent>(handle)
                .ok()
                .and_then(|anim| {
                    let (col, row) = anim.current_grid_coords()?;
                    let texture = sprite.texture.as_ref()?;
                    Some(SubTexture2D::from_coords(
                        texture,
                        glam::Vec2::new(col as f32, row as f32),
                        anim.cell_size,
                        glam::Vec2::ONE,
                    ))
                });

            if let Some(sub_tex) = animated {
                renderer.draw_sub_textured_quad_transformed(
                    &world_transform,
                    &sub_tex,
                    sprite.color,
                    handle.id() as i32,
                );
            } else {
                renderer.draw_sprite(
                    &world_transform,
                    sprite,
                    handle.id() as i32,
                );
            }
        }

        // Draw circles.
        for (handle, circle) in self
            .world
            .query::<(
                hecs::Entity,
                &CircleRendererComponent,
            )>()
            .iter()
        {
            let world_transform = self.get_world_transform(Entity::new(handle));
            renderer.draw_circle_component(
                &world_transform,
                circle,
                handle.id() as i32,
            );
        }

        // Draw text.
        for (handle, text) in self
            .world
            .query::<(
                hecs::Entity,
                &TextComponent,
            )>()
            .iter()
        {
            let world_transform = self.get_world_transform(Entity::new(handle));
            renderer.draw_text_component(
                &world_transform,
                text,
                handle.id() as i32,
            );
        }

        // Draw tilemaps.
        for (handle, tilemap) in self
            .world
            .query::<(
                hecs::Entity,
                &TilemapComponent,
            )>()
            .iter()
        {
            let texture = match tilemap.texture.as_ref() {
                Some(tex) => tex,
                None => continue,
            };
            let entity_world = self.get_world_transform(Entity::new(handle));
            let cols = tilemap.tileset_columns.max(1);
            let tw = texture.width() as f32;
            let th = texture.height() as f32;
            for row in 0..tilemap.height {
                for col in 0..tilemap.width {
                    let raw = tilemap.tiles[(row * tilemap.width + col) as usize];
                    if raw < 0 {
                        continue;
                    }
                    let flip_h = raw & TILE_FLIP_H != 0;
                    let flip_v = raw & TILE_FLIP_V != 0;
                    let tile_id = raw & TILE_ID_MASK;

                    let tex_col = (tile_id as u32) % cols;
                    let tex_row = (tile_id as u32) / cols;

                    // UV calculation with spacing and margin.
                    let px = tilemap.margin.x + tex_col as f32 * (tilemap.cell_size.x + tilemap.spacing.x);
                    let py = tilemap.margin.y + tex_row as f32 * (tilemap.cell_size.y + tilemap.spacing.y);
                    let mut min_uv = glam::Vec2::new(px / tw, py / th);
                    let mut max_uv = glam::Vec2::new((px + tilemap.cell_size.x) / tw, (py + tilemap.cell_size.y) / th);

                    if flip_h { std::mem::swap(&mut min_uv.x, &mut max_uv.x); }
                    if flip_v { std::mem::swap(&mut min_uv.y, &mut max_uv.y); }

                    let sub_tex = SubTexture2D::new(texture, min_uv, max_uv);
                    let tile_transform = entity_world
                        * glam::Mat4::from_scale_rotation_translation(
                            glam::Vec3::new(tilemap.tile_size.x, tilemap.tile_size.y, 1.0),
                            glam::Quat::IDENTITY,
                            glam::Vec3::new(
                                col as f32 * tilemap.tile_size.x,
                                row as f32 * tilemap.tile_size.y,
                                0.0,
                            ),
                        );
                    renderer.draw_sub_textured_quad_transformed(
                        &tile_transform,
                        &sub_tex,
                        glam::Vec4::ONE,
                        handle.id() as i32,
                    );
                }
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_runtime");
        // Find the primary camera entity.
        let mut main_camera_vp: Option<glam::Mat4> = None;
        for (handle, camera) in self
            .world
            .query::<(hecs::Entity, &CameraComponent)>()
            .iter()
        {
            if camera.primary {
                // VP = projection * inverse(camera_world_transform)
                let world = self.get_world_transform(Entity::new(handle));
                main_camera_vp =
                    Some(*camera.camera.projection() * world.inverse());
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_editor");
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

    // -----------------------------------------------------------------
    // Lua Scripting lifecycle
    // -----------------------------------------------------------------

    /// Create the Lua script engine, set up per-entity environments, and
    /// call `on_create()` for each entity with a [`LuaScriptComponent`].
    #[cfg(feature = "lua-scripting")]
    fn on_lua_scripting_start(&mut self) {
        use script_glue::SceneScriptContext;
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_lua_scripting_start");

        let mut engine = ScriptEngine::new();

        // Collect entities with non-empty script paths (avoid borrow conflicts).
        let scripts: Vec<(hecs::Entity, u64, String)> = self
            .world
            .query::<(hecs::Entity, &IdComponent, &LuaScriptComponent)>()
            .iter()
            .filter(|(_, _, lsc)| !lsc.script_path.is_empty())
            .map(|(handle, id, lsc)| (handle, id.id.raw(), lsc.script_path.clone()))
            .collect();

        // Create per-entity environments, apply field overrides, and mark loaded.
        for (handle, uuid, path) in &scripts {
            if engine.create_entity_env(*uuid, path) {
                // Apply editor field overrides before on_create.
                if let Ok(lsc) = self.world.get::<&LuaScriptComponent>(*handle) {
                    for (name, value) in &lsc.field_overrides {
                        engine.set_entity_field(*uuid, name, value);
                    }
                }
                if let Ok(mut lsc) = self.world.get::<&mut LuaScriptComponent>(*handle) {
                    lsc.loaded = true;
                }
            }
        }

        // Store engine in self, then take it out for on_create calls
        // (callbacks need scene pointer via app_data).
        self.script_engine = Some(engine);
        let engine = self.script_engine.take().unwrap();

        // Set scene context (no input during on_create).
        let ctx = SceneScriptContext {
            scene: self as *mut Scene,
            input: std::ptr::null(),
            script_engine: &engine as *const ScriptEngine,
        };
        engine.lua().set_app_data(ctx);

        // Call on_create for each entity.
        let uuids: Vec<u64> = scripts.iter().map(|(_, uuid, _)| *uuid).collect();
        for uuid in &uuids {
            engine.call_entity_on_create(*uuid);
        }

        // Clear context and put engine back.
        engine.lua().remove_app_data::<SceneScriptContext>();
        self.script_engine = Some(engine);
    }

    /// Tear down the Lua script engine: call `on_destroy()` per entity,
    /// drop the engine, and reset loaded flags.
    #[cfg(feature = "lua-scripting")]
    fn on_lua_scripting_stop(&mut self) {
        use script_glue::SceneScriptContext;
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_lua_scripting_stop");

        if let Some(engine) = self.script_engine.take() {
            // Set scene context (no input during on_destroy).
            let ctx = SceneScriptContext {
                scene: self as *mut Scene,
                input: std::ptr::null(),
                script_engine: &engine as *const ScriptEngine,
            };
            engine.lua().set_app_data(ctx);

            // Call on_destroy for each entity.
            let uuids = engine.entity_uuids();
            for uuid in &uuids {
                engine.call_entity_on_destroy(*uuid);
            }

            // Clear context — engine is dropped after this block.
            engine.lua().remove_app_data::<SceneScriptContext>();
        }

        // Reset loaded flags.
        for lsc in self.world.query_mut::<&mut LuaScriptComponent>() {
            lsc.loaded = false;
        }
    }

    /// Reload all Lua scripts from disk without leaving play mode.
    ///
    /// **Play mode**: tears down the current script engine (calling `on_destroy`
    /// for each entity), creates a fresh LuaJIT state, re-loads every script
    /// file from disk, re-applies editor field overrides, and calls `on_create`.
    /// This lets you edit a `.lua` file and see changes immediately without
    /// stopping and restarting.
    ///
    /// **Edit mode**: no-op (scripts are loaded fresh on each play-mode entry).
    #[cfg(feature = "lua-scripting")]
    pub fn reload_lua_scripts(&mut self) {
        if self.script_engine.is_none() {
            log::info!("Scripts will reload on next play (no active script engine)");
            return;
        }

        log::info!("Reloading Lua scripts...");

        // Tear down the running engine (calls on_destroy, resets loaded flags).
        self.on_lua_scripting_stop();

        // Spin up a fresh engine and re-load everything from disk.
        self.on_lua_scripting_start();

        log::info!("Lua scripts reloaded");
    }

    /// Call per-entity `on_update(dt)` for all loaded Lua scripts.
    ///
    /// Call this each frame during play mode, passing the current [`Input`]
    /// so scripts can query key state via `Engine.is_key_down()`.
    #[cfg(feature = "lua-scripting")]
    pub fn on_update_lua_scripts(&mut self, dt: Timestep, input: &Input) {
        use script_glue::SceneScriptContext;
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_lua_scripts");

        // Collect UUIDs of loaded script entities.
        let uuids: Vec<u64> = self
            .world
            .query::<(&IdComponent, &LuaScriptComponent)>()
            .iter()
            .filter(|(_, lsc)| lsc.loaded)
            .map(|(id, _)| id.id.raw())
            .collect();

        if uuids.is_empty() {
            return;
        }

        // Take engine out (take-modify-replace pattern).
        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        // Set scene + input context.
        let ctx = SceneScriptContext {
            scene: self as *mut Scene,
            input: input as *const Input,
            script_engine: &engine as *const ScriptEngine,
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_update(*uuid, dt.seconds());
        }

        // Clear context and put engine back.
        engine.lua().remove_app_data::<SceneScriptContext>();
        self.script_engine = Some(engine);

        // Flush deferred entity destructions.
        self.flush_pending_destroys();
    }

    /// Access the script engine (if active).
    #[cfg(feature = "lua-scripting")]
    pub fn script_engine(&self) -> Option<&ScriptEngine> {
        self.script_engine.as_ref()
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

/// Clone a single component from `src` entity to `dst` entity within the same scene.
/// The hecs::Ref borrow is released (via `.map()`) before mutating the world.
fn duplicate_component_if_has<T: hecs::Component + Clone>(
    scene: &mut Scene,
    src: Entity,
    dst: Entity,
) {
    let cloned = scene.get_component::<T>(src).map(|r| T::clone(&r));
    if let Some(comp) = cloned {
        scene.add_component(dst, comp);
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

    // --- Physics scripting API tests ---

    #[test]
    fn apply_impulse_noop_without_physics() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, RigidBody2DComponent::default());
        // No physics world active — should not panic.
        scene.apply_impulse(e, glam::Vec2::new(1.0, 0.0));
    }

    #[test]
    fn get_linear_velocity_returns_none_without_physics() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, RigidBody2DComponent::default());
        assert!(scene.get_linear_velocity(e).is_none());
    }

    #[test]
    fn get_linear_velocity_returns_none_without_rb() {
        let scene = Scene::new();
        // Entity without RigidBody2DComponent — even create_entity not called,
        // so use a dummy entity from a fresh scene.
        let mut s = Scene::new();
        let e = s.create_entity();
        assert!(s.get_linear_velocity(e).is_none());
        let _ = scene;
    }

    #[test]
    fn get_angular_velocity_returns_none_without_physics() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, RigidBody2DComponent::default());
        assert!(scene.get_angular_velocity(e).is_none());
    }

    // --- Deferred destroy tests ---

    #[test]
    fn queue_and_flush_destroys_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("ToDestroy");
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        assert_eq!(scene.entity_count(), 1);

        scene.queue_entity_destroy(uuid);
        // Not destroyed yet.
        assert_eq!(scene.entity_count(), 1);

        scene.flush_pending_destroys();
        assert_eq!(scene.entity_count(), 0);
    }

    #[test]
    fn flush_deduplicates() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Dup");
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();

        scene.queue_entity_destroy(uuid);
        scene.queue_entity_destroy(uuid);
        scene.queue_entity_destroy(uuid);

        scene.flush_pending_destroys();
        assert_eq!(scene.entity_count(), 0);
    }

    #[test]
    fn flush_nonexistent_uuid_is_noop() {
        let mut scene = Scene::new();
        scene.create_entity();
        scene.queue_entity_destroy(99999);
        scene.flush_pending_destroys();
        assert_eq!(scene.entity_count(), 1);
    }

    #[test]
    fn flush_empty_queue_is_noop() {
        let mut scene = Scene::new();
        scene.create_entity();
        scene.flush_pending_destroys();
        assert_eq!(scene.entity_count(), 1);
    }
}
