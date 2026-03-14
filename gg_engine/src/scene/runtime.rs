#[cfg(feature = "lua-scripting")]
use super::lua_ops::ScriptEngineGuard;
use super::{
    Entity, IdComponent, NativeScript, NativeScriptComponent, RigidBody2DComponent, Scene,
    TransformComponent,
};
#[cfg(feature = "physics-3d")]
use super::RigidBody3DComponent;
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
        #[cfg(feature = "physics-3d")]
        self.on_physics_3d_start();
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
        #[cfg(feature = "physics-3d")]
        self.on_physics_3d_stop();
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
        #[cfg(feature = "physics-3d")]
        self.on_physics_3d_start();
    }

    /// Tear down physics for simulation mode.
    ///
    /// Call this when exiting simulate mode.
    pub fn on_simulation_stop(&mut self) {
        #[cfg(feature = "physics-3d")]
        self.on_physics_3d_stop();
        self.on_physics_2d_stop();
    }

    // -----------------------------------------------------------------
    // Unified physics update (P0-1 fix)
    // -----------------------------------------------------------------

    /// Step both 2D and 3D physics together, calling script `on_fixed_update`
    /// exactly once per substep.
    ///
    /// When `input` is `Some`, script callbacks (both Lua and native) are
    /// interleaved with physics steps (play mode). When `None`, only physics
    /// is stepped (simulate mode).
    ///
    /// Per fixed step: `reset_forces` → `on_fixed_update` → `snapshot` →
    /// `step_once` → collision events. After the loop, interpolated transforms
    /// are written back for smooth rendering.
    pub fn on_update_all_physics(&mut self, dt: Timestep, input: Option<&Input>) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_update_all_physics");

        let has_2d = self.physics_world.is_some();
        #[cfg(feature = "physics-3d")]
        let has_3d = self.physics_world_3d.is_some();
        #[cfg(not(feature = "physics-3d"))]
        let has_3d = false;

        if !has_2d && !has_3d {
            return;
        }

        // Accumulate frame dt to both physics worlds.
        if let Some(p) = self.physics_world.as_mut() {
            p.accumulate(dt.seconds());
        }
        #[cfg(feature = "physics-3d")]
        if let Some(p) = self.physics_world_3d.as_mut() {
            p.accumulate(dt.seconds());
        }

        let fixed_dt = if let Some(p) = self.physics_world.as_ref() {
            p.fixed_timestep()
        } else {
            #[cfg(feature = "physics-3d")]
            {
                self.physics_world_3d.as_ref().unwrap().fixed_timestep()
            }
            #[cfg(not(feature = "physics-3d"))]
            {
                return;
            }
        };

        // Unified fixed-step loop.
        loop {
            let can_2d = self.physics_world.as_ref().is_some_and(|p| p.can_step());
            #[cfg(feature = "physics-3d")]
            let can_3d = self.physics_world_3d.as_ref().is_some_and(|p| p.can_step());
            #[cfg(not(feature = "physics-3d"))]
            let can_3d = false;

            if !can_2d && !can_3d {
                break;
            }

            // Clear accumulated forces before scripts add new ones.
            // rapier 0.22 does NOT auto-clear forces after step().
            if can_2d {
                self.physics_world.as_mut().unwrap().reset_all_forces();
            }
            #[cfg(feature = "physics-3d")]
            if can_3d {
                self.physics_world_3d.as_mut().unwrap().reset_all_forces();
            }

            // Run fixed-update scripts ONCE per substep so impulses/forces are
            // applied at the physics rate, not the render rate.
            if let Some(inp) = input {
                #[cfg(feature = "lua-scripting")]
                self.call_lua_fixed_update(fixed_dt, inp);
                self.run_native_fixed_update(Timestep::from_seconds(fixed_dt), inp);
            }

            // Snapshot + step.
            if can_2d {
                let p = self.physics_world.as_mut().unwrap();
                p.snapshot_transforms();
                p.step_once();
            }
            #[cfg(feature = "physics-3d")]
            if can_3d {
                let p = self.physics_world_3d.as_mut().unwrap();
                p.snapshot_transforms();
                p.step_once();
            }

            // Dispatch collision events to Lua scripts.
            #[cfg(feature = "lua-scripting")]
            if input.is_some() {
                if can_2d {
                    self.dispatch_collision_events();
                }
                #[cfg(feature = "physics-3d")]
                if can_3d {
                    self.dispatch_collision_events_3d();
                }
                self.flush_pending_destroys();
            }
        }

        // Write back interpolated transforms for smooth rendering.
        if has_2d {
            self.interpolate_2d_transforms();
        }
        #[cfg(feature = "physics-3d")]
        if has_3d {
            self.interpolate_3d_transforms();
        }
    }

    /// Interpolate 2D rigid body transforms for smooth rendering.
    fn interpolate_2d_transforms(&mut self) {
        let physics = self.physics_world.as_ref().unwrap();
        let alpha = physics.alpha();

        for (transform, rb) in self
            .core
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
                        transform.set_rotation_z(prev_angle + angle_diff * alpha);
                    } else {
                        // First frame — no previous, use current directly.
                        transform.translation.x = cur_pos.x;
                        transform.translation.y = cur_pos.y;
                        transform.set_rotation_z(cur_angle);
                    }
                }
            }
        }
    }

    /// Interpolate 3D rigid body transforms for smooth rendering.
    #[cfg(feature = "physics-3d")]
    fn interpolate_3d_transforms(&mut self) {
        let physics = self.physics_world_3d.as_ref().unwrap();
        let alpha = physics.alpha();

        for (transform, rb) in self
            .core
            .world
            .query_mut::<(&mut TransformComponent, &RigidBody3DComponent)>()
        {
            if let Some(body_handle) = rb.runtime_body {
                if let Some(body) = physics.bodies.get(body_handle) {
                    let cur_pos = body.translation();
                    let cur_rot = body.rotation();

                    if let Some((prev_pos, prev_rot)) = physics.prev_transform(body_handle) {
                        // Lerp position.
                        transform.translation.x = prev_pos.x + (cur_pos.x - prev_pos.x) * alpha;
                        transform.translation.y = prev_pos.y + (cur_pos.y - prev_pos.y) * alpha;
                        transform.translation.z = prev_pos.z + (cur_pos.z - prev_pos.z) * alpha;
                        // Slerp rotation.
                        let interp = prev_rot.slerp(cur_rot, alpha);
                        let (x, y, z, w) = (interp.i, interp.j, interp.k, interp.w);
                        transform.set_rotation_quat(glam::Quat::from_xyzw(x, y, z, w));
                    } else {
                        // First frame — no previous, use current directly.
                        transform.translation.x = cur_pos.x;
                        transform.translation.y = cur_pos.y;
                        transform.translation.z = cur_pos.z;
                        let (x, y, z, w) = (cur_rot.i, cur_rot.j, cur_rot.k, cur_rot.w);
                        transform.set_rotation_quat(glam::Quat::from_xyzw(x, y, z, w));
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Lua dispatch helpers (with ScriptEngineGuard for P0-2 fix)
    // -----------------------------------------------------------------

    /// Dispatch collision events (enter/exit) to Lua scripts for both entities
    /// in each collision pair. Shared implementation for 2D and 3D physics.
    #[cfg(feature = "lua-scripting")]
    fn dispatch_collision_pairs(&mut self, events: Vec<(u64, u64, bool)>) {
        use super::script_glue::SceneScriptContext;

        if events.is_empty() {
            return;
        }

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

        for (uuid_a, uuid_b, started) in &events {
            let callback = if *started {
                "on_collision_enter"
            } else {
                "on_collision_exit"
            };
            guard
                .engine_mut()
                .call_entity_collision(*uuid_a, callback, *uuid_b);
            guard
                .engine_mut()
                .call_entity_collision(*uuid_b, callback, *uuid_a);
        }

        // Guard drop restores engine and cleans up SceneScriptContext.
    }

    /// Drain 3D collision events and dispatch to Lua scripts.
    #[cfg(all(feature = "lua-scripting", feature = "physics-3d"))]
    pub(super) fn dispatch_collision_events_3d(&mut self) {
        let events = match self.physics_world_3d.as_ref() {
            Some(physics) => physics.drain_collision_events(),
            None => return,
        };
        self.dispatch_collision_pairs(events);
    }

    /// Call `on_fixed_update(dt)` on all loaded Lua scripts.
    ///
    /// Uses the take-modify-replace pattern with [`ScriptEngineGuard`] to ensure
    /// the engine is always restored, even on panic.
    #[cfg(feature = "lua-scripting")]
    pub(super) fn call_lua_fixed_update(&mut self, fixed_dt: f32, input: &Input) {
        use super::script_glue::SceneScriptContext;
        #[cfg(feature = "lua-scripting")]
        use super::LuaScriptComponent;

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

        let scene_ptr: *mut Scene = self;
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for uuid in &uuids {
            guard
                .engine_mut()
                .call_entity_on_fixed_update(*uuid, fixed_dt);
        }

        drop(guard);

        unsafe {
            (*scene_ptr).flush_pending_destroys();
        }
    }

    /// Drain 2D collision events and dispatch to Lua scripts.
    #[cfg(feature = "lua-scripting")]
    pub(super) fn dispatch_collision_events(&mut self) {
        let events = match self.physics_world.as_ref() {
            Some(physics) => physics.drain_collision_events(),
            None => return,
        };
        self.dispatch_collision_pairs(events);
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
        self.run_native_scripts(dt, input, |inst, entity, scene, dt, input| {
            inst.on_update(entity, scene, dt, input);
        });
    }

    /// Run `on_fixed_update` on all [`NativeScriptComponent`] instances.
    ///
    /// Called inside the physics step loop at the fixed physics rate.
    pub(super) fn run_native_fixed_update(&mut self, dt: Timestep, input: &Input) {
        self.run_native_scripts(dt, input, |inst, entity, scene, dt, input| {
            inst.on_fixed_update(entity, scene, dt, input);
        });
    }

    /// Shared take-execute-replace loop for native scripts.
    ///
    /// Collects all entities with a [`NativeScriptComponent`], lazily
    /// instantiates if needed, takes the instance out (releasing the hecs
    /// borrow), calls `callback`, and puts the instance back.
    fn run_native_scripts(
        &mut self,
        dt: Timestep,
        input: &Input,
        callback: impl Fn(&mut dyn NativeScript, Entity, &mut Scene, Timestep, &Input),
    ) {
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
            callback(&mut *instance, entity, self, dt, input);

            // Put the instance back.
            if let Ok(mut nsc) = self.world.get::<&mut NativeScriptComponent>(handle) {
                nsc.instance = Some(instance);
            }
        }
    }
}
