use super::{
    AnimationControllerComponent, CircleRendererComponent, Entity, IdComponent,
    InstancedSpriteAnimator, ParticleEmitterComponent, Scene, SpriteAnimatorComponent,
    SpriteRendererComponent, TextComponent, TilemapComponent, TransformComponent,
    TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
use crate::renderer::{Font, Renderer, SubTexture2D};

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
    pub fn on_update_animations(&mut self, dt: f32) {
        // Advance scene global time.
        self.global_time += dt as f64;

        // Phase 1: tick all SpriteAnimatorComponent timers, collect finished events.
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

        // Phase 2: check InstancedSpriteAnimator non-looping clips for completion.
        let gt = self.global_time;
        for (id_comp, anim) in self
            .world
            .query_mut::<(&IdComponent, &mut InstancedSpriteAnimator)>()
        {
            if anim.is_finished(gt) {
                let clip_name = anim
                    .current_clip_name()
                    .unwrap_or("")
                    .to_owned();
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
    pub fn on_update_animation_previews(&mut self, dt: f32) {
        // Advance global time for instanced animators in editor preview.
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
    pub fn load_fonts_async(
        &mut self,
        asset_manager: &mut crate::asset::EditorAssetManager,
    ) {
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
                                renderer.draw_sprite(
                                    &world_transform,
                                    &sprite,
                                    handle.id() as i32,
                                );
                            }
                        } else {
                            renderer.draw_sprite(
                                &world_transform,
                                &sprite,
                                handle.id() as i32,
                            );
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
            .query::<(hecs::Entity, &super::CameraComponent)>()
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
}
