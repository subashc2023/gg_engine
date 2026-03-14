pub mod animation;
pub(crate) mod audio;
mod audio_ops;
mod components;
pub(crate) mod hrtf;
pub(crate) mod core;
mod entity;
mod hierarchy;
#[cfg(feature = "lua-scripting")]
mod lua_ops;
pub mod native_script;
mod physics_2d;
#[cfg(feature = "physics-3d")]
mod physics_3d;
#[cfg(feature = "physics-3d")]
mod physics_3d_ops;
mod physics_common;
mod physics_ops;
mod rendering;
mod runtime;
mod scene_serializer;
#[cfg(feature = "lua-scripting")]
pub(crate) mod script_engine;
#[cfg(feature = "lua-scripting")]
mod script_glue;
pub(crate) mod spatial;

pub use self::core::SceneCore;
pub use animation::{
    AnimationClip, AnimationControllerComponent, AnimationEvent, AnimationTransition,
    FloatOrdering, InstancedSpriteAnimator, SpriteAnimatorComponent, TransitionCondition,
};
#[cfg(feature = "lua-scripting")]
pub use components::LuaScriptComponent;
pub use components::{
    AmbientLightComponent, AudioCategory, AudioListenerComponent, AudioSourceComponent,
    BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent, CircleRendererComponent,
    DirectionalLightComponent, EnvironmentComponent, IdComponent, MeshPrimitive,
    MeshRendererComponent, MeshSource, NativeScriptComponent, ParticleEmitterComponent,
    PointLightComponent, RelationshipComponent, RigidBody2DComponent, RigidBody2DType,
    RigidBodyType, SkeletalAnimationComponent, SpriteRendererComponent, TagComponent,
    TextComponent, TilemapComponent, TransformComponent, UIAnchorComponent, UIEvent,
    UIImageComponent, UIInteractableComponent, UIInteractionState, UILayoutAlignment,
    UILayoutComponent, UILayoutDirection, UIRectComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
#[cfg(feature = "physics-3d")]
pub use components::{
    BoxCollider3DComponent, CapsuleCollider3DComponent, RigidBody3DComponent, RigidBody3DType,
    SphereCollider3DComponent,
};
pub use entity::Entity;
pub use native_script::NativeScript;
#[cfg(feature = "physics-3d")]
pub use physics_3d::RaycastHit3D;
pub use scene_serializer::SceneSerializer;
#[cfg(feature = "lua-scripting")]
pub use script_engine::{ScriptEngine, ScriptFieldValue};
pub use spatial::{Aabb2D, Aabb3D, Frustum2D, Frustum3D, SpatialGrid, SpatialGrid3D};

use std::collections::HashMap;

use physics_2d::PhysicsWorld2D;
#[cfg(feature = "physics-3d")]
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
/// Wraps a [`SceneCore`] (the `Send + Sync` ECS core containing the
/// `hecs::World`, lookup caches, spatial grids, transform caches, timing,
/// and runtime settings) alongside `!Send` subsystems (physics, scripting,
/// audio).
///
/// Implements `Deref<Target = SceneCore>` and `DerefMut` so that all
/// `SceneCore` methods are callable directly on `Scene`.
pub struct Scene {
    /// Thread-safe ECS core.
    pub(crate) core: SceneCore,

    // -- !Send subsystems (physics, scripting, audio) --
    pub(super) physics_world: Option<PhysicsWorld2D>,
    #[cfg(feature = "physics-3d")]
    pub(super) physics_world_3d: Option<PhysicsWorld3D>,
    #[cfg(feature = "lua-scripting")]
    pub(super) script_engine: Option<ScriptEngine>,
    pub(super) audio_engine: Option<audio::AudioEngine>,
}

impl std::ops::Deref for Scene {
    type Target = SceneCore;
    #[inline]
    fn deref(&self) -> &SceneCore {
        &self.core
    }
}

impl std::ops::DerefMut for Scene {
    #[inline]
    fn deref_mut(&mut self) -> &mut SceneCore {
        &mut self.core
    }
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
            MeshRendererComponent,
            DirectionalLightComponent,
            PointLightComponent,
            AmbientLightComponent,
            EnvironmentComponent,
            UIAnchorComponent,
            UIRectComponent,
            UIImageComponent,
            UIInteractableComponent,
            UILayoutComponent,
            SkeletalAnimationComponent,
        );
        #[cfg(feature = "physics-3d")]
        $callback!(
            RigidBody3DComponent,
            BoxCollider3DComponent,
            SphereCollider3DComponent,
            CapsuleCollider3DComponent,
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
            // 3D colliders handled manually in editor for mesh-aware defaults.
        );
        #[cfg(feature = "physics-3d")]
        $callback!(
            ($crate::scene::RigidBody3DComponent, "Rigidbody 3D"),
        );
        $callback!(
            ($crate::scene::MeshRendererComponent, "Mesh Renderer"),
            (
                $crate::scene::DirectionalLightComponent,
                "Directional Light"
            ),
            ($crate::scene::PointLightComponent, "Point Light"),
            ($crate::scene::AmbientLightComponent, "Ambient Light"),
            ($crate::scene::EnvironmentComponent, "Environment Map"),
            ($crate::scene::UIAnchorComponent, "UI Anchor"),
            ($crate::scene::UIRectComponent, "UI Rect"),
            ($crate::scene::UIImageComponent, "UI Image"),
            ($crate::scene::UIInteractableComponent, "UI Interactable"),
            ($crate::scene::UILayoutComponent, "UI Layout"),
        );
    };
}

