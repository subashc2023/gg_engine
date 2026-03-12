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
mod physics_3d;
mod physics_3d_ops;
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
    AmbientLightComponent, AudioCategory, AudioListenerComponent, AudioSourceComponent,
    BoxCollider2DComponent, BoxCollider3DComponent, CameraComponent, CapsuleCollider3DComponent,
    CircleCollider2DComponent, CircleRendererComponent, DirectionalLightComponent, IdComponent,
    MeshPrimitive, MeshRendererComponent, MeshSource, NativeScriptComponent,
    ParticleEmitterComponent, PointLightComponent, RelationshipComponent, RigidBody2DComponent,
    RigidBody2DType, RigidBody3DComponent, RigidBody3DType, SphereCollider3DComponent,
    SpriteRendererComponent, TagComponent, TextComponent, TilemapComponent, TransformComponent,
    UIAnchorComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
pub use entity::Entity;
pub use native_script::NativeScript;
pub use physics_3d::RaycastHit3D;
pub use scene_serializer::SceneSerializer;
#[cfg(feature = "lua-scripting")]
pub use script_engine::{ScriptEngine, ScriptFieldValue};
pub use spatial::{Aabb2D, Aabb3D, Frustum2D, Frustum3D, SpatialGrid, SpatialGrid3D};

use crate::renderer::VertexArray;
use crate::uuid::Uuid;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use physics_2d::PhysicsWorld2D;
use physics_3d::PhysicsWorld3D;

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

