pub mod animation;
pub(crate) mod audio;
mod audio_ops;
mod components;
mod entity;
mod hierarchy;
#[cfg(feature = "lua-scripting")]
mod lua_ops;
pub mod native_script;
mod physics_2d;
mod physics_ops;
mod rendering;
mod runtime;
mod scene_serializer;
#[cfg(feature = "lua-scripting")]
pub(crate) mod script_engine;
#[cfg(feature = "lua-scripting")]
mod script_glue;
pub(crate) mod spatial;

pub use animation::{
    AnimationClip, AnimationControllerComponent, AnimationTransition, FloatOrdering,
    InstancedSpriteAnimator, SpriteAnimatorComponent, TransitionCondition,
};
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
pub use spatial::{Aabb2D, Frustum2D, SpatialGrid};

use crate::uuid::Uuid;

use std::cell::Cell;
use std::collections::HashMap;

use physics_2d::PhysicsWorld2D;

/// Per-frame frustum culling statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct CullingStats {
    /// Total renderable entities (sprites + circles) considered.
    pub total_cullable: u32,
    /// Entities that passed the frustum test (rendered).
    pub rendered: u32,
    /// Entities culled (skipped because they were off-screen).
    pub culled: u32,
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
    /// Monotonic scene time in seconds. Incremented each frame by `dt`.
    /// Used by [`InstancedSpriteAnimator`] for stateless frame computation.
    global_time: f64,
    /// Spatial grid for efficient 2D region queries.
    /// Rebuilt on demand via [`rebuild_spatial_grid`](Self::rebuild_spatial_grid).
    spatial_grid: Option<SpatialGrid>,
    /// Per-frame frustum culling statistics (interior-mutable, written by render_scene).
    culling_stats: Cell<CullingStats>,
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
            InstancedSpriteAnimator,
            AnimationControllerComponent,
            TilemapComponent,
            AudioSourceComponent,
            AudioListenerComponent,
            ParticleEmitterComponent,
        );
    };
}

