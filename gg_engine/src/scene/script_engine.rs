use mlua::prelude::*;
use std::collections::HashMap;
use std::path::Path;

/// Lua scripting engine backed by LuaJIT via `mlua`.
///
/// Each `ScriptEngine` owns its own Lua state. Designed to be owned by
/// [`Scene`](super::Scene), mirroring the `PhysicsWorld2D` ownership pattern.
/// A new Lua state is created on play-mode start and dropped on stop.
///
/// Each entity with a Lua script gets its own isolated environment table
/// stored in the Lua registry via `entity_envs`.
pub struct ScriptEngine {
    lua: Lua,
    /// Per-entity Lua environments keyed by entity UUID (u64).
    entity_envs: HashMap<u64, LuaRegistryKey>,
}

impl ScriptEngine {
    /// Create a new LuaJIT state with standard libraries loaded.
    ///
    /// Registers all engine bindings (ScriptGlue) into the Lua state.
    pub fn new() -> Self {
        let lua = Lua::new();

        if let Err(e) = super::script_glue::register_all(&lua) {
            log::error!("ScriptEngine: failed to register script glue: {}", e);
        }

        log::info!("ScriptEngine: LuaJIT state initialized");
        Self {
            lua,
            entity_envs: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------
    // Per-entity environment methods
    // -----------------------------------------------------------------

    /// Create an isolated Lua environment for an entity, load a script into it.
    ///
    /// The environment inherits globals (Engine.*, print, etc.) via a metatable
    /// with `__index = _G`. The entity's UUID is set as `entity_id` in the env.
    /// Returns `true` on success.
    pub fn create_entity_env(&mut self, uuid: u64, script_path: &str) -> bool {
        let path = Path::new(script_path);
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "ScriptEngine: failed to read script '{}': {}",
                    path.display(),
                    e
                );
                return false;
            }
        };