/// Fullscreen mode for the player window.
///
/// Used by Lua scripts via `Engine.set_fullscreen()` / `Engine.get_fullscreen()`.
/// The [`Application`](crate::Application) trait returns this from
/// `requested_fullscreen()`, and the engine runner converts it to the appropriate
/// winit type (including video mode enumeration for `Exclusive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FullscreenMode {
    /// Normal windowed mode (no fullscreen).
    #[default]
    Windowed,
    /// Borderless fullscreen (desktop resolution, instant alt-tab).
    Borderless,
    /// Exclusive fullscreen (dedicated video mode, best performance).
    Exclusive,
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
    physics_world_3d: Option<PhysicsWorld3D>,
    #[cfg(feature = "lua-scripting")]
    script_engine: Option<ScriptEngine>,
    audio_engine: Option<audio::AudioEngine>,
    /// O(1) UUID → hecs::Entity lookup cache, maintained on create/destroy.
    uuid_cache: HashMap<u64, hecs::Entity>,
    /// O(1) entity-ID → hecs::Entity lookup cache, maintained on create/destroy.
    /// Used by mouse picking (pixel readback returns raw entity ID).
    id_cache: HashMap<u32, hecs::Entity>,
    /// Lazy name → UUID cache for `find_entity_by_name`. Built on first call,
    /// invalidated on entity create/destroy. Only stores first match per name.
    /// Uses `RefCell` for interior mutability so `find_entity_by_name` can take `&self`.
    name_cache: RefCell<Option<HashMap<String, u64>>>,
    /// Deferred entity destruction queue (UUIDs). Flushed after script callbacks.
    pending_destroy: Vec<u64>,
    /// Monotonic scene time in seconds. Incremented each frame by `dt`.
    /// Used by [`InstancedSpriteAnimator`] for stateless frame computation.
    global_time: f64,
    /// Last frame delta time in seconds, stored for `Engine.delta_time()`.
    last_dt: f32,
    /// Spatial grid for efficient 2D region queries.
    /// Rebuilt on demand via [`rebuild_spatial_grid`](Self::rebuild_spatial_grid).
    spatial_grid: Option<SpatialGrid>,
    /// Spatial grid for efficient 3D region queries.
    /// Rebuilt on demand via [`rebuild_spatial_grid_3d`](Self::rebuild_spatial_grid_3d).
    spatial_grid_3d: Option<SpatialGrid3D>,
    /// Per-frame frustum culling statistics (interior-mutable, written by render_scene).
    culling_stats: Cell<CullingStats>,
    /// Deferred-destroy queue for vertex arrays. Prevents destroying GPU buffers
    /// that may still be in use by in-flight command buffers. Rotated each frame
    /// in [`rotate_va_graveyard`](Self::rotate_va_graveyard); entries survive at
    /// least `MAX_FRAMES_IN_FLIGHT` frames before being dropped.
    va_graveyard: VecDeque<Vec<VertexArray>>,
    /// Persistent world transform cache. Updated lazily by
    /// [`build_world_transform_cache`](Self::build_world_transform_cache)
    /// using snapshot-based dirty detection. Only rebuilds when local
    /// transforms or hierarchy actually change between frames.
    transform_cache: RefCell<HashMap<hecs::Entity, glam::Mat4>>,
    /// Snapshot of each entity's local transform + parent UUID at the time
    /// the transform cache was last built. Used for change detection.
    transform_snapshots: RefCell<HashMap<hecs::Entity, (glam::Mat4, Option<u64>)>>,
    /// When `true`, all texture handles have been resolved and
    /// `resolve_texture_handles_async` can skip scanning every entity.
    /// Reset to `false` when entities are created or components change.
    textures_all_resolved: bool,
    /// Global master volume (0.0–1.0). Multiplied into all sound playback.
    master_volume: f32,
    /// Per-category volume multipliers (0.0–1.0), indexed by [`AudioCategory`].
    category_volumes: [f32; AudioCategory::COUNT],
    /// Stashed cascade VP matrices + split depths + shadow_distance + texel_sizes
    /// from `render_shadow_pass`, consumed by `render_meshes` for the `LightEnvironment`.
    #[allow(clippy::type_complexity)]
    shadow_cascade_cache: RefCell<Option<([glam::Mat4; 4], [f32; 3], f32, [f32; 4])>>,
    /// Cursor mode requested by scripts. Read by the player/runtime each frame
    /// and applied via the [`Application::cursor_mode()`] trait method.
    cursor_mode: crate::cursor::CursorMode,
    /// Window resize requested by scripts. Consumed (taken) each frame by the
    /// player/runtime. Physical pixels.
    requested_window_size: std::cell::Cell<Option<(u32, u32)>>,

    // -- Runtime settings (Lua ↔ Player) ------------------------------------
    /// VSync request from scripts. `Some(true)` = Fifo, `Some(false)` = Mailbox.
    requested_vsync: Cell<Option<bool>>,
    /// Fullscreen request from scripts.
    requested_fullscreen: Cell<Option<FullscreenMode>>,
    /// Shadow quality request from scripts. 0=Low, 1=Medium, 2=High, 3=Ultra.
    requested_shadow_quality: Cell<Option<i32>>,
    /// Quit request from scripts.
    requested_quit: Cell<bool>,
    /// Scene load request from scripts. Path relative to CWD (e.g. `"assets/scenes/foo.ggscene"`).
    requested_load_scene: RefCell<Option<String>>,
    /// Current VSync state — Cell for readback from `&self` context (Lua getters).
    vsync_enabled: Cell<bool>,
    /// Current fullscreen mode — Cell for readback from `&self` context.
    fullscreen_mode: Cell<FullscreenMode>,
    /// Current shadow quality tier (0–3) — Cell for readback from `&self` context.
    shadow_quality_state: Cell<i32>,
    /// Global GUI scale factor for UI-anchored entities. Multiplied into offsets
    /// and text font sizes. Cell for readback from `&self` (Lua getters).
    gui_scale: Cell<f32>,
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
            RigidBody3DComponent,
            BoxCollider3DComponent,
            SphereCollider3DComponent,
            CapsuleCollider3DComponent,
            RelationshipComponent,
            SpriteAnimatorComponent,
            InstancedSpriteAnimator,
            AnimationControllerComponent,
            TilemapComponent,
            AudioSourceComponent,
            AudioListenerComponent,
            ParticleEmitterComponent,
            MeshRendererComponent,
            DirectionalLightComponent,
            PointLightComponent,
            AmbientLightComponent,
            UIAnchorComponent,
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
            ($crate::scene::RigidBody3DComponent, "Rigidbody 3D"),
            // 3D colliders handled manually in editor for mesh-aware defaults.
            ($crate::scene::MeshRendererComponent, "Mesh Renderer"),
            (
                $crate::scene::DirectionalLightComponent,
                "Directional Light"
            ),
            ($crate::scene::PointLightComponent, "Point Light"),
            ($crate::scene::AmbientLightComponent, "Ambient Light"),
            ($crate::scene::UIAnchorComponent, "UI Anchor"),
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
            physics_world_3d: None,
            #[cfg(feature = "lua-scripting")]
            script_engine: None,
            audio_engine: None,
            uuid_cache: HashMap::new(),
            id_cache: HashMap::new(),
            name_cache: RefCell::new(None),
            pending_destroy: Vec::new(),
            global_time: 0.0,
            last_dt: 0.0,
            spatial_grid: None,
            spatial_grid_3d: None,
            culling_stats: Cell::new(CullingStats::default()),
            va_graveyard: VecDeque::new(),
            transform_cache: RefCell::new(HashMap::new()),
            transform_snapshots: RefCell::new(HashMap::new()),
            textures_all_resolved: false,
            master_volume: 1.0,
            category_volumes: [1.0; AudioCategory::COUNT],
            shadow_cascade_cache: RefCell::new(None),
            cursor_mode: crate::cursor::CursorMode::Normal,
            requested_window_size: std::cell::Cell::new(None),
            requested_vsync: Cell::new(None),
            requested_fullscreen: Cell::new(None),
            requested_shadow_quality: Cell::new(None),
            requested_quit: Cell::new(false),
            requested_load_scene: RefCell::new(None),
            vsync_enabled: Cell::new(false),
            fullscreen_mode: Cell::new(FullscreenMode::Windowed),
            shadow_quality_state: Cell::new(3),
            gui_scale: Cell::new(1.0),
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
        self.id_cache.insert(handle.id(), handle);
        *self.name_cache.borrow_mut() = None; // invalidate
        self.textures_all_resolved = false; // new entity may need texture resolution
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

        // Remove from caches and invalidate name cache.
        if let Some(u) = uuid {
            self.uuid_cache.remove(&u);
            *self.name_cache.borrow_mut() = None;
        }
        self.id_cache.remove(&entity.handle().id());

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
                // Clean up script engine state (Lua environment + timers) before
                // destroying the entity, so timers don't fire on dead entities.
                if let Some(ref mut engine) = self.script_engine {
                    engine.remove_entity_timers(uuid);
                    engine.remove_entity_env(uuid);
                }

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

                // Also clean up 3D physics bodies.
                let body_handle_3d = self
                    .get_component::<RigidBody3DComponent>(entity)
                    .and_then(|rb| rb.runtime_body);

                if let (Some(handle), Some(ref mut physics)) =
                    (body_handle_3d, &mut self.physics_world_3d)
                {
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

        // Copy audio volume settings.
        new_scene.master_volume = source.master_volume;
        new_scene.category_volumes = source.category_volumes;

        // Copy runtime settings state.
        new_scene.fullscreen_mode.set(source.fullscreen_mode.get());
        new_scene.gui_scale.set(source.gui_scale.get());

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
    /// O(1) lookup via internal cache maintained on entity create/destroy.
    /// Returns `None` if no living entity has the given ID.
    pub fn find_entity_by_id(&self, id: u32) -> Option<Entity> {
        self.id_cache.get(&id).copied().map(Entity::new)
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
    pub fn find_entity_by_name(&self, name: &str) -> Option<(Entity, u64)> {
        // Build the name cache lazily on first call (interior mutability via RefCell).
        {
            let needs_build = self.name_cache.borrow().is_none();
            if needs_build {
                let mut cache = HashMap::new();
                for (tag, id) in self.world.query::<(&TagComponent, &IdComponent)>().iter() {
                    // First entity registered per name wins (matches old linear scan).
                    cache.entry(tag.tag.clone()).or_insert(id.id.raw());
                }
                *self.name_cache.borrow_mut() = Some(cache);
            }
        }

        let cache = self.name_cache.borrow();
        let uuid = *cache.as_ref().unwrap().get(name)?;
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

    // -----------------------------------------------------------------
    // 3D Spatial queries
    // -----------------------------------------------------------------

    /// Rebuild the 3D spatial grid from current entity transforms and mesh bounds.
    ///
    /// Inserts all entities with a [`MeshRendererComponent`] using their
    /// world-space AABB. Other 3D entities without mesh bounds use a
    /// unit-cube AABB from their world transform.
    pub fn rebuild_spatial_grid_3d(&mut self, cell_size: f32) {
        let wt_cache = self.build_world_transform_cache();
        let mut grid = SpatialGrid3D::new(cell_size);
        for (&handle, wt) in wt_cache.iter() {
            // Use mesh local bounds if available, otherwise unit cube.
            let aabb = if let Ok(mesh) = self.world.get::<&MeshRendererComponent>(handle) {
                if let Some(ref bounds) = mesh.local_bounds {
                    Aabb3D::from_local_bounds(bounds.0, bounds.1, wt)
                } else {
                    Aabb3D::from_unit_cube_transform(wt)
                }
            } else {
                Aabb3D::from_unit_cube_transform(wt)
            };
            grid.insert(handle, &aabb);
        }
        self.spatial_grid_3d = Some(grid);
    }

    /// Query all entities whose 3D AABB overlaps the given world-space region.
    pub fn query_entities_in_region_3d(&self, min: glam::Vec3, max: glam::Vec3) -> Vec<Entity> {
        let Some(ref grid) = self.spatial_grid_3d else {
            return Vec::new();
        };
        let region = Aabb3D::new(min, max);
        grid.query_region_dedup(&region)
            .into_iter()
            .map(Entity::new)
            .collect()
    }

    /// Query all entities within `radius` world units of `center` in 3D.
    pub fn query_entities_in_radius_3d(&self, center: glam::Vec3, radius: f32) -> Vec<Entity> {
        let Some(ref grid) = self.spatial_grid_3d else {
            return Vec::new();
        };
        let region = Aabb3D::new(
            center - glam::Vec3::splat(radius),
            center + glam::Vec3::splat(radius),
        );
        let r2 = radius * radius;
        grid.query_region_dedup(&region)
            .into_iter()
            .filter(|&handle| {
                let entity = Entity::new(handle);
                let wt = self.get_world_transform(entity);
                let pos = glam::Vec3::new(wt.w_axis.x, wt.w_axis.y, wt.w_axis.z);
                (pos - center).length_squared() <= r2
            })
            .map(Entity::new)
            .collect()
    }

    /// Returns a reference to the 3D spatial grid, if built.
    pub fn spatial_grid_3d(&self) -> Option<&SpatialGrid3D> {
        self.spatial_grid_3d.as_ref()
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

    /// Returns the last frame delta time in seconds.
    pub fn last_dt(&self) -> f32 {
        self.last_dt
    }

    /// Current cursor mode requested by scripts within this scene.
    pub fn cursor_mode(&self) -> crate::cursor::CursorMode {
        self.cursor_mode
    }

    /// Set the cursor mode from game logic (scripts, native or Lua).
    ///
    /// The player/runtime reads this each frame and applies it to the window.
    pub fn set_cursor_mode(&mut self, mode: crate::cursor::CursorMode) {
        self.cursor_mode = mode;
    }

    /// Request a window resize from scripts. The value is consumed (taken) once
    /// per frame by the runtime. Physical pixels.
    pub fn request_window_size(&self, width: u32, height: u32) {
        self.requested_window_size.set(Some((width, height)));
    }

    /// Take (consume) the pending window resize request, if any.
    pub fn take_requested_window_size(&self) -> Option<(u32, u32)> {
        self.requested_window_size.take()
    }

    // -- Runtime settings (Lua ↔ Player) ------------------------------------

    /// Request a VSync change from scripts.
    /// Also updates the tracked state optimistically for immediate readback.
    pub fn request_vsync(&self, enabled: bool) {
        self.requested_vsync.set(Some(enabled));
        self.vsync_enabled.set(enabled);
    }
    /// Take (consume) the pending VSync request.
    pub fn take_requested_vsync(&self) -> Option<bool> {
        self.requested_vsync.take()
    }
    /// Current VSync state as reported by the player.
    pub fn vsync_enabled(&self) -> bool {
        self.vsync_enabled.get()
    }
    /// Update the tracked VSync state (called by the player after applying).
    pub fn set_vsync_enabled(&self, val: bool) {
        self.vsync_enabled.set(val);
    }

    /// Request a fullscreen mode change from scripts.
    /// Also updates the tracked state optimistically for immediate readback.
    pub fn request_fullscreen(&self, mode: FullscreenMode) {
        self.requested_fullscreen.set(Some(mode));
        self.fullscreen_mode.set(mode);
    }
    /// Take (consume) the pending fullscreen request.
    pub fn take_requested_fullscreen(&self) -> Option<FullscreenMode> {
        self.requested_fullscreen.take()
    }
    /// Current fullscreen mode.
    pub fn fullscreen_mode(&self) -> FullscreenMode {
        self.fullscreen_mode.get()
    }
    /// Update the tracked fullscreen mode (called by the player after applying).
    pub fn set_fullscreen_mode(&self, mode: FullscreenMode) {
        self.fullscreen_mode.set(mode);
    }
    /// Convenience: true if any fullscreen mode is active.
    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen_mode.get() != FullscreenMode::Windowed
    }

    /// Request a shadow quality change from scripts. Clamped to 0–3.
    /// Also updates the tracked state optimistically for immediate readback.
    pub fn request_shadow_quality(&self, quality: i32) {
        let clamped = quality.clamp(0, 3);
        self.requested_shadow_quality.set(Some(clamped));
        self.shadow_quality_state.set(clamped);
    }
    /// Take (consume) the pending shadow quality request.
    pub fn take_requested_shadow_quality(&self) -> Option<i32> {
        self.requested_shadow_quality.take()
    }
    /// Current shadow quality tier (0–3).
    pub fn shadow_quality(&self) -> i32 {
        self.shadow_quality_state.get()
    }
    /// Update the tracked shadow quality state (called by the player after applying).
    pub fn set_shadow_quality_state(&self, val: i32) {
        self.shadow_quality_state.set(val);
    }

    /// Current GUI scale factor (default 1.0).
    pub fn gui_scale(&self) -> f32 {
        self.gui_scale.get()
    }
    /// Set the GUI scale factor. Clamped to 0.5–2.0.
    pub fn set_gui_scale(&self, scale: f32) {
        self.gui_scale.set(scale.clamp(0.5, 2.0));
    }

    /// Request application exit from scripts.
    pub fn request_quit(&self) {
        self.requested_quit.set(true);
    }
    /// Take (consume) the pending quit request.
    pub fn take_requested_quit(&self) -> bool {
        self.requested_quit.replace(false)
    }

    /// Request a scene load from scripts. Path relative to CWD.
    pub fn request_load_scene(&self, path: String) {
        *self.requested_load_scene.borrow_mut() = Some(path);
    }
    /// Take (consume) the pending scene load request.
    pub fn take_requested_load_scene(&self) -> Option<String> {
        self.requested_load_scene.borrow_mut().take()
    }

    /// Current viewport dimensions in physical pixels.
    pub fn viewport_size(&self) -> (u32, u32) {
        (self.viewport_width, self.viewport_height)
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

    // -----------------------------------------------------------------
    // UI Anchor Layout
    // -----------------------------------------------------------------

    /// Reposition all entities with a [`UIAnchorComponent`] so they stick
    /// to a screen-relative anchor point.
    ///
    /// Call this each frame before rendering. The method finds the primary
    /// camera, computes the visible world-space rectangle from its
    /// orthographic size and aspect ratio, and writes each anchored
    /// entity's `TransformComponent::translation` (X, Y only — Z preserved).
    pub fn apply_ui_anchors(&mut self) {
        // Find primary camera transform + projection params.
        let mut cam_info: Option<(glam::Vec3, f32, f32)> = None; // (position, half_w, half_h)
        for (handle, cam, _tc) in self
            .world
            .query::<(hecs::Entity, &CameraComponent, &TransformComponent)>()
            .iter()
        {
            if cam.primary {
                let world = self.get_world_transform(Entity::new(handle));
                let cam_pos = world.col(3).truncate();
                let half_h = cam.camera.orthographic_size() * 0.5;
                let aspect = if self.viewport_height > 0 {
                    self.viewport_width as f32 / self.viewport_height as f32
                } else {
                    16.0 / 9.0
                };
                let half_w = half_h * aspect;
                cam_info = Some((cam_pos, half_w, half_h));
                break;
            }
        }

        let (cam_pos, half_w, half_h) = match cam_info {
            Some(info) => info,
            None => return, // No primary camera — nothing to anchor.
        };

        // Collect updates (avoid borrow conflict: query borrows world immutably,
        // then we write back).
        let gui_scale = self.gui_scale.get();
        let mut updates: Vec<(hecs::Entity, f32, f32)> = Vec::new();
        for (handle, anchor, _tc) in self
            .world
            .query::<(hecs::Entity, &UIAnchorComponent, &TransformComponent)>()
            .iter()
        {
            // anchor (0,0) = top-left, (1,1) = bottom-right.
            // World X: left = cam_x - half_w, right = cam_x + half_w.
            // World Y: top = cam_y + half_h, bottom = cam_y - half_h.
            // Offsets are scaled by gui_scale for resolution-independent UI sizing.
            let world_x = cam_pos.x
                + (-half_w + anchor.anchor.x * 2.0 * half_w)
                + anchor.offset.x * gui_scale;
            let world_y =
                cam_pos.y + (half_h - anchor.anchor.y * 2.0 * half_h) + anchor.offset.y * gui_scale;
            let _ = _tc; // only needed to ensure entity has a TransformComponent
            updates.push((handle, world_x, world_y));
        }

        for (handle, x, y) in updates {
            if let Ok(mut tc) = self.world.get::<&mut TransformComponent>(handle) {
                tc.translation.x = x;
                tc.translation.y = y;
            }
        }
    }

    /// Return the primary camera's visible world-space rectangle as
    /// `(center, half_w, half_h)`, or `None` if there is no primary camera.
    ///
    /// Useful for drawing a viewport bounds wireframe in the editor.
    pub fn primary_camera_bounds(&self) -> Option<(glam::Vec3, f32, f32)> {
        for (handle, cam, _tc) in self
            .world
            .query::<(hecs::Entity, &CameraComponent, &TransformComponent)>()
            .iter()
        {
            if cam.primary {
                let world = self.get_world_transform(Entity::new(handle));
                let cam_pos = world.col(3).truncate();
                let half_h = cam.camera.orthographic_size() * 0.5;
                let aspect = if self.viewport_height > 0 {
                    self.viewport_width as f32 / self.viewport_height as f32
                } else {
                    16.0 / 9.0
                };
                let half_w = half_h * aspect;
                return Some((cam_pos, half_w, half_h));
            }
        }
        None
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
        const EXPECTED_COUNT: usize = 25;
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
