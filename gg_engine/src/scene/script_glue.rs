//! Centralized Rust↔Lua bindings — the Lua equivalent of Cherno's `ScriptGlue.cpp`.
//!
//! Every Rust function exposed to Lua scripts lives here. Functions are
//! registered under an `Engine` global table so scripts call e.g.
//! `Engine.native_log("hello", 42)`.

use mlua::prelude::*;

use crate::events::KeyCode;
use crate::input::Input;
use super::Scene;
use super::script_engine::ScriptEngine;

/// Runtime context set as Lua `app_data` during script execution.
///
/// Raw pointers are used to sidestep Rust borrow rules (take-modify-replace
/// pattern ensures safety — the scene is exclusively borrowed while scripts
/// run). `input` is null during `on_create` / `on_destroy`.
/// `script_engine` provides access to other entities' Lua environments for
/// cross-entity field reads/writes.
pub(crate) struct SceneScriptContext {
    pub scene: *mut Scene,
    pub input: *const Input,
    pub script_engine: *const ScriptEngine,
}

unsafe impl Send for SceneScriptContext {}
unsafe impl Sync for SceneScriptContext {}

/// Register all engine functions into the Lua state under the `Engine` table.
pub fn register_all(lua: &Lua) -> LuaResult<()> {
    let engine = lua.create_table()?;

    // Utility / debug
    engine.set("rust_function", lua.create_function(rust_function)?)?;
    engine.set("native_log", lua.create_function(native_log)?)?;
    engine.set("native_log_vector", lua.create_function(native_log_vector)?)?;
    engine.set("vector_dot", lua.create_function(vector_dot)?)?;
    engine.set("vector_cross", lua.create_function(vector_cross)?)?;
    engine.set("vector_normalize", lua.create_function(vector_normalize)?)?;

    // Transform
    engine.set("get_translation", lua.create_function(get_translation)?)?;
    engine.set("set_translation", lua.create_function(set_translation)?)?;
    engine.set("get_rotation", lua.create_function(get_rotation)?)?;
    engine.set("set_rotation", lua.create_function(set_rotation)?)?;
    engine.set("get_scale", lua.create_function(get_scale)?)?;
    engine.set("set_scale", lua.create_function(set_scale)?)?;

    // Input
    engine.set("is_key_down", lua.create_function(is_key_down)?)?;

    // Component queries
    engine.set("has_component", lua.create_function(has_component)?)?;

    // Entity lookup / cross-entity scripting
    engine.set("find_entity_by_name", lua.create_function(find_entity_by_name)?)?;
    engine.set("get_script_field", lua.create_function(get_script_field)?)?;
    engine.set("set_script_field", lua.create_function(set_script_field)?)?;

    // Physics
    engine.set("apply_impulse", lua.create_function(lua_apply_impulse)?)?;
    engine.set("apply_impulse_at_point", lua.create_function(lua_apply_impulse_at_point)?)?;
    engine.set("apply_force", lua.create_function(lua_apply_force)?)?;
    engine.set("get_linear_velocity", lua.create_function(lua_get_linear_velocity)?)?;
    engine.set("set_linear_velocity", lua.create_function(lua_set_linear_velocity)?)?;
    engine.set("get_angular_velocity", lua.create_function(lua_get_angular_velocity)?)?;
    engine.set("set_angular_velocity", lua.create_function(lua_set_angular_velocity)?)?;

    lua.globals().set("Engine", engine)?;

    log::info!("ScriptGlue: registered Engine.* functions");
    Ok(())
}

// ---------------------------------------------------------------------------
// Key name → KeyCode mapping
// ---------------------------------------------------------------------------