impl Scene {
    /// Create an empty scene.
    pub fn new() -> Self {
        Self {
            core: SceneCore::new(),
            physics_world: None,
            #[cfg(feature = "physics-3d")]
            physics_world_3d: None,
            #[cfg(feature = "lua-scripting")]
            script_engine: None,
            audio_engine: None,
        }
    }

    // -----------------------------------------------------------------
    // Deferred destruction (needs subsystem access)
    // -----------------------------------------------------------------

    /// Destroy all entities queued via [`queue_entity_destroy`](SceneCore::queue_entity_destroy).
    ///
    /// Cleans up physics bodies/colliders and script state for each destroyed
    /// entity. Safe to call even if the queue is empty.
    pub fn flush_pending_destroys(&mut self) {
        if self.core.pending_destroy.is_empty() {
            return;
        }

        // Deduplicate.
        let uuids: Vec<u64> = {
            let mut v = std::mem::take(&mut self.core.pending_destroy);
            v.sort_unstable();
            v.dedup();
            v
        };

        for uuid in uuids {
            if let Some(entity) = self.core.find_entity_by_uuid(uuid) {
                // Clean up script engine state (Lua environment, timers, coroutines)
                // before destroying the entity, so they don't fire on dead entities.
                #[cfg(feature = "lua-scripting")]
                if let Some(ref mut engine) = self.script_engine {
                    engine.remove_entity_timers(uuid);
                    engine.remove_entity_coroutines(uuid);
                    engine.remove_entity_env(uuid);
                }

                // Extract physics body handle before borrowing physics_world mutably.
                let body_handle = self
                    .core
                    .get_component::<RigidBody2DComponent>(entity)
                    .and_then(|rb| rb.runtime_body);

                if let (Some(handle), Some(ref mut physics)) =
                    (body_handle, &mut self.physics_world)
                {
                    // Clean up collider-to-UUID mappings before removing the body.
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
                #[cfg(feature = "physics-3d")]
                {
                    let body_handle_3d = self
                        .core
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
                }

                let _ = self.core.destroy_entity(entity);
            }
        }
    }

    // -----------------------------------------------------------------
    // Scene / Entity copying
    // -----------------------------------------------------------------

    /// Create a deep copy of the entire scene.
    ///
    /// All entities are recreated with their original UUIDs and all
    /// component data is cloned. Runtime-only handles (physics bodies,
    /// colliders) are reset to `None`. Script instances are not copied —
    /// they will be lazily re-instantiated on the first update.
    pub fn copy(source: &Scene) -> Scene {
        let mut new_scene = Scene::new();
        new_scene.core.viewport_width = source.core.viewport_width;
        new_scene.core.viewport_height = source.core.viewport_height;

        // Phase 1: Create entities with matching UUIDs.
        let mut entity_map: HashMap<hecs::Entity, Entity> = HashMap::new();

        let mut source_entities: Vec<_> = source
            .core
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
                $(copy_component_if_has::<$comp>(&source.core.world, &mut new_scene, &entity_map);)*
            };
        }
        for_each_cloneable_component!(copy_all);
        // NativeScriptComponent — manual copy (not Clone-able).
        for (handle, nsc) in source
            .core
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
        copy_component_if_has::<LuaScriptComponent>(
            &source.core.world,
            &mut new_scene,
            &entity_map,
        );

        // Copy audio volume settings.
        new_scene.core.master_volume = source.core.master_volume;
        new_scene.core.category_volumes = source.core.category_volumes;

        // Copy runtime settings state.
        *new_scene.core.fullscreen_mode.lock() = source.core.fullscreen_mode();
        *new_scene.core.gui_scale.lock() = source.core.gui_scale();

        // Copy script module search path.
        new_scene.core.script_module_search_path = source.core.script_module_search_path.clone();

        // Copy save data directory.
        new_scene.core.save_data_directory = source.core.save_data_directory.clone();

        // Copy loading screen color.
        *new_scene.core.loading_screen_color.lock() = *source.core.loading_screen_color.lock();

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
            .core
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
    #[test]
    fn for_each_cloneable_component_count() {
        let mut count = 0usize;
        macro_rules! count_types {
            ($($t:ty),* $(,)?) => {
                $(let _ = std::mem::size_of::<$t>(); count += 1;)*
            };
        }
        for_each_cloneable_component!(count_types);
        #[cfg(feature = "physics-3d")]
        let expected: usize = 31;
        #[cfg(not(feature = "physics-3d"))]
        let expected: usize = 27;
        assert_eq!(
            count, expected,
            "for_each_cloneable_component! has {} types but expected {}. \
             Update expected if you intentionally added/removed a component.",
            count, expected
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

    #[test]
    fn scene_core_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SceneCore>();
    }
}