        let chunk_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());

        // Create the environment table.
        let env = match self.lua.create_table() {
            Ok(t) => t,
            Err(e) => {
                log::error!("ScriptEngine: failed to create env table: {}", e);
                return false;
            }
        };

        // Set entity_id in the environment.
        if let Err(e) = env.set("entity_id", uuid) {
            log::error!("ScriptEngine: failed to set entity_id: {}", e);
            return false;
        }

        // Create metatable with __index = _G so the env inherits globals.
        let meta = match self.lua.create_table() {
            Ok(t) => t,
            Err(e) => {
                log::error!("ScriptEngine: failed to create metatable: {}", e);
                return false;
            }
        };
        if let Err(e) = meta.set("__index", self.lua.globals()) {
            log::error!("ScriptEngine: failed to set __index: {}", e);
            return false;
        }
        env.set_metatable(Some(meta));

        // Load and execute the script in the entity's environment.
        if let Err(e) = self
            .lua
            .load(&source)
            .set_name(&chunk_name)
            .set_environment(env.clone())
            .exec()
        {
            log::error!(
                "ScriptEngine: error executing '{}' for entity {}: {}",
                path.display(),
                uuid,
                e
            );
            return false;
        }

        // Store the environment in the Lua registry.
        match self.lua.create_registry_value(env) {
            Ok(key) => {
                self.entity_envs.insert(uuid, key);
            }
            Err(e) => {
                log::error!("ScriptEngine: failed to store env in registry: {}", e);
                return false;
            }
        }

        log::info!(
            "ScriptEngine: loaded '{}' for entity {}",
            path.display(),
            uuid
        );
        true
    }

    /// Call `on_create()` in an entity's environment.
    pub fn call_entity_on_create(&self, uuid: u64) -> bool {
        self.call_entity_function(uuid, "on_create", ())
    }

    /// Call `on_update(dt)` in an entity's environment.
    pub fn call_entity_on_update(&self, uuid: u64, dt: f32) -> bool {
        self.call_entity_function(uuid, "on_update", dt)
    }

    /// Call `on_destroy()` in an entity's environment.
    pub fn call_entity_on_destroy(&self, uuid: u64) -> bool {
        self.call_entity_function(uuid, "on_destroy", ())
    }

    /// Returns all tracked entity UUIDs.
    pub fn entity_uuids(&self) -> Vec<u64> {
        self.entity_envs.keys().copied().collect()
    }

    /// Remove an entity's environment from the registry.
    pub fn remove_entity_env(&mut self, uuid: u64) {
        if let Some(key) = self.entity_envs.remove(&uuid) {
            self.lua.remove_registry_value(key).ok();
        }
    }

    /// Shared helper: look up a function by name in an entity's env table
    /// using `raw_get` (does NOT fall through to globals via __index).
    fn call_entity_function<A: IntoLuaMulti>(&self, uuid: u64, name: &str, args: A) -> bool {
        let key = match self.entity_envs.get(&uuid) {
            Some(k) => k,
            None => {
                log::error!("ScriptEngine: no env for entity {}", uuid);
                return false;
            }
        };

        let env: LuaTable = match self.lua.registry_value(key) {
            Ok(t) => t,
            Err(e) => {
                log::error!(
                    "ScriptEngine: failed to retrieve env for entity {}: {}",
                    uuid,
                    e
                );
                return false;
            }
        };

        // Use raw_get to avoid falling through to _G via __index.
        let func: LuaFunction = match env.raw_get(name) {
            Ok(f) => f,
            Err(_) => {
                // Function not defined in this script — not an error.
                return true;
            }
        };

        if let Err(e) = func.call::<()>(args) {
            log::error!(
                "ScriptEngine: error calling '{}' for entity {}: {}",
                name,
                uuid,
                e
            );
            return false;
        }

        true
    }

    // -----------------------------------------------------------------
    // Global methods (kept for tests and backwards compatibility)
    // -----------------------------------------------------------------

    /// Load and execute a Lua script file, registering its globals.
    ///
    /// Returns `true` on success, `false` on failure (errors are logged).
    pub fn load_script(&self, path: &str) -> bool {
        let path = Path::new(path);
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "ScriptEngine: failed to read script '{}': {}",
                    path.display(),
                    e
                );
                return false;
            }
        };

        let chunk_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());

        if let Err(e) = self.lua.load(&source).set_name(&chunk_name).exec() {
            log::error!(
                "ScriptEngine: error executing '{}': {}",
                path.display(),
                e
            );
            return false;
        }

        log::info!("ScriptEngine: loaded '{}'", path.display());
        true
    }

    /// Log all global function names (equivalent of Cherno's "print assembly types").
    pub fn dump_globals(&self) {
        let globals = self.lua.globals();

        let mut functions = Vec::new();
        if let Ok(pairs) = globals.pairs::<String, LuaValue>().collect::<Result<Vec<_>, _>>() {
            for (key, value) in pairs {
                if value.is_function() {
                    functions.push(key);
                }
            }
        }

        functions.sort();
        log::info!("ScriptEngine: {} global functions:", functions.len());
        for name in &functions {
            log::info!("  - {}", name);
        }
    }

    /// Call a global Lua function with no arguments and no return value.
    pub fn call_function(&self, name: &str) -> bool {
        let globals = self.lua.globals();
        let func: LuaFunction = match globals.get(name) {
            Ok(f) => f,
            Err(e) => {
                log::error!("ScriptEngine: function '{}' not found: {}", name, e);
                return false;
            }
        };

        if let Err(e) = func.call::<()>(()) {
            log::error!("ScriptEngine: error calling '{}': {}", name, e);
            return false;
        }

        true
    }

    /// Call a global Lua function with arguments.
    pub fn call_function_with_args<A: IntoLuaMulti>(&self, name: &str, args: A) -> bool {
        let globals = self.lua.globals();
        let func: LuaFunction = match globals.get(name) {
            Ok(f) => f,
            Err(e) => {
                log::error!("ScriptEngine: function '{}' not found: {}", name, e);
                return false;
            }
        };

        if let Err(e) = func.call::<()>(args) {
            log::error!("ScriptEngine: error calling '{}': {}", name, e);
            return false;
        }

        true
    }

    /// Call a global Lua function and return a value.
    pub fn call_function_ret<R: FromLuaMulti>(&self, name: &str) -> Option<R> {
        let globals = self.lua.globals();
        let func: LuaFunction = match globals.get(name) {
            Ok(f) => f,
            Err(e) => {
                log::error!("ScriptEngine: function '{}' not found: {}", name, e);
                return None;
            }
        };

        match func.call::<R>(()) {
            Ok(result) => Some(result),
            Err(e) => {
                log::error!("ScriptEngine: error calling '{}': {}", name, e);
                None
            }
        }
    }

    /// Check if a global function exists.
    pub fn has_function(&self, name: &str) -> bool {
        self.lua
            .globals()
            .get::<LuaValue>(name)
            .map(|v| v.is_function())
            .unwrap_or(false)
    }

    /// Access the underlying Lua state for advanced use.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_valid_state() {
        let engine = ScriptEngine::new();
        // Verify we can execute basic Lua.
        engine
            .lua()
            .load("x = 42")
            .exec()
            .expect("basic exec should work");
        let x: i32 = engine.lua().globals().get("x").unwrap();
        assert_eq!(x, 42);
    }

    #[test]
    fn load_script_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_script.lua");
        std::fs::write(
            &path,
            r#"
function hello()
    return "hello from lua"
end

function add(a, b)
    return a + b
end
"#,
        )
        .unwrap();

        let engine = ScriptEngine::new();
        assert!(engine.load_script(&path.to_string_lossy()));

        // Functions should be registered as globals.
        assert!(engine.has_function("hello"));
        assert!(engine.has_function("add"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_script_nonexistent_file() {
        let engine = ScriptEngine::new();
        assert!(!engine.load_script("nonexistent_script.lua"));
    }

    #[test]
    fn call_function_no_args() {
        let engine = ScriptEngine::new();
        engine
            .lua()
            .load("function greet() result = 'ok' end")
            .exec()
            .unwrap();
        assert!(engine.call_function("greet"));
        let result: String = engine.lua().globals().get("result").unwrap();
        assert_eq!(result, "ok");
    }

    #[test]
    fn call_function_not_found() {
        let engine = ScriptEngine::new();
        assert!(!engine.call_function("nonexistent"));
    }

    #[test]
    fn call_function_with_args() {
        let engine = ScriptEngine::new();
        engine
            .lua()
            .load("function add(a, b) sum = a + b end")
            .exec()
            .unwrap();
        assert!(engine.call_function_with_args("add", (3, 4)));
        let sum: i32 = engine.lua().globals().get("sum").unwrap();
        assert_eq!(sum, 7);
    }

    #[test]
    fn call_function_ret() {
        let engine = ScriptEngine::new();
        engine
            .lua()
            .load("function multiply(a, b) return a * b end")
            .exec()
            .unwrap();

        // call_function_ret requires args passed separately — we test with no-arg returning value.
        engine
            .lua()
            .load("function get_value() return 42 end")
            .exec()
            .unwrap();
        let result: Option<i32> = engine.call_function_ret("get_value");
        assert_eq!(result, Some(42));
    }

    #[test]
    fn has_function() {
        let engine = ScriptEngine::new();
        engine
            .lua()
            .load("function foo() end")
            .exec()
            .unwrap();
        assert!(engine.has_function("foo"));
        assert!(!engine.has_function("bar"));
    }

    #[test]
    fn dump_globals_runs_without_error() {
        let engine = ScriptEngine::new();
        engine
            .lua()
            .load("function test_fn() end")
            .exec()
            .unwrap();
        // Just verify it doesn't panic.
        engine.dump_globals();
    }

    #[test]
    fn engine_table_available_after_new() {
        // ScriptEngine::new() should register the Engine table via script_glue.
        let engine = ScriptEngine::new();
        let has_engine: bool = engine
            .lua()
            .globals()
            .get::<LuaTable>("Engine")
            .is_ok();
        assert!(has_engine, "Engine table should exist after ScriptEngine::new()");
    }

    #[test]
    fn full_integration_load_script_calls_engine_functions() {
        // Simulates the full play-mode flow: ScriptEngine::new() → load script → call on_create.
        let engine = ScriptEngine::new();

        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_glue_integration.lua");
        std::fs::write(
            &path,
            r#"
function on_create()
    Engine.rust_function()
    Engine.native_log("hello", 42)
    Engine.native_log_vector(1, 2, 3)

    dot_result = Engine.vector_dot(1, 0, 0, 0, 1, 0)
    cx, cy, cz = Engine.vector_cross(1, 0, 0, 0, 1, 0)
    nx, ny, nz = Engine.vector_normalize(0, 3, 0)
end
"#,
        )
        .unwrap();

        assert!(engine.load_script(&path.to_string_lossy()));
        assert!(engine.has_function("on_create"));
        assert!(engine.call_function("on_create"));

        // Verify return values were captured in Lua globals.
        let dot: f32 = engine.lua().globals().get("dot_result").unwrap();
        assert!(dot.abs() < 0.001, "dot(x, y) should be 0");

        let cz: f32 = engine.lua().globals().get("cz").unwrap();
        assert!((cz - 1.0).abs() < 0.001, "cross(x, y) should be z");

        let ny: f32 = engine.lua().globals().get("ny").unwrap();
        assert!((ny - 1.0).abs() < 0.001, "normalize(0,3,0) should be (0,1,0)");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn per_entity_env_isolation() {
        let mut engine = ScriptEngine::new();

        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_per_entity.lua");
        std::fs::write(
            &path,
            r#"
local my_value = 0

function on_create()
    my_value = entity_id
end

function on_update(dt)
    my_value = my_value + 1
end

function get_value()
    return my_value
end
"#,
        )
        .unwrap();

        // Create two entities with the same script.
        assert!(engine.create_entity_env(100, &path.to_string_lossy()));
        assert!(engine.create_entity_env(200, &path.to_string_lossy()));

        // Call on_create for both — each should store its own entity_id.
        assert!(engine.call_entity_on_create(100));
        assert!(engine.call_entity_on_create(200));

        // Call on_update a few times for entity 100 only.
        assert!(engine.call_entity_on_update(100, 0.016));
        assert!(engine.call_entity_on_update(100, 0.016));

        // Verify isolation: entity 100's value should be entity_id(100) + 2 calls = 102.
        // Entity 200's value should still be entity_id(200) = 200.
        let env100: LuaTable = engine
            .lua()
            .registry_value(engine.entity_envs.get(&100).unwrap())
            .unwrap();
        let func100: LuaFunction = env100.raw_get("get_value").unwrap();
        let val100: u64 = func100.call(()).unwrap();
        assert_eq!(val100, 102);

        let env200: LuaTable = engine
            .lua()
            .registry_value(engine.entity_envs.get(&200).unwrap())
            .unwrap();
        let func200: LuaFunction = env200.raw_get("get_value").unwrap();
        let val200: u64 = func200.call(()).unwrap();
        assert_eq!(val200, 200);

        // entity_uuids should return both.
        let mut uuids = engine.entity_uuids();
        uuids.sort();
        assert_eq!(uuids, vec![100, 200]);

        // Remove entity 100.
        engine.remove_entity_env(100);
        assert_eq!(engine.entity_uuids().len(), 1);

        std::fs::remove_file(&path).ok();
    }
}
