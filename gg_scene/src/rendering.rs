use super::{
    AmbientLightComponent, AnimationControllerComponent, CircleRendererComponent,
    DirectionalLightComponent, Entity, EnvironmentComponent, IdComponent, InstancedSpriteAnimator,
    MeshPrimitive, MeshRendererComponent, MeshSource, ParticleEmitterComponent,
    PointLightComponent, Scene, SkeletalAnimationComponent, SpriteAnimatorComponent,
    SpriteRendererComponent, TextComponent, TilemapComponent, TransformComponent,
    UIAnchorComponent, UIImageComponent, UIInteractableComponent, UIInteractionState,
    UIRectComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
#[cfg(feature = "physics-3d")]
use super::{RigidBody3DComponent, RigidBody3DType};
use gg_renderer::shadow_map::{
    compute_cascade_vps, compute_directional_light_vp, ShadowCameraInfo,
};
use gg_renderer::{Font, LightEnvironment, Mesh, Renderer, SubTexture2D};

/// Sort key for 2D renderable ordering. Sorted by layer, then sub-order,
/// then Z depth. `kind` discriminates the renderable type for batch flushing.
#[derive(Clone, Copy)]
struct RenderSortKey {
    sorting_layer: i32,
    order_in_layer: i32,
    z: f32,
    /// 0 = Sprite, 1 = Circle, 2 = Text, 3 = Tilemap, 4 = UIImage
    kind: u8,
    entity: hecs::Entity,
}

/// Pre-allocated buffers reused across frames to avoid per-frame heap allocations
/// in rendering and animation hot paths.
///
/// Held behind a `Mutex` on [`SceneCore`] so that `&self` render methods can
/// borrow and clear these buffers instead of allocating fresh `Vec`s every frame.
/// The mutex is uncontested (rendering is single-threaded) so lock cost is ~5 ns.
pub struct RenderBufferPool {
    /// 2D renderable sort keys (sprites, circles, text, tilemaps, UI images).
    sort_keys: Vec<RenderSortKey>,
    /// Sprite entity handles + sorting fields for frustum culling input.
    sprite_handles: Vec<(hecs::Entity, i32, i32)>,
    /// Circle renderable sort keys (kept separate before merge).
    circle_keys: Vec<RenderSortKey>,
    /// 3D mesh entity handles + world transforms.
    mesh_handles: Vec<(hecs::Entity, glam::Mat4)>,
    /// Skinned mesh entity handles + world transforms.
    skinned_handles: Vec<(hecs::Entity, glam::Mat4)>,
    /// Skinned mesh draw list (entity + world transform + bone offset).
    skinned_draw_list: Vec<(hecs::Entity, glam::Mat4, u32)>,
    /// Shadow pass mesh list (entity + world transform + alpha flag).
    shadow_meshes: Vec<(hecs::Entity, glam::Mat4, bool)>,
    /// Shadow pass skinned mesh list.
    shadow_skinned: Vec<(hecs::Entity, glam::Mat4)>,
    /// Shadow pass per-mesh bounding volumes.
    shadow_bounds: Vec<Option<super::spatial::Aabb3D>>,
    /// Shadow pass skinned draw data.
    shadow_skinned_draw: Vec<(hecs::Entity, glam::Mat4, u32)>,
    /// Animation work items for extract-process-writeback.
    anim_work: Vec<AnimWork>,
    /// Finished animation events (uuid, clip_name, default_clip).
    anim_finished: Vec<(u64, String, String)>,
    /// Animation controller transitions to apply.
    anim_transitions: Vec<(u64, String)>,
    /// Animation event callbacks (uuid, event_name, clip_name).
    anim_events: Vec<(u64, String, String)>,
}

impl Default for RenderBufferPool {
    fn default() -> Self {
        Self {
            sort_keys: Vec::new(),
            sprite_handles: Vec::new(),
            circle_keys: Vec::new(),
            mesh_handles: Vec::new(),
            skinned_handles: Vec::new(),
            skinned_draw_list: Vec::new(),
            shadow_meshes: Vec::new(),
            shadow_skinned: Vec::new(),
            shadow_bounds: Vec::new(),
            shadow_skinned_draw: Vec::new(),
            anim_work: Vec::new(),
            anim_finished: Vec::new(),
            anim_transitions: Vec::new(),
            anim_events: Vec::new(),
        }
    }
}

/// Per-entity animation work item for parallel extract-process-writeback.
struct AnimWork {
    entity: hecs::Entity,
    uuid: u64,
    frame_timer: f32,
    current_frame: u32,
    /// Frame value before tick — used to detect frame transitions for events.
    old_frame: u32,
    playing: bool,
    speed_scale: f32,
    clip_start_frame: u32,
    clip_end_frame: u32,
    clip_fps: f32,
    clip_looping: bool,
    clip_name: String,
    default_clip: String,
    finished: bool,
}

// ---------------------------------------------------------------------------
// Animation event helpers (free functions for borrowing ergonomics)
// ---------------------------------------------------------------------------

use super::animation::AnimationEvent;
use gg_renderer::skeleton::SkeletalAnimationEvent;

/// Collect sprite animation events for frames crossed between `old_frame` and
/// `new_frame`. Handles both forward progression and looping wrap-around.
///
/// Fires for each event whose `frame` was crossed during this tick
/// (exclusive of `old_frame`, inclusive of `new_frame`).
#[allow(clippy::too_many_arguments)]
fn collect_sprite_events(
    events: &[AnimationEvent],
    old_frame: u32,
    new_frame: u32,
    looping: bool,
    clip_start: u32,
    clip_end: u32,
    uuid: u64,
    clip_name: &str,
    out: &mut Vec<(u64, String, String)>,
) {
    for ev in events {
        let f = ev.frame;
        let hit = if new_frame > old_frame {
            // Normal forward progression.
            f > old_frame && f <= new_frame
        } else if new_frame < old_frame && looping {
            // Wrapped around (e.g., old=7 new=1 in range 0..7).
            f > old_frame && f <= clip_end || f >= clip_start && f <= new_frame
        } else {
            false
        };
        if hit {
            out.push((uuid, ev.name.clone(), clip_name.to_owned()));
        }
    }
}

/// Collect skeletal animation events for the time window `(old_time, new_time]`.
/// Handles looping wrap-around.
#[allow(clippy::too_many_arguments)]
fn collect_skeletal_events(
    events: &[SkeletalAnimationEvent],
    old_time: f32,
    new_time: f32,
    looping: bool,
    duration: f32,
    uuid: u64,
    clip_name: &str,
    out: &mut Vec<(u64, String, String)>,
) {
    for ev in events {
        let t = ev.time;
        let hit = if new_time > old_time {
            // Normal forward progression.
            t > old_time && t <= new_time
        } else if new_time < old_time && looping {
            // Wrapped around (old near end, new near start).
            t > old_time && t <= duration || t >= 0.0 && t <= new_time
        } else {
            false
        };
        if hit {
            out.push((uuid, ev.name.clone(), clip_name.to_owned()));
        }
    }
}

impl Scene {
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
    ///
    /// For scenes above [`PAR_THRESHOLD`](gg_core::jobs::parallel::PAR_THRESHOLD),
    /// the per-entity animation tick is parallelized via extract-process-writeback.
    pub fn on_update_animations(&mut self, dt: f32) {
        // Advance scene global time and store dt for Engine.delta_time().
        self.global_time += dt as f64;
        self.last_dt = dt;

        // Take reusable buffers from pool (avoids per-frame heap allocations).
        let mut pool = self.core.render_buffers.lock();
        let mut work = std::mem::take(&mut pool.anim_work);
        let mut finished_events = std::mem::take(&mut pool.anim_finished);
        let mut anim_events = std::mem::take(&mut pool.anim_events);
        drop(pool);

        work.clear();
        finished_events.clear();
        anim_events.clear();

        // Phase 1: extract + parallel tick SpriteAnimatorComponent timers.
        work.extend(
            self.world
                .query::<(hecs::Entity, &IdComponent, &SpriteAnimatorComponent)>()
                .iter()
                .filter(|(_, _, anim)| anim.playing)
                .map(|(entity, id, anim)| {
                    let clip = anim.current_clip_index().and_then(|i| anim.clips.get(i));
                    AnimWork {
                        entity,
                        uuid: id.id.raw(),
                        frame_timer: anim.frame_timer,
                        current_frame: anim.current_frame,
                        old_frame: anim.current_frame,
                        playing: true,
                        speed_scale: anim.speed_scale,
                        clip_start_frame: clip.map(|c| c.start_frame).unwrap_or(0),
                        clip_end_frame: clip.map(|c| c.end_frame).unwrap_or(0),
                        clip_fps: clip.map(|c| c.fps).unwrap_or(0.0),
                        clip_looping: clip.map(|c| c.looping).unwrap_or(true),
                        clip_name: clip.map(|c| c.name.clone()).unwrap_or_default(),
                        default_clip: anim.default_clip.clone(),
                        finished: false,
                    }
                }),
        );

        // Parallel tick (pure per-entity computation, no cross-entity deps).
        gg_core::jobs::parallel::par_for_each_mut(&mut work, |item| {
            if item.clip_fps <= 0.0 || item.speed_scale <= 0.0 {
                return;
            }
            item.frame_timer += dt * item.speed_scale;
            let frame_duration = 1.0 / item.clip_fps;
            while item.frame_timer >= frame_duration {
                item.frame_timer -= frame_duration;
                item.current_frame += 1;
                if item.current_frame > item.clip_end_frame {
                    if item.clip_looping {
                        item.current_frame = item.clip_start_frame;
                    } else {
                        item.current_frame = item.clip_end_frame;
                        item.playing = false;
                        item.finished = true;
                        break;
                    }
                }
            }
        });

        // Writeback + collect finished events + animation events.
        for item in &work {
            if let Ok(mut anim) = self.world.get::<&mut SpriteAnimatorComponent>(item.entity) {
                anim.frame_timer = item.frame_timer;
                anim.current_frame = item.current_frame;
                if !item.playing {
                    anim.playing = false;
                }
                if item.finished {
                    anim.finished_clip_name = None;
                    finished_events.push((
                        item.uuid,
                        item.clip_name.clone(),
                        item.default_clip.clone(),
                    ));
                }
                // Collect animation events for frames crossed this tick.
                if item.old_frame != item.current_frame {
                    if let Some(clip) = anim
                        .current_clip_index()
                        .and_then(|i| anim.clips.get(i))
                    {
                        if !clip.events.is_empty() {
                            collect_sprite_events(
                                &clip.events,
                                item.old_frame,
                                item.current_frame,
                                item.clip_looping,
                                item.clip_start_frame,
                                item.clip_end_frame,
                                item.uuid,
                                &item.clip_name,
                                &mut anim_events,
                            );
                        }
                    }
                }
            }
        }

        // Phase 2: check InstancedSpriteAnimator non-looping clips for completion
        // + collect animation events for frame transitions.
        let gt = self.global_time;
        let prev_gt = gt - dt as f64;
        for (id_comp, anim) in self
            .world
            .query_mut::<(&IdComponent, &mut InstancedSpriteAnimator)>()
        {
            if !anim.playing {
                continue;
            }
            // Collect animation events by comparing previous frame to current frame.
            if let Some(clip_idx) = anim.current_clip_index {
                if let Some(clip) = anim.clips.get(clip_idx) {
                    if !clip.events.is_empty() {
                        let prev_frame = anim.current_frame(prev_gt);
                        let curr_frame = anim.current_frame(gt);
                        if let (Some(pf), Some(cf)) = (prev_frame, curr_frame) {
                            if pf != cf {
                                collect_sprite_events(
                                    &clip.events,
                                    pf,
                                    cf,
                                    anim.looping,
                                    anim.start_frame,
                                    anim.start_frame + anim.frame_count.saturating_sub(1),
                                    id_comp.id.raw(),
                                    &clip.name,
                                    &mut anim_events,
                                );
                            }
                        }
                    }
                }
            }
            if anim.is_finished(gt) {
                let clip_name = anim.current_clip_name().unwrap_or("").to_owned();
                let default = anim.default_clip.clone();
                anim.playing = false;
                finished_events.push((id_comp.id.raw(), clip_name, default));
            }
        }

        // Phase 2b: advance skeletal animation playback times + blend state + events.
        for (id_comp, sac) in self
            .world
            .query_mut::<(&IdComponent, &mut SkeletalAnimationComponent)>()
        {
            if !sac.playing {
                continue;
            }
            // Advance the current clip, tracking old time for event detection.
            if let Some(clip_idx) = sac.current_clip {
                if let Some(clip) = sac.clips.get(clip_idx) {
                    let old_time = sac.playback_time;
                    let duration = clip.duration;
                    sac.playback_time += dt * sac.speed;
                    let looping = sac.looping;
                    if sac.playback_time >= duration {
                        if looping {
                            sac.playback_time %= duration;
                        } else {
                            sac.playback_time = duration;
                            sac.playing = false;
                        }
                    }
                    // Collect skeletal animation events for the time window.
                    if let Some(events) = sac.clip_events.get(&clip.name) {
                        if !events.is_empty() {
                            let uuid = id_comp.id.raw();
                            let clip_name = &clip.name;
                            collect_skeletal_events(
                                events,
                                old_time,
                                sac.playback_time,
                                looping,
                                duration,
                                uuid,
                                clip_name,
                                &mut anim_events,
                            );
                        }
                    }
                }
            }
            // Advance the crossfade blend (if active).
            if sac.blend_from_clip.is_some() && sac.blend_duration > 0.0 {
                // Advance the "from" clip playback so it doesn't freeze.
                if let Some(from_clip) = sac
                    .blend_from_clip
                    .and_then(|i| sac.clips.get(i))
                {
                    let from_dur = from_clip.duration;
                    sac.blend_from_time += dt * sac.speed;
                    if from_dur > 0.0 && sac.blend_from_time >= from_dur {
                        sac.blend_from_time %= from_dur;
                    }
                }
                // Advance blend timer (real-time, not speed-scaled).
                sac.blend_elapsed += dt;
                if sac.blend_elapsed >= sac.blend_duration {
                    sac.blend_from_clip = None;
                    sac.blend_elapsed = 0.0;
                    sac.blend_duration = 0.0;
                }
            }
        }

        if !finished_events.is_empty() {
            // Phase 3: dispatch Lua callbacks.
            #[cfg(feature = "lua-scripting")]
            self.dispatch_animation_finished_events(&finished_events);

            // Phase 4: transition to default clip for entities that have one.
            let gt = self.global_time;
            for (uuid, _, default_clip) in &finished_events {
                if default_clip.is_empty() {
                    continue;
                }
                if let Some(entity) = self.find_entity_by_uuid(*uuid) {
                    let has_sa = self.has_component::<SpriteAnimatorComponent>(entity);
                    if has_sa {
                        if let Some(mut animator) =
                            self.get_component_mut::<SpriteAnimatorComponent>(entity)
                        {
                            animator.play(default_clip);
                        }
                    } else if let Some(mut anim) =
                        self.get_component_mut::<InstancedSpriteAnimator>(entity)
                    {
                        anim.play_by_name(default_clip, gt);
                    }
                }
            }
        }

        // Phase 4b: dispatch animation event callbacks.
        if !anim_events.is_empty() {
            #[cfg(feature = "lua-scripting")]
            self.dispatch_animation_events(&anim_events);
        }

        // Phase 5: evaluate animation controllers.
        self.evaluate_animation_controllers();

        // Return buffers to pool for reuse next frame.
        let mut pool = self.core.render_buffers.lock();
        pool.anim_work = work;
        pool.anim_finished = finished_events;
        pool.anim_events = anim_events;
    }

    /// Evaluate all [`AnimationControllerComponent`]s and apply transitions.
    ///
    /// Checks each entity that has both a controller and an animator
    /// (sprite or skeletal). If a transition fires, plays the target clip.
    fn evaluate_animation_controllers(&mut self) {
        // Reuse pooled buffer for transitions.
        let mut pool = self.core.render_buffers.lock();
        let mut to_play = std::mem::take(&mut pool.anim_transitions);
        drop(pool);

        to_play.clear();

        // Sprite animators.
        for (id_comp, animator, ctrl) in self.world.query_mut::<(
            &IdComponent,
            &SpriteAnimatorComponent,
            &AnimationControllerComponent,
        )>() {
            let current = animator.current_clip_name();
            let finished = !animator.is_playing() && animator.current_clip_index().is_some();
            if let Some(target) = ctrl.evaluate(current, finished) {
                to_play.push((id_comp.id.raw(), target.to_owned()));
            }
        }

        for (uuid, target) in &to_play {
            if let Some(entity) = self.find_entity_by_uuid(*uuid) {
                if let Some(mut animator) =
                    self.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    animator.play(target);
                }
            }
        }

        // Skeletal animators (separate pass — entities may have skeletal but not sprite).
        to_play.clear();

        for (id_comp, sac, ctrl) in self.world.query_mut::<(
            &IdComponent,
            &SkeletalAnimationComponent,
            &AnimationControllerComponent,
        )>() {
            let current = sac.current_clip_name();
            let finished = !sac.playing && sac.current_clip.is_some();
            if let Some(target) = ctrl.evaluate(current, finished) {
                to_play.push((id_comp.id.raw(), target.to_owned()));
            }
        }

        for (uuid, target) in &to_play {
            if let Some(entity) = self.find_entity_by_uuid(*uuid) {
                if let Some(mut sac) =
                    self.get_component_mut::<SkeletalAnimationComponent>(entity)
                {
                    sac.play_by_name(&target);
                }
            }
        }

        // Return buffer to pool.
        let mut pool = self.core.render_buffers.lock();
        pool.anim_transitions = to_play;
    }

    /// Advance animations only for entities with `previewing` set (editor preview).
    ///
    /// Also advances `global_time` so instanced GPU animation works in edit mode.
    /// **Must not be called in the same frame as `on_update_animations`** to avoid
    /// double-incrementing `global_time`.
    pub fn on_update_animation_previews(&mut self, dt: f32) {
        self.global_time += dt as f64;
        for animator in self.world.query_mut::<&mut SpriteAnimatorComponent>() {
            if animator.previewing {
                animator.update(dt);
            }
        }
    }

    /// Dispatch `on_animation_finished(clip_name)` Lua callbacks.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_animation_finished_events(&mut self, events: &[(u64, String, String)]) {
        use super::lua_ops::ScriptEngineGuard;
        use super::script_glue::SceneScriptContext;

        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let scene_ptr: *mut Scene = self;
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for (uuid, clip_name, _) in events {
            guard.engine_mut().call_entity_callback_str(
                *uuid,
                "on_animation_finished",
                clip_name.clone(),
            );
        }

        // Guard drop restores engine and cleans up SceneScriptContext.
    }

    /// Dispatch `on_animation_event(event_name, clip_name)` Lua callbacks.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_animation_events(&mut self, events: &[(u64, String, String)]) {
        use super::lua_ops::ScriptEngineGuard;
        use super::script_glue::SceneScriptContext;

        let engine = match self.script_engine.take() {
            Some(e) => e,
            None => return,
        };

        let scene_ptr: *mut Scene = self;
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for (uuid, event_name, clip_name) in events {
            guard.engine_mut().call_entity_callback_str2(
                *uuid,
                "on_animation_event",
                event_name.clone(),
                clip_name.clone(),
            );
        }

        // Guard drop restores engine and cleans up SceneScriptContext.
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
        asset_manager: &mut gg_assets::EditorAssetManager,
        renderer: &Renderer,
    ) {
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::resolve_texture_handles");

        /// Collect entities with an unresolved texture handle and assign textures.
        macro_rules! resolve_textures_sync {
            ($world:expr, $asset_manager:expr, $renderer:expr, $Component:ty, $texture_field:ident) => {
                resolve_textures_sync!(
                    $world,
                    $asset_manager,
                    $renderer,
                    $Component,
                    $texture_field,
                    texture_handle
                )
            };
            ($world:expr, $asset_manager:expr, $renderer:expr, $Component:ty, $texture_field:ident, $handle_field:ident) => {{
                let needs: Vec<(hecs::Entity, gg_core::uuid::Uuid)> = $world
                    .query::<(hecs::Entity, &$Component)>()
                    .iter()
                    .filter_map(|(handle, comp)| {
                        if comp.$handle_field.raw() != 0 && comp.$texture_field.is_none() {
                            Some((handle, comp.$handle_field))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (handle, asset_handle) in needs {
                    $asset_manager.load_asset(&asset_handle, $renderer);
                    if let Some(texture) = $asset_manager.get_texture(&asset_handle) {
                        if let Ok(mut comp) = $world.get::<&mut $Component>(handle) {
                            comp.$texture_field = Some(texture);
                        }
                    }
                }
            }};
        }

        resolve_textures_sync!(
            self.world,
            asset_manager,
            renderer,
            SpriteRendererComponent,
            texture
        );
        resolve_textures_sync!(
            self.world,
            asset_manager,
            renderer,
            TilemapComponent,
            texture
        );
        resolve_textures_sync!(
            self.world,
            asset_manager,
            renderer,
            UIImageComponent,
            texture
        );
        resolve_textures_sync!(
            self.world,
            asset_manager,
            renderer,
            MeshRendererComponent,
            normal_texture,
            normal_texture_handle
        );

        // Resolve per-clip animator textures.
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
        asset_manager: &mut gg_assets::EditorAssetManager,
    ) {
        // Skip scanning all entities when everything is already resolved.
        if self.textures_all_resolved {
            return;
        }
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::resolve_texture_handles_async");

        let mut found_unresolved = false;

        /// Collect entities with an unresolved texture handle and assign or request load.
        macro_rules! resolve_textures_async {
            ($world:expr, $asset_manager:expr, $found:expr, $Component:ty, $texture_field:ident) => {
                resolve_textures_async!(
                    $world,
                    $asset_manager,
                    $found,
                    $Component,
                    $texture_field,
                    texture_handle
                )
            };
            ($world:expr, $asset_manager:expr, $found:expr, $Component:ty, $texture_field:ident, $handle_field:ident) => {{
                let needs: Vec<(hecs::Entity, gg_core::uuid::Uuid)> = $world
                    .query::<(hecs::Entity, &$Component)>()
                    .iter()
                    .filter_map(|(handle, comp)| {
                        if comp.$handle_field.raw() != 0 && comp.$texture_field.is_none() {
                            Some((handle, comp.$handle_field))
                        } else {
                            None
                        }
                    })
                    .collect();

                $found |= !needs.is_empty();
                for (handle, asset_handle) in needs {
                    if let Some(texture) = $asset_manager.get_texture(&asset_handle) {
                        if let Ok(mut comp) = $world.get::<&mut $Component>(handle) {
                            comp.$texture_field = Some(texture);
                        }
                    } else {
                        $asset_manager.request_load(&asset_handle);
                    }
                }
            }};
        }

        resolve_textures_async!(
            self.world,
            asset_manager,
            found_unresolved,
            SpriteRendererComponent,
            texture
        );
        resolve_textures_async!(
            self.world,
            asset_manager,
            found_unresolved,
            TilemapComponent,
            texture
        );
        resolve_textures_async!(
            self.world,
            asset_manager,
            found_unresolved,
            MeshRendererComponent,
            texture
        );
        resolve_textures_async!(
            self.world,
            asset_manager,
            found_unresolved,
            MeshRendererComponent,
            normal_texture,
            normal_texture_handle
        );
        resolve_textures_async!(
            self.world,
            asset_manager,
            found_unresolved,
            UIImageComponent,
            texture
        );

        // Resolve per-clip animator textures.
        found_unresolved |= self.resolve_animator_clip_textures(asset_manager, None);

        if !found_unresolved {
            self.textures_all_resolved = true;
        }
    }

    /// Resolve per-clip texture handles in all [`SpriteAnimatorComponent`]s.
    ///
    /// If `renderer` is `Some`, uses synchronous `load_asset`; otherwise
    /// uses `request_load` for async loading.
    ///
    /// Returns `true` if any clips had unresolved textures.
    fn resolve_animator_clip_textures(
        &mut self,
        asset_manager: &mut gg_assets::EditorAssetManager,
        renderer: Option<&Renderer>,
    ) -> bool {
        let mut had_unresolved = false;

        // Collect (entity, clip_index, handle) for SpriteAnimatorComponent clips.
        let needs: Vec<(hecs::Entity, usize, gg_core::uuid::Uuid)> = self
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

        had_unresolved |= !needs.is_empty();
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

        // Collect for InstancedSpriteAnimator clips.
        let instanced_needs: Vec<(hecs::Entity, usize, gg_core::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &InstancedSpriteAnimator)>()
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

        had_unresolved |= !instanced_needs.is_empty();
        for (entity, clip_idx, asset_handle) in instanced_needs {
            if let Some(r) = renderer {
                asset_manager.load_asset(&asset_handle, r);
            }
            if let Some(texture) = asset_manager.get_texture(&asset_handle) {
                if let Ok(mut animator) = self.world.get::<&mut InstancedSpriteAnimator>(entity) {
                    if let Some(clip) = animator.clips.get_mut(clip_idx) {
                        clip.texture = Some(texture);
                    }
                }
            } else if renderer.is_none() {
                asset_manager.request_load(&asset_handle);
            }
        }
        had_unresolved
    }

    /// Async variant of [`load_fonts`](Self::load_fonts).
    ///
    /// For text components with unresolved fonts:
    /// - If the font is already cached in the asset manager, assigns it immediately.
    /// - Otherwise, requests an async background load (non-blocking).
    ///
    /// On subsequent frames, `poll_loaded` will upload completed fonts,
    /// and this method will find them in the cache and assign them.
    pub fn load_fonts_async(&mut self, asset_manager: &mut gg_assets::EditorAssetManager) {
        let needs: Vec<(hecs::Entity, std::path::PathBuf)> = self
            .world
            .query::<(hecs::Entity, &TextComponent)>()
            .iter()
            .filter_map(|(handle, tc)| {
                if tc.font.is_none() && !tc.font_path.is_empty() {
                    let path = std::path::PathBuf::from(&tc.font_path);
                    if path.exists() {
                        Some((handle, path))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        for (handle, path) in needs {
            if let Some(font) = asset_manager.get_font(&path) {
                if let Ok(mut tc) = self.world.get::<&mut TextComponent>(handle) {
                    tc.font = Some(font);
                }
            } else {
                asset_manager.request_font_load(path);
            }
        }
    }

    /// Load fonts for all [`TextComponent`]s that have a `font_path` set
    /// but no loaded font. Similar to [`resolve_texture_handles`](Self::resolve_texture_handles).
    pub fn load_fonts(&mut self, renderer: &Renderer) {
        use std::collections::HashMap;
        use std::path::PathBuf;
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::load_fonts");

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

        let mut cache: HashMap<PathBuf, gg_core::Ref<Font>> = HashMap::new();
        for (handle, path) in loads {
            if let Some(font) = cache.get(&path).cloned().or_else(|| {
                let f = gg_core::Ref::new(renderer.create_font(&path)?);
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
    // Mesh uploading
    // -----------------------------------------------------------------

    /// Resolve mesh asset references: assigns cached CPU mesh data from the
    /// asset manager to entities with [`MeshSource::Asset`] that don't have
    /// it yet. Also enqueues async loads for missing mesh assets.
    pub fn resolve_mesh_assets(&mut self, asset_manager: &mut gg_assets::EditorAssetManager) {
        let needs: Vec<(hecs::Entity, gg_core::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &MeshRendererComponent)>()
            .iter()
            .filter_map(|(handle, mc)| {
                if let MeshSource::Asset(mesh_handle) = &mc.mesh_source {
                    if mesh_handle.raw() != 0 && mc.loaded_mesh.is_none() {
                        return Some((handle, *mesh_handle));
                    }
                }
                None
            })
            .collect();

        for (handle, mesh_handle) in needs {
            if let Some(mesh_ref) = asset_manager.get_mesh(&mesh_handle) {
                if let Ok(mut mc) = self.world.get::<&mut MeshRendererComponent>(handle) {
                    mc.loaded_mesh = Some(mesh_ref);
                }
            } else {
                asset_manager.request_mesh_load(&mesh_handle);
            }
        }
    }

    /// Upload vertex arrays for any [`MeshRendererComponent`] that doesn't
    /// have one yet. Handles both primitive and asset mesh sources.
    pub fn resolve_meshes(&mut self, renderer: &mut Renderer) {
        use crate::components::MeshSource;

        let needs: Vec<(hecs::Entity, MeshSource, [f32; 4])> = self
            .world
            .query::<(hecs::Entity, &MeshRendererComponent)>()
            .iter()
            .filter_map(|(handle, mesh_comp)| {
                if mesh_comp.vertex_array.is_none() {
                    // For asset meshes, only proceed if CPU data is loaded.
                    if let MeshSource::Asset(_) = &mesh_comp.mesh_source {
                        mesh_comp.loaded_mesh.as_ref()?;
                    }
                    Some((
                        handle,
                        mesh_comp.mesh_source.clone(),
                        mesh_comp.color.into(),
                    ))
                } else {
                    None
                }
            })
            .collect();

        for (handle, source, color) in needs {
            let (mesh_to_upload, bounds) = match &source {
                MeshSource::Primitive(primitive) => {
                    let m = match primitive {
                        MeshPrimitive::Cube => Mesh::cube(color),
                        MeshPrimitive::Sphere => Mesh::sphere(32, 16, color),
                        MeshPrimitive::Plane => Mesh::plane(color),
                        MeshPrimitive::Cylinder => Mesh::cylinder(32, color),
                        MeshPrimitive::Cone => Mesh::cone(32, color),
                        MeshPrimitive::Torus => Mesh::torus(32, 16, color),
                        MeshPrimitive::Capsule => Mesh::capsule(32, 16, color),
                    };
                    (m, None)
                }
                MeshSource::Asset(_) => {
                    // loaded_mesh is guaranteed Some by the filter above.
                    let mc = self.world.get::<&MeshRendererComponent>(handle).unwrap();
                    let mesh_ref = mc.loaded_mesh.as_ref().unwrap().clone();
                    drop(mc); // Release borrow.
                    let bounds = mesh_ref.compute_bounds();
                    // Clone CPU data for upload — the Ref<Mesh> stays cached.
                    let m = Mesh {
                        vertices: mesh_ref.vertices.clone(),
                        indices: mesh_ref.indices.clone(),
                        name: mesh_ref.name.clone(),
                    };
                    (m, Some(bounds))
                }
            };

            match mesh_to_upload.upload(renderer) {
                Ok(va) => {
                    if let Ok(mut comp) = self.world.get::<&mut MeshRendererComponent>(handle) {
                        comp.vertex_array = Some(va);
                        if let Some(b) = bounds {
                            comp.local_bounds = Some(b);
                        }
                    }
                }
                Err(e) => {
                    log::error!("Failed to upload mesh: {e}");
                }
            }
        }
    }

    /// Re-upload the vertex array for a mesh component when its source,
    /// primitive, or color changes. Clears the existing VA so the next
    /// `resolve_meshes` call picks it up. The old VA is moved to a
    /// deferred-destroy queue to avoid destroying GPU buffers still
    /// referenced by in-flight command buffers.
    pub fn invalidate_mesh(&mut self, entity: super::Entity) {
        if let Ok(mut comp) = self
            .core
            .world
            .get::<&mut MeshRendererComponent>(entity.handle())
        {
            // Clear loaded mesh CPU data so it gets re-resolved.
            comp.loaded_mesh = None;
            comp.local_bounds = None;
            if let Some(old_va) = comp.vertex_array.take() {
                // Defer destruction: the old buffers may still be in use by
                // a previously submitted command buffer.
                if self.core.va_graveyard.is_empty() {
                    self.core.va_graveyard.push_back(Vec::new());
                }
                self.core.va_graveyard.back_mut().unwrap().push(old_va);
            }
        }
    }

    /// Load skinned mesh data from the asset manager for any
    /// [`SkeletalAnimationComponent`] that has a `mesh_asset` set but
    /// hasn't loaded its skeleton/clips/mesh yet.
    pub fn resolve_skinned_mesh_assets(
        &mut self,
        asset_manager: &mut gg_assets::EditorAssetManager,
    ) {
        let needs: Vec<(hecs::Entity, gg_core::uuid::Uuid)> = self
            .world
            .query::<(hecs::Entity, &SkeletalAnimationComponent)>()
            .iter()
            .filter_map(|(handle, sac)| {
                if sac.mesh_asset.raw() != 0 && !sac.is_loaded() {
                    Some((handle, sac.mesh_asset))
                } else {
                    None
                }
            })
            .collect();

        for (handle, mesh_handle) in needs {
            if let Some(skin_data) = asset_manager.get_skinned_mesh(&mesh_handle) {
                if let Ok(mut sac) = self.world.get::<&mut SkeletalAnimationComponent>(handle) {
                    sac.skeleton = gg_core::Ref::new(skin_data.skeleton.clone());
                    sac.clips = skin_data.clips.clone();
                    sac.loaded_skinned_mesh = Some(gg_core::Ref::new(skin_data.mesh.clone()));
                    if !sac.clips.is_empty() && sac.current_clip.is_none() {
                        sac.current_clip = Some(0);
                        sac.playing = true;
                    }
                }
            } else {
                asset_manager.request_skinned_mesh_load(&mesh_handle);
            }
        }
    }

    /// Upload vertex arrays for any [`SkeletalAnimationComponent`] that doesn't
    /// have one yet. Called once per frame before rendering.
    pub fn resolve_skinned_meshes(&mut self, renderer: &mut Renderer) {
        let needs: Vec<hecs::Entity> = self
            .world
            .query::<(hecs::Entity, &SkeletalAnimationComponent)>()
            .iter()
            .filter_map(|(handle, sac)| {
                if sac.skinned_vertex_array.is_none() && sac.loaded_skinned_mesh.is_some() {
                    Some(handle)
                } else {
                    None
                }
            })
            .collect();

        for handle in needs {
            let mesh_ref = {
                let sac = self
                    .world
                    .get::<&SkeletalAnimationComponent>(handle)
                    .unwrap();
                sac.loaded_skinned_mesh.as_ref().unwrap().clone()
            };
            match mesh_ref.upload(renderer) {
                Ok(va) => {
                    if let Ok(mut sac) = self.world.get::<&mut SkeletalAnimationComponent>(handle) {
                        sac.skinned_vertex_array = Some(va);
                    }
                }
                Err(e) => {
                    log::error!("Failed to upload skinned mesh: {e}");
                }
            }
        }
    }

    /// Rotate the deferred-destroy queue: push a new frame entry and drop
    /// entries older than `MAX_FRAMES_IN_FLIGHT`. Called once per frame
    /// after the GPU fence wait guarantees old command buffers have completed.
    pub fn rotate_va_graveyard(&mut self) {
        use gg_renderer::MAX_FRAMES_IN_FLIGHT;
        self.va_graveyard.push_back(Vec::new());
        while self.va_graveyard.len() > MAX_FRAMES_IN_FLIGHT {
            self.va_graveyard.pop_front(); // Drop old VAs — GPU is done with them.
        }
    }

    /// Invalidate the texture resolution cache so the next call to
    /// `resolve_texture_handles_async` re-scans all entities.
    /// Call when a texture handle is changed (e.g. from the editor properties panel).
    pub fn invalidate_texture_cache(&mut self) {
        self.textures_all_resolved = false;
    }

    /// Clear all runtime texture references (`Option<Ref<Texture2D>>`) for
    /// components whose `texture_handle` matches the given asset handle.
    ///
    /// This forces `resolve_texture_handles_async` to re-resolve them from the
    /// asset cache on the next frame, picking up the newly-loaded version after
    /// a hot-reload.
    pub fn clear_texture_refs_for_handle(&mut self, asset_handle: gg_core::uuid::Uuid) {
        // Sprite renderers
        for comp in self.world.query::<&mut SpriteRendererComponent>().iter() {
            if comp.texture_handle == asset_handle {
                comp.texture = None;
            }
        }
        // Tilemaps
        for comp in self.world.query::<&mut TilemapComponent>().iter() {
            if comp.texture_handle == asset_handle {
                comp.texture = None;
            }
        }
        // 3D meshes (albedo + normal)
        for comp in self.world.query::<&mut MeshRendererComponent>().iter() {
            if comp.texture_handle == asset_handle {
                comp.texture = None;
            }
            if comp.normal_texture_handle == asset_handle {
                comp.normal_texture = None;
            }
        }
        // UI images
        for comp in self.world.query::<&mut UIImageComponent>().iter() {
            if comp.texture_handle == asset_handle {
                comp.texture = None;
            }
        }
        // Sprite animator clips
        for animator in self.world.query::<&mut SpriteAnimatorComponent>().iter() {
            for clip in &mut animator.clips {
                if clip.texture_handle == asset_handle {
                    clip.texture = None;
                }
            }
        }
        // Instanced sprite animator clips
        for animator in self.world.query::<&mut InstancedSpriteAnimator>().iter() {
            for clip in &mut animator.clips {
                if clip.texture_handle == asset_handle {
                    clip.texture = None;
                }
            }
        }

        self.textures_all_resolved = false;
    }

    /// Resolve the scene's [`EnvironmentComponent`] HDR asset handle by loading
    /// the environment map into the GPU if not already loaded.
    ///
    /// `asset_manager` provides the file path for the asset handle.
    /// Call once per frame (like `resolve_texture_handles_async`).
    pub fn resolve_environment_map(
        &mut self,
        renderer: &mut Renderer,
        asset_manager: &gg_assets::EditorAssetManager,
    ) {
        // Find the first EnvironmentComponent that has a handle but isn't loaded.
        let needs_load: Option<(hecs::Entity, u64)> = self
            .world
            .query::<(hecs::Entity, &EnvironmentComponent)>()
            .iter()
            .find_map(|(handle, ec)| {
                if ec.environment_handle != 0 && !ec.loaded {
                    Some((handle, ec.environment_handle))
                } else {
                    None
                }
            });

        if let Some((entity, asset_handle)) = needs_load {
            let uuid = gg_core::uuid::Uuid::from_raw(asset_handle);
            let path = asset_manager
                .get_metadata(&uuid)
                .map(|m| asset_manager.asset_directory().join(&m.file_path));
            if let Some(path) = path {
                match renderer.load_environment_hdr_from_file(&path) {
                    Ok(()) => {
                        if let Ok(mut ec) = self.world.get::<&mut EnvironmentComponent>(entity) {
                            ec.loaded = true;
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to load environment map: {e}");
                        // Mark loaded to prevent retry spam.
                        if let Ok(mut ec) = self.world.get::<&mut EnvironmentComponent>(entity) {
                            ec.loaded = true;
                        }
                    }
                }
            }
        }
    }

    /// Resolve the scene's [`EnvironmentComponent`] from a direct file path.
    ///
    /// Used by the player binary (no asset manager — paths resolved from project).
    pub fn resolve_environment_map_by_path(
        &mut self,
        renderer: &mut Renderer,
        assets_root: &std::path::Path,
    ) {
        let needs_load: Option<(hecs::Entity, u64)> = self
            .world
            .query::<(hecs::Entity, &EnvironmentComponent)>()
            .iter()
            .find_map(|(handle, ec)| {
                if ec.environment_handle != 0 && !ec.loaded {
                    Some((handle, ec.environment_handle))
                } else {
                    None
                }
            });

        if let Some((entity, _asset_handle)) = needs_load {
            // Try to find the HDR file in assets/hdri/.
            let hdri_dir = assets_root.join("hdri");
            if hdri_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&hdri_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().and_then(|e| e.to_str()) == Some("hdr") {
                            match renderer.load_environment_hdr_from_file(&p) {
                                Ok(()) => {
                                    if let Ok(mut ec) =
                                        self.world.get::<&mut EnvironmentComponent>(entity)
                                    {
                                        ec.loaded = true;
                                    }
                                    return;
                                }
                                Err(e) => {
                                    log::error!("Failed to load environment map: {e}");
                                }
                            }
                        }
                    }
                }
            }
            // Mark loaded to prevent retry.
            if let Ok(mut ec) = self.world.get::<&mut EnvironmentComponent>(entity) {
                ec.loaded = true;
            }
        }
    }

    // -----------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------

    /// Draw all renderable entities sorted by (sorting_layer, order_in_layer, z).
    ///
    /// Shared rendering code used by editor, simulation, and runtime paths.
    /// The caller is responsible for setting the view-projection matrix on
    /// the renderer before calling this.
    fn render_scene(&self, renderer: &mut Renderer) {
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::render_scene");

        // Write scene time to the camera UBO for GPU-computed animation.
        // Both u_time and per-instance start_time are rebased by the same
        // epoch (largest whole hour ≤ global_time) before f64→f32 cast,
        // preserving sub-ms precision even after many hours of runtime.
        let gpu_time_epoch = (self.global_time / 3600.0).floor() * 3600.0;
        renderer.set_scene_time((self.global_time - gpu_time_epoch) as f32);

        // Pre-compute world transforms for all entities once.
        {
            gg_core::profile_scope!("Scene::build_world_transform_cache");
            self.build_world_transform_cache();
        }
        // Render 3D meshes first so they appear behind 2D sprites/particles.
        self.render_meshes(renderer);
        self.render_skinned_meshes(renderer);

        // Render skybox after opaque 3D geometry (uses reverse-Z GREATER_OR_EQUAL
        // depth test — only fills pixels not covered by geometry).
        if renderer.has_environment_map() {
            if let Some(ec) = self.world.query::<&EnvironmentComponent>().iter().next() {
                if ec.loaded && ec.show_skybox {
                    let view = renderer.camera_view();
                    let proj = renderer.camera_projection();
                    let offscreen = renderer.is_offscreen();
                    if let Err(e) = renderer.render_skybox(
                        view,
                        proj,
                        ec.skybox_exposure,
                        ec.skybox_rotation,
                        offscreen,
                    ) {
                        log::error!("Skybox render failed: {e}");
                    }
                }
            }
        }

        let wt_ref = self.transform_cache.read();
        let wt_cache = &*wt_ref;

        // Extract frustum half-planes for entity-level frustum culling.
        let vp = renderer.view_projection();
        let frustum = super::spatial::Frustum2D::from_view_projection(&vp);
        let gui_scale = self.gui_scale();

        // Pixels-per-world-unit for UIRect/9-slice corner sizing.
        let ppwu = {
            let mut val = 1.0f32;
            for cam in self.world.query::<&super::CameraComponent>().iter() {
                if cam.primary {
                    let ortho_size = cam.camera.orthographic_size();
                    if ortho_size > 0.0 && self.viewport_height > 0 {
                        val = self.viewport_height as f32 / ortho_size;
                    }
                    break;
                }
            }
            val
        };

        // Take reusable buffers from pool (avoids per-frame heap allocations).
        let mut pool = self.render_buffers.lock();
        let mut sprites = std::mem::take(&mut pool.sprite_handles);
        let mut renderables = std::mem::take(&mut pool.sort_keys);
        let mut circle_renderables = std::mem::take(&mut pool.circle_keys);
        drop(pool);

        sprites.clear();
        renderables.clear();
        circle_renderables.clear();

        // Collect all renderable entities with sort keys.
        // Sprites and circles are frustum-culled via AABB overlap test.
        // Text is not culled (bounds depend on string content).
        // Tilemaps are not culled here (tile-level culling happens during rendering).
        // 0 = Sprite, 1 = Circle, 2 = Text, 3 = Tilemap
        let mut total_cullable: u32 = 0;
        let mut culled: u32 = 0;

        // --- Parallel frustum culling for sprites ---
        sprites.extend(
            self.world
                .query::<(hecs::Entity, &SpriteRendererComponent)>()
                .iter()
                .map(|(h, s)| (h, s.sorting_layer, s.order_in_layer)),
        );
        total_cullable += sprites.len() as u32;

        {
            use gg_core::jobs::parallel::PAR_THRESHOLD;
            let make_key =
                |handle: hecs::Entity, sorting_layer: i32, order_in_layer: i32, wt: &glam::Mat4| {
                    RenderSortKey {
                        sorting_layer,
                        order_in_layer,
                        z: wt.w_axis.z,
                        kind: 0,
                        entity: handle,
                    }
                };
            if sprites.len() >= PAR_THRESHOLD {
                use rayon::prelude::*;
                // Rayon's par_iter().filter_map().collect() allocates internally;
                // extend renderables from the result to reuse the renderables buffer.
                let par_result: Vec<RenderSortKey> = gg_core::jobs::pool().install(|| {
                    sprites
                        .par_iter()
                        .filter_map(|&(handle, sorting_layer, order_in_layer)| {
                            let wt = wt_cache.get(&handle)?;
                            let aabb = super::spatial::Aabb2D::from_unit_quad_transform(wt);
                            if !frustum.contains_aabb(&aabb) {
                                return None;
                            }
                            Some(make_key(handle, sorting_layer, order_in_layer, wt))
                        })
                        .collect()
                });
                culled += sprites.len() as u32 - par_result.len() as u32;
                renderables.extend_from_slice(&par_result);
            } else {
                let before = renderables.len();
                renderables.extend(
                    sprites
                        .iter()
                        .filter_map(|&(handle, sorting_layer, order_in_layer)| {
                            let wt = wt_cache.get(&handle)?;
                            let aabb = super::spatial::Aabb2D::from_unit_quad_transform(wt);
                            if !frustum.contains_aabb(&aabb) {
                                return None;
                            }
                            Some(make_key(handle, sorting_layer, order_in_layer, wt))
                        }),
                );
                culled += sprites.len() as u32 - (renderables.len() - before) as u32;
            }
        }

        // --- Circles (usually few, keep sequential) ---
        for (handle, circle) in self
            .world
            .query::<(hecs::Entity, &CircleRendererComponent)>()
            .iter()
        {
            let Some(wt) = wt_cache.get(&handle) else {
                continue;
            };
            total_cullable += 1;
            let aabb = super::spatial::Aabb2D::from_unit_quad_transform(wt);
            if !frustum.contains_aabb(&aabb) {
                culled += 1;
                continue;
            }
            circle_renderables.push(RenderSortKey {
                sorting_layer: circle.sorting_layer,
                order_in_layer: circle.order_in_layer,
                z: wt.w_axis.z,
                kind: 1,
                entity: handle,
            });
        }

        *self.culling_stats.lock() = super::CullingStats {
            total_cullable,
            rendered: total_cullable - culled,
            culled,
        };

        // --- Text & tilemaps (sequential, usually few) ---
        renderables.extend(circle_renderables.iter().copied());

        for (handle, text) in self.world.query::<(hecs::Entity, &TextComponent)>().iter() {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push(RenderSortKey {
                sorting_layer: text.sorting_layer,
                order_in_layer: text.order_in_layer,
                z,
                kind: 2,
                entity: handle,
            });
        }

        for (handle, tilemap) in self
            .world
            .query::<(hecs::Entity, &TilemapComponent)>()
            .iter()
        {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push(RenderSortKey {
                sorting_layer: tilemap.sorting_layer,
                order_in_layer: tilemap.order_in_layer,
                z,
                kind: 3,
                entity: handle,
            });
        }

        // --- UIImage entities ---
        for (handle, img) in self
            .world
            .query::<(hecs::Entity, &UIImageComponent)>()
            .iter()
        {
            let z = wt_cache.get(&handle).map(|m| m.w_axis.z).unwrap_or(0.0);
            renderables.push(RenderSortKey {
                sorting_layer: img.sorting_layer,
                order_in_layer: img.order_in_layer,
                z,
                kind: 4,
                entity: handle,
            });
        }

        // --- Parallel sort ---
        let sort_cmp = |a: &RenderSortKey, b: &RenderSortKey| {
            a.sorting_layer
                .cmp(&b.sorting_layer)
                .then(a.order_in_layer.cmp(&b.order_in_layer))
                .then(a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal))
        };
        if renderables.len() >= gg_core::jobs::parallel::PAR_THRESHOLD {
            use rayon::prelude::*;
            gg_core::jobs::pool().install(|| renderables.par_sort_by(sort_cmp));
        } else {
            renderables.sort_by(sort_cmp);
        }

        // Render in sorted order.
        // Flush all pending batches when the renderable type changes so that
        // cross-type draw ordering (e.g. text behind a sprite) is respected.
        let mut prev_kind: u8 = u8::MAX;
        for &RenderSortKey {
            kind,
            entity: handle,
            ..
        } in &renderables
        {
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
                    let Ok(sprite) = self.world.get::<&SpriteRendererComponent>(handle) else {
                        continue;
                    };

                    // GPU animation path: InstancedSpriteAnimator with active playback.
                    // The vertex shader computes UVs from animation params + u_time.
                    let gpu_animated = self
                        .world
                        .get::<&InstancedSpriteAnimator>(handle)
                        .ok()
                        .filter(|anim| anim.playing && anim.frame_count > 0)
                        .and_then(|anim| {
                            let texture =
                                anim.current_clip_texture().or(sprite.texture.as_ref())?;
                            let tex_idx = texture.bindless_index() as f32;
                            let tw = texture.width() as f32;
                            let th = texture.height() as f32;
                            Some((
                                tex_idx,
                                (anim.start_time - gpu_time_epoch) as f32,
                                anim.effective_fps() as f32,
                                anim.start_frame as f32,
                                anim.frame_count as f32,
                                anim.columns as f32,
                                if anim.looping { 1.0f32 } else { 0.0 },
                                [anim.cell_size.x, anim.cell_size.y],
                                [tw, th],
                            ))
                        });

                    if let Some((
                        tex_idx,
                        start_time,
                        fps,
                        start_frame,
                        frame_count,
                        columns,
                        looping,
                        cell_size,
                        tex_size,
                    )) = gpu_animated
                    {
                        renderer.draw_gpu_animated_sprite(
                            &world_transform,
                            sprite.color,
                            tex_idx,
                            handle.id() as i32,
                            start_time,
                            fps,
                            start_frame,
                            frame_count,
                            columns,
                            looping,
                            cell_size,
                            tex_size,
                        );
                    } else {
                        // CPU animation path: SpriteAnimatorComponent (per-entity timers).
                        let animated = self
                            .world
                            .get::<&SpriteAnimatorComponent>(handle)
                            .ok()
                            .and_then(|anim| {
                                let (col, row) = anim.current_grid_coords()?;
                                let texture =
                                    anim.current_clip_texture().or(sprite.texture.as_ref())?;
                                Some(SubTexture2D::from_coords(
                                    texture,
                                    glam::Vec2::new(col as f32, row as f32),
                                    anim.cell_size,
                                    glam::Vec2::ONE,
                                ))
                            });

                        // Stopped InstancedSpriteAnimator: compute last frame UVs on CPU.
                        let instanced_anim = if animated.is_none() {
                            self.world
                                .get::<&InstancedSpriteAnimator>(handle)
                                .ok()
                                .and_then(|anim| {
                                    let gt = self.global_time;
                                    let (col, row) = anim.current_grid_coords(gt)?;
                                    let texture =
                                        anim.current_clip_texture().or(sprite.texture.as_ref())?;
                                    Some(SubTexture2D::from_coords(
                                        texture,
                                        glam::Vec2::new(col as f32, row as f32),
                                        anim.cell_size,
                                        glam::Vec2::ONE,
                                    ))
                                })
                        } else {
                            None
                        };

                        let sub_tex = animated.or(instanced_anim);
                        if let Some(sub_tex) = sub_tex {
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
                                renderer.draw_sprite(&world_transform, sprite.texture.as_deref(), sprite.color, sprite.tiling_factor, handle.id() as i32);
                            }
                        } else {
                            renderer.draw_sprite(&world_transform, sprite.texture.as_deref(), sprite.color, sprite.tiling_factor, handle.id() as i32);
                        }
                    }
                }
                1 => {
                    // Circle
                    let Ok(circle) = self.world.get::<&CircleRendererComponent>(handle) else {
                        continue;
                    };
                    renderer.draw_circle(&world_transform, circle.color, circle.thickness, circle.fade, handle.id() as i32);
                }
                2 => {
                    // Text — apply GUI scale to font size for UI-anchored entities.
                    let Ok(text) = self.world.get::<&TextComponent>(handle) else {
                        continue;
                    };
                    // Text inside a UIRect: font_size is in UI points (matching
                    // the rect coordinate system). Convert to world units, then
                    // render centered inside the rect, ignoring the rect's
                    // non-uniform scale.
                    if self.world.get::<&UIRectComponent>(handle).is_ok() {
                        if let Some(font) = &text.font {
                            // UI points → world units
                            let wup = if ppwu > 0.0 { gui_scale / ppwu } else { 1.0 };
                            let text_font_size = text.font_size * wup;

                            // Measure text bounds for centering.
                            let (text_w, text_h) = font.measure_text(
                                &text.text,
                                text_font_size,
                                text.line_spacing,
                                text.kerning,
                            );

                            // Rect center is the entity's world position (after
                            // apply_ui_anchors with pivot offset). The rect's
                            // world dimensions are encoded in the transform scale.
                            let rect_center = glam::Vec3::new(
                                world_transform.w_axis.x,
                                world_transform.w_axis.y,
                                world_transform.w_axis.z,
                            );
                            let _rect_world_w = world_transform.x_axis.truncate().length();
                            let _rect_world_h = world_transform.y_axis.truncate().length();

                            // Vertical centre helper: offset from text origin
                            // (baseline) to the visual centre of a single line.
                            let v_center = font.text_vertical_center(text_font_size);

                            // Position text so its bounding box is centred in
                            // the rect. draw_text_string starts at the transform
                            // origin and advances rightward / downward.
                            let text_origin = glam::Vec3::new(
                                rect_center.x - text_w * 0.5,
                                rect_center.y + text_h * 0.5 - v_center,
                                rect_center.z,
                            );

                            let text_transform = glam::Mat4::from_translation(text_origin);

                            renderer.draw_text_string(
                                &text.text,
                                &text_transform,
                                font,
                                text_font_size,
                                text.color,
                                text.line_spacing,
                                text.kerning,
                                handle.id() as i32,
                            );
                        }
                    } else if gui_scale != 1.0
                        && self.world.get::<&UIAnchorComponent>(handle).is_ok()
                    {
                        if let Some(font) = &text.font {
                            renderer.draw_text_string(
                                &text.text,
                                &world_transform,
                                font,
                                text.font_size * gui_scale,
                                text.color,
                                text.line_spacing,
                                text.kerning,
                                handle.id() as i32,
                            );
                        }
                    } else {
                        if let Some(font) = &text.font {
                            renderer.draw_text_string(
                                &text.text,
                                &world_transform,
                                font,
                                text.font_size,
                                text.color,
                                text.line_spacing,
                                text.kerning,
                                handle.id() as i32,
                            );
                        }
                    }
                }
                3 => {
                    // Tilemap — frustum culled + precomputed transforms.
                    let Ok(tilemap) = self.world.get::<&TilemapComponent>(handle) else {
                        continue;
                    };
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
                    // Half-texel inset prevents tile seams caused by
                    // floating-point UV precision at exact texel boundaries.
                    let half_texel_u = 0.5 * inv_tw;
                    let half_texel_v = 0.5 * inv_th;

                    // --- Frustum culling: visible tile range ---
                    // Use Frustum2D in tilemap local space to find visible tile
                    // range. This avoids the degenerate NDC unprojection that
                    // breaks for tilted/perspective cameras.
                    let local_vp = vp * world_transform;
                    let local_frustum = super::spatial::Frustum2D::from_view_projection(&local_vp);
                    let w = tilemap.width as f32;
                    let h = tilemap.height as f32;
                    let (min_col, max_col, min_row, max_row) =
                        if let Some(aabb) = local_frustum.visible_aabb() {
                            (
                                ((aabb.min.x / tile_size.x).floor() - 1.0).clamp(0.0, w) as u32,
                                ((aabb.max.x / tile_size.x).ceil() + 1.0).clamp(0.0, w) as u32,
                                ((aabb.min.y / tile_size.y).floor() - 1.0).clamp(0.0, h) as u32,
                                ((aabb.max.y / tile_size.y).ceil() + 1.0).clamp(0.0, h) as u32,
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
                            let mut min_u = px * inv_tw + half_texel_u;
                            let mut min_v = py * inv_th + half_texel_v;
                            let mut max_u = (px + cell_w) * inv_tw - half_texel_u;
                            let mut max_v = (py + cell_h) * inv_th - half_texel_v;

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
                4 => {
                    // UIImage — simple stretch or 9-slice.
                    let Ok(img) = self.world.get::<&UIImageComponent>(handle) else {
                        continue;
                    };

                    // Apply interaction color tinting.
                    let mut color = img.color;
                    if let Ok(inter) = self.world.get::<&UIInteractableComponent>(handle) {
                        let tint = match inter.state {
                            UIInteractionState::Hovered => {
                                inter.hover_color.unwrap_or(glam::Vec4::ONE)
                            }
                            UIInteractionState::Pressed => {
                                inter.press_color.unwrap_or(glam::Vec4::ONE)
                            }
                            UIInteractionState::Disabled => {
                                inter.disabled_color.unwrap_or(glam::Vec4::splat(0.5))
                            }
                            UIInteractionState::Normal => glam::Vec4::ONE,
                        };
                        color *= tint;
                    }

                    let eid = handle.id() as i32;
                    let has_border = img.border.iter().any(|&b| b > 0.0);

                    if has_border {
                        // 9-slice rendering.
                        self.draw_9slice(
                            renderer,
                            &world_transform,
                            &img,
                            color,
                            eid,
                            gui_scale,
                            ppwu,
                        );
                    } else {
                        // Simple stretch.
                        if let Some(ref tex) = img.texture {
                            let tex_idx = tex.bindless_index() as f32;
                            renderer.draw_textured_quad_transformed_uv(
                                &world_transform,
                                tex_idx,
                                [0.0, 0.0],
                                [1.0, 1.0],
                                color,
                                eid,
                            );
                        } else {
                            renderer.draw_quad_transform(&world_transform, color, eid);
                        }
                    }
                }
                _ => {}
            }
        }

        // Emit and render GPU particles from all active ParticleEmitterComponents.
        self.emit_and_render_particles(renderer);

        // Return buffers to pool for reuse next frame.
        let mut pool = self.render_buffers.lock();
        pool.sort_keys = renderables;
        pool.sprite_handles = sprites;
        pool.circle_keys = circle_renderables;
    }

    /// Find the primary ECS camera and return its frustum info for shadow
    /// cascade fitting. Returns `None` if no primary camera exists or it
    /// uses orthographic projection (CSM frustum fitting requires perspective).
    pub fn primary_camera_info(&self) -> Option<ShadowCameraInfo> {
        use gg_renderer::ProjectionType;

        let shadow_distance = self.find_first_shadow_distance().unwrap_or(100.0);

        for (handle, camera) in self
            .world
            .query::<(hecs::Entity, &super::CameraComponent)>()
            .iter()
        {
            if camera.primary && camera.camera.projection_type() == ProjectionType::Perspective {
                let world = self.get_world_transform(Entity::new(handle));
                let (_, _, translation) = world.to_scale_rotation_translation();
                let vp = *camera.camera.projection() * world.inverse();
                return Some(ShadowCameraInfo {
                    view_projection: vp,
                    near: camera.camera.perspective_near(),
                    far: camera.camera.perspective_far(),
                    camera_position: translation,
                    shadow_distance,
                });
            }
        }
        None
    }

    /// Return the `shadow_distance` from the first directional light with
    /// `cast_shadows` enabled, or `None` if none exists.
    pub fn find_first_shadow_distance(&self) -> Option<f32> {
        self.world
            .query::<&DirectionalLightComponent>()
            .iter()
            .find(|dl| dl.cast_shadows)
            .map(|dl| dl.shadow_distance)
    }

    /// Collect all light components from the scene into a [`LightEnvironment`].
    ///
    /// Gathers directional lights, point lights, and ambient light settings.
    /// The `camera_position` should be set by the caller before uploading.
    pub fn collect_lights(&self) -> LightEnvironment {
        let mut env = LightEnvironment::default();

        // Directional light (use the first one found).
        // Direction is derived from the entity's world rotation.
        if let Some((handle, dl)) = self
            .world
            .query::<(hecs::Entity, &DirectionalLightComponent)>()
            .iter()
            .next()
        {
            let world = self.get_world_transform(super::Entity::new(handle));
            let (_, world_rot, _) = world.to_scale_rotation_translation();
            let direction = DirectionalLightComponent::direction(world_rot);
            env.directional = Some((direction, dl.color, dl.intensity));
        }

        // Point lights (from entity transforms).
        for (pl, tf) in self
            .world
            .query::<(&PointLightComponent, &TransformComponent)>()
            .iter()
        {
            env.point_lights
                .push((tf.translation, pl.color, pl.intensity, pl.radius));
        }

        // Ambient light (use the first one found, otherwise keep default).
        if let Some(al) = self.world.query::<&AmbientLightComponent>().iter().next() {
            env.ambient_color = al.color;
            env.ambient_intensity = al.intensity;
        }

        // Environment map (IBL) — use the first EnvironmentComponent found.
        if let Some(ec) = self.world.query::<&EnvironmentComponent>().iter().next() {
            if ec.loaded && ec.environment_handle != 0 {
                env.has_ibl = true;
                env.ibl_intensity = ec.ibl_intensity;
            }
        }

        env
    }

    /// Render all [`MeshRendererComponent`] entities using the default
    /// mesh3d pipeline. Called after 2D rendering is complete.
    /// Render the shadow depth pass for directional light shadows.
    ///
    /// Must be called OUTSIDE the main render pass (before `begin_scene`).
    /// If no directional light with `cast_shadows` exists, this is a no-op.
    ///
    /// When `camera` is provided, each cascade is fitted to a sub-frustum of
    /// the camera for dramatically better shadow resolution near the viewer.
    /// When `None`, falls back to scene-AABB fitting (same VP for both cascades).
    pub fn render_shadow_pass(
        &self,
        renderer: &mut Renderer,
        cmd_buf: ash::vk::CommandBuffer,
        current_frame: usize,
        viewport_index: usize,
        camera: Option<&ShadowCameraInfo>,
    ) {
        // Find the first directional light with shadows enabled.
        let shadow_light = self
            .world
            .query::<(hecs::Entity, &DirectionalLightComponent)>()
            .iter()
            .find(|(_, dl)| dl.cast_shadows)
            .map(|(handle, dl)| (handle, dl.shadow_distance, dl.shadow_cull_front_faces));

        let (handle, shadow_distance, front_face_cull) = match shadow_light {
            Some(h) => h,
            None => return,
        };

        // Compute the light direction from the entity's world rotation.
        let world = self.get_world_transform(super::Entity::new(handle));
        let (_, world_rot, _) = world.to_scale_rotation_translation();
        let direction = DirectionalLightComponent::direction(world_rot);

        // Take reusable buffers from pool.
        let mut pool = self.render_buffers.lock();
        let mut meshes = std::mem::take(&mut pool.shadow_meshes);
        let mut skinned_meshes = std::mem::take(&mut pool.shadow_skinned);
        let mut mesh_bounds = std::mem::take(&mut pool.shadow_bounds);
        let mut skinned_draw_data = std::mem::take(&mut pool.shadow_skinned_draw);
        drop(pool);

        meshes.clear();
        skinned_meshes.clear();
        mesh_bounds.clear();
        skinned_draw_data.clear();

        // Collect meshes, tagging each as opaque or alpha-tested.
        meshes.extend(
            self.world
                .query::<(hecs::Entity, &MeshRendererComponent)>()
                .iter()
                .filter_map(|(h, mc)| {
                    if mc.vertex_array.is_some() {
                        let w = self.get_world_transform(super::Entity::new(h));
                        Some((h, w, mc.cast_alpha_shadow))
                    } else {
                        None
                    }
                }),
        );

        // Collect skinned meshes for shadow rendering.
        skinned_meshes.extend(
            self.world
                .query::<(hecs::Entity, &SkeletalAnimationComponent)>()
                .iter()
                .filter_map(|(h, sac)| {
                    if sac.skinned_vertex_array.is_some() {
                        let w = self.get_world_transform(super::Entity::new(h));
                        Some((h, w))
                    } else {
                        None
                    }
                }),
        );

        if meshes.is_empty() && skinned_meshes.is_empty() {
            let mut pool = self.render_buffers.lock();
            pool.shadow_meshes = meshes;
            pool.shadow_skinned = skinned_meshes;
            pool.shadow_bounds = mesh_bounds;
            pool.shadow_skinned_draw = skinned_draw_data;
            return;
        }

        let has_alpha = meshes.iter().any(|(_, _, alpha)| *alpha);

        // Initialize shadow pipelines as needed.
        if !renderer.has_shadow_pipeline() {
            if let Err(e) = renderer.init_shadow_pipeline() {
                log::error!("Failed to create shadow pipeline: {e}");
                let mut pool = self.render_buffers.lock();
                pool.shadow_meshes = meshes;
                pool.shadow_skinned = skinned_meshes;
                pool.shadow_bounds = mesh_bounds;
                pool.shadow_skinned_draw = skinned_draw_data;
                return;
            }
        }
        if has_alpha && !renderer.has_shadow_alpha_pipeline() {
            if let Err(e) = renderer.init_shadow_alpha_pipeline() {
                log::error!("Failed to create shadow alpha pipeline: {e}");
                // Alpha meshes will fall back to opaque shadow pass.
            }
        }

        let (scene_min, scene_max) = self.compute_mesh_scene_bounds(&meshes);

        // Compute per-cascade VP matrices. When camera frustum info is available,
        // each cascade is fitted to a sub-frustum slice for much better shadow
        // resolution near the viewer. Otherwise fall back to scene-AABB fitting.
        let (cascade_vps, split_depths, effective_shadow_far, texel_sizes) =
            if let Some(cam) = camera {
                compute_cascade_vps(cam, direction, scene_min, scene_max)
            } else {
                let vp = compute_directional_light_vp(direction, scene_min, scene_max);
                (
                    [vp; gg_renderer::NUM_SHADOW_CASCADES],
                    [0.75, 0.5, 0.25],
                    shadow_distance,
                    [1.0; gg_renderer::NUM_SHADOW_CASCADES],
                )
            };

        // Pre-compute world-space AABBs for per-cascade frustum culling.
        mesh_bounds.extend(meshes.iter().map(|(h, w, _)| {
            let mc = self.world.get::<&MeshRendererComponent>(*h).unwrap();
            mc.local_bounds
                .map(|(lmin, lmax)| super::spatial::Aabb3D::from_local_bounds(lmin, lmax, w))
        }));

        let use_alpha_pipeline = has_alpha && renderer.has_shadow_alpha_pipeline();

        // Pre-compute bone poses for skinned meshes (shared across cascades).
        let has_skinned = !skinned_meshes.is_empty();
        if has_skinned {
            if let Err(e) = renderer.ensure_bone_palette() {
                log::error!("Failed to init bone palette for shadow: {e}");
            } else if let Err(e) = renderer.init_skinned_shadow_pipeline() {
                log::error!("Failed to create skinned shadow pipeline: {e}");
            } else {
                for (handle, world_transform) in &skinned_meshes {
                    let sac = self
                        .world
                        .get::<&SkeletalAnimationComponent>(*handle)
                        .unwrap();
                    let pose = sac.compute_current_pose();
                    if let Some(bone_offset) = renderer.write_bone_matrices(&pose.matrices) {
                        skinned_draw_data.push((*handle, *world_transform, bone_offset));
                    }
                }
            }
        }

        // Render each cascade with per-cascade frustum culling.
        for (cascade, cascade_vp) in cascade_vps.iter().enumerate() {
            let frustum = super::spatial::Frustum3D::from_view_projection(cascade_vp);

            renderer.begin_shadow_pass(
                cascade_vp,
                cascade,
                cmd_buf,
                current_frame,
                viewport_index,
                front_face_cull,
            );

            // Pass 1: opaque meshes (standard shadow pipeline, already bound by begin_shadow_pass).
            for (idx, (handle, world_transform, is_alpha)) in meshes.iter().enumerate() {
                if *is_alpha {
                    continue;
                }
                let visible = mesh_bounds[idx]
                    .as_ref()
                    .map(|aabb| frustum.contains_aabb(aabb))
                    .unwrap_or(true);
                if !visible {
                    continue;
                }
                let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
                if let Some(ref va) = mesh_comp.vertex_array {
                    renderer.submit_shadow(va, world_transform, cmd_buf);
                }
            }

            // Pass 2: alpha-tested meshes (switch to alpha shadow pipeline).
            if use_alpha_pipeline {
                renderer.bind_shadow_alpha_pipeline(cascade_vp, cmd_buf, current_frame);

                for (idx, (handle, world_transform, is_alpha)) in meshes.iter().enumerate() {
                    if !*is_alpha {
                        continue;
                    }
                    let visible = mesh_bounds[idx]
                        .as_ref()
                        .map(|aabb| frustum.contains_aabb(aabb))
                        .unwrap_or(true);
                    if !visible {
                        continue;
                    }
                    let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
                    if let Some(ref va) = mesh_comp.vertex_array {
                        // Get the bindless texture slot. -1 means no texture (skip alpha test).
                        let tex_index = mesh_comp
                            .texture
                            .as_ref()
                            .map(|t| t.bindless_index() as i32)
                            .unwrap_or(-1);
                        renderer.submit_shadow_alpha(
                            va,
                            world_transform,
                            0.5, // Default alpha cutoff for shadow pass.
                            tex_index,
                            cmd_buf,
                        );
                    }
                }
            }

            // Pass 3: skinned meshes (switch to skinned shadow pipeline).
            if !skinned_draw_data.is_empty() {
                renderer.bind_skinned_shadow_pipeline(cmd_buf);
                for (handle, world_transform, bone_offset) in &skinned_draw_data {
                    let sac = self
                        .world
                        .get::<&SkeletalAnimationComponent>(*handle)
                        .unwrap();
                    if let Some(ref va) = sac.skinned_vertex_array {
                        renderer.submit_skinned_shadow_with_pipeline(
                            va,
                            cascade_vp,
                            world_transform,
                            *bone_offset,
                            cmd_buf,
                        );
                    }
                }
            }

            renderer.end_shadow_pass(cmd_buf);
        }

        // Stash cascade data for the main pass lighting upload.
        // Use the effective shadow_far (not the component's shadow_distance)
        // so the shader's distance fade matches where cascades actually end.
        self.shadow_cascade_cache.write().replace((
            cascade_vps,
            split_depths,
            effective_shadow_far,
            texel_sizes,
        ));

        // Return buffers to pool.
        let mut pool = self.render_buffers.lock();
        pool.shadow_meshes = meshes;
        pool.shadow_skinned = skinned_meshes;
        pool.shadow_bounds = mesh_bounds;
        pool.shadow_skinned_draw = skinned_draw_data;
    }

    /// Compute a conservative AABB for shadow frustum fitting.
    ///
    /// Only static/kinematic meshes contribute to the AABB so that dynamic
    /// objects (which move every frame) don't cause shadow jitter. Dynamic
    /// meshes still cast and receive shadows — they just don't influence the
    /// frustum bounds. Falls back to all meshes if no static ones exist.
    fn compute_mesh_scene_bounds(
        &self,
        meshes: &[(hecs::Entity, glam::Mat4, bool)],
    ) -> (glam::Vec3, glam::Vec3) {
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
        let mut count = 0;

        // First pass: AABB from non-dynamic meshes only (stable frustum).
        for (handle, world_transform, _) in meshes {
            #[cfg(feature = "physics-3d")]
            let is_dynamic = self
                .world
                .get::<&RigidBody3DComponent>(*handle)
                .map(|rb| rb.body_type == RigidBody3DType::Dynamic)
                .unwrap_or(false);
            #[cfg(not(feature = "physics-3d"))]
            let is_dynamic = false;

            if is_dynamic {
                continue;
            }

            self.expand_aabb_for_mesh(*handle, world_transform, &mut min, &mut max);
            count += 1;
        }

        // Fallback: if every mesh is dynamic, include them all.
        if count == 0 {
            for (handle, world_transform, _) in meshes {
                self.expand_aabb_for_mesh(*handle, world_transform, &mut min, &mut max);
            }
        }

        (min, max)
    }

    /// Expand an AABB by the 8 world-space corners of a mesh entity's local bounds.
    fn expand_aabb_for_mesh(
        &self,
        handle: hecs::Entity,
        world_transform: &glam::Mat4,
        min: &mut glam::Vec3,
        max: &mut glam::Vec3,
    ) {
        let mesh_comp = self.world.get::<&MeshRendererComponent>(handle).unwrap();

        let (local_min, local_max) = if let Some(bounds) = mesh_comp.local_bounds {
            // Asset mesh — use precomputed bounds.
            bounds
        } else {
            // Primitive mesh — use analytical bounds.
            match mesh_comp.mesh_source {
                MeshSource::Primitive(prim) => {
                    let (pmin, pmax) = prim.local_bounds();
                    (pmin, pmax)
                }
                MeshSource::Asset(_) => {
                    // Bounds not yet computed (mesh still loading).
                    return;
                }
            }
        };

        for &sx in &[0.0_f32, 1.0] {
            for &sy in &[0.0_f32, 1.0] {
                for &sz in &[0.0_f32, 1.0] {
                    let local = glam::Vec3::new(
                        if sx == 0.0 { local_min.x } else { local_max.x },
                        if sy == 0.0 { local_min.y } else { local_max.y },
                        if sz == 0.0 { local_min.z } else { local_max.z },
                    );
                    let world = world_transform.transform_point3(local);
                    *min = min.min(world);
                    *max = max.max(world);
                }
            }
        }
    }

    fn render_meshes(&self, renderer: &mut Renderer) {
        use gg_renderer::BlendMode;

        // Take reusable buffer from pool.
        let mut pool = self.render_buffers.lock();
        let mut meshes = std::mem::take(&mut pool.mesh_handles);
        drop(pool);

        meshes.clear();
        meshes.extend(
            self.world
                .query::<(hecs::Entity, &MeshRendererComponent)>()
                .iter()
                .filter_map(|(handle, mesh_comp)| {
                    if mesh_comp.vertex_array.is_some() {
                        let world = self.get_world_transform(super::Entity::new(handle));
                        Some((handle, world))
                    } else {
                        None
                    }
                }),
        );

        if meshes.is_empty() {
            // Return buffer even on early exit.
            self.render_buffers.lock().mesh_handles = meshes;
            return;
        }

        // Collect scene lights and upload to GPU before drawing 3D meshes.
        let mut light_env = self.collect_lights();
        light_env.camera_position = renderer.camera_position();

        // Wire up the actual max prefilter mip from the environment map system
        // instead of relying on the hardcoded default.
        if let Some(env_map) = renderer.environment() {
            light_env.max_prefilter_mip = env_map.max_prefilter_mip();
        }

        // Check for cascade VP data stashed by a prior render_shadow_pass call.
        if light_env.shadow_cascade_vps.is_none() {
            if let Some((vps, splits, dist, tsizes)) = self.shadow_cascade_cache.write().take() {
                light_env.shadow_cascade_vps = Some(vps);
                light_env.cascade_split_depths = splits;
                light_env.shadow_distance = dist;
                light_env.cascade_texel_sizes = tsizes;
            } else if let Some((handle, dl)) = self
                .world
                .query::<(hecs::Entity, &DirectionalLightComponent)>()
                .iter()
                .find(|(_, dl)| dl.cast_shadows)
            {
                // Fallback: compute scene-AABB-based VP (all cascades use same VP).
                let shadow_distance = dl.shadow_distance;
                let world = self.get_world_transform(super::Entity::new(handle));
                let (_, world_rot, _) = world.to_scale_rotation_translation();
                let direction = DirectionalLightComponent::direction(world_rot);
                let meshes_2: Vec<(hecs::Entity, glam::Mat4, bool)> =
                    meshes.iter().map(|&(h, w)| (h, w, false)).collect();
                let (scene_min, scene_max) = self.compute_mesh_scene_bounds(&meshes_2);
                let vp = compute_directional_light_vp(direction, scene_min, scene_max);
                light_env.shadow_cascade_vps = Some([vp; gg_renderer::NUM_SHADOW_CASCADES]);
                light_env.cascade_split_depths = [0.75, 0.5, 0.25];
                light_env.shadow_distance = shadow_distance;
            }
        }

        renderer.upload_lights(&light_env);

        // Provide contact shadow data to the post-processing pipeline.
        {
            let vp = renderer.view_projection();
            let (near, far) = renderer.camera_clip_planes();
            if let Some(pp) = renderer.postprocess_mut() {
                if let Some((dir, _, _)) = light_env.directional {
                    let inv_vp = vp.inverse();
                    pp.set_contact_shadow_data(inv_vp, vp, -dir, near, far);
                } else {
                    pp.clear_contact_shadow_data();
                }
            }
        }

        let pipeline = match renderer.mesh3d_pipeline() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to get mesh3d pipeline: {e}");
                self.render_buffers.lock().mesh_handles = meshes;
                return;
            }
        };

        // Bind opaque pipeline + shared descriptor sets once for all opaque meshes.
        renderer.bind_3d_shared_sets(&pipeline);

        let default_handle = renderer.material_library().default_handle();

        // Partition: opaque first, collect transparent for sorted pass.
        let cam_pos = renderer.camera_position();
        let mut transparent: Vec<(hecs::Entity, glam::Mat4, BlendMode, f32)> = Vec::new();

        for (handle, world_transform) in &meshes {
            let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
            if mesh_comp.vertex_array.is_none() {
                continue;
            }

            if mesh_comp.blend_mode != BlendMode::Opaque {
                // Defer transparent/additive meshes — sort and draw after opaques.
                let (_, _, pos) = world_transform.to_scale_rotation_translation();
                let dist_sq = (pos - cam_pos).length_squared();
                transparent.push((*handle, *world_transform, mesh_comp.blend_mode, dist_sq));
                continue;
            }

            // Draw opaque immediately.
            if let Some(ref va) = mesh_comp.vertex_array {
                {
                    let mat = renderer
                        .material_library_mut()
                        .get_mut(&default_handle)
                        .unwrap();
                    mat.albedo_color = mesh_comp.color;
                    mat.metallic = mesh_comp.metallic;
                    mat.roughness = mesh_comp.roughness;
                    mat.emissive_color = mesh_comp.emissive_color;
                    mat.emissive_strength = mesh_comp.emissive_strength;
                    mat.albedo_texture = mesh_comp.texture.clone();
                    mat.normal_texture = mesh_comp.normal_texture.clone();
                }
                renderer.submit_3d(
                    &pipeline,
                    va,
                    world_transform,
                    Some(&default_handle),
                    handle.id() as i32,
                );
            }
        }

        // Draw transparent meshes sorted back-to-front (farthest first).
        if !transparent.is_empty() {
            // Sort by distance descending (reverse-Z: greater distance = farther).
            transparent.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

            let mut current_blend = BlendMode::Opaque; // sentinel
            let mut current_pipeline = pipeline; // fallback (won't be used)

            for (handle, world_transform, blend_mode, _dist) in &transparent {
                // Switch pipeline when blend mode changes.
                if *blend_mode != current_blend {
                    current_pipeline = match renderer.mesh3d_blend_pipeline(*blend_mode) {
                        Ok(p) => p,
                        Err(e) => {
                            log::error!("Failed to get transparent mesh pipeline: {e}");
                            continue;
                        }
                    };
                    renderer.bind_3d_shared_sets(&current_pipeline);
                    current_blend = *blend_mode;
                }

                let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
                if let Some(ref va) = mesh_comp.vertex_array {
                    {
                        let mat = renderer
                            .material_library_mut()
                            .get_mut(&default_handle)
                            .unwrap();
                        mat.albedo_color = mesh_comp.color;
                        mat.metallic = mesh_comp.metallic;
                        mat.roughness = mesh_comp.roughness;
                        mat.emissive_color = mesh_comp.emissive_color;
                        mat.emissive_strength = mesh_comp.emissive_strength;
                        mat.albedo_texture = mesh_comp.texture.clone();
                        mat.normal_texture = mesh_comp.normal_texture.clone();
                    }
                    renderer.submit_3d(
                        &current_pipeline,
                        va,
                        world_transform,
                        Some(&default_handle),
                        handle.id() as i32,
                    );
                }
            }
        }

        // Return buffer to pool.
        self.render_buffers.lock().mesh_handles = meshes;
    }

    /// Render all [`SkeletalAnimationComponent`] entities using the skinned
    /// mesh pipeline. Computes bone poses, writes bone matrices to the SSBO,
    /// and issues draw calls.
    fn render_skinned_meshes(&self, renderer: &mut Renderer) {
        // Take reusable buffers from pool.
        let mut pool = self.render_buffers.lock();
        let mut skinned = std::mem::take(&mut pool.skinned_handles);
        let mut draw_list = std::mem::take(&mut pool.skinned_draw_list);
        drop(pool);

        skinned.clear();
        draw_list.clear();

        skinned.extend(
            self.world
                .query::<(hecs::Entity, &SkeletalAnimationComponent)>()
                .iter()
                .filter_map(|(handle, sac)| {
                    if sac.skinned_vertex_array.is_some() {
                        let world = self.get_world_transform(super::Entity::new(handle));
                        Some((handle, world))
                    } else {
                        None
                    }
                }),
        );

        if skinned.is_empty() {
            let mut pool = self.render_buffers.lock();
            pool.skinned_handles = skinned;
            pool.skinned_draw_list = draw_list;
            return;
        }

        // Ensure bone palette is initialized and reset for this frame.
        if let Err(e) = renderer.ensure_bone_palette() {
            log::error!("Failed to init bone palette: {e}");
            let mut pool = self.render_buffers.lock();
            pool.skinned_handles = skinned;
            pool.skinned_draw_list = draw_list;
            return;
        }
        for (handle, world_transform) in &skinned {
            let sac = self
                .world
                .get::<&SkeletalAnimationComponent>(*handle)
                .unwrap();

            // Compute the pose (handles blending internally).
            let pose = sac.compute_current_pose();

            // Write bone matrices to the SSBO.
            if let Some(bone_offset) = renderer.write_bone_matrices(&pose.matrices) {
                draw_list.push((*handle, *world_transform, bone_offset));
            } else {
                log::warn!("Bone palette full, skipping skinned entity");
            }
        }

        if draw_list.is_empty() {
            let mut pool = self.render_buffers.lock();
            pool.skinned_handles = skinned;
            pool.skinned_draw_list = draw_list;
            return;
        }

        // Get or create the skinned pipeline.
        let pipeline = match renderer.skinned_mesh3d_pipeline() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to get skinned mesh3d pipeline: {e}");
                let mut pool = self.render_buffers.lock();
                pool.skinned_handles = skinned;
                pool.skinned_draw_list = draw_list;
                return;
            }
        };

        // Bind pipeline + shared descriptor sets (0-5) once.
        renderer.bind_skinned_3d_shared_sets(&pipeline);

        let default_handle = renderer.material_library().default_handle();

        for (handle, world_transform, bone_offset) in &draw_list {
            let sac = self
                .world
                .get::<&SkeletalAnimationComponent>(*handle)
                .unwrap();
            if let Some(ref va) = sac.skinned_vertex_array {
                // Use MeshRendererComponent material properties if present.
                if let Ok(mesh_comp) = self.world.get::<&MeshRendererComponent>(*handle) {
                    let mat = renderer
                        .material_library_mut()
                        .get_mut(&default_handle)
                        .unwrap();
                    mat.albedo_color = mesh_comp.color;
                    mat.metallic = mesh_comp.metallic;
                    mat.roughness = mesh_comp.roughness;
                    mat.emissive_color = mesh_comp.emissive_color;
                    mat.emissive_strength = mesh_comp.emissive_strength;
                    mat.albedo_texture = mesh_comp.texture.clone();
                    mat.normal_texture = mesh_comp.normal_texture.clone();
                }
                renderer.submit_skinned_3d(
                    &pipeline,
                    va,
                    world_transform,
                    Some(&default_handle),
                    handle.id() as i32,
                    *bone_offset,
                );
            }
        }

        // Return buffers to pool.
        let mut pool = self.render_buffers.lock();
        pool.skinned_handles = skinned;
        pool.skinned_draw_list = draw_list;
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
            let props = gg_renderer::ParticleProps {
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
    /// Draw a 9-slice UIImage: 4 corners (fixed size), 4 edges (stretch one axis),
    /// and optional center patch using the existing batch renderer.
    #[allow(clippy::too_many_arguments)]
    fn draw_9slice(
        &self,
        renderer: &mut Renderer,
        world_transform: &glam::Mat4,
        img: &UIImageComponent,
        color: glam::Vec4,
        entity_id: i32,
        gui_scale: f32,
        ppwu: f32,
    ) {
        let tex = match img.texture.as_ref() {
            Some(t) => t,
            None => return,
        };
        let tex_idx = tex.bindless_index() as f32;
        let tw = tex.width() as f32;
        let th = tex.height() as f32;
        if tw == 0.0 || th == 0.0 {
            return;
        }

        let [b_left, b_right, b_top, b_bottom] = img.border;

        // Corner size in world units (fixed pixel size).
        let corner_w_left = b_left * gui_scale / ppwu;
        let corner_w_right = b_right * gui_scale / ppwu;
        let corner_h_top = b_top * gui_scale / ppwu;
        let corner_h_bottom = b_bottom * gui_scale / ppwu;

        // Quad world size from transform scale.
        let world_w = world_transform.x_axis.truncate().length();
        let world_h = world_transform.y_axis.truncate().length();

        // Center stretch region.
        let center_w = (world_w - corner_w_left - corner_w_right).max(0.0);
        let center_h = (world_h - corner_h_top - corner_h_bottom).max(0.0);

        // UV boundaries.
        let u_left = b_left / tw;
        let u_right = 1.0 - b_right / tw;
        let v_top = b_top / th;
        let v_bottom = 1.0 - b_bottom / th;

        // Base position (bottom-left of the quad in world space).
        let pos = glam::Vec2::new(world_transform.w_axis.x, world_transform.w_axis.y);
        let base_x = pos.x - world_w * 0.5;
        let base_y = pos.y - world_h * 0.5;

        // Helper: build a transform for a sub-rect at (x, y) with size (w, h).
        let make_transform = |x: f32, y: f32, w: f32, h: f32| -> glam::Mat4 {
            glam::Mat4::from_scale_rotation_translation(
                glam::Vec3::new(w, h, 1.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(x + w * 0.5, y + h * 0.5, world_transform.w_axis.z),
            )
        };

        // X positions: left edge, center start, right edge start.
        let x0 = base_x;
        let x1 = base_x + corner_w_left;
        let x2 = base_x + corner_w_left + center_w;

        // Y positions: bottom edge, center start, top edge start.
        let y0 = base_y;
        let y1 = base_y + corner_h_bottom;
        let y2 = base_y + corner_h_bottom + center_h;

        // 9 patches: [transform, uv_min, uv_max]
        let patches: [(glam::Mat4, [f32; 2], [f32; 2]); 9] = [
            // Bottom-left corner.
            (
                make_transform(x0, y0, corner_w_left, corner_h_bottom),
                [0.0, v_bottom],
                [u_left, 1.0],
            ),
            // Bottom center edge.
            (
                make_transform(x1, y0, center_w, corner_h_bottom),
                [u_left, v_bottom],
                [u_right, 1.0],
            ),
            // Bottom-right corner.
            (
                make_transform(x2, y0, corner_w_right, corner_h_bottom),
                [u_right, v_bottom],
                [1.0, 1.0],
            ),
            // Middle-left edge.
            (
                make_transform(x0, y1, corner_w_left, center_h),
                [0.0, v_top],
                [u_left, v_bottom],
            ),
            // Center.
            (
                make_transform(x1, y1, center_w, center_h),
                [u_left, v_top],
                [u_right, v_bottom],
            ),
            // Middle-right edge.
            (
                make_transform(x2, y1, corner_w_right, center_h),
                [u_right, v_top],
                [1.0, v_bottom],
            ),
            // Top-left corner.
            (
                make_transform(x0, y2, corner_w_left, corner_h_top),
                [0.0, 0.0],
                [u_left, v_top],
            ),
            // Top center edge.
            (
                make_transform(x1, y2, center_w, corner_h_top),
                [u_left, 0.0],
                [u_right, v_top],
            ),
            // Top-right corner.
            (
                make_transform(x2, y2, corner_w_right, corner_h_top),
                [u_right, 0.0],
                [1.0, v_top],
            ),
        ];

        for (i, (transform, uv_min, uv_max)) in patches.iter().enumerate() {
            // Skip center patch if fill_center is false.
            if i == 4 && !img.fill_center {
                continue;
            }
            // Skip zero-size patches.
            let (_, _, t) = transform.to_scale_rotation_translation();
            let s = glam::Vec2::new(
                transform.x_axis.truncate().length(),
                transform.y_axis.truncate().length(),
            );
            if s.x <= 0.0 || s.y <= 0.0 {
                continue;
            }
            let _ = t;
            renderer.draw_textured_quad_transformed_uv(
                transform, tex_idx, *uv_min, *uv_max, color, entity_id,
            );
        }
    }

    pub fn on_update_runtime(&mut self, renderer: &mut Renderer) {
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::on_update_runtime");

        // Reposition UI-anchored entities before computing world transforms.
        self.apply_ui_anchors();

        // Find the primary camera entity.
        let mut main_camera_vp: Option<glam::Mat4> = None;
        let mut cam_position = glam::Vec3::ZERO;
        for (handle, camera) in self
            .world
            .query::<(hecs::Entity, &super::CameraComponent)>()
            .iter()
        {
            if camera.primary {
                // VP = projection * inverse(camera_world_transform)
                let world = self.get_world_transform(Entity::new(handle));
                cam_position = world.col(3).truncate();
                let view = world.inverse();
                let proj = *camera.camera.projection();
                main_camera_vp = Some(proj * view);
                renderer.set_camera_matrices(view, proj);
                break;
            }
        }

        if let Some(vp) = main_camera_vp {
            renderer.set_view_projection(vp);
            renderer.set_camera_position(cam_position);
            self.render_scene(renderer);
        }
    }

    /// Run UI hit testing with mouse state. Call after `apply_ui_anchors()`.
    /// Returns UI events for Lua dispatch.
    pub fn update_ui_with_input(
        &mut self,
        mouse_world: glam::Vec2,
        mouse_down: bool,
        mouse_just_pressed: bool,
        mouse_just_released: bool,
    ) -> Vec<super::UIEvent> {
        self.core.update_ui_interaction(
            mouse_world,
            mouse_down,
            mouse_just_pressed,
            mouse_just_released,
        )
    }

    /// Render all entities using an externally provided view-projection
    /// matrix (e.g. from an [`EditorCamera`](gg_renderer::EditorCamera)).
    ///
    /// Unlike [`on_update_runtime`](Self::on_update_runtime), this does **not**
    /// look for a primary camera entity — it always renders.
    pub fn on_update_editor(&self, editor_camera_vp: &glam::Mat4, renderer: &mut Renderer) {
        let _timer = gg_core::profiling::ProfileTimer::new("Scene::on_update_editor");
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_sprite_events_forward() {
        let events = vec![
            AnimationEvent { frame: 1, name: "a".into() },
            AnimationEvent { frame: 3, name: "b".into() },
            AnimationEvent { frame: 5, name: "c".into() },
        ];
        let mut out = Vec::new();
        // Crossed from frame 0 → frame 3: should fire "a" (frame 1) and "b" (frame 3).
        collect_sprite_events(&events, 0, 3, true, 0, 7, 42, "walk", &mut out);
        let names: Vec<&str> = out.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, &["a", "b"]);
    }

    #[test]
    fn collect_sprite_events_no_change() {
        let events = vec![AnimationEvent { frame: 2, name: "x".into() }];
        let mut out = Vec::new();
        // Same frame — no events fired.
        collect_sprite_events(&events, 2, 2, true, 0, 7, 42, "walk", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_sprite_events_wrap_around() {
        let events = vec![
            AnimationEvent { frame: 0, name: "start".into() },
            AnimationEvent { frame: 7, name: "end".into() },
        ];
        let mut out = Vec::new();
        // Looping wrap: old=6, new=1, range 0..7.
        // Should fire "end" (frame 7, > old) and "start" (frame 0, <= new).
        collect_sprite_events(&events, 6, 1, true, 0, 7, 42, "walk", &mut out);
        let names: Vec<&str> = out.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, &["start", "end"]);
    }

    #[test]
    fn collect_sprite_events_non_looping_no_wrap() {
        let events = vec![AnimationEvent { frame: 0, name: "x".into() }];
        let mut out = Vec::new();
        // Non-looping, new < old — should fire nothing.
        collect_sprite_events(&events, 5, 3, false, 0, 7, 42, "walk", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_skeletal_events_forward() {
        let events = vec![
            SkeletalAnimationEvent { time: 0.2, name: "foot_l".into() },
            SkeletalAnimationEvent { time: 0.5, name: "foot_r".into() },
            SkeletalAnimationEvent { time: 0.8, name: "extra".into() },
        ];
        let mut out = Vec::new();
        // Time range (0.1, 0.5]: should fire foot_l (0.2) and foot_r (0.5).
        collect_skeletal_events(&events, 0.1, 0.5, true, 1.0, 42, "walk", &mut out);
        let names: Vec<&str> = out.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, &["foot_l", "foot_r"]);
    }

    #[test]
    fn collect_skeletal_events_wrap_around() {
        let events = vec![
            SkeletalAnimationEvent { time: 0.05, name: "start".into() },
            SkeletalAnimationEvent { time: 0.95, name: "end".into() },
        ];
        let mut out = Vec::new();
        // Looping wrap: old=0.9, new=0.1, duration=1.0.
        // "end" (0.95 > 0.9) and "start" (0.05 <= 0.1).
        collect_skeletal_events(&events, 0.9, 0.1, true, 1.0, 42, "run", &mut out);
        let names: Vec<&str> = out.iter().map(|(_, n, _)| n.as_str()).collect();
        assert_eq!(names, &["start", "end"]);
    }

    #[test]
    fn collect_skeletal_events_non_looping_no_wrap() {
        let events = vec![SkeletalAnimationEvent { time: 0.1, name: "x".into() }];
        let mut out = Vec::new();
        // Non-looping, new < old.
        collect_skeletal_events(&events, 0.5, 0.3, false, 1.0, 42, "run", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_events_preserves_uuid_and_clip() {
        let events = vec![AnimationEvent { frame: 2, name: "hit".into() }];
        let mut out = Vec::new();
        collect_sprite_events(&events, 1, 3, false, 0, 7, 999, "attack", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 999); // uuid
        assert_eq!(out[0].1, "hit"); // event_name
        assert_eq!(out[0].2, "attack"); // clip_name
    }
}
