//! Centralized Rust↔Lua bindings — the Lua equivalent of Cherno's `ScriptGlue.cpp`.
//!
//! Every Rust function exposed to Lua scripts lives here. Functions are
//! registered under an `Engine` global table so scripts call e.g.
//! `Engine.native_log("hello", 42)`.

use mlua::prelude::*;

use super::{Entity, Scene};

/// Lua return type for 3D raycast: `(entity_uuid?, hx, hy, hz, nx, ny, nz, toi)`.
type LuaRaycastHit3D = (Option<u64>, f32, f32, f32, f32, f32, f32, f32);
use crate::events::{KeyCode, MouseButton};
use crate::input::Input;

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

impl SceneScriptContext {
    /// Borrow the scene immutably.
    ///
    /// # Safety
    /// Caller must ensure the pointer is valid and no mutable alias exists.
    #[inline]
    unsafe fn scene(&self) -> &Scene {
        debug_assert!(!self.scene.is_null(), "SceneScriptContext::scene is null");
        &*self.scene
    }

    /// Borrow the scene mutably.
    ///
    /// # Safety
    /// Caller must ensure the pointer is valid and no other alias exists.
    #[inline]
    unsafe fn scene_mut(&mut self) -> &mut Scene {
        debug_assert!(!self.scene.is_null(), "SceneScriptContext::scene is null");
        &mut *self.scene
    }

    /// Borrow the input, returning `None` when the pointer is null
    /// (e.g. during `on_create` / `on_destroy`).
    ///
    /// # Safety
    /// Caller must ensure the pointer, when non-null, is valid.
    #[inline]
    unsafe fn input(&self) -> Option<&Input> {
        if self.input.is_null() {
            None
        } else {
            Some(&*self.input)
        }
    }
}

// ---------------------------------------------------------------------------
// Context helpers — eliminate repeated app_data_mut + unsafe boilerplate
// ---------------------------------------------------------------------------

/// Acquire immutable scene access, find entity, and call closure.
/// Returns `default` if context unavailable or entity not found.
fn with_entity<R>(
    lua: &Lua,
    entity_id: u64,
    default: R,
    f: impl FnOnce(&Scene, Entity) -> R,
) -> LuaResult<R> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(default),
    };
    let scene = unsafe { ctx.scene() };
    let entity = scene.find_entity_by_uuid(entity_id);
    Ok(entity.map(|e| f(scene, e)).unwrap_or(default))
}

/// Acquire mutable scene access, find entity, and call closure.
/// Returns `default` if context unavailable or entity not found.
fn with_entity_mut<R>(
    lua: &Lua,
    entity_id: u64,
    default: R,
    f: impl FnOnce(&mut Scene, Entity) -> R,
) -> LuaResult<R> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(default),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = scene.find_entity_by_uuid(entity_id);
    Ok(entity.map(|e| f(scene, e)).unwrap_or(default))
}

/// Acquire mutable scene access (no entity lookup) and call closure.
/// Returns `default` if context unavailable.
fn with_scene_mut<R>(lua: &Lua, default: R, f: impl FnOnce(&mut Scene) -> R) -> LuaResult<R> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(default),
    };
    let scene = unsafe { ctx.scene_mut() };
    Ok(f(scene))
}

