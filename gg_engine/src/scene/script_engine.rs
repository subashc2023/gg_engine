use mlua::prelude::*;
use mlua::{LuaOptions, StdLib};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Lua standard libraries allowed in game scripts.
/// Excludes: io, os, package (filesystem/process access), debug, ffi.
fn script_stdlib() -> StdLib {
    StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::BIT | StdLib::JIT
}

/// A value that can be exposed from a Lua script's `fields` table.
/// Supports the Lua primitive types that map to editor UI widgets.
///
/// The `untagged` serde representation produces clean YAML (e.g. `speed: 5.0`
/// instead of `speed: !Float 5.0`). Variant order matters: `Bool` must come
/// first so that `true`/`false` are not misinterpreted as strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScriptFieldValue {
    Bool(bool),
    Float(f64),
    String(String),
}

impl ScriptFieldValue {
    /// Convert a Lua value into a `ScriptFieldValue`, if supported.
    pub(crate) fn from_lua_value(value: &LuaValue) -> Option<Self> {
        match value {
            LuaValue::Boolean(b) => Some(Self::Bool(*b)),
            LuaValue::Integer(n) => Some(Self::Float(*n as f64)),
            LuaValue::Number(n) => Some(Self::Float(*n)),
            LuaValue::String(s) => s.to_str().ok().map(|s| Self::String(s.to_string())),
            _ => None,
        }
    }

    /// Push this value into Lua.
    pub(crate) fn to_lua(&self, lua: &Lua) -> LuaResult<LuaValue> {
        match self {
            Self::Bool(b) => Ok(LuaValue::Boolean(*b)),
            Self::Float(n) => Ok(LuaValue::Number(*n)),
            Self::String(s) => lua.create_string(s).map(LuaValue::String),
        }
    }
}

/// Lua scripting engine backed by LuaJIT via `mlua`.
///
/// Each `ScriptEngine` owns its own Lua state. Designed to be owned by
/// [`Scene`](super::Scene), mirroring the `PhysicsWorld2D` ownership pattern.
/// A new Lua state is created on play-mode start and dropped on stop.
///
/// Each entity with a Lua script gets its own isolated environment table
/// stored in the Lua registry via `entity_envs`.
/// Maximum consecutive errors before a script is auto-disabled.
const MAX_SCRIPT_ERRORS: u32 = 10;

/// Lua named-registry key for the master entity-environments table.
/// Callbacks (`get_script_field`, `set_script_field`) look up entity envs
/// directly from this table, avoiding any raw pointer to `ScriptEngine`.
pub(crate) const ENTITY_ENVS_REGISTRY_KEY: &str = "__gg_entity_envs";

/// A scheduled timer created by `Engine.set_timeout` or `Engine.set_interval`.
pub(crate) struct ScriptTimer {
    /// Entity that owns this timer (cleaned up when entity is removed).
    pub entity_uuid: u64,
    /// Time remaining until the callback fires (seconds).
    pub remaining: f32,
    /// If Some, the timer repeats with this interval. None = one-shot.
    pub interval: Option<f32>,
    /// Lua registry key for the callback function.
    pub callback_key: LuaRegistryKey,
}

/// Stored in Lua app_data during `call_entity_function` so timer/callback
/// bindings can identify which entity is currently executing.
pub(crate) struct CurrentEntityUuid(pub u64);

/// Deferred timer operations queued by Lua scripts during execution.
/// Stored in Lua app_data because `scene.script_engine` is temporarily
/// taken during script execution.
pub(crate) struct PendingTimerOps {
    pub creates: Vec<PendingTimerCreate>,
    pub cancels: Vec<usize>,
    pub next_id: usize,
}

pub(crate) struct PendingTimerCreate {
    pub id: usize,
    pub entity_uuid: u64,
    pub delay: f32,
    pub repeating: bool,
    pub callback_key: LuaRegistryKey,
}

