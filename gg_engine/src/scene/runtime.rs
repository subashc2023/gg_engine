use super::{
    Entity, IdComponent, NativeScriptComponent, RigidBody2DComponent, Scene, TransformComponent,
};
use crate::input::Input;
use crate::timestep::Timestep;

impl Scene {
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
    pub(super) fn call_lua_fixed_update(&mut self, fixed_dt: f32, input: &Input) {
        use super::script_glue::SceneScriptContext;
        #[cfg(feature = "lua-scripting")]
        use super::LuaScriptComponent;

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
    pub(super) fn dispatch_collision_events(&mut self) {
        use super::script_glue::SceneScriptContext;

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
    // Per-frame update
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
    pub(super) fn run_native_fixed_update(&mut self, dt: Timestep, input: &Input) {
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
}