/// Acquire input access and call closure.
/// Returns `default` if context or input unavailable.
fn with_input<R>(lua: &Lua, default: R, f: impl FnOnce(&Input) -> R) -> LuaResult<R> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(default),
    };
    match unsafe { ctx.input() } {
        Some(input) => Ok(f(input)),
        None => Ok(default),
    }
}

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
    engine.set("get_rotation_quat", lua.create_function(get_rotation_quat)?)?;
    engine.set("set_rotation_quat", lua.create_function(set_rotation_quat)?)?;
    engine.set("get_scale", lua.create_function(get_scale)?)?;
    engine.set("set_scale", lua.create_function(set_scale)?)?;

    // Input
    engine.set("is_key_down", lua.create_function(is_key_down)?)?;
    engine.set("is_key_pressed", lua.create_function(is_key_just_pressed)?)?;
    engine.set(
        "is_mouse_button_down",
        lua.create_function(is_mouse_button_down)?,
    )?;
    engine.set(
        "is_mouse_button_pressed",
        lua.create_function(is_mouse_button_just_pressed)?,
    )?;
    engine.set(
        "get_mouse_position",
        lua.create_function(get_mouse_position)?,
    )?;

    // Component queries
    engine.set("has_component", lua.create_function(has_component)?)?;

    // Entity lookup / cross-entity scripting
    engine.set(
        "find_entity_by_name",
        lua.create_function(find_entity_by_name)?,
    )?;
    engine.set("get_script_field", lua.create_function(get_script_field)?)?;
    engine.set("set_script_field", lua.create_function(set_script_field)?)?;

    // Entity lifecycle
    engine.set("create_entity", lua.create_function(lua_create_entity)?)?;
    engine.set("destroy_entity", lua.create_function(lua_destroy_entity)?)?;
    engine.set("get_entity_name", lua.create_function(lua_get_entity_name)?)?;

    // Hierarchy
    engine.set("set_parent", lua.create_function(lua_set_parent)?)?;
    engine.set(
        "detach_from_parent",
        lua.create_function(lua_detach_from_parent)?,
    )?;
    engine.set("get_parent", lua.create_function(lua_get_parent)?)?;
    engine.set("get_children", lua.create_function(lua_get_children)?)?;

    // Animation
    engine.set("play_animation", lua.create_function(lua_play_animation)?)?;
    engine.set("stop_animation", lua.create_function(lua_stop_animation)?)?;
    engine.set(
        "is_animation_playing",
        lua.create_function(lua_is_animation_playing)?,
    )?;
    engine.set(
        "get_current_animation",
        lua.create_function(lua_get_current_animation)?,
    )?;
    engine.set(
        "get_animation_frame",
        lua.create_function(lua_get_animation_frame)?,
    )?;
    engine.set(
        "set_animation_speed",
        lua.create_function(lua_set_animation_speed)?,
    )?;

    // Instanced animation
    engine.set(
        "play_instanced_animation",
        lua.create_function(lua_play_instanced_animation)?,
    )?;
    engine.set(
        "stop_instanced_animation",
        lua.create_function(lua_stop_instanced_animation)?,
    )?;
    engine.set(
        "get_instanced_animation",
        lua.create_function(lua_get_instanced_animation)?,
    )?;

    // Animation controller
    engine.set("set_anim_param", lua.create_function(lua_set_anim_param)?)?;
    engine.set("get_anim_param", lua.create_function(lua_get_anim_param)?)?;

    // Audio
    engine.set("play_sound", lua.create_function(lua_play_sound)?)?;
    engine.set("stop_sound", lua.create_function(lua_stop_sound)?)?;
    engine.set("pause_sound", lua.create_function(lua_pause_sound)?)?;
    engine.set("resume_sound", lua.create_function(lua_resume_sound)?)?;
    engine.set("set_volume", lua.create_function(lua_set_volume)?)?;
    engine.set("set_panning", lua.create_function(lua_set_panning)?)?;

    // Tilemap
    engine.set("set_tile", lua.create_function(lua_set_tile)?)?;
    engine.set("get_tile", lua.create_function(lua_get_tile)?)?;
    engine.set("TILE_FLIP_H", super::TILE_FLIP_H)?;
    engine.set("TILE_FLIP_V", super::TILE_FLIP_V)?;
    engine.set("TILE_ID_MASK", super::TILE_ID_MASK)?;

    // Physics
    engine.set("apply_impulse", lua.create_function(lua_apply_impulse)?)?;
    engine.set(
        "apply_impulse_at_point",
        lua.create_function(lua_apply_impulse_at_point)?,
    )?;
    engine.set("apply_force", lua.create_function(lua_apply_force)?)?;
    engine.set(
        "get_linear_velocity",
        lua.create_function(lua_get_linear_velocity)?,
    )?;
    engine.set(
        "set_linear_velocity",
        lua.create_function(lua_set_linear_velocity)?,
    )?;
    engine.set(
        "get_angular_velocity",
        lua.create_function(lua_get_angular_velocity)?,
    )?;
    engine.set(
        "set_angular_velocity",
        lua.create_function(lua_set_angular_velocity)?,
    )?;
    engine.set("raycast", lua.create_function(lua_raycast)?)?;

    // 3D Physics
    engine.set(
        "apply_impulse_3d",
        lua.create_function(lua_apply_impulse_3d)?,
    )?;
    engine.set(
        "apply_impulse_at_point_3d",
        lua.create_function(lua_apply_impulse_at_point_3d)?,
    )?;
    engine.set("apply_force_3d", lua.create_function(lua_apply_force_3d)?)?;
    engine.set(
        "apply_torque_impulse_3d",
        lua.create_function(lua_apply_torque_impulse_3d)?,
    )?;
    engine.set("apply_torque_3d", lua.create_function(lua_apply_torque_3d)?)?;
    engine.set(
        "get_linear_velocity_3d",
        lua.create_function(lua_get_linear_velocity_3d)?,
    )?;
    engine.set(
        "set_linear_velocity_3d",
        lua.create_function(lua_set_linear_velocity_3d)?,
    )?;
    engine.set(
        "get_angular_velocity_3d",
        lua.create_function(lua_get_angular_velocity_3d)?,
    )?;
    engine.set(
        "set_angular_velocity_3d",
        lua.create_function(lua_set_angular_velocity_3d)?,
    )?;
    engine.set("raycast_3d", lua.create_function(lua_raycast_3d)?)?;
    engine.set("set_gravity_3d", lua.create_function(lua_set_gravity_3d)?)?;
    engine.set("get_gravity_3d", lua.create_function(lua_get_gravity_3d)?)?;

    // Entity queries
    engine.set(
        "find_entities_with_component",
        lua.create_function(lua_find_entities_with_component)?,
    )?;

    // Component access (sprite, circle, text)
    engine.set(
        "get_sprite_color",
        lua.create_function(lua_get_sprite_color)?,
    )?;
    engine.set(
        "set_sprite_color",
        lua.create_function(lua_set_sprite_color)?,
    )?;
    engine.set(
        "set_sprite_texture",
        lua.create_function(lua_set_sprite_texture)?,
    )?;
    engine.set("get_text", lua.create_function(lua_get_text)?)?;
    engine.set("set_text", lua.create_function(lua_set_text)?)?;

    // Timers
    engine.set("set_timeout", lua.create_function(lua_set_timeout)?)?;
    engine.set("set_interval", lua.create_function(lua_set_interval)?)?;
    engine.set("cancel_timer", lua.create_function(lua_cancel_timer)?)?;

    // Physics: gravity
    engine.set("set_gravity", lua.create_function(lua_set_gravity)?)?;
    engine.set("get_gravity", lua.create_function(lua_get_gravity)?)?;

    // Time
    engine.set("get_time", lua.create_function(lua_get_time)?)?;
    engine.set("delta_time", lua.create_function(lua_delta_time)?)?;

    // Input: key released
    engine.set(
        "is_key_released",
        lua.create_function(lua_is_key_just_released)?,
    )?;

    // Logging
    engine.set("log", lua.create_function(lua_log)?)?;

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
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_component::<super::TransformComponent>(entity)
            .map(|tc| (tc.translation.x, tc.translation.y, tc.translation.z))
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