fn key_name_to_keycode(name: &str) -> Option<KeyCode> {
    match name {
        // Letters
        "A" | "KeyA" => Some(KeyCode::A),
        "B" | "KeyB" => Some(KeyCode::B),
        "C" | "KeyC" => Some(KeyCode::C),
        "D" | "KeyD" => Some(KeyCode::D),
        "E" | "KeyE" => Some(KeyCode::E),
        "F" | "KeyF" => Some(KeyCode::F),
        "G" | "KeyG" => Some(KeyCode::G),
        "H" | "KeyH" => Some(KeyCode::H),
        "I" | "KeyI" => Some(KeyCode::I),
        "J" | "KeyJ" => Some(KeyCode::J),
        "K" | "KeyK" => Some(KeyCode::K),
        "L" | "KeyL" => Some(KeyCode::L),
        "M" | "KeyM" => Some(KeyCode::M),
        "N" | "KeyN" => Some(KeyCode::N),
        "O" | "KeyO" => Some(KeyCode::O),
        "P" | "KeyP" => Some(KeyCode::P),
        "Q" | "KeyQ" => Some(KeyCode::Q),
        "R" | "KeyR" => Some(KeyCode::R),
        "S" | "KeyS" => Some(KeyCode::S),
        "T" | "KeyT" => Some(KeyCode::T),
        "U" | "KeyU" => Some(KeyCode::U),
        "V" | "KeyV" => Some(KeyCode::V),
        "W" | "KeyW" => Some(KeyCode::W),
        "X" | "KeyX" => Some(KeyCode::X),
        "Y" | "KeyY" => Some(KeyCode::Y),
        "Z" | "KeyZ" => Some(KeyCode::Z),

        // Digits
        "0" | "Num0" => Some(KeyCode::Num0),
        "1" | "Num1" => Some(KeyCode::Num1),
        "2" | "Num2" => Some(KeyCode::Num2),
        "3" | "Num3" => Some(KeyCode::Num3),
        "4" | "Num4" => Some(KeyCode::Num4),
        "5" | "Num5" => Some(KeyCode::Num5),
        "6" | "Num6" => Some(KeyCode::Num6),
        "7" | "Num7" => Some(KeyCode::Num7),
        "8" | "Num8" => Some(KeyCode::Num8),
        "9" | "Num9" => Some(KeyCode::Num9),

        // Arrow keys
        "Up" | "ArrowUp" => Some(KeyCode::Up),
        "Down" | "ArrowDown" => Some(KeyCode::Down),
        "Left" | "ArrowLeft" => Some(KeyCode::Left),
        "Right" | "ArrowRight" => Some(KeyCode::Right),

        // Common keys
        "Space" => Some(KeyCode::Space),
        "Enter" | "Return" => Some(KeyCode::Enter),
        "Escape" | "Esc" => Some(KeyCode::Escape),
        "Tab" => Some(KeyCode::Tab),
        "Backspace" => Some(KeyCode::Backspace),
        "Delete" => Some(KeyCode::Delete),
        "Insert" => Some(KeyCode::Insert),
        "Home" => Some(KeyCode::Home),
        "End" => Some(KeyCode::End),
        "PageUp" => Some(KeyCode::PageUp),
        "PageDown" => Some(KeyCode::PageDown),

        // Modifiers
        "LeftShift" | "Shift" => Some(KeyCode::LeftShift),
        "RightShift" => Some(KeyCode::RightShift),
        "LeftCtrl" | "Ctrl" | "Control" => Some(KeyCode::LeftCtrl),
        "RightCtrl" => Some(KeyCode::RightCtrl),
        "LeftAlt" | "Alt" => Some(KeyCode::LeftAlt),
        "RightAlt" => Some(KeyCode::RightAlt),

        // Function keys
        "F1" => Some(KeyCode::F1),
        "F2" => Some(KeyCode::F2),
        "F3" => Some(KeyCode::F3),
        "F4" => Some(KeyCode::F4),
        "F5" => Some(KeyCode::F5),
        "F6" => Some(KeyCode::F6),
        "F7" => Some(KeyCode::F7),
        "F8" => Some(KeyCode::F8),
        "F9" => Some(KeyCode::F9),
        "F10" => Some(KeyCode::F10),
        "F11" => Some(KeyCode::F11),
        "F12" => Some(KeyCode::F12),

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Engine.get_translation / Engine.set_translation / Engine.is_key_down
// ---------------------------------------------------------------------------

/// `Engine.get_translation(entity_id)` — returns `(x, y, z)` from the entity's TransformComponent.
fn get_translation(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0, 0.0)),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(tc) = scene.get_component::<super::TransformComponent>(entity) {
            return Ok((tc.translation.x, tc.translation.y, tc.translation.z));
        }
    }

    Ok((0.0, 0.0, 0.0))
}

/// `Engine.set_translation(entity_id, x, y, z)` — writes to the entity's TransformComponent.
fn set_translation(lua: &Lua, (entity_id, x, y, z): (u64, f32, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.translation.x = x;
            tc.translation.y = y;
            tc.translation.z = z;
        }
    }

    Ok(())
}

