pub mod animation;
pub(crate) mod audio;
mod components;
mod entity;
pub mod native_script;
mod physics_2d;
mod scene_serializer;
#[cfg(feature = "lua-scripting")]
pub(crate) mod script_engine;
#[cfg(feature = "lua-scripting")]
mod script_glue;

use crate::renderer::Font;

pub use animation::{AnimationClip, InstancedSpriteAnimator, SpriteAnimatorComponent};
#[cfg(feature = "lua-scripting")]
pub use components::LuaScriptComponent;
pub use components::{
    AudioListenerComponent, AudioSourceComponent, BoxCollider2DComponent, CameraComponent,
    CircleCollider2DComponent, CircleRendererComponent, IdComponent, NativeScriptComponent,
    ParticleEmitterComponent, RelationshipComponent, RigidBody2DComponent, RigidBody2DType,
    SpriteRendererComponent, TagComponent, TextComponent, TilemapComponent, TransformComponent,
    TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
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

/// Clamp a physics property to a minimum, logging a warning if it was invalid.
fn validate_physics_value(value: f32, min: f32, name: &str, entity_uuid: u64) -> f32 {
    if value < min {
        log::warn!(
            "Entity {}: negative {} ({:.3}), clamped to {}",
            entity_uuid,
            name,
            value,
            min
        );
        min
    } else {
        value
    }
}

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
    /// Lazy name → UUID cache for `find_entity_by_name`. Built on first call,
    /// invalidated on entity create/destroy. Only stores first match per name.
    name_cache: Option<HashMap<String, u64>>,
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
            AudioListenerComponent,
            ParticleEmitterComponent,
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
            name_cache: None,
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
        if let Some(_old_handle) = self.uuid_cache.insert(uuid.raw(), handle) {
            log::warn!(
                "UUID collision: entity with UUID {} already existed in the scene — overwriting",
                uuid.raw()
            );
        }
        self.name_cache = None; // invalidate
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
                if let Ok(mut rel) = self
                    .world
                    .get::<&mut RelationshipComponent>(parent_entity.handle())
                {
                    rel.children.retain(|&c| c != my_uuid);
                }
            }
        }

        // Remove from UUID cache and invalidate name cache.
        if let Some(u) = uuid {
            self.uuid_cache.remove(&u);
            self.name_cache = None;
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
                    // Clean up collider-to-UUID mappings before removing the body,
                    // since rapier will also remove attached colliders internally.
                    if let Some(body) = physics.bodies.get(handle) {
                        for &collider_handle in body.colliders() {
                            physics.collider_to_uuid.remove(&collider_handle);
                        }
                    }
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
    ///
    /// **Note:** If multiple entities share the same name, which one is
    /// returned is arbitrary (depends on hecs iteration order, which is not
    /// guaranteed to be stable). Callers that need deterministic results
    /// should ensure entity names are unique or use UUID-based lookup instead.
    pub fn find_entity_by_name(&mut self, name: &str) -> Option<(Entity, u64)> {
        // Build the name cache lazily on first call.
        if self.name_cache.is_none() {
            let mut cache = HashMap::new();
            for (tag, id) in self.world.query::<(&TagComponent, &IdComponent)>().iter() {
                // First entity registered per name wins (matches old linear scan).
                cache.entry(tag.tag.clone()).or_insert(id.id.raw());
            }
            self.name_cache = Some(cache);
        }

        let uuid = *self.name_cache.as_ref().unwrap().get(name)?;
        let entity = self.find_entity_by_uuid(uuid)?;
        Some((entity, uuid))
    }

    /// Find an entity by its UUID (from [`IdComponent`]).
    ///
    /// O(1) lookup via internal cache maintained on entity create/destroy.
    pub fn find_entity_by_uuid(&self, uuid: u64) -> Option<Entity> {
        self.uuid_cache.get(&uuid).copied().map(Entity::new)
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
            log::warn!("Entity with RigidBody2D parented — physics may not behave correctly");
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
            if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent_entity) {
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

    /// Compute the world transform for `entity`, using and populating `cache`.
    ///
    /// Same logic as [`get_world_transform`](Self::get_world_transform) but
    /// avoids redundant parent-chain walks when many entities share ancestors.
    fn get_world_transform_cached(
        &self,
        entity: Entity,
        cache: &mut HashMap<hecs::Entity, glam::Mat4>,
    ) -> glam::Mat4 {
        if let Some(&cached) = cache.get(&entity.handle()) {
            return cached;
        }

        let local = self
            .get_component::<TransformComponent>(entity)
            .map(|tc| tc.get_transform())
            .unwrap_or(glam::Mat4::IDENTITY);

        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        let world = match parent_uuid {
            Some(puuid) => {
                if let Some(parent_entity) = self.find_entity_by_uuid(puuid) {
                    self.get_world_transform_cached(parent_entity, cache) * local
                } else {
                    local
                }
            }
            None => local,
        };

        cache.insert(entity.handle(), world);
        world
    }

    /// Build a cache of world transforms for all entities.
    ///
    /// Call once per frame before rendering to avoid redundant parent-chain
    /// walks (O(n) total instead of O(n·d) where d is hierarchy depth).
    fn build_world_transform_cache(&self) -> HashMap<hecs::Entity, glam::Mat4> {
        let mut cache = HashMap::with_capacity(self.world.len() as usize);
        let entities: Vec<hecs::Entity> = self.world.query::<hecs::Entity>().iter().collect();
        for handle in entities {
            self.get_world_transform_cached(Entity::new(handle), &mut cache);
        }
        cache
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

    /// Move a child entity to a specific index within its parent's children list.
    ///
    /// No-op if the entity has no parent or the UUID is not found in the children.
    pub fn reorder_child(&mut self, child_uuid: u64, new_index: usize) {
        let Some(child_entity) = self.find_entity_by_uuid(child_uuid) else {
            return;
        };
        let parent_uuid = match self.get_parent(child_entity) {
            Some(p) => p,
            None => return,
        };
        let Some(parent_entity) = self.find_entity_by_uuid(parent_uuid) else {
            return;
        };
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent_entity) {
            let Some(current_pos) = rel.children.iter().position(|&c| c == child_uuid) else {
                return;
            };
            rel.children.remove(current_pos);
            let clamped = new_index.min(rel.children.len());
            rel.children.insert(clamped, child_uuid);
        }
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
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current) {
                // Cycle detected — treat as not an ancestor.
                return false;
            }
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
    ///
    /// After updating, dispatches `on_animation_finished(clip_name)` Lua
    /// callbacks for any non-looping clips that just ended, then transitions
    /// to the default clip if one is configured.
    pub fn on_update_animations(&mut self, dt: f32) {
        // Phase 1: tick all animators, collect finished events.
        let mut finished_events: Vec<(u64, String, String)> = Vec::new();
        for (id_comp, animator) in self
            .world
            .query_mut::<(&IdComponent, &mut SpriteAnimatorComponent)>()
        {
            animator.update(dt);
            if let Some(clip_name) = animator.finished_clip_name.take() {
                finished_events.push((id_comp.id.raw(), clip_name, animator.default_clip.clone()));
            }
        }

        if finished_events.is_empty() {
            return;
        }

        // Phase 2: dispatch Lua callbacks.
        #[cfg(feature = "lua-scripting")]
        self.dispatch_animation_finished_events(&finished_events);

        // Phase 3: transition to default clip for entities that have one.
        for (uuid, _, default_clip) in &finished_events {
            if default_clip.is_empty() {
                continue;
            }
            if let Some(entity) = self.find_entity_by_uuid(*uuid) {
                if let Some(mut animator) =
                    self.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    animator.play(default_clip);
                }
            }
        }
    }

    /// Advance animations only for entities with `previewing` set (editor preview).
    pub fn on_update_animation_previews(&mut self, dt: f32) {
        for animator in self.world.query_mut::<&mut SpriteAnimatorComponent>() {
            if animator.previewing {
                animator.update(dt);
            }
        }
    }

    /// Dispatch `on_animation_finished(clip_name)` Lua callbacks.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_animation_finished_events(&mut self, events: &[(u64, String, String)]) {
        use script_glue::SceneScriptContext;

        let mut engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let scene_ptr: *mut Scene = self;

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        engine.lua().set_app_data(ctx);

        for (uuid, clip_name, _) in events {
            engine.call_entity_callback_str(*uuid, "on_animation_finished", clip_name.clone());
        }

        engine.lua().remove_app_data::<SceneScriptContext>();

        unsafe {
            (*scene_ptr).script_engine = Some(engine);
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

        // Phase 4: resolve per-clip animator textures.
        self.resolve_animator_clip_textures(asset_manager, Some(renderer));
    }

    /// Async variant of [`resolve_texture_handles`](Self::resolve_texture_handles).
    ///
    /// For entities with unresolved texture handles:
    /// - If the texture is already loaded in the asset manager, assigns it immediately.
    /// - Otherwise, requests an async background load (non-blocking).
    ///
    /// On subsequent frames, `poll_loaded` will upload completed textures,
    /// and this method will find them in the cache and assign them.
    pub fn resolve_texture_handles_async(
        &mut self,
        asset_manager: &mut crate::asset::EditorAssetManager,
    ) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::resolve_texture_handles_async");

        // Phase 1: sprites.
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

        for (handle, asset_handle) in needs_resolve {
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut sprite) = self.world.get::<&mut SpriteRendererComponent>(handle) {
                    sprite.texture = Some(texture);
                }
            } else {
                asset_manager.request_load(&asset_handle);
            }
        }

        // Phase 2: tilemaps.
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
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut tilemap) = self.world.get::<&mut TilemapComponent>(handle) {
                    tilemap.texture = Some(texture);
                }
            } else {
                asset_manager.request_load(&asset_handle);
            }
        }

        // Phase 3: resolve per-clip animator textures.
        self.resolve_animator_clip_textures(asset_manager, None);
    }

    /// Resolve per-clip texture handles in all [`SpriteAnimatorComponent`]s.
    ///
    /// If `renderer` is `Some`, uses synchronous `load_asset`; otherwise
    /// uses `request_load` for async loading.
    fn resolve_animator_clip_textures(
        &mut self,
        asset_manager: &mut crate::asset::EditorAssetManager,
        renderer: Option<&Renderer>,
    ) {
        // Collect (entity, clip_index, handle) for clips needing resolution.
        let needs: Vec<(hecs::Entity, usize, crate::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &SpriteAnimatorComponent)>()
            .iter()
            .flat_map(|(entity, animator)| {
                animator
                    .clips
                    .iter()
                    .enumerate()
                    .filter(|(_, clip)| clip.texture_handle.raw() != 0 && clip.texture.is_none())
                    .map(move |(i, clip)| (entity, i, clip.texture_handle))
            })
            .collect();

        for (entity, clip_idx, asset_handle) in needs {
            if let Some(r) = renderer {
                asset_manager.load_asset(&asset_handle, r);
            }
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut animator) = self.world.get::<&mut SpriteAnimatorComponent>(entity) {
                    if let Some(clip) = animator.clips.get_mut(clip_idx) {
                        clip.texture = Some(texture);
                    }
                }
            } else if renderer.is_none() {
                asset_manager.request_load(&asset_handle);
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
            if let Some(font) = cache.get(&path).cloned().or_else(|| {
                let f = crate::Ref::new(renderer.create_font(&path)?);
                cache.insert(path.clone(), f.clone());
                Some(f)
            }) {
                if let Ok(mut tc) = self.world.get::<&mut TextComponent>(handle) {
                    tc.font = Some(font);
                }
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
        // Skip parented entities — physics bodies ignore parent transforms, so
        // allowing them would cause confusing mismatches between visual and physics position.
        let body_entities: Vec<(hecs::Entity, u64, glam::Vec3, glam::Vec3, glam::Vec3, RigidBody2DType, bool)> = self
            .world
            .query::<(
                hecs::Entity,
                &IdComponent,
                &TransformComponent,
                &RigidBody2DComponent,
                &RelationshipComponent,
            )>()
            .iter()
            .filter_map(|(handle, id, transform, rb, rel)| {
                if rel.parent.is_some() {
                    log::warn!(
                        "Entity UUID {} has RigidBody2D but is parented — skipping physics body creation. \
                         Detach from parent or remove the RigidBody2D component.",
                        id.id.raw(),
                    );
                    return None;
                }
                Some((
                    handle,
                    id.id.raw(),
                    transform.translation,
                    transform.rotation,
                    transform.scale,
                    rb.body_type,
                    rb.fixed_rotation,
                ))
            })
            .collect();

        for (handle, entity_uuid, translation, rotation, scale, body_type, fixed_rotation) in
            body_entities
        {
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

                if half_x <= 0.0 || half_y <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-size box collider ({} x {}), skipping",
                        entity_uuid,
                        half_x * 2.0,
                        half_y * 2.0
                    );
                } else {
                    let density = validate_physics_value(bc.density, 0.0, "density", entity_uuid);
                    let friction =
                        validate_physics_value(bc.friction, 0.0, "friction", entity_uuid);
                    let restitution =
                        validate_physics_value(bc.restitution, 0.0, "restitution", entity_uuid);

                    let mut builder = rapier2d::geometry::ColliderBuilder::cuboid(half_x, half_y)
                        .density(density)
                        .friction(friction)
                        .restitution(restitution)
                        .translation(na::Vector2::new(
                            bc.offset.x * scale.x.abs(),
                            bc.offset.y * scale.y.abs(),
                        ))
                        .collision_groups(rapier2d::geometry::InteractionGroups::new(
                            bc.collision_layer.into(),
                            bc.collision_mask.into(),
                        ))
                        .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS);
                    // When friction is 0, use Min combine rule so the zero
                    // wins against any surface (prevents wall sticking).
                    if friction == 0.0 {
                        builder = builder
                            .friction_combine_rule(rapier2d::prelude::CoefficientCombineRule::Min);
                    }
                    let collider = builder.build();

                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    bc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, entity_uuid);
                }
            }

            // If entity also has a CircleCollider2DComponent, create a collider.
            if let Ok(mut cc) = self.world.get::<&mut CircleCollider2DComponent>(handle) {
                let scaled_radius = cc.radius * scale.x.abs().max(scale.y.abs());

                if scaled_radius <= 0.0 {
                    log::warn!(
                        "Entity {} has zero-radius circle collider, skipping",
                        entity_uuid
                    );
                } else {
                    let density = validate_physics_value(cc.density, 0.0, "density", entity_uuid);
                    let friction =
                        validate_physics_value(cc.friction, 0.0, "friction", entity_uuid);
                    let restitution =
                        validate_physics_value(cc.restitution, 0.0, "restitution", entity_uuid);

                    let mut builder = rapier2d::geometry::ColliderBuilder::ball(scaled_radius)
                        .density(density)
                        .friction(friction)
                        .restitution(restitution)
                        .translation(na::Vector2::new(
                            cc.offset.x * scale.x.abs(),
                            cc.offset.y * scale.y.abs(),
                        ))
                        .collision_groups(rapier2d::geometry::InteractionGroups::new(
                            cc.collision_layer.into(),
                            cc.collision_mask.into(),
                        ))
                        .active_events(rapier2d::prelude::ActiveEvents::COLLISION_EVENTS);
                    if friction == 0.0 {
                        builder = builder
                            .friction_combine_rule(rapier2d::prelude::CoefficientCombineRule::Min);
                    }
                    let collider = builder.build();

                    let collider_handle = physics.colliders.insert_with_parent(
                        collider,
                        body_handle,
                        &mut physics.bodies,
                    );
                    cc.runtime_fixture = Some(collider_handle);
                    physics.register_collider(collider_handle, entity_uuid);
                }
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
        let auto_play: Vec<(u64, String, f32, f32, bool, bool)> = self
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
                    asc.streaming,
                )
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, path, volume, pitch, looping, streaming) in auto_play {
                engine.play_sound(uuid, &path, volume, pitch, looping, streaming);
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
    pub fn resolve_audio_handles(&mut self, asset_manager: &mut crate::asset::EditorAssetManager) {
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

    /// Find all entities that reference the given asset handle.
    ///
    /// Scans `SpriteRendererComponent::texture_handle`,
    /// `TilemapComponent::texture_handle`, and
    /// `AudioSourceComponent::audio_handle`.
    ///
    /// Returns a list of `(entity_name, component_kind)` pairs describing
    /// each reference, e.g. `("Player", "Sprite")`.
    pub fn find_asset_references(
        &self,
        asset_handle: crate::uuid::Uuid,
    ) -> Vec<(String, &'static str)> {
        let mut refs = Vec::new();

        for (tag, sprite) in self
            .world
            .query::<(&TagComponent, &SpriteRendererComponent)>()
            .iter()
        {
            if sprite.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "Sprite"));
            }
        }

        for (tag, tilemap) in self
            .world
            .query::<(&TagComponent, &TilemapComponent)>()
            .iter()
        {
            if tilemap.texture_handle == asset_handle {
                refs.push((tag.tag.clone(), "Tilemap"));
            }
        }

        for (tag, asc) in self
            .world
            .query::<(&TagComponent, &AudioSourceComponent)>()
            .iter()
        {
            if asc.audio_handle == asset_handle {
                refs.push((tag.tag.clone(), "Audio"));
            }
        }

        refs
    }

    /// Play audio for an entity (used by Lua scripts).
    pub fn play_entity_sound(&mut self, entity: Entity) {
        let (uuid, path, volume, pitch, looping, streaming) = {
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
            (id, path, asc.volume, asc.pitch, asc.looping, asc.streaming)
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.play_sound(uuid, &path, volume, pitch, looping, streaming);
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

    /// Set panning for an entity (used by Lua scripts).
    /// -1.0 = hard left, 0.0 = center, 1.0 = hard right.
    pub fn set_entity_panning(&mut self, entity: Entity, panning: f32) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        if let Some(ref mut engine) = self.audio_engine {
            engine.set_panning(uuid, panning);
        }
    }

    /// Update spatial audio: compute panning and distance attenuation for
    /// all spatial audio sources based on the listener position.
    ///
    /// If an entity has an active [`AudioListenerComponent`], its position is
    /// used as the listener. Otherwise, the primary camera position is used.
    pub fn update_spatial_audio(&mut self) {
        if self.audio_engine.is_none() {
            return;
        }

        // Prefer explicit AudioListenerComponent, fall back to primary camera.
        let listener_pos = self
            .world
            .query::<(&AudioListenerComponent, &TransformComponent)>()
            .iter()
            .filter(|(al, _)| al.active)
            .map(|(_, tf)| tf.translation.truncate())
            .last()
            .or_else(|| {
                self.world
                    .query::<(&CameraComponent, &TransformComponent)>()
                    .iter()
                    .filter(|(cam, _)| cam.primary)
                    .map(|(_, tf)| tf.translation.truncate())
                    .last()
            })
            .unwrap_or(glam::Vec2::ZERO);

        // Collect spatial updates (uuid, panning, effective_volume).
        let updates: Vec<(u64, f32, f32)> = self
            .world
            .query::<(&IdComponent, &AudioSourceComponent, &TransformComponent)>()
            .iter()
            .filter(|(_, asc, _)| asc.spatial)
            .map(|(id, asc, tf)| {
                let entity_pos = tf.translation.truncate();
                let delta = entity_pos - listener_pos;
                let dist = delta.length();
                // Panning: proportional to horizontal offset relative to max_distance.
                let panning = (delta.x / asc.max_distance.max(0.01)).clamp(-1.0, 1.0);
                // Attenuation: linear falloff between min and max distance.
                // Expressed in decibels: 0 dB at min_distance, -60 dB (silence) at max_distance.
                let atten_db = if dist <= asc.min_distance {
                    0.0
                } else if dist >= asc.max_distance {
                    -60.0
                } else {
                    let t = (dist - asc.min_distance) / (asc.max_distance - asc.min_distance);
                    -60.0 * t
                };
                // Combine: component volume (dB-ish) + distance attenuation.
                let effective_volume = asc.volume + atten_db;
                (id.id.raw(), panning, effective_volume)
            })
            .collect();

        if let Some(ref mut engine) = self.audio_engine {
            for (uuid, panning, volume) in updates {
                engine.set_panning(uuid, panning);
                engine.set_volume(uuid, volume);
            }
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

    /// Cast a ray and return the first hit: `(entity_uuid, hit_x, hit_y, normal_x, normal_y, toi)`.
    ///
    /// `exclude_entity` optionally filters out a specific entity (e.g. the caster).
    pub fn raycast(
        &self,
        origin: glam::Vec2,
        direction: glam::Vec2,
        max_toi: f32,
        exclude_entity: Option<Entity>,
    ) -> Option<(u64, f32, f32, f32, f32, f32)> {
        use rapier2d::na;
        let exclude_uuid =
            exclude_entity.and_then(|e| self.get_component::<IdComponent>(e).map(|id| id.id.raw()));
        if let Some(ref physics) = self.physics_world {
            physics.raycast(
                na::Point2::new(origin.x, origin.y),
                na::Vector2::new(direction.x, direction.y),
                max_toi,
                exclude_uuid,
            )
        } else {
            None
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
    /// When `input` is `Some`, script `on_fixed_update(dt)` callbacks (both
    /// Lua and native) are interleaved with physics steps (play mode). When
    /// `None`, only physics is stepped (simulate mode).
    ///
    /// Per fixed step: `reset_forces` → `on_fixed_update` → `snapshot` → `step_once`.
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

        // Fixed-step loop: clear forces → scripts apply forces → snapshot → rapier step.
        loop {
            if !self.physics_world.as_ref().unwrap().can_step() {
                break;
            }

            // Clear accumulated forces before scripts add new ones.
            // rapier 0.22 does NOT auto-clear forces after step().
            self.physics_world.as_mut().unwrap().reset_all_forces();

            // Run fixed-update scripts so impulses/forces are applied
            // at the physics rate, not the render rate.
            if let Some(inp) = input {
                #[cfg(feature = "lua-scripting")]
                self.call_lua_fixed_update(fixed_dt, inp);
                self.run_native_fixed_update(Timestep::from_seconds(fixed_dt), inp);
            }

            let physics = self.physics_world.as_mut().unwrap();
            physics.snapshot_transforms();
            physics.step_once();

            // Dispatch collision events to Lua scripts.
            #[cfg(feature = "lua-scripting")]
            if input.is_some() {
                self.dispatch_collision_events();
                self.flush_pending_destroys();
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

                    if let Some((prev_x, prev_y, prev_angle)) = physics.prev_transform(body_handle)
                    {
                        transform.translation.x = prev_x + (cur_pos.x - prev_x) * alpha;
                        transform.translation.y = prev_y + (cur_pos.y - prev_y) * alpha;
                        // Shortest-path angle interpolation to avoid
                        // flipping through the wrong direction on wrap.
                        let mut angle_diff = cur_angle - prev_angle;
                        angle_diff = angle_diff
                            - (angle_diff / std::f32::consts::TAU).round() * std::f32::consts::TAU;
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

        // --- Setup phase (uses &mut self) ---
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

        let mut engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        // SAFETY: Convert &mut self to raw pointer before Lua dispatch.
        // Lua callbacks dereference ctx.scene to access Scene (ECS, physics, etc.).
        // Under Rust's aliasing model, using &mut self after callbacks write through
        // *mut Scene is UB. By switching to the raw pointer and never using `self`
        // again, we ensure a single provenance chain for all Scene accesses.
        // The ScriptEngine is taken out of Scene, so &mut engine does not alias.
        let scene_ptr: *mut Scene = self;

        // --- Dispatch phase (uses scene_ptr, never self) ---
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_fixed_update(*uuid, fixed_dt);
        }

        engine.lua().remove_app_data::<SceneScriptContext>();

        unsafe {
            (*scene_ptr).script_engine = Some(engine);
            (*scene_ptr).flush_pending_destroys();
        }
    }

    /// Drain collision events from the physics world and dispatch to Lua scripts.
    ///
    /// Calls `on_collision_enter(other_uuid)` and `on_collision_exit(other_uuid)`
    /// on both entities in each collision pair.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_collision_events(&mut self) {
        use script_glue::SceneScriptContext;

        // --- Setup phase (uses &mut self) ---
        // Drain events from physics.
        let events: Vec<(u64, u64, bool)> = match self.physics_world.as_ref() {
            Some(physics) => physics.drain_collision_events(),
            None => return,
        };

        if events.is_empty() {
            return;
        }

        let mut engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        // SAFETY: Convert &mut self to raw pointer before Lua dispatch.
        // Lua callbacks dereference ctx.scene to access Scene (ECS, physics, etc.).
        // Under Rust's aliasing model, using &mut self after callbacks write through
        // *mut Scene is UB. By switching to the raw pointer and never using `self`
        // again, we ensure a single provenance chain for all Scene accesses.
        // The ScriptEngine is taken out of Scene, so &mut engine does not alias.
        let scene_ptr: *mut Scene = self;

        // --- Dispatch phase (uses scene_ptr, never self) ---
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
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

        unsafe {
            (*scene_ptr).script_engine = Some(engine);
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

    /// Run `on_fixed_update` on all [`NativeScriptComponent`] instances.
    ///
    /// Called inside the physics step loop at the fixed physics rate.
    /// Uses the same take-modify-replace pattern as [`on_update_scripts`].
    fn run_native_fixed_update(&mut self, dt: Timestep, input: &Input) {
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
            instance.on_fixed_update(entity, self, dt, input);

            if let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) {
                nsc.instance = Some(instance);
            }
        }
    }

    /// Draw all renderable entities sorted by (sorting_layer, order_in_layer, z).
    ///
    /// Shared rendering code used by editor, simulation, and runtime paths.
    /// The caller is responsible for setting the view-projection matrix on
    /// the renderer before calling this.
    fn render_scene(&self, renderer: &mut Renderer) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::render_scene");
        // Pre-compute world transforms for all entities once.
        let wt_cache = {
            crate::profile_scope!("Scene::build_world_transform_cache");
            self.build_world_transform_cache()
        };

        // Collect all renderable entities with sort keys.
        // 0 = Sprite, 1 = Circle, 2 = Text, 3 = Tilemap
        let mut renderables: Vec<(i32, i32, f32, u8, hecs::Entity)> = Vec::new();

        for (handle, sprite) in self
            .world
            .query::<(hecs::Entity, &SpriteRendererComponent)>()
            .iter()
        {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push((sprite.sorting_layer, sprite.order_in_layer, z, 0, handle));
        }

        for (handle, circle) in self
            .world
            .query::<(hecs::Entity, &CircleRendererComponent)>()
            .iter()
        {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push((circle.sorting_layer, circle.order_in_layer, z, 1, handle));
        }

        for (handle, text) in self.world.query::<(hecs::Entity, &TextComponent)>().iter() {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push((text.sorting_layer, text.order_in_layer, z, 2, handle));
        }

        for (handle, tilemap) in self
            .world
            .query::<(hecs::Entity, &TilemapComponent)>()
            .iter()
        {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push((tilemap.sorting_layer, tilemap.order_in_layer, z, 3, handle));
        }

        // Sort by (sorting_layer, order_in_layer, z).
        renderables.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        });

        // Precompute inverse VP for tilemap frustum culling.
        let vp_inv = renderer.view_projection().inverse();

        // Render in sorted order.
        // Flush all pending batches when the renderable type changes so that
        // cross-type draw ordering (e.g. text behind a sprite) is respected.
        let mut prev_kind: u8 = u8::MAX;
        for &(_, _, _, kind, handle) in &renderables {
            if kind != prev_kind {
                renderer.flush_all_batches();
                prev_kind = kind;
            }
            let world_transform = wt_cache
                .get(&handle)
                .copied()
                .unwrap_or(glam::Mat4::IDENTITY);
            match kind {
                0 => {
                    // Sprite
                    let sprite = self.world.get::<&SpriteRendererComponent>(handle).unwrap();
                    let animated = self
                        .world
                        .get::<&SpriteAnimatorComponent>(handle)
                        .ok()
                        .and_then(|anim| {
                            let (col, row) = anim.current_grid_coords()?;
                            // Per-clip texture takes priority over the sprite's texture.
                            let texture =
                                anim.current_clip_texture().or(sprite.texture.as_ref())?;
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
                    } else if sprite.is_atlas() {
                        if let Some(ref tex) = sprite.texture {
                            let sub_tex =
                                SubTexture2D::new(tex, sprite.atlas_min, sprite.atlas_max);
                            renderer.draw_sub_textured_quad_transformed(
                                &world_transform,
                                &sub_tex,
                                sprite.color,
                                handle.id() as i32,
                            );
                        } else {
                            renderer.draw_sprite(&world_transform, &sprite, handle.id() as i32);
                        }
                    } else {
                        renderer.draw_sprite(&world_transform, &sprite, handle.id() as i32);
                    }
                }
                1 => {
                    // Circle
                    let circle = self.world.get::<&CircleRendererComponent>(handle).unwrap();
                    renderer.draw_circle_component(&world_transform, &circle, handle.id() as i32);
                }
                2 => {
                    // Text
                    let text = self.world.get::<&TextComponent>(handle).unwrap();
                    renderer.draw_text_component(&world_transform, &text, handle.id() as i32);
                }
                3 => {
                    // Tilemap — frustum culled + precomputed transforms.
                    let tilemap = self.world.get::<&TilemapComponent>(handle).unwrap();
                    let texture = match tilemap.texture.as_ref() {
                        Some(tex) => tex.clone(),
                        None => continue,
                    };
                    let tile_cols = tilemap.tileset_columns.max(1);
                    let tw = texture.width() as f32;
                    let th = texture.height() as f32;
                    if tw == 0.0 || th == 0.0 {
                        continue;
                    }
                    let tile_size = tilemap.tile_size;
                    if tile_size.x <= 0.0 || tile_size.y <= 0.0 {
                        continue;
                    }
                    let tex_idx = texture.bindless_index() as f32;
                    let eid = handle.id() as i32;

                    // Precompute UV constants.
                    let inv_tw = 1.0 / tw;
                    let inv_th = 1.0 / th;
                    let cell_w = tilemap.cell_size.x;
                    let cell_h = tilemap.cell_size.y;
                    let margin_x = tilemap.margin.x;
                    let margin_y = tilemap.margin.y;
                    let step_x = cell_w + tilemap.spacing.x;
                    let step_y = cell_h + tilemap.spacing.y;

                    // --- Frustum culling: visible tile range ---
                    let ndc_to_local = world_transform.inverse() * vp_inv;
                    let mut local_min = glam::Vec2::splat(f32::INFINITY);
                    let mut local_max = glam::Vec2::splat(f32::NEG_INFINITY);
                    for ndc in [
                        glam::Vec3::new(-1.0, -1.0, 0.0),
                        glam::Vec3::new(1.0, -1.0, 0.0),
                        glam::Vec3::new(1.0, 1.0, 0.0),
                        glam::Vec3::new(-1.0, 1.0, 0.0),
                    ] {
                        let p = ndc_to_local.project_point3(ndc);
                        local_min = local_min.min(p.truncate());
                        local_max = local_max.max(p.truncate());
                    }
                    let w = tilemap.width as f32;
                    let h = tilemap.height as f32;
                    let (min_col, max_col, min_row, max_row) =
                        if local_min.is_finite() && local_max.is_finite() {
                            (
                                ((local_min.x / tile_size.x).floor() - 1.0).clamp(0.0, w) as u32,
                                ((local_max.x / tile_size.x).ceil() + 1.0).clamp(0.0, w) as u32,
                                ((local_min.y / tile_size.y).floor() - 1.0).clamp(0.0, h) as u32,
                                ((local_max.y / tile_size.y).ceil() + 1.0).clamp(0.0, h) as u32,
                            )
                        } else {
                            // Degenerate transform — render all tiles.
                            (0, tilemap.width, 0, tilemap.height)
                        };

                    // --- Precomputed transform columns ---
                    // tile_transform columns 0-2 are constant; only col3 varies.
                    let scaled_x = world_transform.x_axis * tile_size.x;
                    let scaled_y = world_transform.y_axis * tile_size.y;
                    let const_col2 = world_transform.z_axis;
                    let base_w = world_transform.w_axis;

                    for row in min_row..max_row {
                        let row_w = base_w + row as f32 * scaled_y;
                        for col in min_col..max_col {
                            let raw = tilemap.tiles[(row * tilemap.width + col) as usize];
                            if raw < 0 {
                                continue;
                            }
                            let flip_h = raw & TILE_FLIP_H != 0;
                            let flip_v = raw & TILE_FLIP_V != 0;
                            let tile_id = raw & TILE_ID_MASK;

                            let tex_col = (tile_id as u32) % tile_cols;
                            let tex_row = (tile_id as u32) / tile_cols;
                            let px = margin_x + tex_col as f32 * step_x;
                            let py = margin_y + tex_row as f32 * step_y;
                            let mut min_u = px * inv_tw;
                            let mut min_v = py * inv_th;
                            let mut max_u = (px + cell_w) * inv_tw;
                            let mut max_v = (py + cell_h) * inv_th;

                            if flip_h {
                                std::mem::swap(&mut min_u, &mut max_u);
                            }
                            if flip_v {
                                std::mem::swap(&mut min_v, &mut max_v);
                            }

                            let col3 = row_w + col as f32 * scaled_x;
                            let tile_transform =
                                glam::Mat4::from_cols(scaled_x, scaled_y, const_col2, col3);
                            renderer.draw_textured_quad_transformed_uv(
                                &tile_transform,
                                tex_idx,
                                [min_u, min_v],
                                [max_u, max_v],
                                glam::Vec4::ONE,
                                eid,
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        // Emit and render GPU particles from all active ParticleEmitterComponents.
        self.emit_and_render_particles(renderer);
    }

    /// Emit particles from all active [`ParticleEmitterComponent`]s and
    /// render the GPU particle system. The GPU particle system is lazily
    /// created on the first emitter encountered.
    fn emit_and_render_particles(&self, renderer: &mut Renderer) {
        let mut any_emitter = false;
        for (pe, tf) in self
            .world
            .query::<(&ParticleEmitterComponent, &TransformComponent)>()
            .iter()
        {
            if !pe.playing || pe.emit_rate == 0 {
                continue;
            }
            // Lazily initialize the GPU particle system on first use.
            if !any_emitter {
                if !renderer.has_gpu_particle_system() {
                    if let Err(e) = renderer.create_gpu_particle_system(pe.max_particles) {
                        log::error!("Failed to create GPU particle system: {e}");
                        return;
                    }
                }
                any_emitter = true;
            }
            let props = crate::particle_system::ParticleProps {
                position: tf.translation.truncate(),
                velocity: pe.velocity,
                velocity_variation: pe.velocity_variation,
                color_begin: pe.color_begin,
                color_end: pe.color_end,
                size_begin: pe.size_begin,
                size_end: pe.size_end,
                size_variation: pe.size_variation,
                lifetime: pe.lifetime,
            };
            for _ in 0..pe.emit_rate {
                renderer.emit_particles(&props);
            }
        }
        if any_emitter {
            renderer.render_gpu_particles();
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
                main_camera_vp = Some(*camera.camera.projection() * world.inverse());
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

        // --- Setup phase (uses &mut self) ---
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
            } else if let Ok(mut lsc) = self.world.get::<&mut LuaScriptComponent>(*handle) {
                lsc.load_failed = true;
            }
        }

        let uuids: Vec<u64> = scripts.iter().map(|(_, uuid, _)| *uuid).collect();

        // SAFETY: Convert &mut self to raw pointer before Lua dispatch.
        // Lua callbacks dereference ctx.scene to access Scene (ECS, physics, etc.).
        // Under Rust's aliasing model, using &mut self after callbacks write through
        // *mut Scene is UB. By switching to the raw pointer and never using `self`
        // again, we ensure a single provenance chain for all Scene accesses.
        // The ScriptEngine is taken out of Scene, so &mut engine does not alias.
        let scene_ptr: *mut Scene = self;

        // --- Dispatch phase (uses scene_ptr, never self) ---
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_create(*uuid);
        }

        engine.lua().remove_app_data::<SceneScriptContext>();

        unsafe {
            (*scene_ptr).script_engine = Some(engine);
        }
    }

    /// Tear down the Lua script engine: call `on_destroy()` per entity,
    /// drop the engine, and reset loaded flags.
    #[cfg(feature = "lua-scripting")]
    fn on_lua_scripting_stop(&mut self) {
        use script_glue::SceneScriptContext;
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_lua_scripting_stop");

        // --- Setup phase (uses &mut self) ---
        let mut engine = match self.script_engine.take() {
            Some(e) => e,
            None => {
                // No engine — just reset loaded/failed flags.
                for lsc in self.world.query_mut::<&mut LuaScriptComponent>() {
                    lsc.loaded = false;
                    lsc.load_failed = false;
                }
                return;
            }
        };

        let uuids = engine.entity_uuids();

        // SAFETY: Convert &mut self to raw pointer before Lua dispatch.
        // Lua callbacks dereference ctx.scene to access Scene (ECS, physics, etc.).
        // Under Rust's aliasing model, using &mut self after callbacks write through
        // *mut Scene is UB. By switching to the raw pointer and never using `self`
        // again, we ensure a single provenance chain for all Scene accesses.
        // The ScriptEngine is taken out of Scene, so &mut engine does not alias.
        let scene_ptr: *mut Scene = self;

        // --- Dispatch phase (uses scene_ptr, never self) ---
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_destroy(*uuid);
        }

        // Clear context — engine is dropped after this block.
        engine.lua().remove_app_data::<SceneScriptContext>();

        // Reset loaded/failed flags via raw pointer.
        unsafe {
            for lsc in (*scene_ptr).world.query_mut::<&mut LuaScriptComponent>() {
                lsc.loaded = false;
                lsc.load_failed = false;
            }
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
    /// Also detects dynamically-spawned entities with unloaded scripts and
    /// initializes them (creates env, applies overrides, calls `on_create`).
    ///
    /// Call this each frame during play mode, passing the current [`Input`]
    /// so scripts can query key state via `Engine.is_key_down()`.
    #[cfg(feature = "lua-scripting")]
    pub fn on_update_lua_scripts(&mut self, dt: Timestep, input: &Input) {
        use script_glue::SceneScriptContext;
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_lua_scripts");

        // --- Setup phase (uses &mut self) ---
        // Take engine out (take-modify-replace pattern).
        let mut engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        // Check for dynamically-spawned entities with unloaded scripts.
        let unloaded: Vec<(hecs::Entity, u64, String)> = self
            .world
            .query::<(hecs::Entity, &IdComponent, &LuaScriptComponent)>()
            .iter()
            .filter(|(_, _, lsc)| !lsc.loaded && !lsc.load_failed && !lsc.script_path.is_empty())
            .map(|(handle, id, lsc)| (handle, id.id.raw(), lsc.script_path.clone()))
            .collect();

        if !unloaded.is_empty() {
            // Initialize newly-spawned scripts (no Lua dispatch yet).
            for (handle, uuid, path) in &unloaded {
                if engine.create_entity_env(*uuid, path) {
                    if let Ok(lsc) = self.world.get::<&LuaScriptComponent>(*handle) {
                        for (name, value) in &lsc.field_overrides {
                            engine.set_entity_field(*uuid, name, value);
                        }
                    }
                    if let Ok(mut lsc) = self.world.get::<&mut LuaScriptComponent>(*handle) {
                        lsc.loaded = true;
                    }
                } else if let Ok(mut lsc) = self.world.get::<&mut LuaScriptComponent>(*handle) {
                    lsc.load_failed = true;
                }
            }
        }

        // Collect UUIDs of loaded script entities (before switching to raw pointer).
        let uuids: Vec<u64> = self
            .world
            .query::<(&IdComponent, &LuaScriptComponent)>()
            .iter()
            .filter(|(_, lsc)| lsc.loaded)
            .map(|(id, _)| id.id.raw())
            .collect();

        // SAFETY: Convert &mut self to raw pointer before Lua dispatch.
        // Lua callbacks dereference ctx.scene to access Scene (ECS, physics, etc.).
        // Under Rust's aliasing model, using &mut self after callbacks write through
        // *mut Scene is UB. By switching to the raw pointer and never using `self`
        // again, we ensure a single provenance chain for all Scene accesses.
        // The ScriptEngine is taken out of Scene, so &mut engine does not alias.
        let scene_ptr: *mut Scene = self;

        // --- Dispatch phase (uses scene_ptr, never self) ---

        // Call on_create for newly loaded scripts.
        if !unloaded.is_empty() {
            let ctx = SceneScriptContext {
                scene: scene_ptr,
                input: input as *const Input,
            };
            engine.lua().set_app_data(ctx);

            for (_, uuid, _) in &unloaded {
                engine.call_entity_on_create(*uuid);
            }

            engine.lua().remove_app_data::<SceneScriptContext>();
        }

        if uuids.is_empty() {
            unsafe {
                (*scene_ptr).script_engine = Some(engine);
            }
            return;
        }

        // Set scene + input context for on_update.
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        engine.lua().set_app_data(ctx);

        for uuid in &uuids {
            engine.call_entity_on_update(*uuid, dt.seconds());
        }

        // Tick timers (set_timeout / set_interval).
        // Context must be active so timer callbacks can access the scene.
        let timer_ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        engine.lua().set_app_data(timer_ctx);
        engine.tick_timers(dt.seconds());
        engine.lua().remove_app_data::<SceneScriptContext>();

        unsafe {
            (*scene_ptr).script_engine = Some(engine);
            (*scene_ptr).flush_pending_destroys();
        }
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

    /// Compile-time guard: if you add or remove a component from
    /// `for_each_cloneable_component!`, update EXPECTED_COUNT here.
    /// This test prevents silent drift between the macro and the
    /// actual set of cloneable components.
    #[test]
    fn for_each_cloneable_component_count() {
        macro_rules! count_types {
            ($($t:ty),* $(,)?) => {
                const MACRO_COUNT: usize = 0 $(+ { let _ = std::mem::size_of::<$t>(); 1 })*;
            };
        }
        for_each_cloneable_component!(count_types);
        // Update this constant when adding or removing cloneable components.
        const EXPECTED_COUNT: usize = 14;
        assert_eq!(
            MACRO_COUNT, EXPECTED_COUNT,
            "for_each_cloneable_component! has {} types but expected {}. \
             Update EXPECTED_COUNT if you intentionally added/removed a component.",
            MACRO_COUNT, EXPECTED_COUNT
        );
    }

    #[test]
    fn validate_physics_value_clamps_negative() {
        // Negative → clamped to min.
        assert_eq!(super::validate_physics_value(-1.0, 0.0, "test", 0), 0.0);
        // Valid → unchanged.
        assert_eq!(super::validate_physics_value(0.5, 0.0, "test", 0), 0.5);
        // Zero → unchanged (not < min).
        assert_eq!(super::validate_physics_value(0.0, 0.0, "test", 0), 0.0);
    }

    #[test]
    fn find_asset_references_sprite() {
        let mut scene = Scene::new();
        let handle = crate::uuid::Uuid::from_raw(42);

        let e = scene.create_entity_with_tag("Player");
        scene.add_component(e, SpriteRendererComponent { texture_handle: handle, ..Default::default() });

        let refs = scene.find_asset_references(handle);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "Player");
        assert_eq!(refs[0].1, "Sprite");
    }

    #[test]
    fn find_asset_references_audio() {
        let mut scene = Scene::new();
        let handle = crate::uuid::Uuid::from_raw(99);

        let e = scene.create_entity_with_tag("BGM");
        scene.add_component(e, AudioSourceComponent { audio_handle: handle, ..Default::default() });

        let refs = scene.find_asset_references(handle);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "BGM");
        assert_eq!(refs[0].1, "Audio");
    }

    #[test]
    fn find_asset_references_tilemap() {
        let mut scene = Scene::new();
        let handle = crate::uuid::Uuid::from_raw(77);

        let e = scene.create_entity_with_tag("Level");
        scene.add_component(e, TilemapComponent { texture_handle: handle, ..Default::default() });

        let refs = scene.find_asset_references(handle);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "Level");
        assert_eq!(refs[0].1, "Tilemap");
    }

    #[test]
    fn find_asset_references_none_when_unused() {
        let mut scene = Scene::new();
        scene.create_entity_with_tag("Empty");
        let handle = crate::uuid::Uuid::from_raw(123);
        let refs = scene.find_asset_references(handle);
        assert!(refs.is_empty());
    }

    #[test]
    fn find_asset_references_multiple_entities() {
        let mut scene = Scene::new();
        let handle = crate::uuid::Uuid::from_raw(55);

        let e1 = scene.create_entity_with_tag("A");
        scene.add_component(e1, SpriteRendererComponent { texture_handle: handle, ..Default::default() });

        let e2 = scene.create_entity_with_tag("B");
        scene.add_component(e2, SpriteRendererComponent { texture_handle: handle, ..Default::default() });

        let refs = scene.find_asset_references(handle);
        assert_eq!(refs.len(), 2);
    }
}