/// `Engine.set_translation(entity_id, x, y, z)` — writes to the entity's TransformComponent
/// and syncs the physics body position if one exists.
fn set_translation(lua: &Lua, (entity_id, x, y, z): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.translation = glam::Vec3::new(x, y, z);
        }
        scene.sync_physics_translation(entity, x, y);
        scene.sync_physics_translation_3d(entity, x, y, z);
    })
}

/// `Engine.is_key_down(key_name)` — returns true if the named key is currently pressed.
fn is_key_down(lua: &Lua, key_name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        key_name_to_keycode(&key_name)
            .map(|kc| input.is_key_pressed(kc))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown key name '{}'", key_name);
                false
            })
    })
}

/// `Engine.is_key_pressed(key_name)` — returns true only on the first frame the key is pressed.
fn is_key_just_pressed(lua: &Lua, key_name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        key_name_to_keycode(&key_name)
            .map(|kc| input.is_key_just_pressed(kc))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown key name '{}'", key_name);
                false
            })
    })
}

/// `Engine.is_mouse_button_down(button_name)` — returns true if the named mouse button is currently pressed.
fn is_mouse_button_down(lua: &Lua, button_name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        mouse_button_name_to_enum(&button_name)
            .map(|btn| input.is_mouse_button_pressed(btn))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown mouse button name '{}'", button_name);
                false
            })
    })
}

/// `Engine.is_mouse_button_pressed(button_name)` — returns true only on the first frame the button is pressed.
fn is_mouse_button_just_pressed(lua: &Lua, button_name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        mouse_button_name_to_enum(&button_name)
            .map(|btn| input.is_mouse_button_just_pressed(btn))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown mouse button name '{}'", button_name);
                false
            })
    })
}

/// `Engine.get_mouse_position()` — returns `(x, y)` screen-space mouse position.
fn get_mouse_position(lua: &Lua, _: ()) -> LuaResult<(f64, f64)> {
    with_input(lua, (0.0, 0.0), |input| input.mouse_position())
}

// ---------------------------------------------------------------------------
// Engine.get_rotation / Engine.set_rotation / Engine.get_scale / Engine.set_scale
// ---------------------------------------------------------------------------

/// `Engine.get_rotation(entity_id)` — returns `(rx, ry, rz)` Euler angles in radians.
fn get_rotation(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_component::<super::TransformComponent>(entity)
            .map(|tc| {
                let e = tc.euler_angles();
                (e.x, e.y, e.z)
            })
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

/// `Engine.set_rotation(entity_id, rx, ry, rz)` — sets rotation from Euler angles in radians.
fn set_rotation(lua: &Lua, (entity_id, rx, ry, rz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.set_euler_angles(glam::Vec3::new(rx, ry, rz));
        }
    })
}

/// `Engine.get_rotation_quat(entity_id)` — returns `(x, y, z, w)` quaternion.
fn get_rotation_quat(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0, 1.0), |scene, entity| {
        scene
            .get_component::<super::TransformComponent>(entity)
            .map(|tc| (tc.rotation.x, tc.rotation.y, tc.rotation.z, tc.rotation.w))
            .unwrap_or((0.0, 0.0, 0.0, 1.0))
    })
}

/// `Engine.set_rotation_quat(entity_id, x, y, z, w)` — sets rotation as quaternion.
fn set_rotation_quat(
    lua: &Lua,
    (entity_id, x, y, z, w): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.rotation = glam::Quat::from_xyzw(x, y, z, w).normalize();
        }
    })
}

/// `Engine.get_scale(entity_id)` — returns `(sx, sy, sz)`.
fn get_scale(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (1.0, 1.0, 1.0), |scene, entity| {
        scene
            .get_component::<super::TransformComponent>(entity)
            .map(|tc| (tc.scale.x, tc.scale.y, tc.scale.z))
            .unwrap_or((1.0, 1.0, 1.0))
    })
}

/// `Engine.set_scale(entity_id, sx, sy, sz)` — sets scale.
fn set_scale(lua: &Lua, (entity_id, sx, sy, sz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TransformComponent>(entity) {
            tc.scale = glam::Vec3::new(sx, sy, sz);
        }
    })
}

// ---------------------------------------------------------------------------
// Engine.has_component
// ---------------------------------------------------------------------------

/// `Engine.has_component(entity_id, component_name)` — string-based component check.
fn has_component(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    with_entity(lua, entity_id, false, |scene, entity| match name.as_str() {
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
        "AudioListener" => scene.has_component::<super::AudioListenerComponent>(entity),
        "ParticleEmitter" => scene.has_component::<super::ParticleEmitterComponent>(entity),
        "Text" => scene.has_component::<super::TextComponent>(entity),
        "SpriteAnimator" => scene.has_component::<super::SpriteAnimatorComponent>(entity),
        "LuaScript" => {
            #[cfg(feature = "lua-scripting")]
            {
                scene.has_component::<super::LuaScriptComponent>(entity)
            }
            #[cfg(not(feature = "lua-scripting"))]
            {
                false
            }
        }
        _ => {
            log::warn!("ScriptGlue: unknown component name '{}'", name);
            false
        }
    })
}

// ---------------------------------------------------------------------------
// Entity lookup / cross-entity scripting
// ---------------------------------------------------------------------------

