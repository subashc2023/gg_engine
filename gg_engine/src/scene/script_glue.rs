//! Centralized Rust↔Lua bindings — the Lua equivalent of Cherno's `ScriptGlue.cpp`.
//!
//! Every Rust function exposed to Lua scripts lives here. Functions are
//! registered under an `Engine` global table so scripts call e.g.
//! `Engine.native_log("hello", 42)`.

use mlua::prelude::*;

use crate::events::{KeyCode, MouseButton};
use crate::input::Input;
use super::Scene;

/// Runtime context set as Lua `app_data` during script execution.
///
/// Raw pointers are used to sidestep Rust borrow rules (take-modify-replace
/// pattern ensures safety — the scene is exclusively borrowed while scripts
/// run). `input` is null during `on_create` / `on_destroy`.
pub(crate) struct SceneScriptContext {
    pub scene: *mut Scene,
    pub input: *const Input,
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
    engine.set("is_key_pressed", lua.create_function(is_key_just_pressed)?)?;
    engine.set("is_mouse_button_down", lua.create_function(is_mouse_button_down)?)?;
    engine.set("is_mouse_button_pressed", lua.create_function(is_mouse_button_just_pressed)?)?;
    engine.set("get_mouse_position", lua.create_function(get_mouse_position)?)?;

    // Component queries
    engine.set("has_component", lua.create_function(has_component)?)?;

    // Entity lookup / cross-entity scripting
    engine.set("find_entity_by_name", lua.create_function(find_entity_by_name)?)?;
    engine.set("get_script_field", lua.create_function(get_script_field)?)?;
    engine.set("set_script_field", lua.create_function(set_script_field)?)?;

    // Entity lifecycle
    engine.set("create_entity", lua.create_function(lua_create_entity)?)?;
    engine.set("destroy_entity", lua.create_function(lua_destroy_entity)?)?;
    engine.set("get_entity_name", lua.create_function(lua_get_entity_name)?)?;

    // Hierarchy
    engine.set("set_parent", lua.create_function(lua_set_parent)?)?;
    engine.set("detach_from_parent", lua.create_function(lua_detach_from_parent)?)?;
    engine.set("get_parent", lua.create_function(lua_get_parent)?)?;
    engine.set("get_children", lua.create_function(lua_get_children)?)?;

    // Animation
    engine.set("play_animation", lua.create_function(lua_play_animation)?)?;
    engine.set("stop_animation", lua.create_function(lua_stop_animation)?)?;
    engine.set("is_animation_playing", lua.create_function(lua_is_animation_playing)?)?;

    // Audio
    engine.set("play_sound", lua.create_function(lua_play_sound)?)?;
    engine.set("stop_sound", lua.create_function(lua_stop_sound)?)?;
    engine.set("set_volume", lua.create_function(lua_set_volume)?)?;

    // Tilemap
    engine.set("set_tile", lua.create_function(lua_set_tile)?)?;
    engine.set("get_tile", lua.create_function(lua_get_tile)?)?;
    engine.set("TILE_FLIP_H", super::TILE_FLIP_H)?;
    engine.set("TILE_FLIP_V", super::TILE_FLIP_V)?;
    engine.set("TILE_ID_MASK", super::TILE_ID_MASK)?;

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
// Mouse button name → MouseButton mapping
// ---------------------------------------------------------------------------

fn mouse_button_name_to_enum(name: &str) -> Option<MouseButton> {
    match name {
        "Left" => Some(MouseButton::Left),
        "Right" => Some(MouseButton::Right),
        "Middle" => Some(MouseButton::Middle),
        "Back" => Some(MouseButton::Back),
        "Forward" => Some(MouseButton::Forward),
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

/// `Engine.is_key_pressed(key_name)` — returns true only on the first frame the key is pressed.
fn is_key_just_pressed(lua: &Lua, key_name: String) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    if ctx.input.is_null() {
        return Ok(false);
    }

    let input = unsafe { &*ctx.input };
    if let Some(key_code) = key_name_to_keycode(&key_name) {
        Ok(input.is_key_just_pressed(key_code))
    } else {
        log::warn!("ScriptGlue: unknown key name '{}'", key_name);
        Ok(false)
    }
}

/// `Engine.is_mouse_button_down(button_name)` — returns true if the named mouse button is currently pressed.
fn is_mouse_button_down(lua: &Lua, button_name: String) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    if ctx.input.is_null() {
        return Ok(false);
    }

    let input = unsafe { &*ctx.input };
    if let Some(button) = mouse_button_name_to_enum(&button_name) {
        Ok(input.is_mouse_button_pressed(button))
    } else {
        log::warn!("ScriptGlue: unknown mouse button name '{}'", button_name);
        Ok(false)
    }
}

/// `Engine.is_mouse_button_pressed(button_name)` — returns true only on the first frame the button is pressed.
fn is_mouse_button_just_pressed(lua: &Lua, button_name: String) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    if ctx.input.is_null() {
        return Ok(false);
    }