/// `Engine.is_key_down(key_name)` — returns true if the named key is currently pressed.
fn is_key_down(lua: &Lua, key_name: String) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    if ctx.input.is_null() {
        return Ok(false);
    }

    let input = unsafe { &*ctx.input };
    if let Some(key_code) = key_name_to_keycode(&key_name) {
        Ok(input.is_key_pressed(key_code))
    } else {
        log::warn!("ScriptGlue: unknown key name '{}'", key_name);
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Engine.get_rotation / Engine.set_rotation / Engine.get_scale / Engine.set_scale
// ---------------------------------------------------------------------------

/// `Engine.get_rotation(entity_id)` — returns `(rx, ry, rz)` in radians.
fn get_rotation(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0, 0.0)),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(tc) = scene.get_component::<super::TransformComponent>(entity) {
            return Ok((tc.rotation.x, tc.rotation.y, tc.rotation.z));
        }
    }

    Ok((0.0, 0.0, 0.0))
}

/// `Engine.set_rotation(entity_id, rx, ry, rz)` — sets rotation in radians.
fn set_rotation(lua: &Lua, (entity_id, rx, ry, rz): (u64, f32, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.rotation.x = rx;
            tc.rotation.y = ry;
            tc.rotation.z = rz;
        }
    }

    Ok(())
}

/// `Engine.get_scale(entity_id)` — returns `(sx, sy, sz)`.
fn get_scale(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((1.0, 1.0, 1.0)),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(tc) = scene.get_component::<super::TransformComponent>(entity) {
            return Ok((tc.scale.x, tc.scale.y, tc.scale.z));
        }
    }

    Ok((1.0, 1.0, 1.0))
}

/// `Engine.set_scale(entity_id, sx, sy, sz)` — sets scale.
fn set_scale(lua: &Lua, (entity_id, sx, sy, sz): (u64, f32, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.scale.x = sx;
            tc.scale.y = sy;
            tc.scale.z = sz;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Engine.has_component
// ---------------------------------------------------------------------------

/// `Engine.has_component(entity_id, component_name)` — string-based component check.
fn has_component(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    let scene = unsafe { &*ctx.scene };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(false),
    };

    let result = match name.as_str() {
        "Transform" => scene.has_component::<super::TransformComponent>(entity),
        "Camera" => scene.has_component::<super::CameraComponent>(entity),
        "SpriteRenderer" => scene.has_component::<super::SpriteRendererComponent>(entity),
        "CircleRenderer" => scene.has_component::<super::CircleRendererComponent>(entity),
        "RigidBody2D" => scene.has_component::<super::RigidBody2DComponent>(entity),
        "BoxCollider2D" => scene.has_component::<super::BoxCollider2DComponent>(entity),
        "CircleCollider2D" => scene.has_component::<super::CircleCollider2DComponent>(entity),
        "NativeScript" => scene.has_component::<super::NativeScriptComponent>(entity),
        "LuaScript" => {
            #[cfg(feature = "lua-scripting")]
            { scene.has_component::<super::LuaScriptComponent>(entity) }
            #[cfg(not(feature = "lua-scripting"))]
            { false }
        }
        _ => {
            log::warn!("ScriptGlue: unknown component name '{}'", name);
            false
        }
    };

    Ok(result)
}

// ---------------------------------------------------------------------------
// Entity lookup / cross-entity scripting
// ---------------------------------------------------------------------------

/// `Engine.find_entity_by_name(name)` — returns the UUID of the first entity
/// with the given tag name, or `nil` if not found.
fn find_entity_by_name(lua: &Lua, name: String) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(LuaValue::Nil),
    };

    let scene = unsafe { &*ctx.scene };
    match scene.find_entity_by_name(&name) {
        Some((_entity, uuid)) => Ok(LuaValue::Integer(uuid as i64)),
        None => Ok(LuaValue::Nil),
    }
}

/// `Engine.get_script_field(entity_id, field_name)` — read a field from
/// another entity's running Lua script. Returns the value or `nil`.
fn get_script_field(lua: &Lua, (entity_id, field_name): (u64, String)) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(LuaValue::Nil),
    };

    if ctx.script_engine.is_null() {
        return Ok(LuaValue::Nil);
    }

    let engine = unsafe { &*ctx.script_engine };
    match engine.get_entity_field(entity_id, &field_name) {
        Some(value) => value.to_lua(lua).map(Into::into),
        None => Ok(LuaValue::Nil),
    }
}