/// `Engine.find_entity_by_name(name)` — returns the UUID of the first entity
/// with the given tag name, or `nil` if not found.
fn find_entity_by_name(lua: &Lua, name: String) -> LuaResult<LuaValue> {
    with_scene_mut(lua, LuaValue::Nil, |scene| {
        match scene.find_entity_by_name(&name) {
            Some((_entity, uuid)) => LuaValue::Integer(uuid as i64),
            None => LuaValue::Nil,
        }
    })
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
fn set_script_field(
    lua: &Lua,
    (entity_id, field_name, value): (u64, String, LuaValue),
) -> LuaResult<()> {
    use super::script_engine::ENTITY_ENVS_REGISTRY_KEY;

    // Validate value type before touching the env table.
    match &value {
        LuaValue::Boolean(_) | LuaValue::Integer(_) | LuaValue::Number(_) | LuaValue::String(_) => {
        }
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
    with_scene_mut(lua, LuaValue::Integer(0), |scene| {
        let entity = scene.create_entity_with_tag(&name);
        let uuid = scene
            .get_component::<super::IdComponent>(entity)
            .map(|id| id.id.raw())
            .unwrap_or(0);
        LuaValue::Integer(uuid as i64)
    })
}

/// `Engine.destroy_entity(uuid)` — queue an entity for deferred destruction.
fn lua_destroy_entity(lua: &Lua, uuid: u64) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.queue_entity_destroy(uuid))
}

/// `Engine.get_entity_name(uuid)` — returns the entity's tag name, or nil.
fn lua_get_entity_name(lua: &Lua, uuid: u64) -> LuaResult<LuaValue> {
    let name = with_entity(lua, uuid, None, |scene, entity| {
        scene
            .get_component::<super::TagComponent>(entity)
            .map(|tag| tag.tag.clone())
    })?;
    match name {
        Some(s) => Ok(LuaValue::String(lua.create_string(&s)?)),
        None => Ok(LuaValue::Nil),
    }
}

// ---------------------------------------------------------------------------
// Hierarchy (parent-child relationships)
// ---------------------------------------------------------------------------

/// `Engine.set_parent(child_id, parent_id)` — reparent an entity, preserving world transform.
/// Returns `true` on success, `false` if either entity not found or cycle detected.
fn lua_set_parent(lua: &Lua, (child_id, parent_id): (u64, u64)) -> LuaResult<bool> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };

    let scene = unsafe { ctx.scene_mut() };
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
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.detach_from_parent(entity, true);
    })
}

/// `Engine.get_parent(entity_id)` — returns parent UUID as integer, or `nil` if root entity.
fn lua_get_parent(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    with_entity(lua, entity_id, LuaValue::Nil, |scene, entity| {
        match scene.get_parent(entity) {
            Some(parent_uuid) => LuaValue::Integer(parent_uuid as i64),
            None => LuaValue::Nil,
        }
    })
}

/// `Engine.get_children(entity_id)` — returns a Lua table (1-indexed array) of child UUIDs.
fn lua_get_children(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => {
            let empty = lua.create_table()?;
            return Ok(LuaValue::Table(empty));
        }
    };

    let scene = unsafe { ctx.scene() };
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
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        scene
            .get_component_mut::<super::SpriteAnimatorComponent>(entity)
            .map(|mut a| a.play(&name))
            .unwrap_or(false)
    })
}

/// `Engine.stop_animation(entity_id)` — stop the current animation.
fn lua_stop_animation(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut a) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            a.stop();
        }
    })
}

/// `Engine.is_animation_playing(entity_id)` — returns true if an animation is currently playing.
fn lua_is_animation_playing(lua: &Lua, entity_id: u64) -> LuaResult<bool> {
    with_entity(lua, entity_id, false, |scene, entity| {
        scene
            .get_component::<super::SpriteAnimatorComponent>(entity)
            .map(|a| a.is_playing())
            .unwrap_or(false)
    })
}

/// `Engine.get_current_animation(entity_id)` — returns the name of the currently
/// playing clip, or `nil` if no clip is active.
fn lua_get_current_animation(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let name = with_entity(lua, entity_id, None, |scene, entity| {
        scene
            .get_component::<super::SpriteAnimatorComponent>(entity)
            .and_then(|a| a.current_clip_name().map(|s| s.to_owned()))
    })?;
    match name {
        Some(s) => Ok(LuaValue::String(lua.create_string(&s)?)),
        None => Ok(LuaValue::Nil),
    }
}

/// `Engine.get_animation_frame(entity_id)` — returns the current frame number,
/// or `-1` if no animation is active.
fn lua_get_animation_frame(lua: &Lua, entity_id: u64) -> LuaResult<i32> {
    with_entity(lua, entity_id, -1, |scene, entity| {
        scene
            .get_component::<super::SpriteAnimatorComponent>(entity)
            .filter(|a| a.current_clip_index().is_some())
            .map(|a| a.current_frame() as i32)
            .unwrap_or(-1)
    })
}

/// `Engine.set_animation_speed(entity_id, speed_scale)` — sets the playback
/// speed multiplier (1.0 = normal, 0.5 = half speed, 2.0 = double speed).
fn lua_set_animation_speed(lua: &Lua, (entity_id, speed): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut a) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            a.speed_scale = speed;
        }
    })
}

// ---------------------------------------------------------------------------
// Instanced animation bindings
// ---------------------------------------------------------------------------

/// `Engine.play_instanced_animation(entity_id, clip_name)` — play a clip on
/// an InstancedSpriteAnimator by name. Returns true if the clip was found.
fn lua_play_instanced_animation(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        let gt = scene.global_time();
        scene
            .get_component_mut::<super::InstancedSpriteAnimator>(entity)
            .map(|mut a| a.play_by_name(&name, gt))
            .unwrap_or(false)
    })
}