    let input = unsafe { &*ctx.input };
    if let Some(button) = mouse_button_name_to_enum(&button_name) {
        Ok(input.is_mouse_button_just_pressed(button))
    } else {
        log::warn!("ScriptGlue: unknown mouse button name '{}'", button_name);
        Ok(false)
    }
}

/// `Engine.get_mouse_position()` — returns `(x, y)` screen-space mouse position.
fn get_mouse_position(lua: &Lua, _: ()) -> LuaResult<(f64, f64)> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0)),
    };

    if ctx.input.is_null() {
        return Ok((0.0, 0.0));
    }

    let input = unsafe { &*ctx.input };
    Ok(input.mouse_position())
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
        "Tilemap" => scene.has_component::<super::TilemapComponent>(entity),
        "AudioSource" | "Audio" => scene.has_component::<super::AudioSourceComponent>(entity),
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

    let scene = unsafe { &mut *ctx.scene };
    match scene.find_entity_by_name(&name) {
        Some((_entity, uuid)) => Ok(LuaValue::Integer(uuid as i64)),
        None => Ok(LuaValue::Nil),
    }
}

/// `Engine.get_script_field(entity_id, field_name)` — read a field from
/// another entity's running Lua script. Returns the value or `nil`.
///
/// Accesses entity environments directly from the Lua-side registry table
/// (`__gg_entity_envs`) — no ScriptEngine pointer needed.
fn get_script_field(lua: &Lua, (entity_id, field_name): (u64, String)) -> LuaResult<LuaValue> {
    use super::script_engine::ENTITY_ENVS_REGISTRY_KEY;

    let envs_table: LuaTable = match lua.named_registry_value(ENTITY_ENVS_REGISTRY_KEY) {
        Ok(t) => t,
        Err(_) => return Ok(LuaValue::Nil),
    };

    let env: LuaTable = match envs_table.get(entity_id) {
        Ok(t) => t,
        Err(_) => return Ok(LuaValue::Nil),
    };

    let fields_table: LuaTable = match env.raw_get("fields") {
        Ok(t) => t,
        Err(_) => return Ok(LuaValue::Nil),
    };

    fields_table.get(field_name)
}

/// `Engine.set_script_field(entity_id, field_name, value)` — write a field on
/// another entity's running Lua script.
///
/// Accesses entity environments directly from the Lua-side registry table
/// (`__gg_entity_envs`) — no ScriptEngine pointer needed.
/// Only Bool, Integer, Number, and String values are accepted.
fn set_script_field(lua: &Lua, (entity_id, field_name, value): (u64, String, LuaValue)) -> LuaResult<()> {
    use super::script_engine::ENTITY_ENVS_REGISTRY_KEY;

    // Validate value type before touching the env table.
    match &value {
        LuaValue::Boolean(_) | LuaValue::Integer(_) | LuaValue::Number(_) | LuaValue::String(_) => {}
        _ => {
            log::warn!(
                "ScriptGlue: set_script_field unsupported value type for field '{}'",
                field_name
            );
            return Ok(());
        }
    }

    let envs_table: LuaTable = match lua.named_registry_value(ENTITY_ENVS_REGISTRY_KEY) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };

    let env: LuaTable = match envs_table.get(entity_id) {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };

    let fields_table: LuaTable = match env.raw_get("fields") {
        Ok(t) => t,
        Err(_) => return Ok(()),
    };

    fields_table.set(field_name, value)
}