/// Invokes `$callback!` with every component that can be added from the editor
/// "Add Component" popup. Each entry is `(Type, "Display Name")`.
///
/// When adding a new component type, add it here and it will automatically
/// appear in the editor's Add Component menu.
/// `LuaScriptComponent` is excluded (feature-gated — handled separately).
#[macro_export]
macro_rules! for_each_addable_component {
    ($callback:ident) => {
        $callback!(
            ($crate::scene::CameraComponent, "Camera"),
            ($crate::scene::SpriteRendererComponent, "Sprite Renderer"),
            ($crate::scene::CircleRendererComponent, "Circle Renderer"),
            ($crate::scene::SpriteAnimatorComponent, "Sprite Animator"),
            (
                $crate::scene::InstancedSpriteAnimator,
                "Instanced Sprite Animator"
            ),
            (
                $crate::scene::AnimationControllerComponent,
                "Animation Controller"
            ),
            ($crate::scene::TextComponent, "Text"),
            ($crate::scene::RigidBody2DComponent, "Rigidbody 2D"),
            ($crate::scene::BoxCollider2DComponent, "Box Collider 2D"),
            (
                $crate::scene::CircleCollider2DComponent,
                "Circle Collider 2D"
            ),
            ($crate::scene::TilemapComponent, "Tilemap"),
            ($crate::scene::AudioSourceComponent, "Audio Source"),
            ($crate::scene::AudioListenerComponent, "Audio Listener"),
            ($crate::scene::ParticleEmitterComponent, "Particle Emitter"),
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
            global_time: 0.0,
            spatial_grid: None,
            culling_stats: Cell::new(CullingStats::default()),
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
    // Spatial queries
    // -----------------------------------------------------------------

    /// Rebuild the spatial grid from current entity transforms.
    ///
    /// Call once per frame (during the update phase) to enable spatial
    /// queries via [`query_entities_in_region`](Self::query_entities_in_region)
    /// and [`query_entities_in_radius`](Self::query_entities_in_radius).
    ///
    /// `cell_size` controls the grid granularity in world units. Smaller
    /// cells give more precise culling but use more memory. A good default
    /// is 16.0 for typical 2D scenes.
    pub fn rebuild_spatial_grid(&mut self, cell_size: f32) {
        let wt_cache = self.build_world_transform_cache();
        let mut grid = SpatialGrid::new(cell_size);
        for (&handle, wt) in &wt_cache {
            let aabb = Aabb2D::from_unit_quad_transform(wt);
            grid.insert(handle, &aabb);
        }
        self.spatial_grid = Some(grid);
    }

    /// Query all entities whose AABB overlaps the given world-space region.
    ///
    /// Returns an empty Vec if [`rebuild_spatial_grid`](Self::rebuild_spatial_grid)
    /// has not been called.
    pub fn query_entities_in_region(&self, min: glam::Vec2, max: glam::Vec2) -> Vec<Entity> {
        let Some(ref grid) = self.spatial_grid else {
            return Vec::new();
        };
        let region = Aabb2D::new(min, max);
        grid.query_region_dedup(&region)
            .into_iter()
            .map(Entity::new)
            .collect()
    }

    /// Query all entities within `radius` world units of `center`.
    ///
    /// Uses the spatial grid for a broad-phase AABB query, then refines
    /// with an exact distance check against each entity's world position.
    ///
    /// Returns an empty Vec if [`rebuild_spatial_grid`](Self::rebuild_spatial_grid)
    /// has not been called.
    pub fn query_entities_in_radius(&self, center: glam::Vec2, radius: f32) -> Vec<Entity> {
        let Some(ref grid) = self.spatial_grid else {
            return Vec::new();
        };
        let region = Aabb2D::new(
            center - glam::Vec2::splat(radius),
            center + glam::Vec2::splat(radius),
        );
        let r2 = radius * radius;
        grid.query_region_dedup(&region)
            .into_iter()
            .filter(|&handle| {
                let entity = Entity::new(handle);
                let wt = self.get_world_transform(entity);
                let pos = glam::Vec2::new(wt.w_axis.x, wt.w_axis.y);
                (pos - center).length_squared() <= r2
            })
            .map(Entity::new)
            .collect()
    }

    /// Returns a reference to the spatial grid, if built.
    pub fn spatial_grid(&self) -> Option<&SpatialGrid> {
        self.spatial_grid.as_ref()
    }

    /// Returns the frustum culling statistics from the last `render_scene` call.
    pub fn culling_stats(&self) -> CullingStats {
        self.culling_stats.get()
    }

    // -----------------------------------------------------------------
    // Viewport
    // -----------------------------------------------------------------

    /// Returns the current scene global time in seconds.
    pub fn global_time(&self) -> f64 {
        self.global_time
    }

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
        let tc = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(tc.get_transform(), Mat4::IDENTITY);
    }

    #[test]
    fn destroy_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        assert!(scene.is_alive(e));
        scene.destroy_entity(e).unwrap();
        assert!(!scene.is_alive(e));
    }

    #[test]
    fn add_and_get_component() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let camera = CameraComponent::default();
        scene.add_component(e, camera);
        assert!(scene.has_component::<CameraComponent>(e));
        let retrieved = scene.get_component::<CameraComponent>(e);
        assert!(retrieved.is_some());
    }

    #[test]
    fn get_component_mut() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        {
            let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
            tc.translation = Vec3::new(1.0, 2.0, 3.0);
        }
        let tc = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(tc.translation, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn remove_component() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, CameraComponent::default());
        assert!(scene.has_component::<CameraComponent>(e));
        let removed = scene.remove_component::<CameraComponent>(e);
        assert!(removed.is_some());
        assert!(!scene.has_component::<CameraComponent>(e));
    }

    #[test]
    fn get_nonexistent_component_returns_none() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        // Entity exists but has no CameraComponent.
        let result = scene.get_component::<CameraComponent>(e);
        assert!(result.is_none());
    }

    #[test]
    fn entity_is_copy() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let e_copy = e; // Copy (Entity is Copy).
        assert_eq!(e.id(), e_copy.id());
        assert!(scene.is_alive(e_copy));
    }

    #[test]
    fn multiple_entities() {
        let mut scene = Scene::new();
        let e1 = scene.create_entity_with_tag("A");
        let e2 = scene.create_entity_with_tag("B");
        assert_ne!(e1.id(), e2.id());
        let tag1 = scene.get_component::<TagComponent>(e1).unwrap();
        let tag2 = scene.get_component::<TagComponent>(e2).unwrap();
        assert_eq!(tag1.tag, "A");
        assert_eq!(tag2.tag, "B");
    }

    #[test]
    fn query_world_directly() {
        let mut scene = Scene::new();
        scene.create_entity_with_tag("One");
        scene.create_entity_with_tag("Two");
        let count = scene.world().query::<&TagComponent>().iter().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn copy_preserves_entity_count() {
        let mut scene = Scene::new();
        scene.create_entity();
        scene.create_entity();
        scene.create_entity();
        let copy = Scene::copy(&scene);
        assert_eq!(copy.entity_count(), scene.entity_count());
    }

    #[test]
    fn copy_preserves_uuids() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        let copy = Scene::copy(&scene);
        // The copy should have an entity with the same UUID.
        let found = copy.find_entity_by_uuid(uuid);
        assert!(found.is_some());
    }

    #[test]
    fn copy_preserves_transform() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        {
            let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
            tc.translation = Vec3::new(10.0, 20.0, 30.0);
        }
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        let copy = Scene::copy(&scene);
        let copy_entity = copy.find_entity_by_uuid(uuid).unwrap();
        let tc = copy
            .get_component::<TransformComponent>(copy_entity)
            .unwrap();
        assert_eq!(tc.translation, Vec3::new(10.0, 20.0, 30.0));
    }

    #[test]
    fn copy_is_independent() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        let mut copy = Scene::copy(&scene);
        // Modify the copy.
        let copy_entity = copy.find_entity_by_uuid(uuid).unwrap();
        {
            let mut tc = copy
                .get_component_mut::<TransformComponent>(copy_entity)
                .unwrap();
            tc.translation = Vec3::new(99.0, 99.0, 99.0);
        }
        // Original should be unchanged.
        let tc = scene.get_component::<TransformComponent>(e).unwrap();
        assert_eq!(tc.translation, Vec3::ZERO);
    }

    #[test]
    fn duplicate_entity_creates_new_entity() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Orig");
        assert_eq!(scene.entity_count(), 1);
        let _ = scene.duplicate_entity(e);
        assert_eq!(scene.entity_count(), 2);
    }

    #[test]
    fn duplicate_entity_copies_tag() {
        let mut scene = Scene::new();
        let e = scene.create_entity_with_tag("Hero");
        let dup = scene.duplicate_entity(e);
        let tag = scene.get_component::<TagComponent>(dup).unwrap();
        assert_eq!(tag.tag, "Hero");
    }

    #[test]
    fn duplicate_entity_new_uuid() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let dup = scene.duplicate_entity(e);
        let id_orig = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        let id_dup = scene.get_component::<IdComponent>(dup).unwrap().id.raw();
        assert_ne!(id_orig, id_dup);
    }

    #[test]
    fn duplicate_entity_copies_transform() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        {
            let mut tc = scene.get_component_mut::<TransformComponent>(e).unwrap();
            tc.translation = Vec3::new(5.0, 6.0, 7.0);
        }
        let dup = scene.duplicate_entity(e);
        let tc = scene.get_component::<TransformComponent>(dup).unwrap();
        assert_eq!(tc.translation, Vec3::new(5.0, 6.0, 7.0));
    }

    // --- Physics scripting API tests ---

    #[test]
    fn apply_impulse_noop_without_physics() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        scene.add_component(e, RigidBody2DComponent::default());
        // Should not panic — just no-op.
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
        let mut scene = Scene::new();
        let e = scene.create_entity();
        // No RigidBody2DComponent at all.
        assert!(scene.get_linear_velocity(e).is_none());
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
        let e = scene.create_entity();
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        assert_eq!(scene.entity_count(), 1);
        scene.queue_entity_destroy(uuid);
        scene.flush_pending_destroys();
        assert_eq!(scene.entity_count(), 0);
    }

    #[test]
    fn flush_deduplicates() {
        let mut scene = Scene::new();
        let e = scene.create_entity();
        let uuid = scene.get_component::<IdComponent>(e).unwrap().id.raw();
        scene.queue_entity_destroy(uuid);
        scene.queue_entity_destroy(uuid);
        scene.flush_pending_destroys(); // should not panic on double-destroy
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
        const EXPECTED_COUNT: usize = 16;
        assert_eq!(
            MACRO_COUNT, EXPECTED_COUNT,
            "for_each_cloneable_component! has {} types but expected {}. \
             Update EXPECTED_COUNT if you intentionally added/removed a component.",
            MACRO_COUNT, EXPECTED_COUNT
        );
    }

    #[test]
    fn find_asset_references_sprite() {
        let mut scene = Scene::new();
        let handle = crate::uuid::Uuid::from_raw(42);

        let e = scene.create_entity_with_tag("Player");
        scene.add_component(
            e,
            SpriteRendererComponent {
                texture_handle: handle,
                ..Default::default()
            },
        );

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
        scene.add_component(
            e,
            AudioSourceComponent {
                audio_handle: handle,
                ..Default::default()
            },
        );

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
        scene.add_component(
            e,
            TilemapComponent {
                texture_handle: handle,
                ..Default::default()
            },
        );

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
        scene.add_component(
            e1,
            SpriteRendererComponent {
                texture_handle: handle,
                ..Default::default()
            },
        );

        let e2 = scene.create_entity_with_tag("B");
        scene.add_component(
            e2,
            SpriteRendererComponent {
                texture_handle: handle,
                ..Default::default()
            },
        );

        let refs = scene.find_asset_references(handle);
        assert_eq!(refs.len(), 2);
    }
}