/// `Engine.stop_instanced_animation(entity_id)` — stop an InstancedSpriteAnimator.
fn lua_stop_instanced_animation(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut a) = scene.get_component_mut::<super::InstancedSpriteAnimator>(entity) {
            a.stop();
        }
    })
}

/// `Engine.get_instanced_animation(entity_id)` — get current clip name, or nil.
fn lua_get_instanced_animation(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let name = with_entity(lua, entity_id, None, |scene, entity| {
        scene
            .get_component::<super::InstancedSpriteAnimator>(entity)
            .and_then(|a| a.current_clip_name().map(|s| s.to_owned()))
    })?;
    match name {
        Some(s) => Ok(LuaValue::String(lua.create_string(&s)?)),
        None => Ok(LuaValue::Nil),
    }
}

// ---------------------------------------------------------------------------
// Animation controller bindings
// ---------------------------------------------------------------------------

/// `Engine.set_anim_param(entity_id, name, value)` — set a bool or float
/// parameter on an AnimationControllerComponent. Auto-detects type.
fn lua_set_anim_param(
    lua: &Lua,
    (entity_id, name, value): (u64, String, LuaValue),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut ctrl) =
            scene.get_component_mut::<super::AnimationControllerComponent>(entity)
        {
            match value {
                LuaValue::Boolean(b) => {
                    ctrl.bool_params.insert(name, b);
                }
                LuaValue::Number(n) => {
                    ctrl.float_params.insert(name, n as f32);
                }
                LuaValue::Integer(n) => {
                    ctrl.float_params.insert(name, n as f32);
                }
                _ => {
                    log::warn!("set_anim_param: unsupported value type, expected bool or number");
                }
            }
        }
    })
}

/// `Engine.get_anim_param(entity_id, name)` — get a parameter value.
/// Returns the bool or float value, or nil if not found.
fn lua_get_anim_param(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<LuaValue> {
    with_entity(lua, entity_id, LuaValue::Nil, |scene, entity| {
        if let Some(ctrl) = scene.get_component::<super::AnimationControllerComponent>(entity) {
            if let Some(&b) = ctrl.bool_params.get(&name) {
                return LuaValue::Boolean(b);
            }
            if let Some(&f) = ctrl.float_params.get(&name) {
                return LuaValue::Number(f as f64);
            }
        }
        LuaValue::Nil
    })
}

// ---------------------------------------------------------------------------
// Audio bindings
// ---------------------------------------------------------------------------

/// `Engine.play_sound(entity_id)` — play the entity's audio source.
fn lua_play_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.play_entity_sound(entity);
    })
}

/// `Engine.stop_sound(entity_id)` — stop the entity's audio playback.
fn lua_stop_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.stop_entity_sound(entity);
    })
}

/// `Engine.pause_sound(entity_id)` — pause the entity's audio playback (can be resumed).
fn lua_pause_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.pause_entity_sound(entity);
    })
}

/// `Engine.resume_sound(entity_id)` — resume paused audio.
fn lua_resume_sound(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.resume_entity_sound(entity);
    })
}

/// `Engine.set_volume(entity_id, volume)` — adjust volume at runtime.
fn lua_set_volume(lua: &Lua, (entity_id, volume): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_entity_volume(entity, volume);
    })
}

/// `Engine.set_panning(entity_id, panning)` — adjust stereo panning at runtime.
/// Panning: -1.0 = hard left, 0.0 = center, 1.0 = hard right.
fn lua_set_panning(lua: &Lua, (entity_id, panning): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_entity_panning(entity, panning);
    })
}

// ---------------------------------------------------------------------------
// Tilemap bindings
// ---------------------------------------------------------------------------

/// `Engine.set_tile(entity_id, x, y, tile_id)` — set tile at grid position.
fn lua_set_tile(lua: &Lua, (entity_id, x, y, tile_id): (u64, u32, u32, i32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tilemap) = scene.get_component_mut::<super::TilemapComponent>(entity) {
            tilemap.set_tile(x, y, tile_id);
        }
    })
}

/// `Engine.get_tile(entity_id, x, y)` — returns tile ID at grid position, -1 if empty/OOB.
fn lua_get_tile(lua: &Lua, (entity_id, x, y): (u64, u32, u32)) -> LuaResult<i32> {
    with_entity(lua, entity_id, -1, |scene, entity| {
        scene
            .get_component::<super::TilemapComponent>(entity)
            .map(|tm| tm.get_tile(x, y))
            .unwrap_or(-1)
    })
}

// ---------------------------------------------------------------------------
// Physics bindings (delegate to Scene methods)
// ---------------------------------------------------------------------------

/// `Engine.apply_impulse(entity_id, ix, iy)`
fn lua_apply_impulse(lua: &Lua, (entity_id, ix, iy): (u64, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_impulse(entity, glam::Vec2::new(ix, iy));
    })
}

/// `Engine.apply_impulse_at_point(entity_id, ix, iy, px, py)`
fn lua_apply_impulse_at_point(
    lua: &Lua,
    (entity_id, ix, iy, px, py): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_impulse_at_point(entity, glam::Vec2::new(ix, iy), glam::Vec2::new(px, py));
    })
}

/// `Engine.apply_force(entity_id, fx, fy)`
fn lua_apply_force(lua: &Lua, (entity_id, fx, fy): (u64, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_force(entity, glam::Vec2::new(fx, fy));
    })
}

/// `Engine.get_linear_velocity(entity_id)` — returns `(vx, vy)`.
fn lua_get_linear_velocity(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0), |scene, entity| {
        scene
            .get_linear_velocity(entity)
            .map(|v| (v.x, v.y))
            .unwrap_or((0.0, 0.0))
    })
}