// ---------------------------------------------------------------------------
// Entity lifecycle (create / destroy / get_entity_name)
// ---------------------------------------------------------------------------

/// `Engine.create_entity(name)` — create a new entity with the given name, returns its UUID.
fn lua_create_entity(lua: &Lua, name: String) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(LuaValue::Integer(0)),
    };

    let scene = unsafe { &mut *ctx.scene };
    let entity = scene.create_entity_with_tag(&name);
    let uuid = scene
        .get_component::<super::IdComponent>(entity)
        .map(|id| id.id.raw())
        .unwrap_or(0);
    Ok(LuaValue::Integer(uuid as i64))
}

/// `Engine.destroy_entity(uuid)` — queue an entity for deferred destruction.
fn lua_destroy_entity(lua: &Lua, uuid: u64) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    scene.queue_entity_destroy(uuid);
    Ok(())
}

/// `Engine.get_entity_name(uuid)` — returns the entity's tag name, or nil.
fn lua_get_entity_name(lua: &Lua, uuid: u64) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(LuaValue::Nil),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(uuid) {
        if let Some(tag) = scene.get_component::<super::TagComponent>(entity) {
            return Ok(LuaValue::String(lua.create_string(&tag.tag)?));
        }
    }
    Ok(LuaValue::Nil)
}

// ---------------------------------------------------------------------------
// Hierarchy (parent-child relationships)
// ---------------------------------------------------------------------------

/// `Engine.set_parent(child_id, parent_id)` — reparent an entity, preserving world transform.
/// Returns `true` on success, `false` if either entity not found or cycle detected.
fn lua_set_parent(lua: &Lua, (child_id, parent_id): (u64, u64)) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    let scene = unsafe { &mut *ctx.scene };
    let child = match scene.find_entity_by_uuid(child_id) {
        Some(e) => e,
        None => return Ok(false),
    };
    let parent = match scene.find_entity_by_uuid(parent_id) {
        Some(e) => e,
        None => return Ok(false),
    };

    Ok(scene.set_parent(child, parent, true))
}

/// `Engine.detach_from_parent(entity_id)` — make an entity a root entity, preserving world transform.
fn lua_detach_from_parent(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.detach_from_parent(entity, true);
    }

    Ok(())
}

/// `Engine.get_parent(entity_id)` — returns parent UUID as integer, or `nil` if root entity.
fn lua_get_parent(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(LuaValue::Nil),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        match scene.get_parent(entity) {
            Some(parent_uuid) => Ok(LuaValue::Integer(parent_uuid as i64)),
            None => Ok(LuaValue::Nil),
        }
    } else {
        Ok(LuaValue::Nil)
    }
}

/// `Engine.get_children(entity_id)` — returns a Lua table (1-indexed array) of child UUIDs.
fn lua_get_children(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => {
            let empty = lua.create_table()?;
            return Ok(LuaValue::Table(empty));
        }
    };

    let scene = unsafe { &*ctx.scene };
    let table = lua.create_table()?;
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        let children = scene.get_children(entity);
        for (i, uuid) in children.iter().enumerate() {
            table.set(i + 1, *uuid as i64)?;
        }
    }
    Ok(LuaValue::Table(table))
}

// ---------------------------------------------------------------------------
// Animation bindings
// ---------------------------------------------------------------------------

/// `Engine.play_animation(entity_id, name)` — play an animation clip by name. Returns true if found.
fn lua_play_animation(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut animator) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            return Ok(animator.play(&name));
        }
    }
    Ok(false)
}

