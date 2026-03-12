use crate::cursor::CursorMode;
use crate::renderer::VertexArray;
use crate::uuid::Uuid;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use parking_lot::{Mutex, RwLock};

use super::components::{
    AudioCategory, CameraComponent, IdComponent, MeshRendererComponent, RelationshipComponent,
    TagComponent, TransformComponent, UIAnchorComponent,
};
use super::entity::Entity;
use super::spatial::{Aabb2D, Aabb3D, SpatialGrid, SpatialGrid3D};
use super::{CullingStats, FullscreenMode};

/// Thread-safe ECS core of a scene.
///
/// Contains all `Send + Sync` data: the `hecs::World`, lookup caches, spatial
/// grids, transform caches, timing, viewport dimensions, runtime settings, and
/// deferred-destroy queues.
///
/// [`Scene`](super::Scene) wraps a `SceneCore` alongside the `!Send` subsystems
/// (physics, scripting, audio) and provides `Deref<Target = SceneCore>` so that
/// all `SceneCore` methods are callable directly on `Scene`.
pub struct SceneCore {
    // ECS World
    pub(super) world: hecs::World,

    // Viewport dimensions
    pub(super) viewport_width: u32,
    pub(super) viewport_height: u32,

    // Lookup caches
    /// O(1) UUID → hecs::Entity lookup cache, maintained on create/destroy.
    pub(super) uuid_cache: HashMap<u64, hecs::Entity>,
    /// O(1) entity-ID → hecs::Entity lookup cache, maintained on create/destroy.
    /// Used by mouse picking (pixel readback returns raw entity ID).
    pub(super) id_cache: HashMap<u32, hecs::Entity>,
    /// Lazy name → UUID cache for `find_entity_by_name`. Built on first call,
    /// invalidated on entity create/destroy. Only stores first match per name.
    /// Uses `RwLock` for interior mutability so `find_entity_by_name` can take `&self`.
    pub(super) name_cache: RwLock<Option<HashMap<String, u64>>>,

    // Deferred operations
    /// Deferred entity destruction queue (UUIDs). Flushed after script callbacks.
    pub(super) pending_destroy: Vec<u64>,

    // Time management
    /// Monotonic scene time in seconds. Incremented each frame by `dt`.
    /// Used by [`InstancedSpriteAnimator`] for stateless frame computation.
    pub(super) global_time: f64,
    /// Last frame delta time in seconds, stored for `Engine.delta_time()`.
    pub(super) last_dt: f32,

    // Spatial data structures
    /// Spatial grid for efficient 2D region queries.
    pub(super) spatial_grid: Option<SpatialGrid>,
    /// Spatial grid for efficient 3D region queries.
    pub(super) spatial_grid_3d: Option<SpatialGrid3D>,

    // Rendering state
    /// Per-frame frustum culling statistics.
    /// Written by `render_scene` (which takes `&self`), read via `&self` getter.
    pub(super) culling_stats: Mutex<CullingStats>,
    /// Deferred-destroy queue for vertex arrays. Prevents destroying GPU buffers
    /// that may still be in use by in-flight command buffers.
    pub(super) va_graveyard: VecDeque<Vec<VertexArray>>,

    // Transform caching (persistent with dirty detection)
    /// Persistent world transform cache. Updated lazily by
    /// `build_world_transform_cache` using snapshot-based dirty detection.
    pub(super) transform_cache: RwLock<HashMap<hecs::Entity, glam::Mat4>>,
    /// Snapshot of each entity's local transform + parent UUID at the time
    /// the transform cache was last built. Used for change detection.
    pub(super) transform_snapshots: RwLock<HashMap<hecs::Entity, (glam::Mat4, Option<u64>)>>,

    // Asset resolution tracking
    /// When `true`, all texture handles have been resolved.
    pub(super) textures_all_resolved: bool,