/// `Engine.set_linear_velocity(entity_id, vx, vy)`
fn lua_set_linear_velocity(lua: &Lua, (entity_id, vx, vy): (u64, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_linear_velocity(entity, glam::Vec2::new(vx, vy));
    })
}

/// `Engine.get_angular_velocity(entity_id)` — returns angular velocity in rad/s.
fn lua_get_angular_velocity(lua: &Lua, entity_id: u64) -> LuaResult<f32> {
    with_entity(lua, entity_id, 0.0, |scene, entity| {
        scene.get_angular_velocity(entity).unwrap_or(0.0)
    })
}

/// `Engine.set_angular_velocity(entity_id, omega)`
fn lua_set_angular_velocity(lua: &Lua, (entity_id, omega): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_angular_velocity(entity, omega);
    })
}

/// `Engine.raycast(origin_x, origin_y, dir_x, dir_y, max_distance, exclude_entity_id)`
///
/// Returns `(hit_entity_id, hit_x, hit_y, normal_x, normal_y, distance)` or `nil` on miss.
/// `exclude_entity_id` is optional (pass `nil` to hit everything).
fn lua_raycast(
    lua: &Lua,
    (ox, oy, dx, dy, max_dist, exclude_id): (f32, f32, f32, f32, f32, Option<u64>),
) -> LuaResult<(Option<u64>, f32, f32, f32, f32, f32)> {
    let zero = (None, 0.0, 0.0, 0.0, 0.0, 0.0);
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(zero),
    };
    let scene = unsafe { ctx.scene() };
    let exclude_entity = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    Ok(scene
        .raycast(
            glam::Vec2::new(ox, oy),
            glam::Vec2::new(dx, dy),
            max_dist,
            exclude_entity,
        )
        .map(|(uuid, hx, hy, nx, ny, toi)| (Some(uuid), hx, hy, nx, ny, toi))
        .unwrap_or(zero))
}

// ---------------------------------------------------------------------------
// 3D Physics bindings
// ---------------------------------------------------------------------------

/// `Engine.apply_impulse_3d(entity_id, ix, iy, iz)`
fn lua_apply_impulse_3d(lua: &Lua, (entity_id, ix, iy, iz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_impulse_3d(entity, glam::Vec3::new(ix, iy, iz));
    })
}

/// `Engine.apply_impulse_at_point_3d(entity_id, ix, iy, iz, px, py, pz)`
fn lua_apply_impulse_at_point_3d(
    lua: &Lua,
    (entity_id, ix, iy, iz, px, py, pz): (u64, f32, f32, f32, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_impulse_at_point_3d(
            entity,
            glam::Vec3::new(ix, iy, iz),
            glam::Vec3::new(px, py, pz),
        );
    })
}

/// `Engine.apply_force_3d(entity_id, fx, fy, fz)`
fn lua_apply_force_3d(lua: &Lua, (entity_id, fx, fy, fz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_force_3d(entity, glam::Vec3::new(fx, fy, fz));
    })
}

/// `Engine.apply_torque_impulse_3d(entity_id, tx, ty, tz)`
fn lua_apply_torque_impulse_3d(
    lua: &Lua,
    (entity_id, tx, ty, tz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque_impulse_3d(entity, glam::Vec3::new(tx, ty, tz));
    })
}

/// `Engine.apply_torque_3d(entity_id, tx, ty, tz)`
fn lua_apply_torque_3d(lua: &Lua, (entity_id, tx, ty, tz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque_3d(entity, glam::Vec3::new(tx, ty, tz));
    })
}

/// `Engine.get_linear_velocity_3d(entity_id)` — returns `(vx, vy, vz)`.
fn lua_get_linear_velocity_3d(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_linear_velocity_3d(entity)
            .map(|v| (v.x, v.y, v.z))
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

/// `Engine.set_linear_velocity_3d(entity_id, vx, vy, vz)`
fn lua_set_linear_velocity_3d(
    lua: &Lua,
    (entity_id, vx, vy, vz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_linear_velocity_3d(entity, glam::Vec3::new(vx, vy, vz));
    })
}

/// `Engine.get_angular_velocity_3d(entity_id)` — returns `(wx, wy, wz)` in rad/s.
fn lua_get_angular_velocity_3d(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_angular_velocity_3d(entity)
            .map(|w| (w.x, w.y, w.z))
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

/// `Engine.set_angular_velocity_3d(entity_id, wx, wy, wz)`
fn lua_set_angular_velocity_3d(
    lua: &Lua,
    (entity_id, wx, wy, wz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_angular_velocity_3d(entity, glam::Vec3::new(wx, wy, wz));
    })
}

/// `Engine.raycast_3d(ox, oy, oz, dx, dy, dz, max_distance, exclude_entity_id)`
///
/// Returns `(hit_entity_id, hx, hy, hz, nx, ny, nz, distance)` or `nil` values on miss.
fn lua_raycast_3d(
    lua: &Lua,
    (ox, oy, oz, dx, dy, dz, max_dist, exclude_id): (
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        Option<u64>,
    ),
) -> LuaResult<LuaRaycastHit3D> {
    let zero: LuaRaycastHit3D = (None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(zero),
    };
    let scene = unsafe { ctx.scene() };
    let exclude_entity = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    Ok(scene
        .raycast_3d(
            glam::Vec3::new(ox, oy, oz),
            glam::Vec3::new(dx, dy, dz),
            max_dist,
            exclude_entity,
        )
        .map(|h| {
            (
                Some(h.entity_uuid),
                h.hit_x,
                h.hit_y,
                h.hit_z,
                h.normal_x,
                h.normal_y,
                h.normal_z,
                h.toi,
            )
        })
        .unwrap_or(zero))
}