/// `Engine.stop_animation(entity_id)` — stop the current animation.
fn lua_stop_animation(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut animator) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            animator.stop();
        }
    }
    Ok(())
}

/// `Engine.is_animation_playing(entity_id)` — returns true if an animation is currently playing.
fn lua_is_animation_playing(lua: &Lua, entity_id: u64) -> LuaResult<bool> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(animator) = scene.get_component::<super::SpriteAnimatorComponent>(entity) {
            return Ok(animator.is_playing());
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Audio bindings
// ---------------------------------------------------------------------------

/// `Engine.play_sound(entity_id)` — play the entity's audio source.
fn lua_play_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.play_entity_sound(entity);
    }

    Ok(())
}

/// `Engine.stop_sound(entity_id)` — stop the entity's audio playback.
fn lua_stop_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.stop_entity_sound(entity);
    }

    Ok(())
}

/// `Engine.set_volume(entity_id, volume)` — adjust volume at runtime.
fn lua_set_volume(lua: &Lua, (entity_id, volume): (u64, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        scene.set_entity_volume(entity, volume);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tilemap bindings
// ---------------------------------------------------------------------------

/// `Engine.set_tile(entity_id, x, y, tile_id)` — set tile at grid position.
fn lua_set_tile(lua: &Lua, (entity_id, x, y, tile_id): (u64, u32, u32, i32)) -> LuaResult<()> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };

    let scene = unsafe { &mut *ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(mut tilemap) = scene.get_component_mut::<super::TilemapComponent>(entity) {
            tilemap.set_tile(x, y, tile_id);
        }
    }

    Ok(())
}

/// `Engine.get_tile(entity_id, x, y)` — returns tile ID at grid position, -1 if empty/OOB.
fn lua_get_tile(lua: &Lua, (entity_id, x, y): (u64, u32, u32)) -> LuaResult<i32> {
    let ctx = match lua.app_data_ref::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(-1),
    };

    let scene = unsafe { &*ctx.scene };
    if let Some(entity) = scene.find_entity_by_uuid(entity_id) {
        if let Some(tilemap) = scene.get_component::<super::TilemapComponent>(entity) {
            return Ok(tilemap.get_tile(x, y));
        }
    }

    Ok(-1)
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