    // Audio volume management
    /// Global master volume (0.0–1.0). Multiplied into all sound playback.
    pub(super) master_volume: f32,
    /// Per-category volume multipliers (0.0–1.0), indexed by [`AudioCategory`].
    pub(super) category_volumes: [f32; AudioCategory::COUNT],

    // Shadow mapping caching
    /// Stashed cascade VP matrices + split depths + shadow_distance + texel_sizes
    /// from `render_shadow_pass`, consumed by `render_meshes`.
    #[allow(clippy::type_complexity)]
    pub(super) shadow_cascade_cache: RwLock<Option<([glam::Mat4; 4], [f32; 3], f32, [f32; 4])>>,

    // Cursor and window management
    /// Cursor mode requested by scripts.
    pub(super) cursor_mode: CursorMode,
    /// Window resize requested by scripts. Consumed each frame by the runtime.
    pub(super) requested_window_size: Mutex<Option<(u32, u32)>>,

    // -- Runtime settings (Lua ↔ Player) ------------------------------------
    /// VSync request from scripts. `Some(true)` = Fifo, `Some(false)` = Mailbox.
    pub(super) requested_vsync: Mutex<Option<bool>>,
    /// Fullscreen request from scripts.
    pub(super) requested_fullscreen: Mutex<Option<FullscreenMode>>,
    /// Shadow quality request from scripts. 0=Low, 1=Medium, 2=High, 3=Ultra.
    pub(super) requested_shadow_quality: Mutex<Option<i32>>,
    /// Quit request from scripts.
    pub(super) requested_quit: AtomicBool,
    /// Scene load request from scripts. Path relative to CWD.
    pub(super) requested_load_scene: Mutex<Option<String>>,
    /// Current VSync state.
    pub(super) vsync_enabled: AtomicBool,
    /// Current fullscreen mode.
    pub(super) fullscreen_mode: Mutex<FullscreenMode>,
    /// Current shadow quality tier (0–3).
    pub(super) shadow_quality_state: AtomicI32,
    /// Global GUI scale factor for UI-anchored entities.
    pub(super) gui_scale: Mutex<f32>,
}

// Compile-time verification that SceneCore is Send + Sync.
static_assertions::assert_impl_all!(SceneCore: Send, Sync);