/// `Engine.set_gravity_3d(x, y, z)`
fn lua_set_gravity_3d(lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.set_gravity_3d(x, y, z))
}

/// `Engine.get_gravity_3d()` — returns `(x, y, z)`.
fn lua_get_gravity_3d(lua: &Lua, _: ()) -> LuaResult<(f32, f32, f32)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, -9.81, 0.0)),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.get_gravity_3d())
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
fn vector_dot(
    _lua: &Lua,
    (x1, y1, z1, x2, y2, z2): (f32, f32, f32, f32, f32, f32),
) -> LuaResult<f32> {
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

// ---------------------------------------------------------------------------
// Timer bindings
// ---------------------------------------------------------------------------

/// `Engine.set_timeout(callback, delay_seconds)` → timer_id
/// Schedules `callback` to be called once after `delay_seconds`.
fn lua_set_timeout(lua: &Lua, (callback, delay): (LuaFunction, f32)) -> LuaResult<usize> {
    use super::script_engine::{CurrentEntityUuid, PendingTimerCreate, PendingTimerOps};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);
    let callback_key = lua.create_registry_value(callback)?;

    let mut ops = lua
        .app_data_mut::<PendingTimerOps>()
        .ok_or_else(|| mlua::Error::RuntimeError("Timer system not available".into()))?;
    let id = ops.next_id;
    ops.next_id += 1;
    ops.creates.push(PendingTimerCreate {
        id,
        entity_uuid,
        delay,
        repeating: false,
        callback_key,
    });
    Ok(id)
}

/// `Engine.set_interval(callback, interval_seconds)` → timer_id
/// Schedules `callback` to be called repeatedly every `interval_seconds`.
fn lua_set_interval(lua: &Lua, (callback, interval): (LuaFunction, f32)) -> LuaResult<usize> {
    use super::script_engine::{CurrentEntityUuid, PendingTimerCreate, PendingTimerOps};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);
    let callback_key = lua.create_registry_value(callback)?;

    let mut ops = lua
        .app_data_mut::<PendingTimerOps>()
        .ok_or_else(|| mlua::Error::RuntimeError("Timer system not available".into()))?;
    let id = ops.next_id;
    ops.next_id += 1;
    ops.creates.push(PendingTimerCreate {
        id,
        entity_uuid,
        delay: interval,
        repeating: true,
        callback_key,
    });
    Ok(id)
}

/// `Engine.cancel_timer(timer_id)` — cancel a pending timeout or interval.
fn lua_cancel_timer(lua: &Lua, timer_id: usize) -> LuaResult<()> {
    use super::script_engine::PendingTimerOps;

    if let Some(mut ops) = lua.app_data_mut::<PendingTimerOps>() {
        ops.cancels.push(timer_id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entity queries
// ---------------------------------------------------------------------------

/// `Engine.find_entities_with_component(component_name)` → Lua table of entity UUIDs.
fn lua_find_entities_with_component(lua: &Lua, name: String) -> LuaResult<LuaTable> {
    let table = lua.create_table()?;

    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(table),
    };
    let scene = unsafe { ctx.scene() };

    let mut results: Vec<u64> = Vec::new();

    macro_rules! collect_uuids {
        ($comp_type:ty) => {{
            for (id,) in scene
                .world()
                .query::<(&super::IdComponent,)>()
                .with::<&$comp_type>()
                .iter()
            {
                results.push(id.id.raw());
            }
        }};
    }

    match name.as_str() {
        "Transform" => collect_uuids!(super::TransformComponent),
        "Camera" => collect_uuids!(super::CameraComponent),
        "SpriteRenderer" => collect_uuids!(super::SpriteRendererComponent),
        "CircleRenderer" => collect_uuids!(super::CircleRendererComponent),
        "RigidBody2D" => collect_uuids!(super::RigidBody2DComponent),
        "BoxCollider2D" => collect_uuids!(super::BoxCollider2DComponent),
        "CircleCollider2D" => collect_uuids!(super::CircleCollider2DComponent),
        "NativeScript" => collect_uuids!(super::NativeScriptComponent),
        "Tilemap" => collect_uuids!(super::TilemapComponent),
        "AudioSource" | "Audio" => collect_uuids!(super::AudioSourceComponent),
        "AudioListener" => collect_uuids!(super::AudioListenerComponent),
        "ParticleEmitter" => collect_uuids!(super::ParticleEmitterComponent),
        "Text" => collect_uuids!(super::TextComponent),
        "SpriteAnimator" => collect_uuids!(super::SpriteAnimatorComponent),
        #[cfg(feature = "lua-scripting")]
        "LuaScript" => collect_uuids!(super::LuaScriptComponent),
        _ => {
            log::warn!(
                "ScriptGlue: unknown component name '{}' in find_entities_with_component",
                name
            );
        }
    }

    for (i, uuid) in results.iter().enumerate() {
        table.set(i + 1, *uuid as f64)?;
    }
    Ok(table)
}

// ---------------------------------------------------------------------------
// Component access (sprite, circle, text)
// ---------------------------------------------------------------------------

/// `Engine.get_sprite_color(entity_id)` → (r, g, b, a) or (1,1,1,1) if not found.
fn lua_get_sprite_color(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32, f32)> {
    with_entity(lua, entity_id, (1.0, 1.0, 1.0, 1.0), |scene, entity| {
        scene
            .get_component::<super::SpriteRendererComponent>(entity)
            .map(|s| (s.color.x, s.color.y, s.color.z, s.color.w))
            .unwrap_or((1.0, 1.0, 1.0, 1.0))
    })
}

/// `Engine.set_sprite_color(entity_id, r, g, b, a)` — set sprite tint color.
fn lua_set_sprite_color(
    lua: &Lua,
    (entity_id, r, g, b, a): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut s) = scene.get_component_mut::<super::SpriteRendererComponent>(entity) {
            s.color = glam::Vec4::new(r, g, b, a);
        }
    })
}