/// `Engine.vector_normalize(x, y, z)` — returns normalized (x, y, z), or (0,0,0) for zero vectors.
fn vector_normalize(_lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<(f32, f32, f32)> {
    let v = glam::Vec3::new(x, y, z);
    let n = v.try_normalize().unwrap_or(glam::Vec3::ZERO);
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
        assert!(engine.get::<LuaFunction>("is_key_pressed").is_ok());
        assert!(engine.get::<LuaFunction>("is_mouse_button_down").is_ok());
        assert!(engine.get::<LuaFunction>("is_mouse_button_pressed").is_ok());
        assert!(engine.get::<LuaFunction>("get_mouse_position").is_ok());
        // Component queries
        assert!(engine.get::<LuaFunction>("has_component").is_ok());
        // Entity lifecycle
        assert!(engine.get::<LuaFunction>("create_entity").is_ok());
        assert!(engine.get::<LuaFunction>("destroy_entity").is_ok());
        assert!(engine.get::<LuaFunction>("get_entity_name").is_ok());
        // Entity lookup / cross-entity scripting
        assert!(engine.get::<LuaFunction>("find_entity_by_name").is_ok());
        assert!(engine.get::<LuaFunction>("get_script_field").is_ok());
        assert!(engine.get::<LuaFunction>("set_script_field").is_ok());
        // Hierarchy
        assert!(engine.get::<LuaFunction>("set_parent").is_ok());
        assert!(engine.get::<LuaFunction>("detach_from_parent").is_ok());
        assert!(engine.get::<LuaFunction>("get_parent").is_ok());
        assert!(engine.get::<LuaFunction>("get_children").is_ok());
        // Animation
        assert!(engine.get::<LuaFunction>("play_animation").is_ok());
        assert!(engine.get::<LuaFunction>("stop_animation").is_ok());
        assert!(engine.get::<LuaFunction>("is_animation_playing").is_ok());
        // Tilemap
        assert!(engine.get::<LuaFunction>("set_tile").is_ok());
        assert!(engine.get::<LuaFunction>("get_tile").is_ok());
        assert_eq!(engine.get::<i32>("TILE_FLIP_H").unwrap(), 0x4000_0000);
        assert_eq!(engine.get::<i32>("TILE_FLIP_V").unwrap(), 0x2000_0000);
        assert_eq!(engine.get::<i32>("TILE_ID_MASK").unwrap(), 0x1FFF_FFFF);
        // Audio
        assert!(engine.get::<LuaFunction>("play_sound").is_ok());
        assert!(engine.get::<LuaFunction>("stop_sound").is_ok());
        assert!(engine.get::<LuaFunction>("set_volume").is_ok());
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

    #[test]
    fn mouse_button_name_mapping() {
        assert_eq!(mouse_button_name_to_enum("Left"), Some(MouseButton::Left));
        assert_eq!(mouse_button_name_to_enum("Right"), Some(MouseButton::Right));
        assert_eq!(mouse_button_name_to_enum("Middle"), Some(MouseButton::Middle));
        assert_eq!(mouse_button_name_to_enum("Back"), Some(MouseButton::Back));
        assert_eq!(mouse_button_name_to_enum("Forward"), Some(MouseButton::Forward));
        assert_eq!(mouse_button_name_to_enum("bogus"), None);
    }

    #[test]
    fn is_mouse_button_down_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.is_mouse_button_down("Left")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn get_mouse_position_no_context_returns_zero() {
        let lua = setup();
        lua.load("mx, my = Engine.get_mouse_position()")
            .exec()
            .unwrap();
        let mx: f64 = lua.globals().get("mx").unwrap();
        let my: f64 = lua.globals().get("my").unwrap();
        assert!(mx.abs() < 0.001);
        assert!(my.abs() < 0.001);
    }

    #[test]
    fn create_entity_no_context_returns_zero() {
        let lua = setup();
        lua.load(r#"result = Engine.create_entity("Test")"#)
            .exec()
            .unwrap();
        let result: i64 = lua.globals().get("result").unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn destroy_entity_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.destroy_entity(12345)")
            .exec()
            .unwrap();
    }

    #[test]
    fn get_entity_name_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.get_entity_name(12345)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn set_tile_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_tile(12345, 0, 0, 1)")
            .exec()
            .unwrap();
    }

    #[test]
    fn get_tile_no_context_returns_neg1() {
        let lua = setup();
        lua.load("result = Engine.get_tile(12345, 0, 0)")
            .exec()
            .unwrap();
        let result: i32 = lua.globals().get("result").unwrap();
        assert_eq!(result, -1);
    }

    #[test]
    fn play_animation_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.play_animation(12345, "idle")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn stop_animation_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.stop_animation(12345)")
            .exec()
            .unwrap();
    }

    #[test]
    fn is_animation_playing_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.is_animation_playing(12345)")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn play_sound_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.play_sound(12345)")
            .exec()
            .expect("play_sound should not error without context");
    }

    #[test]
    fn stop_sound_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.stop_sound(12345)")
            .exec()
            .expect("stop_sound should not error without context");
    }

    #[test]
    fn set_volume_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_volume(12345, 0.5)")
            .exec()
            .expect("set_volume should not error without context");
    }

    #[test]
    fn set_parent_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.set_parent(111, 222)")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn detach_from_parent_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.detach_from_parent(12345)")
            .exec()
            .unwrap();
    }

    #[test]
    fn get_parent_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.get_parent(12345)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn get_children_no_context_returns_empty_table() {
        let lua = setup();
        lua.load("result = Engine.get_children(12345)")
            .exec()
            .unwrap();
        let result: LuaTable = lua.globals().get("result").unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }
}