/// `Engine.set_script_field(entity_id, field_name, value)` — write a field on
/// another entity's running Lua script.
fn set_script_field(lua: &Lua, (entity_id, field_name, value): (u64, String, LuaValue)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    if ctx.script_engine.is_null() {
        return Ok(());
    }

    let engine = unsafe { &*ctx.script_engine };
    if let Some(sfv) = super::script_engine::ScriptFieldValue::from_lua_value(&value) {
        engine.set_entity_field(entity_id, &field_name, &sfv);
    } else {
        log::warn!(
            "ScriptGlue: set_script_field unsupported value type for field '{}'",
            field_name
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Physics bindings (delegate to Scene methods)
// ---------------------------------------------------------------------------

/// `Engine.apply_impulse(entity_id, ix, iy)`
fn lua_apply_impulse(lua: &Lua, (entity_id, ix, iy): (u64, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.apply_impulse(entity, glam::Vec2::new(ix, iy));
    }

    Ok(())
}

/// `Engine.apply_impulse_at_point(entity_id, ix, iy, px, py)`
fn lua_apply_impulse_at_point(
    lua: &Lua,
    (entity_id, ix, iy, px, py): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.apply_impulse_at_point(entity, glam::Vec2::new(ix, iy), glam::Vec2::new(px, py));
    }

    Ok(())
}

/// `Engine.apply_force(entity_id, fx, fy)`
fn lua_apply_force(lua: &Lua, (entity_id, fx, fy): (u64, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.apply_force(entity, glam::Vec2::new(fx, fy));
    }

    Ok(())
}

/// `Engine.get_linear_velocity(entity_id)` — returns `(vx, vy)`.
fn lua_get_linear_velocity(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32)> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0)),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(v) = scene.get_linear_velocity(entity) {
            return Ok((v.x, v.y));
        }
    }

    Ok((0.0, 0.0))
}

/// `Engine.set_linear_velocity(entity_id, vx, vy)`
fn lua_set_linear_velocity(lua: &Lua, (entity_id, vx, vy): (u64, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.set_linear_velocity(entity, glam::Vec2::new(vx, vy));
    }

    Ok(())
}

/// `Engine.get_angular_velocity(entity_id)` — returns angular velocity in rad/s.
fn lua_get_angular_velocity(lua: &Lua, entity_id: u64) -> LuaResult<f32> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(0.0),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(omega) = scene.get_angular_velocity(entity) {
            return Ok(omega);
        }
    }

    Ok(0.0)
}

