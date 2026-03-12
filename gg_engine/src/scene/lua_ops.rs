use super::script_glue::SceneScriptContext;
use super::ScriptEngine;
use super::{IdComponent, Scene};
use crate::input::Input;
use crate::timestep::Timestep;

#[cfg(feature = "lua-scripting")]
use super::LuaScriptComponent;

// ---------------------------------------------------------------------------
// RAII guard for ScriptEngine take-dispatch-replace pattern (P0-2 fix)
// ---------------------------------------------------------------------------

/// RAII guard that ensures a taken `ScriptEngine` is always restored to its
/// `Scene`, even if a Lua callback triggers a panic caught by mlua.
///
/// On drop, cleans up any lingering [`SceneScriptContext`] app_data and writes
/// the engine back to `(*scene_ptr).script_engine`.
pub(super) struct ScriptEngineGuard {
    engine: Option<ScriptEngine>,
    scene_ptr: *mut Scene,
}

impl ScriptEngineGuard {
    /// Create a new guard that will restore `engine` to `scene_ptr` on drop.
    pub fn new(engine: ScriptEngine, scene_ptr: *mut Scene) -> Self {
        Self {
            engine: Some(engine),
            scene_ptr,
        }
    }

    /// Access the engine mutably.
    pub fn engine_mut(&mut self) -> &mut ScriptEngine {
        self.engine
            .as_mut()
            .expect("ScriptEngineGuard: engine already consumed")
    }

    /// Consume the guard without restoring the engine to the scene.
    /// Used when the engine should be intentionally dropped (e.g. on stop).
    pub fn into_engine(mut self) -> ScriptEngine {
        self.engine
            .take()
            .expect("ScriptEngineGuard: engine already consumed")
    }
}

impl Drop for ScriptEngineGuard {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            // Clean up any lingering script context from app_data.
            engine.lua().remove_app_data::<SceneScriptContext>();
            // SAFETY: Restore engine to scene. The scene_ptr is valid for the
            // lifetime of the Scene method that created this guard.
            unsafe {
                (*self.scene_ptr).script_engine = Some(engine);
            }
        }
    }
}

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
        // ScriptEngineGuard ensures the engine is restored even on panic.
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for uuid in &uuids {
            guard.engine_mut().call_entity_on_create(*uuid);
        }

        // Guard drop restores engine and cleans up SceneScriptContext.
    }

    /// Tear down the Lua script engine: call `on_destroy()` per entity,
    /// drop the engine, and reset loaded flags.
    pub(super) fn on_lua_scripting_stop(&mut self) {
        let _timer = crate::profiling::ProfileTimer::new("Scene::on_lua_scripting_stop");

        // --- Setup phase (uses &mut self) ---
        let engine = match self.script_engine.take() {
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
        // Use guard for cleanup safety, then consume without restoring
        // (engine is intentionally dropped on stop).
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: std::ptr::null(),
        };
        guard.engine_mut().lua().set_app_data(ctx);

        for uuid in &uuids {
            guard.engine_mut().call_entity_on_destroy(*uuid);
        }

        // Consume the guard and drop the engine (intentional — we're stopping).
        let engine = guard.into_engine();
        engine.lua().remove_app_data::<SceneScriptContext>();
        drop(engine);

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
        // ScriptEngineGuard ensures the engine is restored even on panic.
        let mut guard = ScriptEngineGuard::new(engine, scene_ptr);

        // Call on_create for newly loaded scripts.
        if !unloaded.is_empty() {
            let ctx = SceneScriptContext {
                scene: scene_ptr,
                input: input as *const Input,
            };
            guard.engine_mut().lua().set_app_data(ctx);

            for (_, uuid, _) in &unloaded {
                guard.engine_mut().call_entity_on_create(*uuid);
            }

            guard
                .engine_mut()
                .lua()
                .remove_app_data::<SceneScriptContext>();
        }

        if uuids.is_empty() {
            // Guard drop restores engine.
            return;
        }

        // Set scene + input context for on_update.
        let ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        guard.engine_mut().lua().set_app_data(ctx);

        // Initialize deferred timer ops so Lua timer bindings work during scripts.
        guard.engine_mut().init_pending_timer_ops();

        for uuid in &uuids {
            guard
                .engine_mut()
                .call_entity_on_update(*uuid, dt.seconds());
        }

        // Apply any timer creates/cancels that scripts requested.
        guard.engine_mut().apply_pending_timer_ops();

        // Tick timers (set_timeout / set_interval).
        // Context must be active so timer callbacks can access the scene.
        let timer_ctx = SceneScriptContext {
            scene: scene_ptr,
            input: input as *const Input,
        };
        guard.engine_mut().lua().set_app_data(timer_ctx);
        guard.engine_mut().tick_timers(dt.seconds());

        // Guard drop restores engine and cleans up SceneScriptContext.
        drop(guard);

        unsafe {
            (*scene_ptr).flush_pending_destroys();
        }
    }

    /// Access the script engine (if active).
    pub fn script_engine(&self) -> Option<&ScriptEngine> {
        self.script_engine.as_ref()
    }
}