impl SceneCore {
    /// Create an empty scene core.
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            viewport_width: 0,
            viewport_height: 0,
            uuid_cache: HashMap::new(),
            id_cache: HashMap::new(),
            name_cache: RwLock::new(None),
            pending_destroy: Vec::new(),
            global_time: 0.0,
            last_dt: 0.0,
            spatial_grid: None,
            spatial_grid_3d: None,
            culling_stats: Mutex::new(CullingStats::default()),
            va_graveyard: VecDeque::new(),
            transform_cache: RwLock::new(HashMap::new()),
            transform_snapshots: RwLock::new(HashMap::new()),
            textures_all_resolved: false,
            master_volume: 1.0,
            category_volumes: [1.0; AudioCategory::COUNT],
            shadow_cascade_cache: RwLock::new(None),
            cursor_mode: CursorMode::Normal,
            requested_window_size: Mutex::new(None),
            requested_vsync: Mutex::new(None),
            requested_fullscreen: Mutex::new(None),
            requested_shadow_quality: Mutex::new(None),
            requested_quit: AtomicBool::new(false),
            requested_load_scene: Mutex::new(None),
            vsync_enabled: AtomicBool::new(false),
            fullscreen_mode: Mutex::new(FullscreenMode::Windowed),
            shadow_quality_state: AtomicI32::new(3),
            gui_scale: Mutex::new(1.0),
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
        *self.name_cache.write() = None; // invalidate
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
            *self.name_cache.write() = None;
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
    /// [`flush_pending_destroys`](super::Scene::flush_pending_destroys) after all
    /// script callbacks complete. Duplicates are ignored during flush.
    pub fn queue_entity_destroy(&mut self, uuid: u64) {
        self.pending_destroy.push(uuid);
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
    pub fn find_entity_by_id(&self, id: u32) -> Option<Entity> {
        self.id_cache.get(&id).copied().map(Entity::new)
    }

    /// Find the first entity whose [`TagComponent`] name matches `name`.
    ///
    /// O(n) scan on first call (builds cache), O(1) afterwards.
    pub fn find_entity_by_name(&self, name: &str) -> Option<(Entity, u64)> {
        // Build the name cache lazily on first call (interior mutability via RwLock).
        {
            let needs_build = self.name_cache.read().is_none();
            if needs_build {
                let mut cache = HashMap::new();
                for (tag, id) in self.world.query::<(&TagComponent, &IdComponent)>().iter() {
                    cache.entry(tag.tag.clone()).or_insert(id.id.raw());
                }
                *self.name_cache.write() = Some(cache);
            }
        }

        let cache = self.name_cache.read();
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
    /// all other [`CameraComponent`]s.
    pub fn set_primary_camera(&mut self, entity: Entity) {
        for (handle, camera) in self
            .world
            .query_mut::<(hecs::Entity, &mut CameraComponent)>()
        {
            camera.primary = handle == entity.handle();
        }
    }

    // -----------------------------------------------------------------
    // Spatial queries (2D)
    // -----------------------------------------------------------------

    /// Rebuild the spatial grid from current entity transforms.
    pub fn rebuild_spatial_grid(&mut self, cell_size: f32) {
        self.build_world_transform_cache();
        let wt_ref = self.transform_cache.read();
        let mut grid = SpatialGrid::new(cell_size);
        for (&handle, wt) in &*wt_ref {
            let aabb = Aabb2D::from_unit_quad_transform(wt);
            grid.insert(handle, &aabb);
        }
        self.spatial_grid = Some(grid);
    }

    /// Query all entities whose AABB overlaps the given world-space region.
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
    pub fn rebuild_spatial_grid_3d(&mut self, cell_size: f32) {
        self.build_world_transform_cache();
        let wt_ref = self.transform_cache.read();
        let mut grid = SpatialGrid3D::new(cell_size);
        for (&handle, wt) in wt_ref.iter() {
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
        *self.culling_stats.lock()
    }

    // -----------------------------------------------------------------
    // Viewport / Time
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
    pub fn cursor_mode(&self) -> CursorMode {
        self.cursor_mode
    }

    /// Set the cursor mode from game logic (scripts, native or Lua).
    pub fn set_cursor_mode(&mut self, mode: CursorMode) {
        self.cursor_mode = mode;
    }

    /// Request a window resize from scripts. Physical pixels.
    pub fn request_window_size(&self, width: u32, height: u32) {
        *self.requested_window_size.lock() = Some((width, height));
    }

    /// Take (consume) the pending window resize request, if any.
    pub fn take_requested_window_size(&self) -> Option<(u32, u32)> {
        self.requested_window_size.lock().take()
    }

    // -- Runtime settings (Lua ↔ Player) ------------------------------------

    /// Request a VSync change from scripts.
    pub fn request_vsync(&self, enabled: bool) {
        *self.requested_vsync.lock() = Some(enabled);
        self.vsync_enabled.store(enabled, Ordering::Relaxed);
    }
    /// Take (consume) the pending VSync request.
    pub fn take_requested_vsync(&self) -> Option<bool> {
        self.requested_vsync.lock().take()
    }
    /// Current VSync state as reported by the player.
    pub fn vsync_enabled(&self) -> bool {
        self.vsync_enabled.load(Ordering::Relaxed)
    }
    /// Update the tracked VSync state (called by the player after applying).
    pub fn set_vsync_enabled(&self, val: bool) {
        self.vsync_enabled.store(val, Ordering::Relaxed);
    }

    /// Request a fullscreen mode change from scripts.
    pub fn request_fullscreen(&self, mode: FullscreenMode) {
        *self.requested_fullscreen.lock() = Some(mode);
        *self.fullscreen_mode.lock() = mode;
    }
    /// Take (consume) the pending fullscreen request.
    pub fn take_requested_fullscreen(&self) -> Option<FullscreenMode> {
        self.requested_fullscreen.lock().take()
    }
    /// Current fullscreen mode.
    pub fn fullscreen_mode(&self) -> FullscreenMode {
        *self.fullscreen_mode.lock()
    }
    /// Update the tracked fullscreen mode (called by the player after applying).
    pub fn set_fullscreen_mode(&self, mode: FullscreenMode) {
        *self.fullscreen_mode.lock() = mode;
    }
    /// Convenience: true if any fullscreen mode is active.
    pub fn is_fullscreen(&self) -> bool {
        *self.fullscreen_mode.lock() != FullscreenMode::Windowed
    }

    /// Request a shadow quality change from scripts. Clamped to 0–3.
    pub fn request_shadow_quality(&self, quality: i32) {
        let clamped = quality.clamp(0, 3);
        *self.requested_shadow_quality.lock() = Some(clamped);
        self.shadow_quality_state.store(clamped, Ordering::Relaxed);
    }
    /// Take (consume) the pending shadow quality request.
    pub fn take_requested_shadow_quality(&self) -> Option<i32> {
        self.requested_shadow_quality.lock().take()
    }
    /// Current shadow quality tier (0–3).
    pub fn shadow_quality(&self) -> i32 {
        self.shadow_quality_state.load(Ordering::Relaxed)
    }
    /// Update the tracked shadow quality state (called by the player after applying).
    pub fn set_shadow_quality_state(&self, val: i32) {
        self.shadow_quality_state.store(val, Ordering::Relaxed);
    }

    /// Current GUI scale factor (default 1.0).
    pub fn gui_scale(&self) -> f32 {
        *self.gui_scale.lock()
    }
    /// Set the GUI scale factor. Clamped to 0.5–2.0.
    pub fn set_gui_scale(&self, scale: f32) {
        *self.gui_scale.lock() = scale.clamp(0.5, 2.0);
    }

    /// Request application exit from scripts.
    pub fn request_quit(&self) {
        self.requested_quit.store(true, Ordering::Relaxed);
    }
    /// Take (consume) the pending quit request.
    pub fn take_requested_quit(&self) -> bool {
        self.requested_quit.swap(false, Ordering::Relaxed)
    }

    /// Request a scene load from scripts. Path relative to CWD.
    pub fn request_load_scene(&self, path: String) {
        *self.requested_load_scene.lock() = Some(path);
    }
    /// Take (consume) the pending scene load request.
    pub fn take_requested_load_scene(&self) -> Option<String> {
        self.requested_load_scene.lock().take()
    }

    /// Current viewport dimensions in physical pixels.
    pub fn viewport_size(&self) -> (u32, u32) {
        (self.viewport_width, self.viewport_height)
    }

    /// Notify the scene that the viewport dimensions changed.
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
    pub fn apply_ui_anchors(&mut self) {
        let mut cam_info: Option<(glam::Vec3, f32, f32)> = None;
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
            None => return,
        };

        let gui_scale = *self.gui_scale.lock();
        let mut updates: Vec<(hecs::Entity, f32, f32)> = Vec::new();
        for (handle, anchor, _tc) in self
            .world
            .query::<(hecs::Entity, &UIAnchorComponent, &TransformComponent)>()
            .iter()
        {
            let world_x = cam_pos.x
                + (-half_w + anchor.anchor.x * 2.0 * half_w)
                + anchor.offset.x * gui_scale;
            let world_y =
                cam_pos.y + (half_h - anchor.anchor.y * 2.0 * half_h) + anchor.offset.y * gui_scale;
            let _ = _tc;
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

impl Default for SceneCore {
    fn default() -> Self {
        Self::new()
    }
}
