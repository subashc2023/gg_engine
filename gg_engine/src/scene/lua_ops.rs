use super::{IdComponent, Scene};
use super::script_glue::SceneScriptContext;
use super::ScriptEngine;
use crate::input::Input;
use crate::timestep::Timestep;

#[cfg(feature = "lua-scripting")]
use super::LuaScriptComponent;

impl Scene {
    // -----------------------------------------------------------------
    // Lua Scripting lifecycle
    // -----------------------------------------------------------------

    /// Create the Lua script engine, set up per-entity environments, and
    /// call `on_create()` for each entity with a [`LuaScriptComponent`].
    pub(super) fn on_lua_scripting_start(&mut self) {
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
    pub(super) fn on_lua_scripting_stop(&mut self) {
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
    pub fn on_update_lua_scripts(&mut self, dt: Timestep, input: &Input) {
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
    pub fn script_engine(&self) -> Option<&ScriptEngine> {
        self.script_engine.as_ref()
    }
}