/// `Engine.set_sprite_texture(entity_id, texture_handle)` — swap the sprite's texture at runtime.
///
/// `texture_handle` is the asset UUID (u64). The texture must already be loaded
/// (e.g. used elsewhere in the scene or pre-loaded via the asset manager).
/// Pass 0 to clear the texture.
fn lua_set_sprite_texture(lua: &Lua, (entity_id, handle_raw): (u64, u64)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut s) = scene.get_component_mut::<super::SpriteRendererComponent>(entity) {
            s.texture_handle = crate::uuid::Uuid::from_raw(handle_raw);
            s.texture = None;
        }
    })
}

/// `Engine.get_text(entity_id)` → string or "" if not found.
fn lua_get_text(lua: &Lua, entity_id: u64) -> LuaResult<String> {
    with_entity(lua, entity_id, String::new(), |scene, entity| {
        scene
            .get_component::<super::TextComponent>(entity)
            .map(|tc| tc.text.clone())
            .unwrap_or_default()
    })
}

/// `Engine.set_text(entity_id, text)` — set text component content.
fn lua_set_text(lua: &Lua, (entity_id, text): (u64, String)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TextComponent>(entity) {
            tc.text = text;
        }
    })
}

// ---------------------------------------------------------------------------
// Physics: gravity
// ---------------------------------------------------------------------------

/// `Engine.set_gravity(x, y)` — set the physics world gravity vector.
fn lua_set_gravity(lua: &Lua, (x, y): (f32, f32)) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.set_gravity(x, y))
}

/// `Engine.get_gravity()` → (x, y).
fn lua_get_gravity(lua: &Lua, _: ()) -> LuaResult<(f32, f32)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, -9.81)),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.get_gravity())
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// `Engine.get_time()` → scene elapsed time in seconds (f64).
fn lua_get_time(lua: &Lua, _: ()) -> LuaResult<f64> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(0.0),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.global_time())
}

/// `Engine.delta_time()` → current frame delta time in seconds.
fn lua_delta_time(lua: &Lua, _: ()) -> LuaResult<f32> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(0.0),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.last_dt())
}

// ---------------------------------------------------------------------------
// Input: is_key_just_released
// ---------------------------------------------------------------------------

/// `Engine.is_key_released(key_name)` — returns true on the first frame the key is released.
fn lua_is_key_just_released(lua: &Lua, key_name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        key_name_to_keycode(&key_name)
            .map(|kc| input.is_key_just_released(kc))
            .unwrap_or(false)
    })
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

/// `Engine.log(message)` — log to the engine console at Info level.
fn lua_log(lua: &Lua, args: mlua::Variadic<LuaValue>) -> LuaResult<()> {
    let _ = lua; // suppress unused warning
    let parts: Vec<String> = args
        .iter()
        .map(|v| match v {
            LuaValue::Nil => "nil".to_string(),
            LuaValue::Boolean(b) => b.to_string(),
            LuaValue::Integer(i) => i.to_string(),
            LuaValue::Number(n) => n.to_string(),
            LuaValue::String(s) => s
                .to_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|_| "<invalid utf8>".to_string()),
            _ => format!("{:?}", v),
        })
        .collect();
    log::info!("[Lua] {}", parts.join("\t"));
    Ok(())
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
        let engine: LuaTable = lua
            .globals()
            .get("Engine")
            .expect("Engine table should exist");
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
        assert!(engine.get::<LuaFunction>("get_current_animation").is_ok());
        assert!(engine.get::<LuaFunction>("get_animation_frame").is_ok());
        assert!(engine.get::<LuaFunction>("set_animation_speed").is_ok());
        // Sprite
        assert!(engine.get::<LuaFunction>("set_sprite_texture").is_ok());
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
        assert!(engine.get::<LuaFunction>("set_panning").is_ok());
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
        lua.load("Engine.rust_function()")
            .exec()
            .expect("rust_function should not error");
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
        assert_eq!(
            mouse_button_name_to_enum("Middle"),
            Some(MouseButton::Middle)
        );
        assert_eq!(mouse_button_name_to_enum("Back"), Some(MouseButton::Back));
        assert_eq!(
            mouse_button_name_to_enum("Forward"),
            Some(MouseButton::Forward)
        );
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
        lua.load("Engine.destroy_entity(12345)").exec().unwrap();
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
        lua.load("Engine.set_tile(12345, 0, 0, 1)").exec().unwrap();
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
        lua.load("Engine.stop_animation(12345)").exec().unwrap();
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
    fn get_current_animation_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.get_current_animation(12345)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result == LuaValue::Nil);
    }

    #[test]
    fn get_animation_frame_no_context_returns_neg1() {
        let lua = setup();
        lua.load("result = Engine.get_animation_frame(12345)")
            .exec()
            .unwrap();
        let result: i32 = lua.globals().get("result").unwrap();
        assert_eq!(result, -1);
    }

    #[test]
    fn set_animation_speed_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_animation_speed(12345, 2.0)")
            .exec()
            .unwrap();
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
    fn set_panning_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_panning(12345, -0.5)")
            .exec()
            .expect("set_panning should not error without context");
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
        lua.load("Engine.detach_from_parent(12345)").exec().unwrap();
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
