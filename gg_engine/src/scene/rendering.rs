use super::{
    AmbientLightComponent, AnimationControllerComponent, CircleRendererComponent,
    DirectionalLightComponent, Entity, IdComponent, InstancedSpriteAnimator, MeshPrimitive,
    MeshRendererComponent, ParticleEmitterComponent, PointLightComponent, Scene,
    SpriteAnimatorComponent, SpriteRendererComponent, TextComponent, TilemapComponent,
    TransformComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
use crate::renderer::shadow_map::compute_directional_light_vp;
use crate::renderer::{Font, LightEnvironment, Mesh, Renderer, SubTexture2D};

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
    /// For scenes above [`PAR_THRESHOLD`](crate::jobs::parallel::PAR_THRESHOLD),
    /// the per-entity animation tick is parallelized via extract-process-writeback.
    pub fn on_update_animations(&mut self, dt: f32) {
        // Advance scene global time and store dt for Engine.delta_time().
        self.global_time += dt as f64;
        self.last_dt = dt;

        // Phase 1: extract + parallel tick SpriteAnimatorComponent timers.
        struct AnimWork {
            entity: hecs::Entity,
            uuid: u64,
            frame_timer: f32,
            current_frame: u32,
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

        let mut work: Vec<AnimWork> = self
            .world
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
            })
            .collect();

        // Parallel tick (pure per-entity computation, no cross-entity deps).
        crate::jobs::parallel::par_for_each_mut(&mut work, |item| {
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

        // Writeback + collect finished events.
        let mut finished_events: Vec<(u64, String, String)> = Vec::new();
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
            }
        }

        // Phase 2: check InstancedSpriteAnimator non-looping clips for completion.
        let gt = self.global_time;
        for (id_comp, anim) in self
            .world
            .query_mut::<(&IdComponent, &mut InstancedSpriteAnimator)>()
        {
            if anim.is_finished(gt) {
                let clip_name = anim.current_clip_name().unwrap_or("").to_owned();
                let default = anim.default_clip.clone();
                anim.playing = false;
                finished_events.push((id_comp.id.raw(), clip_name, default));
            }
        }

        if finished_events.is_empty() {
            self.evaluate_animation_controllers();
            return;
        }

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

        // Phase 5: evaluate animation controllers.
        self.evaluate_animation_controllers();
    }

    /// Evaluate all [`AnimationControllerComponent`]s and apply transitions.
    ///
    /// Checks each entity that has both a controller and an animator.
    /// If a transition fires, plays the target clip on the animator.
    fn evaluate_animation_controllers(&mut self) {
        // Collect transitions to apply (uuid, target_clip).
        let mut to_play: Vec<(u64, String)> = Vec::new();

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

        for (uuid, target) in to_play {
            if let Some(entity) = self.find_entity_by_uuid(uuid) {
                if let Some(mut animator) =
                    self.get_component_mut::<SpriteAnimatorComponent>(entity)
                {
                    animator.play(&target);
                }
            }
        }
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
        use super::script_glue::SceneScriptContext;

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
        // Collect (entity, clip_index, handle) for SpriteAnimatorComponent clips.
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

        // Collect for InstancedSpriteAnimator clips.
        let instanced_needs: Vec<(hecs::Entity, usize, crate::uuid::Uuid)> = self
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
    }

    /// Async variant of [`load_fonts`](Self::load_fonts).
    ///
    /// For text components with unresolved fonts:
    /// - If the font is already cached in the asset manager, assigns it immediately.
    /// - Otherwise, requests an async background load (non-blocking).
    ///
    /// On subsequent frames, `poll_loaded` will upload completed fonts,
    /// and this method will find them in the cache and assign them.
    pub fn load_fonts_async(&mut self, asset_manager: &mut crate::asset::EditorAssetManager) {
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
    // Mesh uploading
    // -----------------------------------------------------------------

    /// Upload vertex arrays for any [`MeshRendererComponent`] that doesn't
    /// have one yet. Call before rendering (similar to `resolve_texture_handles`).
    pub fn resolve_meshes(&mut self, renderer: &mut Renderer) {
        let needs: Vec<(hecs::Entity, MeshPrimitive, [f32; 4])> = self
            .world
            .query::<(hecs::Entity, &MeshRendererComponent)>()
            .iter()
            .filter_map(|(handle, mesh_comp)| {
                if mesh_comp.vertex_array.is_none() {
                    Some((handle, mesh_comp.primitive, mesh_comp.color.into()))
                } else {
                    None
                }
            })
            .collect();

        for (handle, primitive, color) in needs {
            let mesh = match primitive {
                MeshPrimitive::Cube => Mesh::cube(color),
                MeshPrimitive::Sphere => Mesh::sphere(32, 16, color),
                MeshPrimitive::Plane => Mesh::plane(color),
            };
            match mesh.upload(renderer) {
                Ok(va) => {
                    if let Ok(mut comp) = self.world.get::<&mut MeshRendererComponent>(handle) {
                        comp.vertex_array = Some(va);
                    }
                }
                Err(e) => {
                    log::error!("Failed to upload mesh: {e}");
                }
            }
        }
    }

    /// Re-upload the vertex array for a mesh component when its primitive or
    /// color changes. Clears the existing VA so the next `resolve_meshes`
    /// call picks it up. The old VA is moved to a deferred-destroy queue
    /// to avoid destroying GPU buffers still referenced by in-flight command
    /// buffers.
    pub fn invalidate_mesh(&mut self, entity: super::Entity) {
        if let Ok(mut comp) = self
            .world
            .get::<&mut MeshRendererComponent>(entity.handle())
        {
            if let Some(old_va) = comp.vertex_array.take() {
                // Defer destruction: the old buffers may still be in use by
                // a previously submitted command buffer.
                if self.va_graveyard.is_empty() {
                    self.va_graveyard.push_back(Vec::new());
                }
                self.va_graveyard.back_mut().unwrap().push(old_va);
            }
        }
    }

    /// Rotate the deferred-destroy queue: push a new frame entry and drop
    /// entries older than `MAX_FRAMES_IN_FLIGHT`. Called once per frame
    /// after the GPU fence wait guarantees old command buffers have completed.
    pub fn rotate_va_graveyard(&mut self) {
        use crate::renderer::MAX_FRAMES_IN_FLIGHT;
        self.va_graveyard.push_back(Vec::new());
        while self.va_graveyard.len() > MAX_FRAMES_IN_FLIGHT {
            self.va_graveyard.pop_front(); // Drop old VAs — GPU is done with them.
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
        let _timer = crate::profiling::ProfileTimer::new("Scene::render_scene");

        // Write scene time to the camera UBO for GPU-computed animation.
        renderer.set_scene_time(self.global_time as f32);

        // Pre-compute world transforms for all entities once.
        let wt_cache = {
            crate::profile_scope!("Scene::build_world_transform_cache");
            self.build_world_transform_cache()
        };

        // Extract frustum half-planes for entity-level frustum culling.
        let vp = renderer.view_projection();
        let frustum = super::spatial::Frustum2D::from_view_projection(&vp);

        // Collect all renderable entities with sort keys.
        // Sprites and circles are frustum-culled via AABB overlap test.
        // Text is not culled (bounds depend on string content).
        // Tilemaps are not culled here (tile-level culling happens during rendering).
        // 0 = Sprite, 1 = Circle, 2 = Text, 3 = Tilemap
        let mut total_cullable: u32 = 0;
        let mut culled: u32 = 0;

        // --- Parallel frustum culling for sprites ---
        let sprites: Vec<(hecs::Entity, i32, i32)> = self
            .world
            .query::<(hecs::Entity, &SpriteRendererComponent)>()
            .iter()
            .map(|(h, s)| (h, s.sorting_layer, s.order_in_layer))
            .collect();
        total_cullable += sprites.len() as u32;

        let sprite_renderables: Vec<(i32, i32, f32, u8, hecs::Entity)> = {
            use crate::jobs::parallel::PAR_THRESHOLD;
            if sprites.len() >= PAR_THRESHOLD {
                use rayon::prelude::*;
                crate::jobs::pool().install(|| {
                    sprites
                        .par_iter()
                        .filter_map(|&(handle, sorting_layer, order_in_layer)| {
                            let wt = wt_cache.get(&handle)?;
                            let aabb = super::spatial::Aabb2D::from_unit_quad_transform(wt);
                            if !frustum.contains_aabb(&aabb) {
                                return None;
                            }
                            Some((sorting_layer, order_in_layer, wt.w_axis.z, 0u8, handle))
                        })
                        .collect()
                })
            } else {
                sprites
                    .iter()
                    .filter_map(|&(handle, sorting_layer, order_in_layer)| {
                        let wt = wt_cache.get(&handle)?;
                        let aabb = super::spatial::Aabb2D::from_unit_quad_transform(wt);
                        if !frustum.contains_aabb(&aabb) {
                            return None;
                        }
                        Some((sorting_layer, order_in_layer, wt.w_axis.z, 0u8, handle))
                    })
                    .collect()
            }
        };
        culled += sprites.len() as u32 - sprite_renderables.len() as u32;

        // --- Circles (usually few, keep sequential) ---
        let mut circle_renderables: Vec<(i32, i32, f32, u8, hecs::Entity)> = Vec::new();
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
            circle_renderables.push((
                circle.sorting_layer,
                circle.order_in_layer,
                wt.w_axis.z,
                1,
                handle,
            ));
        }

        self.culling_stats.set(super::CullingStats {
            total_cullable,
            rendered: total_cullable - culled,
            culled,
        });

        // --- Text & tilemaps (sequential, usually few) ---
        let mut renderables = sprite_renderables;
        renderables.extend(circle_renderables);

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

        // --- Parallel sort ---
        let sort_cmp = |a: &(i32, i32, f32, u8, hecs::Entity),
                        b: &(i32, i32, f32, u8, hecs::Entity)| {
            a.0.cmp(&b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        };
        if renderables.len() >= crate::jobs::parallel::PAR_THRESHOLD {
            use rayon::prelude::*;
            crate::jobs::pool().install(|| renderables.par_sort_by(sort_cmp));
        } else {
            renderables.sort_by(sort_cmp);
        }

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
                                anim.start_time as f32,
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
                                renderer.draw_sprite(&world_transform, &sprite, handle.id() as i32);
                            }
                        } else {
                            renderer.draw_sprite(&world_transform, &sprite, handle.id() as i32);
                        }
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

        // Render 3D meshes (after flushing all 2D batches).
        self.render_meshes(renderer);
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

        env
    }

    /// Render all [`MeshRendererComponent`] entities using the default
    /// mesh3d pipeline. Called after 2D rendering is complete.
    /// Render the shadow depth pass for directional light shadows.
    ///
    /// Must be called OUTSIDE the main render pass (before `begin_scene`).
    /// If no directional light with `cast_shadows` exists, this is a no-op.
    ///
    /// Returns the light-space VP matrix if shadows were rendered, so the
    /// caller can set it on the `LightEnvironment` for the main pass.
    pub fn render_shadow_pass(
        &self,
        renderer: &mut Renderer,
        cmd_buf: ash::vk::CommandBuffer,
        current_frame: usize,
        viewport_index: usize,
    ) -> Option<glam::Mat4> {
        // Find the first directional light with shadows enabled.
        let shadow_light = self
            .world
            .query::<(hecs::Entity, &DirectionalLightComponent)>()
            .iter()
            .find(|(_, dl)| dl.cast_shadows)
            .map(|(handle, _dl)| handle);

        let handle = shadow_light?;

        // Compute the light direction from the entity's world rotation.
        let world = self.get_world_transform(super::Entity::new(handle));
        let (_, world_rot, _) = world.to_scale_rotation_translation();
        let direction = DirectionalLightComponent::direction(world_rot);

        // Collect meshes.
        let meshes: Vec<(hecs::Entity, glam::Mat4)> = self
            .world
            .query::<(hecs::Entity, &MeshRendererComponent)>()
            .iter()
            .filter_map(|(h, mc)| {
                if mc.vertex_array.is_some() {
                    let w = self.get_world_transform(super::Entity::new(h));
                    Some((h, w))
                } else {
                    None
                }
            })
            .collect();

        if meshes.is_empty() {
            return None;
        }

        // Compute scene AABB from mesh transforms.
        let (scene_min, scene_max) = self.compute_mesh_scene_bounds(&meshes);

        // Compute the light-space VP matrix.
        let light_vp = compute_directional_light_vp(direction, scene_min, scene_max);

        // Initialize shadow pipeline if needed.
        if !renderer.has_shadow_pipeline() {
            if let Err(e) = renderer.init_shadow_pipeline() {
                log::error!("Failed to create shadow pipeline: {e}");
                return None;
            }
        }

        // Render shadow pass.
        renderer.begin_shadow_pass(&light_vp, cmd_buf, current_frame, viewport_index);

        for (handle, world_transform) in &meshes {
            let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
            if let Some(ref va) = mesh_comp.vertex_array {
                renderer.submit_shadow(va, world_transform, cmd_buf);
            }
        }

        renderer.end_shadow_pass(cmd_buf);

        Some(light_vp)
    }

    /// Compute a conservative AABB enclosing all mesh entities.
    fn compute_mesh_scene_bounds(
        &self,
        meshes: &[(hecs::Entity, glam::Mat4)],
    ) -> (glam::Vec3, glam::Vec3) {
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::NEG_INFINITY);

        for (handle, world_transform) in meshes {
            let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
            // Use the mesh primitive's approximate AABB (unit cube).
            let half = match mesh_comp.primitive {
                MeshPrimitive::Cube => glam::Vec3::splat(0.5),
                MeshPrimitive::Sphere => glam::Vec3::splat(0.5),
                MeshPrimitive::Plane => glam::Vec3::new(0.5, 0.0, 0.5),
            };

            // Transform the 8 corners of the local AABB to world space.
            for &sx in &[-1.0_f32, 1.0] {
                for &sy in &[-1.0_f32, 1.0] {
                    for &sz in &[-1.0_f32, 1.0] {
                        let local = glam::Vec3::new(sx * half.x, sy * half.y, sz * half.z);
                        let world = world_transform.transform_point3(local);
                        min = min.min(world);
                        max = max.max(world);
                    }
                }
            }
        }

        (min, max)
    }

    fn render_meshes(&self, renderer: &mut Renderer) {
        // Check if there are any mesh entities before creating the pipeline.
        let meshes: Vec<(hecs::Entity, glam::Mat4)> = self
            .world
            .query::<(hecs::Entity, &MeshRendererComponent)>()
            .iter()
            .filter_map(|(handle, mesh_comp)| {
                if mesh_comp.vertex_array.is_some() {
                    let world = self.get_world_transform(super::Entity::new(handle));
                    Some((handle, world))
                } else {
                    None
                }
            })
            .collect();

        if meshes.is_empty() {
            return;
        }

        // Collect scene lights and upload to GPU before drawing 3D meshes.
        let mut light_env = self.collect_lights();
        light_env.camera_position = renderer.camera_position();

        // Check for shadow VP stashed by a prior render_shadow_pass call.
        // The shadow VP is also set by collect_lights_with_shadows if the
        // directional light has cast_shadows enabled.
        if light_env.shadow_light_vp.is_none() {
            // Check if any directional light has cast_shadows.
            if let Some((handle, _dl)) = self
                .world
                .query::<(hecs::Entity, &DirectionalLightComponent)>()
                .iter()
                .find(|(_, dl)| dl.cast_shadows)
            {
                let world = self.get_world_transform(super::Entity::new(handle));
                let (_, world_rot, _) = world.to_scale_rotation_translation();
                let direction = DirectionalLightComponent::direction(world_rot);
                let (scene_min, scene_max) = self.compute_mesh_scene_bounds(&meshes);
                light_env.shadow_light_vp = Some(compute_directional_light_vp(
                    direction, scene_min, scene_max,
                ));
            }
        }

        renderer.upload_lights(&light_env);

        let pipeline = match renderer.mesh3d_pipeline() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to get mesh3d pipeline: {e}");
                return;
            }
        };

        let default_handle = renderer.material_library().default_handle();

        for (handle, world_transform) in &meshes {
            let mesh_comp = self.world.get::<&MeshRendererComponent>(*handle).unwrap();
            if let Some(ref va) = mesh_comp.vertex_array {
                // Update the default material with per-entity properties.
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
                main_camera_vp = Some(*camera.camera.projection() * world.inverse());
                break;
            }
        }

        if let Some(vp) = main_camera_vp {
            renderer.set_view_projection(vp);
            renderer.set_camera_position(cam_position);
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
}