pub struct ScriptEngine {
    lua: Lua,
    /// Per-entity Lua environments keyed by entity UUID (u64).
    entity_envs: HashMap<u64, LuaRegistryKey>,
    /// Consecutive error counts per entity per callback name — used to auto-disable broken scripts.
    /// Two-level map so lookups can use `&str` (no per-frame String allocation).
    error_counts: HashMap<u64, HashMap<String, u32>>,
    /// Active timers keyed by timer ID.
    pub(crate) timers: HashMap<usize, ScriptTimer>,
    /// Next timer ID (monotonically increasing to avoid reuse within a session).
    pub(crate) next_timer_id: usize,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptEngine {
    /// Create a new LuaJIT state with standard libraries loaded.
    ///
    /// Registers all engine bindings (ScriptGlue) into the Lua state.
    /// Maximum instructions before a Lua callback is interrupted.
    /// 10 million instructions ≈ a few seconds of CPU time.
    const INSTRUCTION_LIMIT: u32 = 10_000_000;

    pub fn new() -> Self {
        let lua = Lua::new_with(script_stdlib(), LuaOptions::default()).unwrap_or_else(|e| {
            panic!(
                "ScriptEngine: failed to create sandboxed Lua state \
                 (requested libs: table, string, math, bit, jit): {e}"
            )
        });

        // Install an instruction count hook to prevent infinite loops.
        lua.set_hook(
            mlua::HookTriggers::new().every_nth_instruction(Self::INSTRUCTION_LIMIT),
            |_lua, _debug| {
                Err(mlua::Error::RuntimeError(
                    "Script exceeded instruction limit (possible infinite loop)".into(),
                ))
            },
        );

        if let Err(e) = super::script_glue::register_all(&lua) {
            log::error!("ScriptEngine: failed to register script glue: {}", e);
        }

        // Create the master entity-envs table in the Lua registry so that
        // Lua callbacks can look up entity environments without needing a
        // raw pointer back to ScriptEngine.
        match lua.create_table() {
            Ok(t) => {
                if let Err(e) = lua.set_named_registry_value(ENTITY_ENVS_REGISTRY_KEY, t) {
                    log::error!("ScriptEngine: failed to register entity envs table: {}", e);
                }
            }
            Err(e) => {
                log::error!("ScriptEngine: failed to create entity envs table: {}", e);
            }
        }

        log::info!("ScriptEngine: LuaJIT state initialized");
        Self {
            lua,
            entity_envs: HashMap::new(),
            error_counts: HashMap::new(),
            timers: HashMap::new(),
            next_timer_id: 0,
        }
    }

    // -----------------------------------------------------------------
    // Module system
    // -----------------------------------------------------------------

    /// Register a safe `require()` function that loads `.lua` modules from the
    /// given search path. Modules are cached — each file is loaded at most once.
    ///
    /// Module names use dot-separated paths (e.g. `require("utils.math")` loads
    /// `<search_path>/utils/math.lua`). Traversal outside the search path is
    /// blocked (no `..` allowed).
    pub fn register_module_loader(&self, search_path: std::path::PathBuf) {
        // Create a module cache table in the Lua registry.
        let cache = match self.lua.create_table() {
            Ok(t) => t,
            Err(e) => {
                log::error!("ScriptEngine: failed to create module cache: {e}");
                return;
            }
        };
        let cache_key = match self.lua.create_registry_value(cache) {
            Ok(k) => k,
            Err(e) => {
                log::error!("ScriptEngine: failed to register module cache: {e}");
                return;
            }
        };

        // Share the search path and cache key with the closure via Arc.
        let search_path = std::sync::Arc::new(search_path);
        let cache_key = std::sync::Arc::new(cache_key);

        let require_fn = self.lua.create_function(move |lua, module_name: String| {
            // Validate module name: no ".." segments, no absolute paths.
            if module_name.contains("..") || module_name.starts_with('/') || module_name.starts_with('\\') {
                return Err(mlua::Error::RuntimeError(format!(
                    "require: invalid module name '{module_name}' (path traversal not allowed)"
                )));
            }

            // Check the cache first.
            let cache: LuaTable = lua.registry_value(&cache_key)?;
            if let Ok(cached) = cache.get::<LuaValue>(module_name.as_str()) {
                if !cached.is_nil() {
                    return Ok(cached);
                }
            }

            // Convert dot-separated module name to a relative path.
            let rel_path = module_name.replace('.', std::path::MAIN_SEPARATOR_STR) + ".lua";
            let full_path = search_path.join(&rel_path);

            // Verify the resolved path is within the search directory.
            let canonical_search = match search_path.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "require: module search path '{}' not found",
                        search_path.display()
                    )));
                }
            };
            let canonical_module = match full_path.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "require: module '{module_name}' not found (looked for '{}')",
                        full_path.display()
                    )));
                }
            };
            if !canonical_module.starts_with(&canonical_search) {
                return Err(mlua::Error::RuntimeError(format!(
                    "require: module '{module_name}' escapes the search path"
                )));
            }

            // Read and execute the module file.
            let source = std::fs::read_to_string(&full_path).map_err(|e| {
                mlua::Error::RuntimeError(format!(
                    "require: failed to read module '{module_name}' ({}): {e}",
                    full_path.display()
                ))
            })?;

            let chunk_name = full_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| module_name.clone());

            let result: LuaValue = lua
                .load(&source)
                .set_name(&chunk_name)
                .eval()?;

            // If the module returned nil/nothing, store `true` as a sentinel so
            // subsequent requires don't re-execute the file.
            let to_cache = if result.is_nil() {
                LuaValue::Boolean(true)
            } else {
                result.clone()
            };
            cache.set(module_name.as_str(), to_cache)?;

            Ok(result)
        });

        match require_fn {
            Ok(f) => {
                if let Err(e) = self.lua.globals().set("require", f) {
                    log::error!("ScriptEngine: failed to register require(): {e}");
                }
            }
            Err(e) => {
                log::error!("ScriptEngine: failed to create require function: {e}");
            }
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
        let _timer = crate::profiling::ProfileTimer::new("ScriptEngine::create_entity_env");
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

        // Mirror the env into the Lua-side master table so callbacks can
        // access it without going through a ScriptEngine pointer.
        if let Ok(envs_table) = self
            .lua
            .named_registry_value::<LuaTable>(ENTITY_ENVS_REGISTRY_KEY)
        {
            if let Err(e) = envs_table.set(uuid, env.clone()) {
                log::error!(
                    "ScriptEngine: failed to mirror env to registry table: {}",
                    e
                );
            }
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
    pub fn call_entity_on_create(&mut self, uuid: u64) -> bool {
        self.call_entity_function(uuid, "on_create", ())
    }

    /// Call `on_update(dt)` in an entity's environment.
    pub fn call_entity_on_update(&mut self, uuid: u64, dt: f32) -> bool {
        self.call_entity_function(uuid, "on_update", dt)
    }

    /// Call `on_fixed_update(dt)` in an entity's environment.
    ///
    /// Called once per physics fixed step so that impulses/forces are applied
    /// at a consistent rate regardless of render frame rate.
    pub fn call_entity_on_fixed_update(&mut self, uuid: u64, dt: f32) -> bool {
        self.call_entity_function(uuid, "on_fixed_update", dt)
    }

    /// Call `on_destroy()` in an entity's environment.
    pub fn call_entity_on_destroy(&mut self, uuid: u64) -> bool {
        self.call_entity_function(uuid, "on_destroy", ())
    }

    /// Call `on_sound_finished()` in an entity's environment.
    pub fn call_entity_on_sound_finished(&mut self, uuid: u64) -> bool {
        self.call_entity_function(uuid, "on_sound_finished", ())
    }

    /// Call a collision callback (e.g. `on_collision_enter` / `on_collision_exit`)
    /// in an entity's environment, passing the other entity's UUID.
    pub fn call_entity_collision(
        &mut self,
        uuid: u64,
        callback_name: &str,
        other_uuid: u64,
    ) -> bool {
        self.call_entity_function(uuid, callback_name, other_uuid)
    }

    /// Call a UI interaction callback (e.g. `on_ui_hover_enter`, `on_ui_click`)
    /// in an entity's environment. No arguments.
    pub fn call_entity_ui_callback(&mut self, uuid: u64, callback_name: &str) -> bool {
        self.call_entity_function(uuid, callback_name, ())
    }

    /// Call a named callback with a string argument (e.g. `on_animation_finished(clip_name)`).
    pub fn call_entity_callback_str(
        &mut self,
        uuid: u64,
        callback_name: &str,
        arg: String,
    ) -> bool {
        self.call_entity_function(uuid, callback_name, arg)
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
        // Also remove from the Lua-side master table.
        if let Ok(envs_table) = self
            .lua
            .named_registry_value::<LuaTable>(ENTITY_ENVS_REGISTRY_KEY)
        {
            envs_table.set(uuid, LuaValue::Nil).ok();
        }
    }

    /// Shared helper: look up a function by name in an entity's env table
    /// using `raw_get` (does NOT fall through to globals via __index).
    ///
    /// Tracks consecutive errors per entity and auto-disables scripts that
    /// exceed [`MAX_SCRIPT_ERRORS`] failures to prevent log spam.
    fn call_entity_function<A: IntoLuaMulti>(&mut self, uuid: u64, name: &str, args: A) -> bool {
        // Skip entities that have been auto-disabled for this callback.
        // Uses &str lookup on inner HashMap to avoid String allocation.
        if let Some(entity_errors) = self.error_counts.get(&uuid) {
            if let Some(&count) = entity_errors.get(name) {
                if count >= MAX_SCRIPT_ERRORS {
                    return false;
                }
            }
        }

        let key = match self.entity_envs.get(&uuid) {
            Some(k) => k,
            None => return false,
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

        // Track which entity is currently executing (used by timer bindings).
        self.lua.set_app_data(CurrentEntityUuid(uuid));

        if let Err(e) = func.call::<()>(args) {
            let count = self
                .error_counts
                .entry(uuid)
                .or_default()
                .entry(name.to_string())
                .or_insert(0);
            *count += 1;
            if *count == MAX_SCRIPT_ERRORS {
                log::error!(
                    "ScriptEngine: entity {} '{}' disabled after {} consecutive errors. \
                     Last error: {}",
                    uuid,
                    name,
                    MAX_SCRIPT_ERRORS,
                    e
                );
            } else {
                log::error!(
                    "ScriptEngine: error calling '{}' for entity {} ({}/{}): {}",
                    name,
                    uuid,
                    count,
                    MAX_SCRIPT_ERRORS,
                    e
                );
            }
            self.lua.remove_app_data::<CurrentEntityUuid>();
            return false;
        }

        self.lua.remove_app_data::<CurrentEntityUuid>();

        // Reset error count for this specific callback on success.
        if let Some(entity_errors) = self.error_counts.get_mut(&uuid) {
            entity_errors.remove(name);
        }
        true
    }

    // -----------------------------------------------------------------
    // Script field discovery and access
    // -----------------------------------------------------------------

    /// Execute a script in a temporary Lua state and read its `fields` table.
    /// Returns `(name, default_value)` pairs sorted by name for stable UI order.
    /// Used by the editor in edit mode (no running ScriptEngine needed).
    pub fn discover_fields(script_path: &str) -> Vec<(String, ScriptFieldValue)> {
        let path = Path::new(script_path);
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!(
                    "ScriptEngine::discover_fields: failed to read '{}': {}",
                    path.display(),
                    e
                );
                return Vec::new();
            }
        };

        let lua = match Lua::new_with(script_stdlib(), LuaOptions::default()) {
            Ok(l) => l,
            Err(e) => {
                log::warn!(
                    "ScriptEngine::discover_fields: failed to create Lua state: {}",
                    e
                );
                return Vec::new();
            }
        };

        // Apply the same instruction limit as the main engine to prevent hangs.
        lua.set_hook(
            mlua::HookTriggers::new().every_nth_instruction(Self::INSTRUCTION_LIMIT),
            |_lua, _debug| {
                Err(mlua::Error::RuntimeError(
                    "Script exceeded instruction limit (possible infinite loop)".into(),
                ))
            },
        );

        // Register a stub Engine table so scripts that reference Engine.* at file
        // scope don't crash during field discovery. Any method call returns nil.
        if let Ok(stub) = lua.create_table() {
            if let Ok(meta) = lua.create_table() {
                let _ = meta.set(
                    "__index",
                    lua.create_function(|lua, (_t, _k): (LuaValue, LuaValue)| {
                        lua.create_function(|_, _: mlua::MultiValue| Ok(LuaValue::Nil))
                    })
                    .unwrap_or_else(|_| lua.create_function(|_, _: ()| Ok(())).unwrap()),
                );
                stub.set_metatable(Some(meta));
            }
            let _ = lua.globals().set("Engine", stub);
        }

        if let Err(e) = lua.load(&source).exec() {
            log::warn!(
                "ScriptEngine::discover_fields: error executing '{}': {}",
                path.display(),
                e
            );
            return Vec::new();
        }

        let fields_table: LuaTable = match lua.globals().get("fields") {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };

        let mut fields = Vec::new();
        if let Ok(pairs) = fields_table
            .pairs::<String, LuaValue>()
            .collect::<Result<Vec<_>, _>>()
        {
            for (key, value) in pairs {
                if let Some(sfv) = ScriptFieldValue::from_lua_value(&value) {
                    fields.push((key, sfv));
                }
            }
        }
        fields.sort_by(|(a, _), (b, _)| a.cmp(b));
        fields
    }

    /// Read all fields from a running entity's `fields` table.
    /// Returns `None` if the entity has no environment.
    pub fn get_entity_fields(&self, uuid: u64) -> Option<Vec<(String, ScriptFieldValue)>> {
        let key = self.entity_envs.get(&uuid)?;
        let env: LuaTable = self.lua.registry_value(key).ok()?;

        // Use raw_get so we don't fall through to _G.
        let fields_table: LuaTable = match env.raw_get("fields") {
            Ok(t) => t,
            Err(_) => return Some(Vec::new()),
        };

        let mut fields = Vec::new();
        if let Ok(pairs) = fields_table
            .pairs::<String, LuaValue>()
            .collect::<Result<Vec<_>, _>>()
        {
            for (key, value) in pairs {
                if let Some(sfv) = ScriptFieldValue::from_lua_value(&value) {
                    fields.push((key, sfv));
                }
            }
        }
        fields.sort_by(|(a, _), (b, _)| a.cmp(b));
        Some(fields)
    }

    /// Read a single field value from a running entity's `fields` table.
    pub fn get_entity_field(&self, uuid: u64, name: &str) -> Option<ScriptFieldValue> {
        let key = self.entity_envs.get(&uuid)?;
        let env: LuaTable = self.lua.registry_value(key).ok()?;
        let fields_table: LuaTable = env.raw_get("fields").ok()?;
        let value: LuaValue = fields_table.get(name).ok()?;
        ScriptFieldValue::from_lua_value(&value)
    }

    /// Set a single field value on a running entity's `fields` table.
    /// Returns `true` on success.
    pub fn set_entity_field(&self, uuid: u64, name: &str, value: &ScriptFieldValue) -> bool {
        let key = match self.entity_envs.get(&uuid) {
            Some(k) => k,
            None => return false,
        };
        let env: LuaTable = match self.lua.registry_value(key) {
            Ok(t) => t,
            Err(_) => return false,
        };
        let fields_table: LuaTable = match env.raw_get("fields") {
            Ok(t) => t,
            Err(_) => return false,
        };
        match value.to_lua(&self.lua) {
            Ok(lua_val) => fields_table.set(name, lua_val).is_ok(),
            Err(_) => false,
        }
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
            log::error!("ScriptEngine: error executing '{}': {}", path.display(), e);
            return false;
        }

        log::info!("ScriptEngine: loaded '{}'", path.display());
        true
    }

    /// Log all global function names (equivalent of Cherno's "print assembly types").
    pub fn dump_globals(&self) {
        let globals = self.lua.globals();

        let mut functions = Vec::new();
        if let Ok(pairs) = globals
            .pairs::<String, LuaValue>()
            .collect::<Result<Vec<_>, _>>()
        {
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

    // -----------------------------------------------------------------
    // Timer system
    // -----------------------------------------------------------------

    /// Schedule a timer. Returns the timer ID.
    pub fn add_timer(
        &mut self,
        entity_uuid: u64,
        delay: f32,
        repeating: bool,
        callback: LuaFunction,
    ) -> usize {
        let id = self.next_timer_id;
        self.next_timer_id += 1;

        let key = match self.lua.create_registry_value(callback) {
            Ok(k) => k,
            Err(e) => {
                log::error!("ScriptEngine: failed to register timer callback: {}", e);
                return id;
            }
        };

        let timer = ScriptTimer {
            entity_uuid,
            remaining: delay,
            interval: if repeating { Some(delay) } else { None },
            callback_key: key,
        };

        self.timers.insert(id, timer);
        id
    }

    /// Cancel a timer by ID.
    pub fn cancel_timer(&mut self, timer_id: usize) {
        self.timers.remove(&timer_id);
    }

    /// Tick all timers, firing callbacks whose time has elapsed.
    pub fn tick_timers(&mut self, dt: f32) {
        // Collect IDs of timers that need to fire (avoids borrow conflict).
        let fire_ids: Vec<usize> = self
            .timers
            .iter_mut()
            .filter_map(|(&id, timer)| {
                timer.remaining -= dt;
                if timer.remaining <= 0.0 {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        for id in fire_ids {
            // Temporarily remove the timer to call its callback.
            let Some(mut timer) = self.timers.remove(&id) else {
                continue;
            };

            let callback: Result<LuaFunction, _> = self.lua.registry_value(&timer.callback_key);
            if let Ok(func) = callback {
                if let Err(e) = func.call::<()>(()) {
                    log::error!(
                        "ScriptEngine: timer callback error for entity {}: {}",
                        timer.entity_uuid,
                        e
                    );
                }
            }

            // Re-arm if repeating, otherwise drop.
            if let Some(interval) = timer.interval {
                timer.remaining = interval;
                self.timers.insert(id, timer);
            }
        }
    }

    /// Remove all timers owned by a specific entity.
    pub fn remove_entity_timers(&mut self, entity_uuid: u64) {
        self.timers
            .retain(|_, timer| timer.entity_uuid != entity_uuid);
    }

    /// Initialize pending timer ops in Lua app_data before script execution.
    /// Must be called before `call_entity_on_update` so Lua timer bindings work.
    pub fn init_pending_timer_ops(&mut self) {
        self.lua.set_app_data(PendingTimerOps {
            creates: Vec::new(),
            cancels: Vec::new(),
            next_id: self.next_timer_id,
        });
    }

    /// Drain pending timer ops from Lua app_data into the engine.
    /// Must be called after all `call_entity_on_update` calls.
    pub fn apply_pending_timer_ops(&mut self) {
        let Some(pending) = self.lua.remove_app_data::<PendingTimerOps>() else {
            return;
        };

        // Update next_timer_id to stay in sync with deferred allocations.
        self.next_timer_id = pending.next_id;

        // Process cancellations first.
        for id in pending.cancels {
            self.timers.remove(&id);
        }

        // Process creations.
        for op in pending.creates {
            let timer = ScriptTimer {
                entity_uuid: op.entity_uuid,
                remaining: op.delay,
                interval: if op.repeating { Some(op.delay) } else { None },
                callback_key: op.callback_key,
            };
            self.timers.insert(op.id, timer);
        }
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
        engine.lua().load("function foo() end").exec().unwrap();
        assert!(engine.has_function("foo"));
        assert!(!engine.has_function("bar"));
    }

    #[test]
    fn dump_globals_runs_without_error() {
        let engine = ScriptEngine::new();
        engine.lua().load("function test_fn() end").exec().unwrap();
        // Just verify it doesn't panic.
        engine.dump_globals();
    }

    #[test]
    fn engine_table_available_after_new() {
        // ScriptEngine::new() should register the Engine table via script_glue.
        let engine = ScriptEngine::new();
        let has_engine: bool = engine.lua().globals().get::<LuaTable>("Engine").is_ok();
        assert!(
            has_engine,
            "Engine table should exist after ScriptEngine::new()"
        );
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
        assert!(
            (ny - 1.0).abs() < 0.001,
            "normalize(0,3,0) should be (0,1,0)"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn discover_fields_from_script() {
        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_discover_fields.lua");
        std::fs::write(
            &path,
            r#"
fields = {
    speed = 5.0,
    is_active = true,
    name = "player",
}

function on_update(dt)
end
"#,
        )
        .unwrap();

        let fields = ScriptEngine::discover_fields(&path.to_string_lossy());
        assert_eq!(fields.len(), 3);

        // Sorted by name: is_active, name, speed
        assert_eq!(fields[0].0, "is_active");
        assert_eq!(fields[0].1, ScriptFieldValue::Bool(true));
        assert_eq!(fields[1].0, "name");
        assert_eq!(fields[1].1, ScriptFieldValue::String("player".into()));
        assert_eq!(fields[2].0, "speed");
        assert_eq!(fields[2].1, ScriptFieldValue::Float(5.0));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn discover_fields_no_fields_table() {
        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_no_fields.lua");
        std::fs::write(&path, "function on_update(dt) end\n").unwrap();

        let fields = ScriptEngine::discover_fields(&path.to_string_lossy());
        assert!(fields.is_empty());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn entity_field_get_set() {
        let mut engine = ScriptEngine::new();

        let dir = std::env::temp_dir();
        let path = dir.join("gg_test_entity_fields.lua");
        std::fs::write(
            &path,
            r#"
fields = {
    speed = 1.0,
    jump = true,
}

function on_update(dt)
    -- use fields.speed
end
"#,
        )
        .unwrap();

        assert!(engine.create_entity_env(42, &path.to_string_lossy()));

        // Read fields.
        let fields = engine.get_entity_fields(42).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(
            engine.get_entity_field(42, "speed"),
            Some(ScriptFieldValue::Float(1.0))
        );
        assert_eq!(
            engine.get_entity_field(42, "jump"),
            Some(ScriptFieldValue::Bool(true))
        );

        // Write field.
        assert!(engine.set_entity_field(42, "speed", &ScriptFieldValue::Float(9.9)));
        assert_eq!(
            engine.get_entity_field(42, "speed"),
            Some(ScriptFieldValue::Float(9.9))
        );

        // Non-existent entity.
        assert!(engine.get_entity_fields(999).is_none());
        assert!(!engine.set_entity_field(999, "speed", &ScriptFieldValue::Float(0.0)));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn script_field_value_serde_round_trip() {
        let values = vec![
            ScriptFieldValue::Bool(true),
            ScriptFieldValue::Float(3.15),
            ScriptFieldValue::String("hello".into()),
        ];

        for v in &values {
            let yaml = serde_yaml_ng::to_string(v).unwrap();
            let back: ScriptFieldValue = serde_yaml_ng::from_str(&yaml).unwrap();
            assert_eq!(&back, v);
        }
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