/// `Engine.set_angular_velocity(entity_id, omega)`
fn lua_set_angular_velocity(lua: &Lua, (entity_id, omega): (u64, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.set_angular_velocity(entity, omega);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Existing Engine functions
// ---------------------------------------------------------------------------

/// `Engine.rust_function()` — prints a message proving Rust code was called.
fn rust_function(_lua: &Lua, _: ()) -> LuaResult<()> {
    log::info!("[Rust] This is written in Rust!");
    Ok(())
}

/// `Engine.native_log(text, param)` — logs a string + number (demonstrates marshalling).
fn native_log(_lua: &Lua, (text, param): (String, f64)) -> LuaResult<()> {
    log::info!("[Rust] {} {}", text, param);
    Ok(())
}

/// `Engine.native_log_vector(x, y, z)` — logs 3 floats as a Vec3.
fn native_log_vector(_lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<()> {
    let v = glam::Vec3::new(x, y, z);
    log::info!("[Rust] value: Vec3({}, {}, {})", v.x, v.y, v.z);
    Ok(())
}

/// `Engine.vector_dot(x1,y1,z1, x2,y2,z2)` — returns dot product as f32.
fn vector_dot(_lua: &Lua, (x1, y1, z1, x2, y2, z2): (f32, f32, f32, f32, f32, f32)) -> LuaResult<f32> {
    let a = glam::Vec3::new(x1, y1, z1);
    let b = glam::Vec3::new(x2, y2, z2);
    Ok(a.dot(b))
}

/// `Engine.vector_cross(x1,y1,z1, x2,y2,z2)` — returns (x, y, z) cross product.
fn vector_cross(
    _lua: &Lua,
    (x1, y1, z1, x2, y2, z2): (f32, f32, f32, f32, f32, f32),
) -> LuaResult<(f32, f32, f32)> {
    let a = glam::Vec3::new(x1, y1, z1);
    let b = glam::Vec3::new(x2, y2, z2);
    let c = a.cross(b);
    Ok((c.x, c.y, c.z))
}

/// `Engine.vector_normalize(x, y, z)` — returns normalized (x, y, z).
fn vector_normalize(_lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<(f32, f32, f32)> {
    let v = glam::Vec3::new(x, y, z);
    let n = v.normalize();
    Ok((n.x, n.y, n.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Lua {
        let lua = Lua::new();
        register_all(&lua).expect("register_all should succeed");
        lua
    }

    #[test]
    fn engine_table_exists() {
        let lua = setup();
        let engine: LuaTable = lua.globals().get("Engine").expect("Engine table should exist");
        // Utility / debug
        assert!(engine.get::<LuaFunction>("rust_function").is_ok());
        assert!(engine.get::<LuaFunction>("native_log").is_ok());
        assert!(engine.get::<LuaFunction>("native_log_vector").is_ok());
        assert!(engine.get::<LuaFunction>("vector_dot").is_ok());
        assert!(engine.get::<LuaFunction>("vector_cross").is_ok());
        assert!(engine.get::<LuaFunction>("vector_normalize").is_ok());
        // Transform
        assert!(engine.get::<LuaFunction>("get_translation").is_ok());
        assert!(engine.get::<LuaFunction>("set_translation").is_ok());
        assert!(engine.get::<LuaFunction>("get_rotation").is_ok());
        assert!(engine.get::<LuaFunction>("set_rotation").is_ok());
        assert!(engine.get::<LuaFunction>("get_scale").is_ok());
        assert!(engine.get::<LuaFunction>("set_scale").is_ok());
        // Input
        assert!(engine.get::<LuaFunction>("is_key_down").is_ok());
        // Component queries
        assert!(engine.get::<LuaFunction>("has_component").is_ok());
        // Entity lookup / cross-entity scripting
        assert!(engine.get::<LuaFunction>("find_entity_by_name").is_ok());
        assert!(engine.get::<LuaFunction>("get_script_field").is_ok());
        assert!(engine.get::<LuaFunction>("set_script_field").is_ok());
        // Physics
        assert!(engine.get::<LuaFunction>("apply_impulse").is_ok());
        assert!(engine.get::<LuaFunction>("apply_impulse_at_point").is_ok());
        assert!(engine.get::<LuaFunction>("apply_force").is_ok());
        assert!(engine.get::<LuaFunction>("get_linear_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("set_linear_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("get_angular_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("set_angular_velocity").is_ok());
    }

    #[test]
    fn rust_function_runs() {
        let lua = setup();
        lua.load("Engine.rust_function()").exec().expect("rust_function should not error");
    }

    #[test]
    fn native_log_runs() {
        let lua = setup();
        lua.load(r#"Engine.native_log("test", 123.4)"#)
            .exec()
            .expect("native_log should not error");
    }

    #[test]
    fn native_log_vector_runs() {
        let lua = setup();
        lua.load("Engine.native_log_vector(1.0, 2.0, 3.0)")
            .exec()
            .expect("native_log_vector should not error");
    }

    #[test]
    fn vector_dot_returns_correct_value() {
        let lua = setup();
        lua.load("dot_result = Engine.vector_dot(5, 2.5, 1, 5, 2.5, 1)")
            .exec()
            .unwrap();
        let result: f32 = lua.globals().get("dot_result").unwrap();
        assert!((result - 32.25).abs() < 0.001);
    }

    #[test]
    fn vector_cross_returns_correct_values() {
        let lua = setup();
        lua.load("cx, cy, cz = Engine.vector_cross(1, 0, 0, 0, 1, 0)")
            .exec()
            .unwrap();
        let cx: f32 = lua.globals().get("cx").unwrap();
        let cy: f32 = lua.globals().get("cy").unwrap();
        let cz: f32 = lua.globals().get("cz").unwrap();
        // (1,0,0) × (0,1,0) = (0,0,1)
        assert!(cx.abs() < 0.001);
        assert!(cy.abs() < 0.001);
        assert!((cz - 1.0).abs() < 0.001);
    }

    #[test]
    fn vector_normalize_returns_unit_vector() {
        let lua = setup();
        lua.load("nx, ny, nz = Engine.vector_normalize(3, 0, 0)")
            .exec()
            .unwrap();
        let nx: f32 = lua.globals().get("nx").unwrap();
        let ny: f32 = lua.globals().get("ny").unwrap();
        let nz: f32 = lua.globals().get("nz").unwrap();
        assert!((nx - 1.0).abs() < 0.001);
        assert!(ny.abs() < 0.001);
        assert!(nz.abs() < 0.001);
    }

    #[test]
    fn key_name_mapping() {
        assert_eq!(key_name_to_keycode("W"), Some(KeyCode::W));
        assert_eq!(key_name_to_keycode("KeyW"), Some(KeyCode::W));
        assert_eq!(key_name_to_keycode("Space"), Some(KeyCode::Space));
        assert_eq!(key_name_to_keycode("Escape"), Some(KeyCode::Escape));
        assert_eq!(key_name_to_keycode("Esc"), Some(KeyCode::Escape));
        assert_eq!(key_name_to_keycode("0"), Some(KeyCode::Num0));
        assert_eq!(key_name_to_keycode("F1"), Some(KeyCode::F1));
        assert_eq!(key_name_to_keycode("ArrowUp"), Some(KeyCode::Up));
        assert_eq!(key_name_to_keycode("bogus"), None);
    }

    #[test]
    fn get_translation_no_context_returns_zero() {
        let lua = setup();
        lua.load("gx, gy, gz = Engine.get_translation(12345)")
            .exec()
            .unwrap();
        let gx: f32 = lua.globals().get("gx").unwrap();
        let gy: f32 = lua.globals().get("gy").unwrap();
        let gz: f32 = lua.globals().get("gz").unwrap();
        assert!(gx.abs() < 0.001);
        assert!(gy.abs() < 0.001);
        assert!(gz.abs() < 0.001);
    }

    #[test]
    fn is_key_down_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"key_result = Engine.is_key_down("W")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("key_result").unwrap();
        assert!(!result);
    }

    #[test]
    fn find_entity_by_name_no_context_returns_nil() {
        let lua = setup();
        lua.load(r#"result = Engine.find_entity_by_name("Player")"#)
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn get_script_field_no_context_returns_nil() {
        let lua = setup();
        lua.load(r#"result = Engine.get_script_field(12345, "speed")"#)
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn set_script_field_no_context_no_error() {
        let lua = setup();
        lua.load(r#"Engine.set_script_field(12345, "speed", 5.0)"#)
            .exec()
            .unwrap();
    }

    #[test]
    fn get_rotation_no_context_returns_zero() {
        let lua = setup();
        lua.load("rx, ry, rz = Engine.get_rotation(12345)")
            .exec()
            .unwrap();
        let rx: f32 = lua.globals().get("rx").unwrap();
        let ry: f32 = lua.globals().get("ry").unwrap();
        let rz: f32 = lua.globals().get("rz").unwrap();
        assert!(rx.abs() < 0.001);
        assert!(ry.abs() < 0.001);
        assert!(rz.abs() < 0.001);
    }

    #[test]
    fn get_scale_no_context_returns_one() {
        let lua = setup();
        lua.load("sx, sy, sz = Engine.get_scale(12345)")
            .exec()
            .unwrap();
        let sx: f32 = lua.globals().get("sx").unwrap();
        let sy: f32 = lua.globals().get("sy").unwrap();
        let sz: f32 = lua.globals().get("sz").unwrap();
        assert!((sx - 1.0).abs() < 0.001);
        assert!((sy - 1.0).abs() < 0.001);
        assert!((sz - 1.0).abs() < 0.001);
    }

    #[test]
    fn has_component_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.has_component(12345, "Transform")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn get_linear_velocity_no_context_returns_zero() {
        let lua = setup();
        lua.load("vx, vy = Engine.get_linear_velocity(12345)")
            .exec()
            .unwrap();
        let vx: f32 = lua.globals().get("vx").unwrap();
        let vy: f32 = lua.globals().get("vy").unwrap();
        assert!(vx.abs() < 0.001);
        assert!(vy.abs() < 0.001);
    }

    #[test]
    fn get_angular_velocity_no_context_returns_zero() {
        let lua = setup();
        lua.load("omega = Engine.get_angular_velocity(12345)")
            .exec()
            .unwrap();
        let omega: f32 = lua.globals().get("omega").unwrap();
        assert!(omega.abs() < 0.001);
    }
}
