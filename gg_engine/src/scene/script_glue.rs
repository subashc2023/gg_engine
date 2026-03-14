//! Centralized Rust↔Lua bindings — the Lua equivalent of Cherno's `ScriptGlue.cpp`.
//!
//! Every Rust function exposed to Lua scripts lives here. Functions are
//! registered under an `Engine` global table so scripts call e.g.
//! `Engine.native_log("hello", 42)`.

use mlua::prelude::*;

use super::{CameraComponent, Entity, Scene, TransformComponent};

/// Lua return type for 3D raycast: `(entity_uuid?, hx, hy, hz, nx, ny, nz, toi)`.
#[cfg(feature = "physics-3d")]
type LuaRaycastHit3D = (Option<u64>, f32, f32, f32, f32, f32, f32, f32);
use crate::events::gamepad::{GamepadAxis, GamepadButton};
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

// SAFETY: SceneScriptContext contains raw pointers to Scene and Input.
// It is only stored as mlua `app_data` for the duration of a single Lua
// dispatch (on_create / on_update / on_destroy), then immediately removed.
// The take-modify-replace pattern in `lua_ops.rs` guarantees exclusive access
// to Scene while scripts run. The `input` pointer is valid for the entire
// frame (stack-borrowed in `on_update_lua_scripts`). No cross-thread access
// is possible: mlua::Lua is `!Send` and runs on the main thread only.
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
    engine.set("vector_length", lua.create_function(vector_length)?)?;
    engine.set("distance", lua.create_function(lua_distance)?)?;
    engine.set("distance_2d", lua.create_function(lua_distance_2d)?)?;
    engine.set("lerp", lua.create_function(lua_lerp)?)?;
    engine.set("lerp_vec3", lua.create_function(lua_lerp_vec3)?)?;
    engine.set("slerp", lua.create_function(lua_slerp)?)?;
    engine.set("clamp", lua.create_function(lua_clamp)?)?;
    engine.set("move_toward", lua.create_function(lua_move_toward)?)?;
    engine.set(
        "move_toward_vec3",
        lua.create_function(lua_move_toward_vec3)?,
    )?;

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
    engine.set("get_mouse_delta", lua.create_function(get_mouse_delta)?)?;
    engine.set("get_scroll_delta", lua.create_function(get_scroll_delta)?)?;

    // Gamepad input
    engine.set(
        "is_gamepad_button_down",
        lua.create_function(lua_is_gamepad_button_down)?,
    )?;
    engine.set(
        "is_gamepad_button_pressed",
        lua.create_function(lua_is_gamepad_button_just_pressed)?,
    )?;
    engine.set(
        "is_gamepad_button_released",
        lua.create_function(lua_is_gamepad_button_just_released)?,
    )?;
    engine.set(
        "get_gamepad_axis",
        lua.create_function(lua_get_gamepad_axis)?,
    )?;
    engine.set(
        "is_gamepad_connected",
        lua.create_function(lua_is_gamepad_connected)?,
    )?;

    // Input action mapping
    engine.set(
        "is_action_pressed",
        lua.create_function(lua_is_action_pressed)?,
    )?;
    engine.set(
        "is_action_just_pressed",
        lua.create_function(lua_is_action_just_pressed)?,
    )?;
    engine.set(
        "is_action_just_released",
        lua.create_function(lua_is_action_just_released)?,
    )?;
    engine.set(
        "get_action_value",
        lua.create_function(lua_get_action_value)?,
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

    // Skeletal animation
    engine.set(
        "play_skeletal_animation",
        lua.create_function(lua_play_skeletal_animation)?,
    )?;
    engine.set(
        "play_skeletal_animation_blended",
        lua.create_function(lua_play_skeletal_animation_blended)?,
    )?;
    engine.set(
        "stop_skeletal_animation",
        lua.create_function(lua_stop_skeletal_animation)?,
    )?;
    engine.set(
        "is_skeletal_animation_playing",
        lua.create_function(lua_is_skeletal_animation_playing)?,
    )?;
    engine.set(
        "get_skeletal_animation",
        lua.create_function(lua_get_skeletal_animation)?,
    )?;
    engine.set(
        "set_skeletal_animation_speed",
        lua.create_function(lua_set_skeletal_animation_speed)?,
    )?;
    engine.set(
        "get_skeletal_animation_time",
        lua.create_function(lua_get_skeletal_animation_time)?,
    )?;
    engine.set(
        "list_skeletal_animations",
        lua.create_function(lua_list_skeletal_animations)?,
    )?;

    // Animation events
    engine.set(
        "add_animation_event",
        lua.create_function(lua_add_animation_event)?,
    )?;
    engine.set(
        "remove_animation_event",
        lua.create_function(lua_remove_animation_event)?,
    )?;
    engine.set(
        "add_skeletal_anim_event",
        lua.create_function(lua_add_skeletal_anim_event)?,
    )?;
    engine.set(
        "remove_skeletal_anim_event",
        lua.create_function(lua_remove_skeletal_anim_event)?,
    )?;

    // Audio
    engine.set("play_sound", lua.create_function(lua_play_sound)?)?;
    engine.set("stop_sound", lua.create_function(lua_stop_sound)?)?;
    engine.set("pause_sound", lua.create_function(lua_pause_sound)?)?;
    engine.set("resume_sound", lua.create_function(lua_resume_sound)?)?;
    engine.set("set_volume", lua.create_function(lua_set_volume)?)?;
    engine.set("set_panning", lua.create_function(lua_set_panning)?)?;
    engine.set("fade_in", lua.create_function(lua_fade_in)?)?;
    engine.set("fade_out", lua.create_function(lua_fade_out)?)?;
    engine.set("fade_to", lua.create_function(lua_fade_to)?)?;
    engine.set(
        "set_master_volume",
        lua.create_function(lua_set_master_volume)?,
    )?;
    engine.set(
        "get_master_volume",
        lua.create_function(lua_get_master_volume)?,
    )?;
    engine.set(
        "set_category_volume",
        lua.create_function(lua_set_category_volume)?,
    )?;
    engine.set(
        "get_category_volume",
        lua.create_function(lua_get_category_volume)?,
    )?;

    engine.set("set_hrtf", lua.create_function(lua_set_hrtf)?)?;
    engine.set("get_hrtf", lua.create_function(lua_get_hrtf)?)?;

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
    engine.set("raycast_all", lua.create_function(lua_raycast_all)?)?;
    engine.set(
        "apply_torque_impulse",
        lua.create_function(lua_apply_torque_impulse)?,
    )?;
    engine.set("apply_torque", lua.create_function(lua_apply_torque)?)?;
    engine.set(
        "set_gravity_scale",
        lua.create_function(lua_set_gravity_scale)?,
    )?;
    engine.set(
        "get_gravity_scale",
        lua.create_function(lua_get_gravity_scale)?,
    )?;
    engine.set("screen_to_world", lua.create_function(lua_screen_to_world)?)?;

    // 3D Physics
    #[cfg(feature = "physics-3d")]
    {
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
    }

    // Runtime body type changes
    engine.set("set_body_type", lua.create_function(lua_set_body_type)?)?;
    engine.set("get_body_type", lua.create_function(lua_get_body_type)?)?;
    #[cfg(feature = "physics-3d")]
    {
        engine.set(
            "set_body_type_3d",
            lua.create_function(lua_set_body_type_3d)?,
        )?;
        engine.set(
            "get_body_type_3d",
            lua.create_function(lua_get_body_type_3d)?,
        )?;
    }

    // Shape overlap queries
    engine.set("point_query", lua.create_function(lua_point_query)?)?;
    engine.set("aabb_query", lua.create_function(lua_aabb_query)?)?;
    engine.set("overlap_circle", lua.create_function(lua_overlap_circle)?)?;
    engine.set("overlap_box", lua.create_function(lua_overlap_box)?)?;
    #[cfg(feature = "physics-3d")]
    {
        engine.set("point_query_3d", lua.create_function(lua_point_query_3d)?)?;
        engine.set("aabb_query_3d", lua.create_function(lua_aabb_query_3d)?)?;
        engine.set("overlap_sphere", lua.create_function(lua_overlap_sphere)?)?;
        engine.set("overlap_box_3d", lua.create_function(lua_overlap_box_3d)?)?;
    }

    // Spatial grid queries (non-physics, transform-based)
    engine.set(
        "rebuild_spatial_grid",
        lua.create_function(lua_rebuild_spatial_grid)?,
    )?;
    engine.set(
        "rebuild_spatial_grid_3d",
        lua.create_function(lua_rebuild_spatial_grid_3d)?,
    )?;
    engine.set("query_region", lua.create_function(lua_query_region)?)?;
    engine.set("query_radius", lua.create_function(lua_query_radius)?)?;
    engine.set(
        "query_region_3d",
        lua.create_function(lua_query_region_3d)?,
    )?;
    engine.set(
        "query_radius_3d",
        lua.create_function(lua_query_radius_3d)?,
    )?;

    // Joints (2D)
    engine.set(
        "create_revolute_joint",
        lua.create_function(lua_create_revolute_joint)?,
    )?;
    engine.set(
        "create_fixed_joint",
        lua.create_function(lua_create_fixed_joint)?,
    )?;
    engine.set(
        "create_prismatic_joint",
        lua.create_function(lua_create_prismatic_joint)?,
    )?;
    engine.set("remove_joint", lua.create_function(lua_remove_joint)?)?;

    // Joints (3D)
    #[cfg(feature = "physics-3d")]
    {
        engine.set(
            "create_revolute_joint_3d",
            lua.create_function(lua_create_revolute_joint_3d)?,
        )?;
        engine.set(
            "create_fixed_joint_3d",
            lua.create_function(lua_create_fixed_joint_3d)?,
        )?;
        engine.set(
            "create_ball_joint_3d",
            lua.create_function(lua_create_ball_joint_3d)?,
        )?;
        engine.set(
            "create_prismatic_joint_3d",
            lua.create_function(lua_create_prismatic_joint_3d)?,
        )?;
        engine.set("remove_joint_3d", lua.create_function(lua_remove_joint_3d)?)?;
    }

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

    // Coroutines
    engine.set(
        "start_coroutine",
        lua.create_function(lua_start_coroutine)?,
    )?;
    engine.set(
        "stop_all_coroutines",
        lua.create_function(lua_stop_all_coroutines)?,
    )?;

    // Event bus
    engine.set("emit", lua.create_function(lua_emit)?)?;
    engine.set("on", lua.create_function(lua_on)?)?;
    engine.set("off", lua.create_function(lua_off)?)?;

    // Physics: gravity
    engine.set("set_gravity", lua.create_function(lua_set_gravity)?)?;
    engine.set("get_gravity", lua.create_function(lua_get_gravity)?)?;

    // Cursor
    engine.set("set_cursor_mode", lua.create_function(lua_set_cursor_mode)?)?;
    engine.set("get_cursor_mode", lua.create_function(lua_get_cursor_mode)?)?;
    engine.set("set_window_size", lua.create_function(lua_set_window_size)?)?;
    engine.set("get_window_size", lua.create_function(lua_get_window_size)?)?;

    // UI Anchor
    engine.set("set_ui_anchor", lua.create_function(lua_set_ui_anchor)?)?;
    engine.set("get_ui_anchor", lua.create_function(lua_get_ui_anchor)?)?;

    // UI Rect / Image / Interactable
    engine.set("set_ui_rect", lua.create_function(lua_set_ui_rect)?)?;
    engine.set("get_ui_rect", lua.create_function(lua_get_ui_rect)?)?;
    engine.set("set_ui_pivot", lua.create_function(lua_set_ui_pivot)?)?;
    engine.set("set_ui_image", lua.create_function(lua_set_ui_image)?)?;
    engine.set(
        "set_ui_image_color",
        lua.create_function(lua_set_ui_image_color)?,
    )?;
    engine.set("set_ui_border", lua.create_function(lua_set_ui_border)?)?;
    engine.set(
        "set_ui_interactable",
        lua.create_function(lua_set_ui_interactable)?,
    )?;
    engine.set("get_ui_state", lua.create_function(lua_get_ui_state)?)?;
    engine.set(
        "create_ui_entity",
        lua.create_function(lua_create_ui_entity)?,
    )?;

    // Time
    engine.set("get_time", lua.create_function(lua_get_time)?)?;
    engine.set("delta_time", lua.create_function(lua_delta_time)?)?;

    // Input: key released
    engine.set(
        "is_key_released",
        lua.create_function(lua_is_key_just_released)?,
    )?;

    // Text color
    engine.set("get_text_color", lua.create_function(lua_get_text_color)?)?;
    engine.set("set_text_color", lua.create_function(lua_set_text_color)?)?;

    // Runtime settings
    engine.set("get_vsync", lua.create_function(lua_get_vsync)?)?;
    engine.set("set_vsync", lua.create_function(lua_set_vsync)?)?;
    engine.set("get_fullscreen", lua.create_function(lua_get_fullscreen)?)?;
    engine.set("set_fullscreen", lua.create_function(lua_set_fullscreen)?)?;
    engine.set(
        "get_shadow_quality",
        lua.create_function(lua_get_shadow_quality)?,
    )?;
    engine.set(
        "set_shadow_quality",
        lua.create_function(lua_set_shadow_quality)?,
    )?;
    engine.set("quit", lua.create_function(lua_quit)?)?;
    engine.set("load_scene", lua.create_function(lua_load_scene)?)?;
    engine.set("get_gui_scale", lua.create_function(lua_get_gui_scale)?)?;
    engine.set("set_gui_scale", lua.create_function(lua_set_gui_scale)?)?;

    // Loading screen
    engine.set(
        "set_loading_screen_color",
        lua.create_function(lua_set_loading_screen_color)?,
    )?;
    engine.set(
        "get_loading_screen_color",
        lua.create_function(lua_get_loading_screen_color)?,
    )?;

    // Component manipulation
    engine.set("add_component", lua.create_function(lua_add_component)?)?;
    engine.set(
        "remove_component",
        lua.create_function(lua_remove_component)?,
    )?;

    // Logging
    engine.set("log", lua.create_function(lua_log)?)?;

    // Save/load game data
    engine.set("save_data", lua.create_function(lua_save_data)?)?;
    engine.set("load_data", lua.create_function(lua_load_data)?)?;
    engine.set("delete_save", lua.create_function(lua_delete_save)?)?;
    engine.set("save_exists", lua.create_function(lua_save_exists)?)?;
    engine.set("list_saves", lua.create_function(lua_list_saves)?)?;

    lua.globals().set("Engine", engine)?;

    // Register coroutine helpers as Lua closures on the Engine table.
    // These must run after Engine is in globals so they can index it.
    lua.load(
        r#"
        Engine.wait = function(seconds)
            coroutine.yield(seconds or 0)
        end
        Engine.wait_frame = function()
            coroutine.yield(0)
        end
        "#,
    )
    .exec()?;

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
        "0" | "Num0" | "Key0" | "Digit0" => Some(KeyCode::Num0),
        "1" | "Num1" | "Key1" | "Digit1" => Some(KeyCode::Num1),
        "2" | "Num2" | "Key2" | "Digit2" => Some(KeyCode::Num2),
        "3" | "Num3" | "Key3" | "Digit3" => Some(KeyCode::Num3),
        "4" | "Num4" | "Key4" | "Digit4" => Some(KeyCode::Num4),
        "5" | "Num5" | "Key5" | "Digit5" => Some(KeyCode::Num5),
        "6" | "Num6" | "Key6" | "Digit6" => Some(KeyCode::Num6),
        "7" | "Num7" | "Key7" | "Digit7" => Some(KeyCode::Num7),
        "8" | "Num8" | "Key8" | "Digit8" => Some(KeyCode::Num8),
        "9" | "Num9" | "Key9" | "Digit9" => Some(KeyCode::Num9),

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
        #[cfg(feature = "physics-3d")]
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

/// `Engine.get_mouse_delta()` — returns `(dx, dy)` raw mouse motion delta this frame.
fn get_mouse_delta(lua: &Lua, _: ()) -> LuaResult<(f64, f64)> {
    with_input(lua, (0.0, 0.0), |input| input.mouse_delta())
}

/// `Engine.get_scroll_delta()` — returns `(dx, dy)` scroll wheel delta this frame.
fn get_scroll_delta(lua: &Lua, _: ()) -> LuaResult<(f64, f64)> {
    with_input(lua, (0.0, 0.0), |input| input.scroll_delta())
}

// ---------------------------------------------------------------------------
// Input action mapping
// ---------------------------------------------------------------------------

/// `Engine.is_action_pressed("jump")` — true while the action is active.
fn lua_is_action_pressed(lua: &Lua, name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| input.is_action_pressed(&name))
}

/// `Engine.is_action_just_pressed("jump")` — true on the first frame the action becomes active.
fn lua_is_action_just_pressed(lua: &Lua, name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| input.is_action_just_pressed(&name))
}

/// `Engine.is_action_just_released("jump")` — true on the first frame the action becomes inactive.
fn lua_is_action_just_released(lua: &Lua, name: String) -> LuaResult<bool> {
    with_input(lua, false, |input| input.is_action_just_released(&name))
}

/// `Engine.get_action_value("move_horizontal")` — returns the continuous axis value (-1.0..1.0).
fn lua_get_action_value(lua: &Lua, name: String) -> LuaResult<f32> {
    with_input(lua, 0.0, |input| input.action_value(&name))
}

// ---------------------------------------------------------------------------
// Gamepad button/axis name → enum mapping
// ---------------------------------------------------------------------------

fn gamepad_button_name_to_enum(name: &str) -> Option<GamepadButton> {
    match name {
        "South" | "A" | "Cross" => Some(GamepadButton::South),
        "East" | "B" | "Circle" => Some(GamepadButton::East),
        "West" | "X" | "Square" => Some(GamepadButton::West),
        "North" | "Y" | "Triangle" => Some(GamepadButton::North),
        "LeftBumper" | "L1" => Some(GamepadButton::LeftBumper),
        "RightBumper" | "R1" => Some(GamepadButton::RightBumper),
        "LeftTrigger" | "L2" => Some(GamepadButton::LeftTrigger),
        "RightTrigger" | "R2" => Some(GamepadButton::RightTrigger),
        "Select" | "Back" | "Share" => Some(GamepadButton::Select),
        "Start" | "Options" => Some(GamepadButton::Start),
        "Guide" | "Home" | "PS" => Some(GamepadButton::Guide),
        "LeftStick" | "L3" => Some(GamepadButton::LeftStick),
        "RightStick" | "R3" => Some(GamepadButton::RightStick),
        "DPadUp" => Some(GamepadButton::DPadUp),
        "DPadDown" => Some(GamepadButton::DPadDown),
        "DPadLeft" => Some(GamepadButton::DPadLeft),
        "DPadRight" => Some(GamepadButton::DPadRight),
        _ => None,
    }
}

fn gamepad_axis_name_to_enum(name: &str) -> Option<GamepadAxis> {
    match name {
        "LeftStickX" => Some(GamepadAxis::LeftStickX),
        "LeftStickY" => Some(GamepadAxis::LeftStickY),
        "RightStickX" => Some(GamepadAxis::RightStickX),
        "RightStickY" => Some(GamepadAxis::RightStickY),
        "LeftTrigger" => Some(GamepadAxis::LeftTrigger),
        "RightTrigger" => Some(GamepadAxis::RightTrigger),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Gamepad input bindings
// ---------------------------------------------------------------------------

/// `Engine.is_gamepad_button_down(gamepad_id, button_name)` — true while held.
fn lua_is_gamepad_button_down(lua: &Lua, (gamepad_id, name): (usize, String)) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        gamepad_button_name_to_enum(&name)
            .map(|btn| input.is_gamepad_button_pressed(gamepad_id, btn))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown gamepad button name '{}'", name);
                false
            })
    })
}

/// `Engine.is_gamepad_button_pressed(gamepad_id, button_name)` — true on first frame pressed.
fn lua_is_gamepad_button_just_pressed(
    lua: &Lua,
    (gamepad_id, name): (usize, String),
) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        gamepad_button_name_to_enum(&name)
            .map(|btn| input.is_gamepad_button_just_pressed(gamepad_id, btn))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown gamepad button name '{}'", name);
                false
            })
    })
}

/// `Engine.is_gamepad_button_released(gamepad_id, button_name)` — true on first frame released.
fn lua_is_gamepad_button_just_released(
    lua: &Lua,
    (gamepad_id, name): (usize, String),
) -> LuaResult<bool> {
    with_input(lua, false, |input| {
        gamepad_button_name_to_enum(&name)
            .map(|btn| input.is_gamepad_button_just_released(gamepad_id, btn))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown gamepad button name '{}'", name);
                false
            })
    })
}

/// `Engine.get_gamepad_axis(gamepad_id, axis_name)` — returns analog axis value (f32).
fn lua_get_gamepad_axis(lua: &Lua, (gamepad_id, name): (usize, String)) -> LuaResult<f32> {
    with_input(lua, 0.0, |input| {
        gamepad_axis_name_to_enum(&name)
            .map(|axis| input.gamepad_axis(gamepad_id, axis))
            .unwrap_or_else(|| {
                log::warn!("ScriptGlue: unknown gamepad axis name '{}'", name);
                0.0
            })
    })
}

/// `Engine.is_gamepad_connected(gamepad_id)` — true if the gamepad is connected.
fn lua_is_gamepad_connected(lua: &Lua, gamepad_id: usize) -> LuaResult<bool> {
    with_input(lua, false, |input| input.is_gamepad_connected(gamepad_id))
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
                let e = tc.euler_angles_stable();
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
            tc.set_rotation_quat(glam::Quat::from_xyzw(x, y, z, w).normalize());
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
        "InstancedSpriteAnimator" => scene.has_component::<super::InstancedSpriteAnimator>(entity),
        "AnimationController" => scene.has_component::<super::AnimationControllerComponent>(entity),
        #[cfg(feature = "physics-3d")]
        "RigidBody3D" => scene.has_component::<super::RigidBody3DComponent>(entity),
        #[cfg(feature = "physics-3d")]
        "BoxCollider3D" => scene.has_component::<super::BoxCollider3DComponent>(entity),
        #[cfg(feature = "physics-3d")]
        "SphereCollider3D" => scene.has_component::<super::SphereCollider3DComponent>(entity),
        #[cfg(feature = "physics-3d")]
        "CapsuleCollider3D" => scene.has_component::<super::CapsuleCollider3DComponent>(entity),
        "MeshRenderer" => scene.has_component::<super::MeshRendererComponent>(entity),
        "DirectionalLight" => scene.has_component::<super::DirectionalLightComponent>(entity),
        "PointLight" => scene.has_component::<super::PointLightComponent>(entity),
        "AmbientLight" => scene.has_component::<super::AmbientLightComponent>(entity),
        "UIAnchor" => scene.has_component::<super::UIAnchorComponent>(entity),
        "UIRect" => scene.has_component::<super::UIRectComponent>(entity),
        "UIImage" => scene.has_component::<super::UIImageComponent>(entity),
        "UIInteractable" => scene.has_component::<super::UIInteractableComponent>(entity),
        "UILayout" => scene.has_component::<super::UILayoutComponent>(entity),
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
// Skeletal animation bindings
// ---------------------------------------------------------------------------

/// `Engine.play_skeletal_animation(entity_id, name)` — play a skeletal
/// animation clip by name (hard cut). Returns true if the clip was found.
fn lua_play_skeletal_animation(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        if let Some(mut sac) =
            scene.get_component_mut::<super::SkeletalAnimationComponent>(entity)
        {
            let found = sac.clips.iter().position(|c| c.name == name);
            if let Some(idx) = found {
                sac.play(idx);
                return true;
            }
        }
        false
    })
}

/// `Engine.play_skeletal_animation_blended(entity_id, name, blend_duration)`
/// — crossfade to a skeletal animation clip. Returns true if the clip was found.
fn lua_play_skeletal_animation_blended(
    lua: &Lua,
    (entity_id, name, blend_secs): (u64, String, f32),
) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        scene
            .get_component_mut::<super::SkeletalAnimationComponent>(entity)
            .map(|mut sac| sac.play_by_name_blended(&name, blend_secs))
            .unwrap_or(false)
    })
}

/// `Engine.stop_skeletal_animation(entity_id)` — stop skeletal animation playback.
fn lua_stop_skeletal_animation(lua: &Lua, entity_id: u64) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut sac) =
            scene.get_component_mut::<super::SkeletalAnimationComponent>(entity)
        {
            sac.stop();
        }
    })
}

/// `Engine.is_skeletal_animation_playing(entity_id)` — returns true if a
/// skeletal animation is currently playing.
fn lua_is_skeletal_animation_playing(lua: &Lua, entity_id: u64) -> LuaResult<bool> {
    with_entity(lua, entity_id, false, |scene, entity| {
        scene
            .get_component::<super::SkeletalAnimationComponent>(entity)
            .map(|sac| sac.playing)
            .unwrap_or(false)
    })
}

/// `Engine.get_skeletal_animation(entity_id)` — returns the name of the
/// currently playing skeletal clip, or `nil`.
fn lua_get_skeletal_animation(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let name = with_entity(lua, entity_id, None, |scene, entity| {
        scene
            .get_component::<super::SkeletalAnimationComponent>(entity)
            .and_then(|sac| sac.current_clip_name().map(|s| s.to_owned()))
    })?;
    match name {
        Some(s) => Ok(LuaValue::String(lua.create_string(&s)?)),
        None => Ok(LuaValue::Nil),
    }
}

/// `Engine.set_skeletal_animation_speed(entity_id, speed)` — set the
/// playback speed multiplier for skeletal animation.
fn lua_set_skeletal_animation_speed(
    lua: &Lua,
    (entity_id, speed): (u64, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut sac) =
            scene.get_component_mut::<super::SkeletalAnimationComponent>(entity)
        {
            sac.speed = speed;
        }
    })
}

/// `Engine.get_skeletal_animation_time(entity_id)` — returns the current
/// playback time (seconds) of the skeletal animation, or 0.
fn lua_get_skeletal_animation_time(lua: &Lua, entity_id: u64) -> LuaResult<f32> {
    with_entity(lua, entity_id, 0.0, |scene, entity| {
        scene
            .get_component::<super::SkeletalAnimationComponent>(entity)
            .map(|sac| sac.playback_time)
            .unwrap_or(0.0)
    })
}

/// `Engine.list_skeletal_animations(entity_id)` — returns a table of clip
/// names, or an empty table if the entity has no skeletal animation.
fn lua_list_skeletal_animations(lua: &Lua, entity_id: u64) -> LuaResult<LuaValue> {
    let names: Vec<String> = with_entity(lua, entity_id, Vec::new(), |scene, entity| {
        scene
            .get_component::<super::SkeletalAnimationComponent>(entity)
            .map(|sac| sac.clips.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default()
    })?;
    let table = lua.create_table()?;
    for (i, name) in names.iter().enumerate() {
        table.set(i as i64 + 1, name.as_str())?;
    }
    Ok(LuaValue::Table(table))
}

// ---------------------------------------------------------------------------
// Animation event bindings
// ---------------------------------------------------------------------------

/// `Engine.add_animation_event(entity_id, clip_name, frame, event_name)` —
/// add a frame-based event to a sprite/instanced animation clip.
fn lua_add_animation_event(
    lua: &Lua,
    (entity_id, clip_name, frame, event_name): (u64, String, u32, String),
) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        // Try SpriteAnimatorComponent first.
        if let Some(mut anim) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            if let Some(clip) = anim.clips.iter_mut().find(|c| c.name == clip_name) {
                clip.events.push(super::animation::AnimationEvent {
                    frame,
                    name: event_name.clone(),
                });
                return true;
            }
        }
        // Fall back to InstancedSpriteAnimator.
        if let Some(mut anim) = scene.get_component_mut::<super::InstancedSpriteAnimator>(entity) {
            if let Some(clip) = anim.clips.iter_mut().find(|c| c.name == clip_name) {
                clip.events.push(super::animation::AnimationEvent {
                    frame,
                    name: event_name,
                });
                return true;
            }
        }
        false
    })
}

/// `Engine.remove_animation_event(entity_id, clip_name, event_name)` —
/// remove all events with the given name from a sprite/instanced animation clip.
fn lua_remove_animation_event(
    lua: &Lua,
    (entity_id, clip_name, event_name): (u64, String, String),
) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        if let Some(mut anim) = scene.get_component_mut::<super::SpriteAnimatorComponent>(entity) {
            if let Some(clip) = anim.clips.iter_mut().find(|c| c.name == clip_name) {
                let before = clip.events.len();
                clip.events.retain(|e| e.name != event_name);
                if clip.events.len() != before {
                    return true;
                }
            }
        }
        if let Some(mut anim) = scene.get_component_mut::<super::InstancedSpriteAnimator>(entity) {
            if let Some(clip) = anim.clips.iter_mut().find(|c| c.name == clip_name) {
                let before = clip.events.len();
                clip.events.retain(|e| e.name != event_name);
                if clip.events.len() != before {
                    return true;
                }
            }
        }
        false
    })
}

/// `Engine.add_skeletal_anim_event(entity_id, clip_name, time, event_name)` —
/// add a time-based event to a skeletal animation clip.
fn lua_add_skeletal_anim_event(
    lua: &Lua,
    (entity_id, clip_name, time, event_name): (u64, String, f32, String),
) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        if let Some(mut sac) =
            scene.get_component_mut::<super::SkeletalAnimationComponent>(entity)
        {
            sac.clip_events
                .entry(clip_name)
                .or_default()
                .push(crate::renderer::skeleton::SkeletalAnimationEvent {
                    time,
                    name: event_name,
                });
            true
        } else {
            false
        }
    })
}

/// `Engine.remove_skeletal_anim_event(entity_id, clip_name, event_name)` —
/// remove all events with the given name from a skeletal animation clip.
fn lua_remove_skeletal_anim_event(
    lua: &Lua,
    (entity_id, clip_name, event_name): (u64, String, String),
) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        if let Some(mut sac) =
            scene.get_component_mut::<super::SkeletalAnimationComponent>(entity)
        {
            if let Some(events) = sac.clip_events.get_mut(&clip_name) {
                let before = events.len();
                events.retain(|e| e.name != event_name);
                return events.len() != before;
            }
        }
        false
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

/// `Engine.fade_in(entity_id, duration_secs)` — play (or resume) with fade from silence.
fn lua_fade_in(lua: &Lua, (entity_id, duration): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.fade_in_entity_sound(entity, duration);
    })
}

/// `Engine.fade_out(entity_id, duration_secs)` — fade to silence and stop.
fn lua_fade_out(lua: &Lua, (entity_id, duration): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.fade_out_entity_sound(entity, duration);
    })
}

/// `Engine.fade_to(entity_id, volume, duration_secs)` — fade to target volume.
fn lua_fade_to(lua: &Lua, (entity_id, volume, duration): (u64, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.fade_to_entity_volume(entity, volume, duration);
    })
}

/// `Engine.set_master_volume(volume)` — set global master volume (0.0–1.0).
fn lua_set_master_volume(lua: &Lua, volume: f32) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| {
        scene.set_master_volume(volume);
    })
}

/// `Engine.get_master_volume()` — get global master volume.
fn lua_get_master_volume(lua: &Lua, _: ()) -> LuaResult<f32> {
    with_scene_mut(lua, 1.0, |scene| scene.get_master_volume())
}

/// `Engine.set_category_volume(category, volume)` — set volume for a sound category.
/// Category: "sfx", "music", "ambient", "voice" (case-insensitive).
fn lua_set_category_volume(lua: &Lua, (cat_str, volume): (String, f32)) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| {
        if let Some(cat) = super::AudioCategory::from_str_loose(&cat_str) {
            scene.set_category_volume(cat, volume);
        } else {
            log::warn!("Unknown audio category '{cat_str}'. Use: sfx, music, ambient, voice.");
        }
    })
}

/// `Engine.get_category_volume(category)` — get volume for a sound category.
fn lua_get_category_volume(lua: &Lua, cat_str: String) -> LuaResult<f32> {
    with_scene_mut(lua, 1.0, |scene| {
        if let Some(cat) = super::AudioCategory::from_str_loose(&cat_str) {
            scene.get_category_volume(cat)
        } else {
            log::warn!("Unknown audio category '{cat_str}'. Use: sfx, music, ambient, voice.");
            1.0
        }
    })
}

/// `Engine.set_hrtf(entity_id, enabled)` — enable/disable HRTF on an entity's audio source.
fn lua_set_hrtf(lua: &Lua, (entity_id, enabled): (u64, bool)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut asc) = scene.get_component_mut::<super::AudioSourceComponent>(entity) {
            asc.hrtf = enabled;
        }
    })
}

/// `Engine.get_hrtf(entity_id)` — get whether HRTF is enabled on an entity's audio source.
fn lua_get_hrtf(lua: &Lua, entity_id: u64) -> LuaResult<bool> {
    with_entity_mut(lua, entity_id, false, |scene, entity| {
        scene
            .get_component::<super::AudioSourceComponent>(entity)
            .map(|asc| asc.hrtf)
            .unwrap_or(false)
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

/// `Engine.raycast_all(origin_x, origin_y, dir_x, dir_y, max_distance, exclude_entity_id)`
///
/// Returns a Lua table of hits, each `{entity_id, x, y, normal_x, normal_y, distance}`,
/// sorted by distance. `exclude_entity_id` is optional.
fn lua_raycast_all(
    lua: &Lua,
    (ox, oy, dx, dy, max_dist, exclude_id): (f32, f32, f32, f32, f32, Option<u64>),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let exclude_entity = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    let hits = scene.raycast_all(
        glam::Vec2::new(ox, oy),
        glam::Vec2::new(dx, dy),
        max_dist,
        exclude_entity,
    );
    let result = lua.create_table()?;
    for (i, (uuid, hx, hy, nx, ny, toi)) in hits.into_iter().enumerate() {
        let hit = lua.create_table()?;
        hit.set("entity_id", uuid as i64)?;
        hit.set("x", hx)?;
        hit.set("y", hy)?;
        hit.set("normal_x", nx)?;
        hit.set("normal_y", ny)?;
        hit.set("distance", toi)?;
        result.set(i + 1, hit)?;
    }
    Ok(result)
}

/// `Engine.apply_torque_impulse(entity_id, torque)`
fn lua_apply_torque_impulse(lua: &Lua, (entity_id, torque): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque_impulse(entity, torque);
    })
}

/// `Engine.apply_torque(entity_id, torque)`
fn lua_apply_torque(lua: &Lua, (entity_id, torque): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque(entity, torque);
    })
}

/// `Engine.set_gravity_scale(entity_id, scale)`
fn lua_set_gravity_scale(lua: &Lua, (entity_id, scale): (u64, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_gravity_scale(entity, scale);
    })
}

/// `Engine.get_gravity_scale(entity_id)` — returns the current gravity scale, or 1.0.
fn lua_get_gravity_scale(lua: &Lua, entity_id: u64) -> LuaResult<f32> {
    with_entity(lua, entity_id, 1.0, |scene, entity| {
        scene.get_gravity_scale(entity).unwrap_or(1.0)
    })
}

/// `Engine.screen_to_world(screen_x, screen_y)` — convert screen pixels to world coords.
///
/// Uses the primary camera's position, zoom, and rotation. Returns `(world_x, world_y)`.
fn lua_screen_to_world(lua: &Lua, (sx, sy): (f64, f64)) -> LuaResult<(f32, f32)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0)),
    };
    let scene = unsafe { ctx.scene() };
    let vw = scene.viewport_width;
    let vh = scene.viewport_height;
    if vw == 0 || vh == 0 {
        return Ok((0.0, 0.0));
    }
    // Find the primary camera's transform + camera component.
    let result = scene
        .get_primary_camera_entity()
        .and_then(|cam_entity| {
            let cam = scene.get_component::<CameraComponent>(cam_entity)?;
            let tf = scene.get_component::<TransformComponent>(cam_entity)?;
            let aspect = vw as f32 / vh as f32;
            let ortho_size = cam.camera.orthographic_size(); // full height
            let half_h = ortho_size * 0.5;
            let half_w = half_h * aspect;
            // Camera-local space (centered on camera).
            let local_x = (sx as f32 / vw as f32 - 0.5) * half_w * 2.0;
            let local_y = (0.5 - sy as f32 / vh as f32) * half_h * 2.0;
            // Rotate by camera Z rotation.
            let euler_z = tf.rotation.to_euler(glam::EulerRot::XYZ).2;
            let (sin, cos) = euler_z.sin_cos();
            let wx = cos * local_x - sin * local_y + tf.translation.x;
            let wy = sin * local_x + cos * local_y + tf.translation.y;
            Some((wx, wy))
        })
        .unwrap_or((0.0, 0.0));
    Ok(result)
}

// ---------------------------------------------------------------------------
// 3D Physics bindings (feature-gated behind "physics-3d")
// ---------------------------------------------------------------------------

#[cfg(feature = "physics-3d")]
/// `Engine.apply_impulse_3d(entity_id, ix, iy, iz)`
fn lua_apply_impulse_3d(lua: &Lua, (entity_id, ix, iy, iz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_impulse_3d(entity, glam::Vec3::new(ix, iy, iz));
    })
}

#[cfg(feature = "physics-3d")]
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

#[cfg(feature = "physics-3d")]
/// `Engine.apply_force_3d(entity_id, fx, fy, fz)`
fn lua_apply_force_3d(lua: &Lua, (entity_id, fx, fy, fz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_force_3d(entity, glam::Vec3::new(fx, fy, fz));
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.apply_torque_impulse_3d(entity_id, tx, ty, tz)`
fn lua_apply_torque_impulse_3d(
    lua: &Lua,
    (entity_id, tx, ty, tz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque_impulse_3d(entity, glam::Vec3::new(tx, ty, tz));
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.apply_torque_3d(entity_id, tx, ty, tz)`
fn lua_apply_torque_3d(lua: &Lua, (entity_id, tx, ty, tz): (u64, f32, f32, f32)) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.apply_torque_3d(entity, glam::Vec3::new(tx, ty, tz));
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.get_linear_velocity_3d(entity_id)` — returns `(vx, vy, vz)`.
fn lua_get_linear_velocity_3d(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_linear_velocity_3d(entity)
            .map(|v| (v.x, v.y, v.z))
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.set_linear_velocity_3d(entity_id, vx, vy, vz)`
fn lua_set_linear_velocity_3d(
    lua: &Lua,
    (entity_id, vx, vy, vz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_linear_velocity_3d(entity, glam::Vec3::new(vx, vy, vz));
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.get_angular_velocity_3d(entity_id)` — returns `(wx, wy, wz)` in rad/s.
fn lua_get_angular_velocity_3d(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32)> {
    with_entity(lua, entity_id, (0.0, 0.0, 0.0), |scene, entity| {
        scene
            .get_angular_velocity_3d(entity)
            .map(|w| (w.x, w.y, w.z))
            .unwrap_or((0.0, 0.0, 0.0))
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.set_angular_velocity_3d(entity_id, wx, wy, wz)`
fn lua_set_angular_velocity_3d(
    lua: &Lua,
    (entity_id, wx, wy, wz): (u64, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_angular_velocity_3d(entity, glam::Vec3::new(wx, wy, wz));
    })
}

#[cfg(feature = "physics-3d")]
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

#[cfg(feature = "physics-3d")]
/// `Engine.set_gravity_3d(x, y, z)`
fn lua_set_gravity_3d(lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.set_gravity_3d(x, y, z))
}

#[cfg(feature = "physics-3d")]
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
// Runtime body type changes
// ---------------------------------------------------------------------------

/// `Engine.set_body_type(entity_id, type_str)` — change 2D body type at runtime.
/// `type_str`: "static", "dynamic", or "kinematic".
fn lua_set_body_type(lua: &Lua, (entity_id, type_str): (u64, String)) -> LuaResult<()> {
    let body_type = match super::RigidBody2DType::from_str_loose(&type_str) {
        Some(bt) => bt,
        None => {
            log::warn!("set_body_type: unknown body type '{}'", type_str);
            return Ok(());
        }
    };
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_body_type(entity, body_type);
    })
}

/// `Engine.get_body_type(entity_id)` — returns "static", "dynamic", or "kinematic".
fn lua_get_body_type(lua: &Lua, entity_id: u64) -> LuaResult<Option<String>> {
    with_entity(lua, entity_id, None, |scene, entity| {
        scene
            .get_body_type(entity)
            .map(|bt| bt.label().to_ascii_lowercase())
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.set_body_type_3d(entity_id, type_str)` — change 3D body type at runtime.
fn lua_set_body_type_3d(lua: &Lua, (entity_id, type_str): (u64, String)) -> LuaResult<()> {
    let body_type = match super::RigidBody3DType::from_str_loose(&type_str) {
        Some(bt) => bt,
        None => {
            log::warn!("set_body_type_3d: unknown body type '{}'", type_str);
            return Ok(());
        }
    };
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        scene.set_body_type_3d(entity, body_type);
    })
}

#[cfg(feature = "physics-3d")]
/// `Engine.get_body_type_3d(entity_id)` — returns "static", "dynamic", or "kinematic".
fn lua_get_body_type_3d(lua: &Lua, entity_id: u64) -> LuaResult<Option<String>> {
    with_entity(lua, entity_id, None, |scene, entity| {
        scene
            .get_body_type_3d(entity)
            .map(|bt| bt.label().to_ascii_lowercase())
    })
}

// ---------------------------------------------------------------------------
// Shape overlap queries
// ---------------------------------------------------------------------------

/// Helper: turn a `Vec<u64>` of entity UUIDs into a Lua table of i64.
fn uuid_vec_to_lua_table(lua: &Lua, uuids: Vec<u64>) -> LuaResult<LuaTable> {
    let table = lua.create_table()?;
    for (i, uuid) in uuids.into_iter().enumerate() {
        table.set(i + 1, uuid as i64)?;
    }
    Ok(table)
}

/// `Engine.point_query(x, y)` — find all entities at a 2D point.
fn lua_point_query(lua: &Lua, (x, y): (f32, f32)) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.point_query(glam::Vec2::new(x, y));
    uuid_vec_to_lua_table(lua, results)
}

/// `Engine.aabb_query(min_x, min_y, max_x, max_y)` — find entities in a 2D AABB.
fn lua_aabb_query(
    lua: &Lua,
    (min_x, min_y, max_x, max_y): (f32, f32, f32, f32),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.aabb_query(glam::Vec2::new(min_x, min_y), glam::Vec2::new(max_x, max_y));
    uuid_vec_to_lua_table(lua, results)
}

/// `Engine.overlap_circle(cx, cy, radius, exclude_entity_id)` — shape overlap test.
fn lua_overlap_circle(
    lua: &Lua,
    (cx, cy, radius, exclude_id): (f32, f32, f32, Option<u64>),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let exclude = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    let results = scene.overlap_circle(glam::Vec2::new(cx, cy), radius, exclude);
    uuid_vec_to_lua_table(lua, results)
}

/// `Engine.overlap_box(cx, cy, half_w, half_h, exclude_entity_id)` — box overlap test.
fn lua_overlap_box(
    lua: &Lua,
    (cx, cy, half_w, half_h, exclude_id): (f32, f32, f32, f32, Option<u64>),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let exclude = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    let results = scene.overlap_box(
        glam::Vec2::new(cx, cy),
        glam::Vec2::new(half_w, half_h),
        exclude,
    );
    uuid_vec_to_lua_table(lua, results)
}

#[cfg(feature = "physics-3d")]
/// `Engine.point_query_3d(x, y, z)` — find all entities at a 3D point.
fn lua_point_query_3d(lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.point_query_3d(glam::Vec3::new(x, y, z));
    uuid_vec_to_lua_table(lua, results)
}

#[cfg(feature = "physics-3d")]
/// `Engine.aabb_query_3d(min_x, min_y, min_z, max_x, max_y, max_z)` — 3D AABB query.
#[allow(clippy::too_many_arguments)]
fn lua_aabb_query_3d(
    lua: &Lua,
    (min_x, min_y, min_z, max_x, max_y, max_z): (f32, f32, f32, f32, f32, f32),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.aabb_query_3d(
        glam::Vec3::new(min_x, min_y, min_z),
        glam::Vec3::new(max_x, max_y, max_z),
    );
    uuid_vec_to_lua_table(lua, results)
}

#[cfg(feature = "physics-3d")]
/// `Engine.overlap_sphere(cx, cy, cz, radius, exclude_entity_id)` — sphere overlap test.
fn lua_overlap_sphere(
    lua: &Lua,
    (cx, cy, cz, radius, exclude_id): (f32, f32, f32, f32, Option<u64>),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let exclude = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    let results = scene.overlap_sphere(glam::Vec3::new(cx, cy, cz), radius, exclude);
    uuid_vec_to_lua_table(lua, results)
}

#[cfg(feature = "physics-3d")]
/// `Engine.overlap_box_3d(cx, cy, cz, hx, hy, hz, exclude_entity_id)` — 3D box overlap.
#[allow(clippy::too_many_arguments)]
fn lua_overlap_box_3d(
    lua: &Lua,
    (cx, cy, cz, hx, hy, hz, exclude_id): (f32, f32, f32, f32, f32, f32, Option<u64>),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let exclude = exclude_id.and_then(|uuid| scene.find_entity_by_uuid(uuid));
    let results = scene.overlap_box_3d(
        glam::Vec3::new(cx, cy, cz),
        glam::Vec3::new(hx, hy, hz),
        exclude,
    );
    uuid_vec_to_lua_table(lua, results)
}

// ---------------------------------------------------------------------------
// Spatial grid queries (non-physics, transform-based)
// ---------------------------------------------------------------------------

/// Helper: convert a `Vec<Entity>` (hecs entities) to a `Vec<u64>` of UUIDs.
fn entities_to_uuids(scene: &Scene, entities: Vec<Entity>) -> Vec<u64> {
    entities
        .into_iter()
        .filter_map(|entity| {
            scene
                .get_component::<super::IdComponent>(entity)
                .map(|id| id.id.raw())
        })
        .collect()
}

/// `Engine.rebuild_spatial_grid(cell_size)` — rebuild the 2D spatial grid.
fn lua_rebuild_spatial_grid(lua: &Lua, cell_size: f32) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| {
        scene.rebuild_spatial_grid(cell_size);
    })
}

/// `Engine.rebuild_spatial_grid_3d(cell_size)` — rebuild the 3D spatial grid.
fn lua_rebuild_spatial_grid_3d(lua: &Lua, cell_size: f32) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| {
        scene.rebuild_spatial_grid_3d(cell_size);
    })
}

/// `Engine.query_region(min_x, min_y, max_x, max_y)` — find entities in a 2D region.
fn lua_query_region(
    lua: &Lua,
    (min_x, min_y, max_x, max_y): (f32, f32, f32, f32),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.query_entities_in_region(
        glam::Vec2::new(min_x, min_y),
        glam::Vec2::new(max_x, max_y),
    );
    let uuids = entities_to_uuids(scene, results);
    uuid_vec_to_lua_table(lua, uuids)
}

/// `Engine.query_radius(x, y, radius)` — find entities within a 2D radius.
fn lua_query_radius(lua: &Lua, (x, y, radius): (f32, f32, f32)) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.query_entities_in_radius(glam::Vec2::new(x, y), radius);
    let uuids = entities_to_uuids(scene, results);
    uuid_vec_to_lua_table(lua, uuids)
}

/// `Engine.query_region_3d(min_x, min_y, min_z, max_x, max_y, max_z)` — 3D region query.
#[allow(clippy::too_many_arguments)]
fn lua_query_region_3d(
    lua: &Lua,
    (min_x, min_y, min_z, max_x, max_y, max_z): (f32, f32, f32, f32, f32, f32),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.query_entities_in_region_3d(
        glam::Vec3::new(min_x, min_y, min_z),
        glam::Vec3::new(max_x, max_y, max_z),
    );
    let uuids = entities_to_uuids(scene, results);
    uuid_vec_to_lua_table(lua, uuids)
}

/// `Engine.query_radius_3d(x, y, z, radius)` — find entities within a 3D radius.
fn lua_query_radius_3d(
    lua: &Lua,
    (x, y, z, radius): (f32, f32, f32, f32),
) -> LuaResult<LuaTable> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return lua.create_table(),
    };
    let scene = unsafe { ctx.scene() };
    let results = scene.query_entities_in_radius_3d(glam::Vec3::new(x, y, z), radius);
    let uuids = entities_to_uuids(scene, results);
    uuid_vec_to_lua_table(lua, uuids)
}

// ---------------------------------------------------------------------------
// Joints (2D)
// ---------------------------------------------------------------------------

/// `Engine.create_revolute_joint(entity_a, entity_b, ax, ay, bx, by)` — hinge joint.
/// Returns joint_id or nil.
fn lua_create_revolute_joint(
    lua: &Lua,
    (entity_a, entity_b, ax, ay, bx, by): (u64, u64, f32, f32, f32, f32),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(entity_a) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(entity_b) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_revolute_joint(ea, eb, glam::Vec2::new(ax, ay), glam::Vec2::new(bx, by))
        .map(|id| id as i64))
}

/// `Engine.create_fixed_joint(entity_a, entity_b, ax, ay, bx, by)` — fixed joint.
fn lua_create_fixed_joint(
    lua: &Lua,
    (entity_a, entity_b, ax, ay, bx, by): (u64, u64, f32, f32, f32, f32),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(entity_a) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(entity_b) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_fixed_joint(ea, eb, glam::Vec2::new(ax, ay), glam::Vec2::new(bx, by))
        .map(|id| id as i64))
}

/// `Engine.create_prismatic_joint(entity_a, entity_b, ax, ay, bx, by, dir_x, dir_y)` — slider.
#[allow(clippy::too_many_arguments)]
fn lua_create_prismatic_joint(
    lua: &Lua,
    (entity_a, entity_b, ax, ay, bx, by, dx, dy): (u64, u64, f32, f32, f32, f32, f32, f32),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(entity_a) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(entity_b) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_prismatic_joint(
            ea,
            eb,
            glam::Vec2::new(ax, ay),
            glam::Vec2::new(bx, by),
            glam::Vec2::new(dx, dy),
        )
        .map(|id| id as i64))
}

/// `Engine.remove_joint(joint_id)` — remove a 2D joint.
fn lua_remove_joint(lua: &Lua, joint_id: i64) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.remove_joint(joint_id as u64))
}

// ---------------------------------------------------------------------------
// Joints (3D) — feature-gated behind "physics-3d"
// ---------------------------------------------------------------------------

#[cfg(feature = "physics-3d")]
/// `Engine.create_revolute_joint_3d(ea, eb, ax, ay, az, bx, by, bz, axis_x, axis_y, axis_z)`
#[allow(clippy::too_many_arguments)]
fn lua_create_revolute_joint_3d(
    lua: &Lua,
    (ea_id, eb_id, ax, ay, az, bx, by, bz, dx, dy, dz): (
        u64,
        u64,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    ),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(ea_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(eb_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_revolute_joint_3d(
            ea,
            eb,
            glam::Vec3::new(ax, ay, az),
            glam::Vec3::new(bx, by, bz),
            glam::Vec3::new(dx, dy, dz),
        )
        .map(|id| id as i64))
}

#[cfg(feature = "physics-3d")]
/// `Engine.create_fixed_joint_3d(ea, eb, ax, ay, az, bx, by, bz)`
#[allow(clippy::too_many_arguments)]
fn lua_create_fixed_joint_3d(
    lua: &Lua,
    (ea_id, eb_id, ax, ay, az, bx, by, bz): (u64, u64, f32, f32, f32, f32, f32, f32),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(ea_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(eb_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_fixed_joint_3d(
            ea,
            eb,
            glam::Vec3::new(ax, ay, az),
            glam::Vec3::new(bx, by, bz),
        )
        .map(|id| id as i64))
}

#[cfg(feature = "physics-3d")]
/// `Engine.create_ball_joint_3d(ea, eb, ax, ay, az, bx, by, bz)`
#[allow(clippy::too_many_arguments)]
fn lua_create_ball_joint_3d(
    lua: &Lua,
    (ea_id, eb_id, ax, ay, az, bx, by, bz): (u64, u64, f32, f32, f32, f32, f32, f32),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(ea_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(eb_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_ball_joint_3d(
            ea,
            eb,
            glam::Vec3::new(ax, ay, az),
            glam::Vec3::new(bx, by, bz),
        )
        .map(|id| id as i64))
}

#[cfg(feature = "physics-3d")]
/// `Engine.create_prismatic_joint_3d(ea, eb, ax, ay, az, bx, by, bz, dx, dy, dz)`
#[allow(clippy::too_many_arguments)]
fn lua_create_prismatic_joint_3d(
    lua: &Lua,
    (ea_id, eb_id, ax, ay, az, bx, by, bz, dx, dy, dz): (
        u64,
        u64,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    ),
) -> LuaResult<Option<i64>> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene_mut() };
    let ea = match scene.find_entity_by_uuid(ea_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    let eb = match scene.find_entity_by_uuid(eb_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    Ok(scene
        .create_prismatic_joint_3d(
            ea,
            eb,
            glam::Vec3::new(ax, ay, az),
            glam::Vec3::new(bx, by, bz),
            glam::Vec3::new(dx, dy, dz),
        )
        .map(|id| id as i64))
}

#[cfg(feature = "physics-3d")]
/// `Engine.remove_joint_3d(joint_id)` — remove a 3D joint.
fn lua_remove_joint_3d(lua: &Lua, joint_id: i64) -> LuaResult<()> {
    with_scene_mut(lua, (), |scene| scene.remove_joint_3d(joint_id as u64))
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

/// `Engine.vector_length(x, y, z)` → scalar length.
fn vector_length(_lua: &Lua, (x, y, z): (f32, f32, f32)) -> LuaResult<f32> {
    Ok(glam::Vec3::new(x, y, z).length())
}

/// `Engine.distance(x1, y1, z1, x2, y2, z2)` → Euclidean distance between two 3D points.
fn lua_distance(
    _lua: &Lua,
    (x1, y1, z1, x2, y2, z2): (f32, f32, f32, f32, f32, f32),
) -> LuaResult<f32> {
    Ok(glam::Vec3::new(x1, y1, z1).distance(glam::Vec3::new(x2, y2, z2)))
}

/// `Engine.distance_2d(x1, y1, x2, y2)` → Euclidean distance between two 2D points.
fn lua_distance_2d(_lua: &Lua, (x1, y1, x2, y2): (f32, f32, f32, f32)) -> LuaResult<f32> {
    Ok(glam::Vec2::new(x1, y1).distance(glam::Vec2::new(x2, y2)))
}

/// `Engine.lerp(a, b, t)` → linear interpolation between two scalars.
fn lua_lerp(_lua: &Lua, (a, b, t): (f32, f32, f32)) -> LuaResult<f32> {
    Ok(a + (b - a) * t)
}

/// `Engine.lerp_vec3(x1, y1, z1, x2, y2, z2, t)` → component-wise lerp of two 3D vectors.
fn lua_lerp_vec3(
    _lua: &Lua,
    (x1, y1, z1, x2, y2, z2, t): (f32, f32, f32, f32, f32, f32, f32),
) -> LuaResult<(f32, f32, f32)> {
    let v = glam::Vec3::new(x1, y1, z1).lerp(glam::Vec3::new(x2, y2, z2), t);
    Ok((v.x, v.y, v.z))
}

/// `Engine.slerp(x1, y1, z1, w1, x2, y2, z2, w2, t)` → spherical interpolation of two quaternions.
/// Returns `(x, y, z, w)`.
fn lua_slerp(
    _lua: &Lua,
    (x1, y1, z1, w1, x2, y2, z2, w2, t): (f32, f32, f32, f32, f32, f32, f32, f32, f32),
) -> LuaResult<(f32, f32, f32, f32)> {
    let q = glam::Quat::from_xyzw(x1, y1, z1, w1)
        .normalize()
        .slerp(glam::Quat::from_xyzw(x2, y2, z2, w2).normalize(), t);
    Ok((q.x, q.y, q.z, q.w))
}

/// `Engine.clamp(value, min, max)` → clamped scalar.
fn lua_clamp(_lua: &Lua, (value, min, max): (f32, f32, f32)) -> LuaResult<f32> {
    Ok(value.clamp(min, max))
}

/// `Engine.move_toward(current, target, max_delta)` → scalar moved toward target by at most max_delta.
fn lua_move_toward(_lua: &Lua, (current, target, max_delta): (f32, f32, f32)) -> LuaResult<f32> {
    let diff = target - current;
    if diff.abs() <= max_delta {
        Ok(target)
    } else {
        Ok(current + diff.signum() * max_delta)
    }
}

/// `Engine.move_toward_vec3(x1, y1, z1, x2, y2, z2, max_delta)` → 3D point moved toward target.
fn lua_move_toward_vec3(
    _lua: &Lua,
    (x1, y1, z1, x2, y2, z2, max_delta): (f32, f32, f32, f32, f32, f32, f32),
) -> LuaResult<(f32, f32, f32)> {
    let from = glam::Vec3::new(x1, y1, z1);
    let to = glam::Vec3::new(x2, y2, z2);
    let diff = to - from;
    let dist = diff.length();
    if dist <= max_delta || dist < f32::EPSILON {
        Ok((to.x, to.y, to.z))
    } else {
        let v = from + diff / dist * max_delta;
        Ok((v.x, v.y, v.z))
    }
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
// Coroutines
// ---------------------------------------------------------------------------

/// `Engine.start_coroutine(fn)` — start a coroutine that runs `fn`.
///
/// The function can call `Engine.wait(seconds)` or `coroutine.yield()` to
/// pause execution. It resumes automatically on subsequent frames.
fn lua_start_coroutine(lua: &Lua, func: LuaFunction) -> LuaResult<()> {
    use super::script_engine::{CurrentEntityUuid, PendingCoroutineCreate, PendingCoroutineOps};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);

    // Create a Lua coroutine thread from the function.
    let thread = lua.create_thread(func)?;
    let thread_key = lua.create_registry_value(thread)?;

    let mut ops = lua
        .app_data_mut::<PendingCoroutineOps>()
        .ok_or_else(|| mlua::Error::RuntimeError("Coroutine system not available".into()))?;
    ops.creates.push(PendingCoroutineCreate {
        entity_uuid,
        thread_key,
    });
    Ok(())
}

/// `Engine.stop_all_coroutines()` — cancel all coroutines for the calling entity.
fn lua_stop_all_coroutines(lua: &Lua, (): ()) -> LuaResult<()> {
    use super::script_engine::{CurrentEntityUuid, PendingCoroutineOps};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);

    if let Some(mut ops) = lua.app_data_mut::<PendingCoroutineOps>() {
        ops.cancels_entity.push(entity_uuid);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

/// `Engine.emit(event_name, data?)` — broadcast an event to all listeners.
/// `data` is an optional Lua value (typically a table) passed to listener callbacks.
fn lua_emit(lua: &Lua, (event_name, data): (String, Option<LuaValue>)) -> LuaResult<()> {
    use super::script_engine::{PendingEmit, PendingEventBusOps};

    let data_key = match data {
        Some(val) if !val.is_nil() => Some(lua.create_registry_value(val)?),
        _ => None,
    };

    let mut ops = lua
        .app_data_mut::<PendingEventBusOps>()
        .ok_or_else(|| mlua::Error::RuntimeError("Event bus not available".into()))?;

    ops.emits.push(PendingEmit {
        event_name,
        data_key,
    });

    Ok(())
}

/// `Engine.on(event_name, callback)` — register a listener for an event.
/// Tied to the calling entity. One listener per entity per event (last wins).
fn lua_on(lua: &Lua, (event_name, callback): (String, LuaFunction)) -> LuaResult<()> {
    use super::script_engine::{CurrentEntityUuid, PendingEventBusOps, PendingEventRegister};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);

    let callback_key = lua.create_registry_value(callback)?;

    let mut ops = lua
        .app_data_mut::<PendingEventBusOps>()
        .ok_or_else(|| mlua::Error::RuntimeError("Event bus not available".into()))?;

    ops.registers.push(PendingEventRegister {
        event_name,
        entity_uuid,
        callback_key,
    });

    Ok(())
}

/// `Engine.off(event_name)` — unregister listener for an event from calling entity.
fn lua_off(lua: &Lua, event_name: String) -> LuaResult<()> {
    use super::script_engine::{CurrentEntityUuid, PendingEventBusOps};

    let entity_uuid = lua
        .app_data_ref::<CurrentEntityUuid>()
        .map(|u| u.0)
        .unwrap_or(0);

    if let Some(mut ops) = lua.app_data_mut::<PendingEventBusOps>() {
        ops.unregisters.push((event_name, entity_uuid));
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
        "InstancedSpriteAnimator" => collect_uuids!(super::InstancedSpriteAnimator),
        "AnimationController" => collect_uuids!(super::AnimationControllerComponent),
        #[cfg(feature = "physics-3d")]
        "RigidBody3D" => collect_uuids!(super::RigidBody3DComponent),
        #[cfg(feature = "physics-3d")]
        "BoxCollider3D" => collect_uuids!(super::BoxCollider3DComponent),
        #[cfg(feature = "physics-3d")]
        "SphereCollider3D" => collect_uuids!(super::SphereCollider3DComponent),
        #[cfg(feature = "physics-3d")]
        "CapsuleCollider3D" => collect_uuids!(super::CapsuleCollider3DComponent),
        "MeshRenderer" => collect_uuids!(super::MeshRendererComponent),
        "DirectionalLight" => collect_uuids!(super::DirectionalLightComponent),
        "PointLight" => collect_uuids!(super::PointLightComponent),
        "AmbientLight" => collect_uuids!(super::AmbientLightComponent),
        "UIAnchor" => collect_uuids!(super::UIAnchorComponent),
        "UIRect" => collect_uuids!(super::UIRectComponent),
        "UIImage" => collect_uuids!(super::UIImageComponent),
        "UIInteractable" => collect_uuids!(super::UIInteractableComponent),
        "UILayout" => collect_uuids!(super::UILayoutComponent),
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
        table.set(i + 1, *uuid as i64)?;
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
// Cursor
// ---------------------------------------------------------------------------

/// `Engine.set_cursor_mode(mode)` — set the cursor mode.
///
/// `mode` is a string: `"normal"`, `"confined"`, or `"locked"`.
fn lua_set_cursor_mode(lua: &Lua, mode_str: String) -> LuaResult<()> {
    let mode = match mode_str.as_str() {
        "normal" => crate::cursor::CursorMode::Normal,
        "confined" => crate::cursor::CursorMode::Confined,
        "locked" => crate::cursor::CursorMode::Locked,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "Invalid cursor mode '{}'. Expected 'normal', 'confined', or 'locked'.",
                mode_str
            )));
        }
    };
    with_scene_mut(lua, (), |scene| scene.set_cursor_mode(mode))
}

/// `Engine.get_cursor_mode()` → string (`"normal"`, `"confined"`, or `"locked"`).
fn lua_get_cursor_mode(lua: &Lua, _: ()) -> LuaResult<String> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok("normal".to_string()),
    };
    let scene = unsafe { ctx.scene() };
    let s = match scene.cursor_mode() {
        crate::cursor::CursorMode::Normal => "normal",
        crate::cursor::CursorMode::Confined => "confined",
        crate::cursor::CursorMode::Locked => "locked",
    };
    Ok(s.to_string())
}

// ---------------------------------------------------------------------------
// Window size
// ---------------------------------------------------------------------------

/// `Engine.set_window_size(w, h)` — request a window resize (physical pixels).
fn lua_set_window_size(lua: &Lua, (w, h): (u32, u32)) -> LuaResult<()> {
    if w < 320 || h < 240 {
        return Err(mlua::Error::runtime("Window size too small (min 320x240)"));
    }
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_window_size(w, h);
    Ok(())
}

/// `Engine.get_window_size()` → `(width, height)` in physical pixels.
fn lua_get_window_size(lua: &Lua, _: ()) -> LuaResult<(u32, u32)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0, 0)),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.viewport_size())
}

// ---------------------------------------------------------------------------
// Text color
// ---------------------------------------------------------------------------

/// `Engine.get_text_color(entity_id)` → `(r, g, b, a)`.
fn lua_get_text_color(lua: &Lua, entity_id: u64) -> LuaResult<(f32, f32, f32, f32)> {
    with_entity(lua, entity_id, (1.0, 1.0, 1.0, 1.0), |scene, entity| {
        scene
            .get_component::<super::TextComponent>(entity)
            .map(|tc| (tc.color.x, tc.color.y, tc.color.z, tc.color.w))
            .unwrap_or((1.0, 1.0, 1.0, 1.0))
    })
}

/// `Engine.set_text_color(entity_id, r, g, b, a)` — set text component color.
fn lua_set_text_color(
    lua: &Lua,
    (entity_id, r, g, b, a): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    with_entity_mut(lua, entity_id, (), |scene, entity| {
        if let Some(mut tc) = scene.get_component_mut::<super::TextComponent>(entity) {
            tc.color = glam::Vec4::new(r, g, b, a);
        }
    })
}

// ---------------------------------------------------------------------------
// Runtime settings
// ---------------------------------------------------------------------------

/// `Engine.get_vsync()` → `bool`.
fn lua_get_vsync(lua: &Lua, _: ()) -> LuaResult<bool> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.vsync_enabled())
}

/// `Engine.set_vsync(enabled)` — request VSync on (`true` → Fifo) or off (`false` → Mailbox).
fn lua_set_vsync(lua: &Lua, enabled: bool) -> LuaResult<()> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_vsync(enabled);
    Ok(())
}

/// `Engine.get_fullscreen()` → `"windowed"`, `"borderless"`, or `"exclusive"`.
fn lua_get_fullscreen(lua: &Lua, _: ()) -> LuaResult<String> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok("windowed".to_string()),
    };
    let scene = unsafe { ctx.scene() };
    Ok(match scene.fullscreen_mode() {
        super::FullscreenMode::Windowed => "windowed",
        super::FullscreenMode::Borderless => "borderless",
        super::FullscreenMode::Exclusive => "exclusive",
    }
    .to_string())
}

/// `Engine.set_fullscreen(mode)` — `"windowed"`, `"borderless"`, or `"exclusive"`.
fn lua_set_fullscreen(lua: &Lua, mode_str: String) -> LuaResult<()> {
    let mode = match mode_str.as_str() {
        "windowed" => super::FullscreenMode::Windowed,
        "borderless" => super::FullscreenMode::Borderless,
        "exclusive" => super::FullscreenMode::Exclusive,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "Invalid fullscreen mode '{}'. Expected 'windowed', 'borderless', or 'exclusive'.",
                mode_str
            )));
        }
    };
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_fullscreen(mode);
    Ok(())
}

/// `Engine.get_shadow_quality()` → `int` (0=Low, 1=Medium, 2=High, 3=Ultra).
fn lua_get_shadow_quality(lua: &Lua, _: ()) -> LuaResult<i32> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(3),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.shadow_quality())
}

/// `Engine.set_shadow_quality(level)` — 0=Low, 1=Medium, 2=High, 3=Ultra.
fn lua_set_shadow_quality(lua: &Lua, quality: i32) -> LuaResult<()> {
    if !(0..=3).contains(&quality) {
        return Err(mlua::Error::runtime(format!(
            "Shadow quality must be 0–3, got {}",
            quality
        )));
    }
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_shadow_quality(quality);
    Ok(())
}

/// `Engine.quit()` — request application exit.
fn lua_quit(lua: &Lua, _: ()) -> LuaResult<()> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_quit();
    Ok(())
}

/// `Engine.load_scene(path)` — request scene transition. Deferred to next frame.
fn lua_load_scene(lua: &Lua, path: String) -> LuaResult<()> {
    if path.is_empty() {
        return Err(mlua::Error::runtime("Scene path cannot be empty"));
    }
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.request_load_scene(path);
    Ok(())
}

/// `Engine.get_gui_scale()` → `number` (default 1.0).
fn lua_get_gui_scale(lua: &Lua, _: ()) -> LuaResult<f32> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(1.0),
    };
    let scene = unsafe { ctx.scene() };
    Ok(scene.gui_scale())
}

/// `Engine.set_gui_scale(scale)` — clamped to 0.5–2.0.
fn lua_set_gui_scale(lua: &Lua, scale: f32) -> LuaResult<()> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.set_gui_scale(scale);
    Ok(())
}

// ---------------------------------------------------------------------------
// Loading screen
// ---------------------------------------------------------------------------

/// `Engine.set_loading_screen_color(r, g, b)` — set the background color shown
/// during scene transitions. RGB values are 0.0–1.0. Alpha is always 1.0.
fn lua_set_loading_screen_color(lua: &Lua, (r, g, b): (f32, f32, f32)) -> LuaResult<()> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene() };
    scene.set_loading_screen_color([r, g, b, 1.0]);
    Ok(())
}

/// `Engine.get_loading_screen_color()` → (r, g, b).
fn lua_get_loading_screen_color(lua: &Lua, _: ()) -> LuaResult<(f32, f32, f32)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((0.0, 0.0, 0.0)),
    };
    let scene = unsafe { ctx.scene() };
    let c = scene.loading_screen_color();
    Ok((c[0], c[1], c[2]))
}

// ---------------------------------------------------------------------------
// UI Anchor
// ---------------------------------------------------------------------------

/// `Engine.set_ui_anchor(entity_id, anchor_x, anchor_y, offset_x, offset_y)`.
///
/// Adds or updates a UIAnchorComponent on the entity. Anchor is 0-1 normalized
/// ((0,0) = top-left, (1,1) = bottom-right). Offset is in world units.
fn lua_set_ui_anchor(
    lua: &Lua,
    (entity_id, ax, ay, ox, oy): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if scene.has_component::<super::UIAnchorComponent>(entity) {
        if let Some(mut ua) = scene.get_component_mut::<super::UIAnchorComponent>(entity) {
            ua.anchor = glam::Vec2::new(ax, ay);
            ua.offset = glam::Vec2::new(ox, oy);
        }
    } else {
        scene.add_component(
            entity,
            super::UIAnchorComponent {
                anchor: glam::Vec2::new(ax, ay),
                offset: glam::Vec2::new(ox, oy),
            },
        );
    }
    Ok(())
}

/// `Engine.get_ui_anchor(entity_id)` → `(anchor_x, anchor_y, offset_x, offset_y)` or `(nil)`.
#[allow(clippy::type_complexity)]
fn lua_get_ui_anchor(
    lua: &Lua,
    entity_id: u64,
) -> LuaResult<(Option<f32>, Option<f32>, Option<f32>, Option<f32>)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((None, None, None, None)),
    };
    let scene = unsafe { ctx.scene() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok((None, None, None, None)),
    };
    let ua = match scene.get_component::<super::UIAnchorComponent>(entity) {
        Some(ua) => ua,
        None => return Ok((None, None, None, None)),
    };
    Ok((
        Some(ua.anchor.x),
        Some(ua.anchor.y),
        Some(ua.offset.x),
        Some(ua.offset.y),
    ))
}

// ---------------------------------------------------------------------------
// UI Rect / Image / Interactable
// ---------------------------------------------------------------------------

/// `Engine.set_ui_rect(entity_id, width, height)`.
fn lua_set_ui_rect(lua: &Lua, (entity_id, w, h): (u64, f32, f32)) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if scene.has_component::<super::UIRectComponent>(entity) {
        if let Some(mut rect) = scene.get_component_mut::<super::UIRectComponent>(entity) {
            rect.size = glam::Vec2::new(w, h);
        }
    } else {
        scene.add_component(
            entity,
            super::UIRectComponent {
                size: glam::Vec2::new(w, h),
                ..Default::default()
            },
        );
    }
    Ok(())
}

/// `Engine.get_ui_rect(entity_id)` → `(width, height)` or `(nil)`.
fn lua_get_ui_rect(lua: &Lua, entity_id: u64) -> LuaResult<(Option<f32>, Option<f32>)> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok((None, None)),
    };
    let scene = unsafe { ctx.scene() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok((None, None)),
    };
    let result = match scene.get_component::<super::UIRectComponent>(entity) {
        Some(r) => (Some(r.size.x), Some(r.size.y)),
        None => (None, None),
    };
    Ok(result)
}

/// `Engine.set_ui_pivot(entity_id, px, py)`.
fn lua_set_ui_pivot(lua: &Lua, (entity_id, px, py): (u64, f32, f32)) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if let Some(mut rect) = scene.get_component_mut::<super::UIRectComponent>(entity) {
        rect.pivot = glam::Vec2::new(px, py);
    }
    Ok(())
}

/// `Engine.set_ui_image(entity_id, texture_handle)` — 0 to clear texture.
fn lua_set_ui_image(lua: &Lua, (entity_id, tex_handle): (u64, u64)) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if scene.has_component::<super::UIImageComponent>(entity) {
        if let Some(mut img) = scene.get_component_mut::<super::UIImageComponent>(entity) {
            img.texture_handle = crate::uuid::Uuid::from_raw(tex_handle);
            img.texture = None;
        }
    } else {
        scene.add_component(
            entity,
            super::UIImageComponent {
                texture_handle: crate::uuid::Uuid::from_raw(tex_handle),
                ..Default::default()
            },
        );
    }
    scene.core.textures_all_resolved = false;
    Ok(())
}

/// `Engine.set_ui_image_color(entity_id, r, g, b, a)`.
fn lua_set_ui_image_color(
    lua: &Lua,
    (entity_id, r, g, b, a): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if let Some(mut img) = scene.get_component_mut::<super::UIImageComponent>(entity) {
        img.color = glam::Vec4::new(r, g, b, a);
    }
    Ok(())
}

/// `Engine.set_ui_border(entity_id, left, right, top, bottom)`.
fn lua_set_ui_border(
    lua: &Lua,
    (entity_id, left, right, top, bottom): (u64, f32, f32, f32, f32),
) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if let Some(mut img) = scene.get_component_mut::<super::UIImageComponent>(entity) {
        img.border = [left, right, top, bottom];
    }
    Ok(())
}

/// `Engine.set_ui_interactable(entity_id, enabled)`.
fn lua_set_ui_interactable(lua: &Lua, (entity_id, enabled): (u64, bool)) -> LuaResult<()> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(()),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(()),
    };
    if scene.has_component::<super::UIInteractableComponent>(entity) {
        if let Some(mut inter) = scene.get_component_mut::<super::UIInteractableComponent>(entity) {
            inter.interactable = enabled;
        }
    } else {
        scene.add_component(
            entity,
            super::UIInteractableComponent {
                interactable: enabled,
                ..Default::default()
            },
        );
    }
    Ok(())
}

/// `Engine.get_ui_state(entity_id)` → `"normal"|"hovered"|"pressed"|"disabled"` or `nil`.
fn lua_get_ui_state(lua: &Lua, entity_id: u64) -> LuaResult<Option<String>> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    let result = scene
        .get_component::<super::UIInteractableComponent>(entity)
        .map(|inter| {
            match inter.state {
                super::UIInteractionState::Normal => "normal",
                super::UIInteractionState::Hovered => "hovered",
                super::UIInteractionState::Pressed => "pressed",
                super::UIInteractionState::Disabled => "disabled",
            }
            .to_string()
        });
    Ok(result)
}

/// `Engine.create_ui_entity(name, width, height, anchor_x, anchor_y)` → entity_id.
fn lua_create_ui_entity(
    lua: &Lua,
    (name, width, height, ax, ay): (String, f32, f32, f32, f32),
) -> LuaResult<u64> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(0),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = scene.create_entity_with_tag(&name);
    let uuid = scene
        .get_component::<super::IdComponent>(entity)
        .map(|id| id.id.raw())
        .unwrap_or(0);
    scene.add_component(
        entity,
        super::UIAnchorComponent {
            anchor: glam::Vec2::new(ax, ay),
            offset: glam::Vec2::ZERO,
        },
    );
    scene.add_component(
        entity,
        super::UIRectComponent {
            size: glam::Vec2::new(width, height),
            ..Default::default()
        },
    );
    Ok(uuid)
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
// Component manipulation
// ---------------------------------------------------------------------------

/// Extract an f32 from a LuaValue (supports Integer and Number).
fn lua_value_as_f32(v: &LuaValue) -> Option<f32> {
    match v {
        LuaValue::Integer(n) => Some(*n as f32),
        LuaValue::Number(n) => Some(*n as f32),
        _ => None,
    }
}

/// `Engine.add_component(entity_id, component_name, ...)` — add a component to an entity at runtime.
///
/// Supported components and their arguments:
/// - `"SpriteRenderer"` — no extra args (default white sprite)
/// - `"CircleRenderer"` — no extra args (default white circle)
/// - `"Text", text_string` — creates text component with given string
/// - `"AudioSource"` — no extra args (empty audio source)
/// - `"AudioListener"` — no extra args
/// - `"ParticleEmitter"` — no extra args (default emitter)
/// - `"UIAnchor", ax, ay, ox, oy` — creates UI anchor
/// - `"Camera"` — no extra args (default camera)
/// - `"RigidBody2D"` — no extra args (default dynamic body)
/// - `"BoxCollider2D"` — no extra args (default 1x1 box)
/// - `"CircleCollider2D"` — no extra args (default radius 0.5)
fn lua_add_component(lua: &Lua, args: mlua::Variadic<LuaValue>) -> LuaResult<bool> {
    if args.len() < 2 {
        return Err(mlua::Error::RuntimeError(
            "add_component requires at least (entity_id, component_name)".into(),
        ));
    }
    let entity_id = match &args[0] {
        LuaValue::Integer(n) => *n as u64,
        LuaValue::Number(n) => *n as u64,
        _ => {
            return Err(mlua::Error::RuntimeError(
                "entity_id must be a number".into(),
            ))
        }
    };
    let name = match &args[1] {
        LuaValue::String(s) => s.to_str().map(|s| s.to_string()).unwrap_or_default(),
        _ => {
            return Err(mlua::Error::RuntimeError(
                "component_name must be a string".into(),
            ))
        }
    };

    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(false),
    };

    match name.as_str() {
        "SpriteRenderer" => {
            if !scene.has_component::<super::SpriteRendererComponent>(entity) {
                scene.add_component(entity, super::SpriteRendererComponent::default());
            }
        }
        "CircleRenderer" => {
            if !scene.has_component::<super::CircleRendererComponent>(entity) {
                scene.add_component(entity, super::CircleRendererComponent::default());
            }
        }
        "Text" => {
            if !scene.has_component::<super::TextComponent>(entity) {
                let text = args
                    .get(2)
                    .and_then(|v| match v {
                        LuaValue::String(s) => s.to_str().ok().map(|s| s.to_string()),
                        _ => None,
                    })
                    .unwrap_or_default();
                scene.add_component(
                    entity,
                    super::TextComponent {
                        text,
                        ..Default::default()
                    },
                );
            }
        }
        "AudioSource" => {
            if !scene.has_component::<super::AudioSourceComponent>(entity) {
                scene.add_component(entity, super::AudioSourceComponent::default());
            }
        }
        "AudioListener" => {
            if !scene.has_component::<super::AudioListenerComponent>(entity) {
                scene.add_component(entity, super::AudioListenerComponent::default());
            }
        }
        "ParticleEmitter" => {
            if !scene.has_component::<super::ParticleEmitterComponent>(entity) {
                scene.add_component(entity, super::ParticleEmitterComponent::default());
            }
        }
        "UIAnchor" => {
            let ax = args.get(2).and_then(lua_value_as_f32).unwrap_or(0.0);
            let ay = args.get(3).and_then(lua_value_as_f32).unwrap_or(0.0);
            let ox = args.get(4).and_then(lua_value_as_f32).unwrap_or(0.0);
            let oy = args.get(5).and_then(lua_value_as_f32).unwrap_or(0.0);
            if scene.has_component::<super::UIAnchorComponent>(entity) {
                if let Some(mut ua) = scene.get_component_mut::<super::UIAnchorComponent>(entity) {
                    ua.anchor = glam::Vec2::new(ax, ay);
                    ua.offset = glam::Vec2::new(ox, oy);
                }
            } else {
                scene.add_component(
                    entity,
                    super::UIAnchorComponent {
                        anchor: glam::Vec2::new(ax, ay),
                        offset: glam::Vec2::new(ox, oy),
                    },
                );
            }
        }
        "UIRect" => {
            let w = args.get(2).and_then(lua_value_as_f32).unwrap_or(100.0);
            let h = args.get(3).and_then(lua_value_as_f32).unwrap_or(100.0);
            if !scene.has_component::<super::UIRectComponent>(entity) {
                scene.add_component(
                    entity,
                    super::UIRectComponent {
                        size: glam::Vec2::new(w, h),
                        ..Default::default()
                    },
                );
            }
        }
        "UIImage" => {
            if !scene.has_component::<super::UIImageComponent>(entity) {
                scene.add_component(entity, super::UIImageComponent::default());
            }
        }
        "UIInteractable" => {
            if !scene.has_component::<super::UIInteractableComponent>(entity) {
                scene.add_component(entity, super::UIInteractableComponent::default());
            }
        }
        "UILayout" => {
            if !scene.has_component::<super::UILayoutComponent>(entity) {
                scene.add_component(entity, super::UILayoutComponent::default());
            }
        }
        "Camera" => {
            if !scene.has_component::<super::CameraComponent>(entity) {
                scene.add_component(entity, super::CameraComponent::default());
            }
        }
        "RigidBody2D" => {
            if !scene.has_component::<super::RigidBody2DComponent>(entity) {
                scene.add_component(entity, super::RigidBody2DComponent::default());
            }
        }
        "BoxCollider2D" => {
            if !scene.has_component::<super::BoxCollider2DComponent>(entity) {
                scene.add_component(entity, super::BoxCollider2DComponent::default());
            }
        }
        "CircleCollider2D" => {
            if !scene.has_component::<super::CircleCollider2DComponent>(entity) {
                scene.add_component(entity, super::CircleCollider2DComponent::default());
            }
        }
        "SpriteAnimator" => {
            if !scene.has_component::<super::SpriteAnimatorComponent>(entity) {
                scene.add_component(entity, super::SpriteAnimatorComponent::default());
            }
        }
        _ => {
            log::warn!(
                "ScriptGlue: add_component: unsupported component '{}'",
                name
            );
            return Ok(false);
        }
    }

    Ok(true)
}

/// `Engine.remove_component(entity_id, component_name)` — remove a component from an entity at runtime.
fn lua_remove_component(lua: &Lua, (entity_id, name): (u64, String)) -> LuaResult<bool> {
    let mut ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(false),
    };
    let scene = unsafe { ctx.scene_mut() };
    let entity = match scene.find_entity_by_uuid(entity_id) {
        Some(e) => e,
        None => return Ok(false),
    };

    let removed = match name.as_str() {
        "SpriteRenderer" => scene
            .remove_component::<super::SpriteRendererComponent>(entity)
            .is_some(),
        "CircleRenderer" => scene
            .remove_component::<super::CircleRendererComponent>(entity)
            .is_some(),
        "Text" => scene
            .remove_component::<super::TextComponent>(entity)
            .is_some(),
        "AudioSource" => scene
            .remove_component::<super::AudioSourceComponent>(entity)
            .is_some(),
        "AudioListener" => scene
            .remove_component::<super::AudioListenerComponent>(entity)
            .is_some(),
        "ParticleEmitter" => scene
            .remove_component::<super::ParticleEmitterComponent>(entity)
            .is_some(),
        "UIAnchor" => scene
            .remove_component::<super::UIAnchorComponent>(entity)
            .is_some(),
        "UIRect" => scene
            .remove_component::<super::UIRectComponent>(entity)
            .is_some(),
        "UIImage" => scene
            .remove_component::<super::UIImageComponent>(entity)
            .is_some(),
        "UIInteractable" => scene
            .remove_component::<super::UIInteractableComponent>(entity)
            .is_some(),
        "UILayout" => scene
            .remove_component::<super::UILayoutComponent>(entity)
            .is_some(),
        "Camera" => scene
            .remove_component::<super::CameraComponent>(entity)
            .is_some(),
        "SpriteAnimator" => scene
            .remove_component::<super::SpriteAnimatorComponent>(entity)
            .is_some(),
        "RigidBody2D" => scene
            .remove_component::<super::RigidBody2DComponent>(entity)
            .is_some(),
        "BoxCollider2D" => scene
            .remove_component::<super::BoxCollider2DComponent>(entity)
            .is_some(),
        "CircleCollider2D" => scene
            .remove_component::<super::CircleCollider2DComponent>(entity)
            .is_some(),
        _ => {
            log::warn!(
                "ScriptGlue: remove_component: unsupported component '{}'",
                name
            );
            false
        }
    };

    Ok(removed)
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

// ---------------------------------------------------------------------------
// Save/load game data — Lua table ↔ JSON file persistence
// ---------------------------------------------------------------------------

/// Validate a save slot name: non-empty, no path separators, no `.`.
fn validate_slot_name(name: &str) -> LuaResult<()> {
    if name.is_empty() {
        return Err(mlua::Error::runtime("Save slot name cannot be empty"));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(mlua::Error::runtime(
            "Save slot name must not contain path separators or '..'",
        ));
    }
    Ok(())
}

/// Resolve the save file path for a given slot. Returns `None` if save directory is not configured.
fn resolve_save_path(lua: &Lua, slot: &str) -> LuaResult<Option<std::path::PathBuf>> {
    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let scene = unsafe { ctx.scene() };
    match scene.save_data_directory() {
        Some(dir) => Ok(Some(dir.join(format!("{}.json", slot)))),
        None => Ok(None),
    }
}

/// Convert a Lua value to a `serde_json::Value`.
fn lua_value_to_json(value: &LuaValue) -> LuaResult<serde_json::Value> {
    match value {
        LuaValue::Nil => Ok(serde_json::Value::Null),
        LuaValue::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        LuaValue::Integer(n) => Ok(serde_json::Value::Number(
            serde_json::Number::from(*n),
        )),
        LuaValue::Number(n) => {
            if n.is_finite() {
                Ok(serde_json::Value::Number(
                    serde_json::Number::from_f64(*n)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            } else {
                Ok(serde_json::Value::Null)
            }
        }
        LuaValue::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_owned())),
        LuaValue::Table(t) => lua_table_to_json(t),
        _ => Err(mlua::Error::runtime(format!(
            "Cannot serialize Lua type '{}' to JSON",
            value.type_name()
        ))),
    }
}

/// Convert a Lua table to a JSON value (array or object).
///
/// Tables with consecutive integer keys starting at 1 become JSON arrays.
/// All other tables become JSON objects (keys converted to strings).
fn lua_table_to_json(table: &LuaTable) -> LuaResult<serde_json::Value> {
    // Check if it looks like an array: keys 1..N with no gaps.
    let len = table.raw_len();
    let mut is_array = len > 0;

    if is_array {
        // Verify there are no non-integer keys beyond the array portion.
        let mut count = 0u64;
        for pair in table.pairs::<LuaValue, LuaValue>() {
            let _ = pair?;
            count += 1;
        }
        is_array = count == len as u64;
    }

    if is_array {
        let mut arr = Vec::with_capacity(len as usize);
        for i in 1..=len {
            let v: LuaValue = table.raw_get(i)?;
            arr.push(lua_value_to_json(&v)?);
        }
        Ok(serde_json::Value::Array(arr))
    } else {
        let mut map = serde_json::Map::new();
        for pair in table.pairs::<LuaValue, LuaValue>() {
            let (k, v) = pair?;
            let key = match &k {
                LuaValue::String(s) => s.to_str()?.to_owned(),
                LuaValue::Integer(n) => n.to_string(),
                LuaValue::Number(n) => n.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(format!(
                        "Cannot use Lua type '{}' as JSON key",
                        k.type_name()
                    )));
                }
            };
            map.insert(key, lua_value_to_json(&v)?);
        }
        Ok(serde_json::Value::Object(map))
    }
}

/// Convert a `serde_json::Value` to a Lua value.
fn json_to_lua_value(lua: &Lua, value: &serde_json::Value) -> LuaResult<LuaValue> {
    match value {
        serde_json::Value::Null => Ok(LuaValue::Nil),
        serde_json::Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else {
                Ok(LuaValue::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        serde_json::Value::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.raw_set(i + 1, json_to_lua_value(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.raw_set(k.as_str(), json_to_lua_value(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

/// `Engine.save_data(slot_name, data)` — serialize a Lua table to `saves/<slot>.json`.
///
/// Returns `true` on success, `false` on failure (error is logged).
fn lua_save_data(lua: &Lua, (slot, data): (String, LuaTable)) -> LuaResult<bool> {
    validate_slot_name(&slot)?;

    let save_path = match resolve_save_path(lua, &slot)? {
        Some(p) => p,
        None => {
            log::warn!("Engine.save_data: no save directory configured");
            return Ok(false);
        }
    };

    let json_value = lua_table_to_json(&data)?;
    let json_string = match serde_json::to_string_pretty(&json_value) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Engine.save_data: JSON serialization failed: {}", e);
            return Ok(false);
        }
    };

    // Ensure the saves directory exists.
    if let Some(parent) = save_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("Engine.save_data: failed to create saves directory: {}", e);
            return Ok(false);
        }
    }

    match crate::platform_utils::atomic_write(&save_path, &json_string) {
        Ok(()) => {
            log::info!("Engine.save_data: saved to '{}'", save_path.display());
            Ok(true)
        }
        Err(e) => {
            log::error!("Engine.save_data: write failed: {}", e);
            Ok(false)
        }
    }
}

/// `Engine.load_data(slot_name)` — load a Lua table from `saves/<slot>.json`.
///
/// Returns the table on success, or `nil` if the file doesn't exist or fails to parse.
fn lua_load_data(lua: &Lua, slot: String) -> LuaResult<LuaValue> {
    validate_slot_name(&slot)?;

    let save_path = match resolve_save_path(lua, &slot)? {
        Some(p) => p,
        None => {
            log::warn!("Engine.load_data: no save directory configured");
            return Ok(LuaValue::Nil);
        }
    };

    let json_string = match std::fs::read_to_string(&save_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LuaValue::Nil),
        Err(e) => {
            log::error!("Engine.load_data: read failed: {}", e);
            return Ok(LuaValue::Nil);
        }
    };

    let json_value: serde_json::Value = match serde_json::from_str(&json_string) {
        Ok(v) => v,
        Err(e) => {
            log::error!("Engine.load_data: JSON parse failed: {}", e);
            return Ok(LuaValue::Nil);
        }
    };

    json_to_lua_value(lua, &json_value)
}

/// `Engine.delete_save(slot_name)` — delete a save file. Returns `true` if deleted.
fn lua_delete_save(lua: &Lua, slot: String) -> LuaResult<bool> {
    validate_slot_name(&slot)?;

    let save_path = match resolve_save_path(lua, &slot)? {
        Some(p) => p,
        None => return Ok(false),
    };

    match std::fs::remove_file(&save_path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => {
            log::error!("Engine.delete_save: {}", e);
            Ok(false)
        }
    }
}

/// `Engine.save_exists(slot_name)` — check if a save file exists.
fn lua_save_exists(lua: &Lua, slot: String) -> LuaResult<bool> {
    validate_slot_name(&slot)?;

    let save_path = match resolve_save_path(lua, &slot)? {
        Some(p) => p,
        None => return Ok(false),
    };

    Ok(save_path.exists())
}

/// `Engine.list_saves()` — return an array of all save slot names.
fn lua_list_saves(lua: &Lua, _: ()) -> LuaResult<LuaTable> {
    let table = lua.create_table()?;

    let ctx = match lua.app_data_mut::<SceneScriptContext>() {
        Some(ctx) => ctx,
        None => return Ok(table),
    };
    let scene = unsafe { ctx.scene() };
    let save_dir = match scene.save_data_directory() {
        Some(d) => d.to_path_buf(),
        None => return Ok(table),
    };
    // Drop the borrow before doing I/O.
    drop(ctx);

    let entries = match std::fs::read_dir(&save_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(table),
    };

    let mut idx = 1i64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                table.raw_set(idx, stem)?;
                idx += 1;
            }
        }
    }

    Ok(table)
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
        assert!(engine.get::<LuaFunction>("vector_length").is_ok());
        assert!(engine.get::<LuaFunction>("distance").is_ok());
        assert!(engine.get::<LuaFunction>("distance_2d").is_ok());
        assert!(engine.get::<LuaFunction>("lerp").is_ok());
        assert!(engine.get::<LuaFunction>("lerp_vec3").is_ok());
        assert!(engine.get::<LuaFunction>("slerp").is_ok());
        assert!(engine.get::<LuaFunction>("clamp").is_ok());
        assert!(engine.get::<LuaFunction>("move_toward").is_ok());
        assert!(engine.get::<LuaFunction>("move_toward_vec3").is_ok());
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
        assert!(engine.get::<LuaFunction>("get_mouse_delta").is_ok());
        assert!(engine.get::<LuaFunction>("get_scroll_delta").is_ok());
        // Gamepad
        assert!(engine.get::<LuaFunction>("is_gamepad_button_down").is_ok());
        assert!(engine
            .get::<LuaFunction>("is_gamepad_button_pressed")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("is_gamepad_button_released")
            .is_ok());
        assert!(engine.get::<LuaFunction>("get_gamepad_axis").is_ok());
        assert!(engine.get::<LuaFunction>("is_gamepad_connected").is_ok());
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
        // Skeletal animation
        assert!(engine.get::<LuaFunction>("play_skeletal_animation").is_ok());
        assert!(engine
            .get::<LuaFunction>("play_skeletal_animation_blended")
            .is_ok());
        assert!(engine.get::<LuaFunction>("stop_skeletal_animation").is_ok());
        assert!(engine
            .get::<LuaFunction>("is_skeletal_animation_playing")
            .is_ok());
        assert!(engine.get::<LuaFunction>("get_skeletal_animation").is_ok());
        assert!(engine
            .get::<LuaFunction>("set_skeletal_animation_speed")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("get_skeletal_animation_time")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("list_skeletal_animations")
            .is_ok());
        // Animation events
        assert!(engine
            .get::<LuaFunction>("add_animation_event")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("remove_animation_event")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("add_skeletal_anim_event")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("remove_skeletal_anim_event")
            .is_ok());
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
        assert!(engine.get::<LuaFunction>("fade_in").is_ok());
        assert!(engine.get::<LuaFunction>("fade_out").is_ok());
        assert!(engine.get::<LuaFunction>("fade_to").is_ok());
        assert!(engine.get::<LuaFunction>("set_master_volume").is_ok());
        assert!(engine.get::<LuaFunction>("get_master_volume").is_ok());
        assert!(engine.get::<LuaFunction>("set_category_volume").is_ok());
        assert!(engine.get::<LuaFunction>("get_category_volume").is_ok());
        assert!(engine.get::<LuaFunction>("set_hrtf").is_ok());
        assert!(engine.get::<LuaFunction>("get_hrtf").is_ok());
        // Physics
        assert!(engine.get::<LuaFunction>("apply_impulse").is_ok());
        assert!(engine.get::<LuaFunction>("apply_impulse_at_point").is_ok());
        assert!(engine.get::<LuaFunction>("apply_force").is_ok());
        assert!(engine.get::<LuaFunction>("get_linear_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("set_linear_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("get_angular_velocity").is_ok());
        assert!(engine.get::<LuaFunction>("set_angular_velocity").is_ok());
        // Body type
        assert!(engine.get::<LuaFunction>("set_body_type").is_ok());
        assert!(engine.get::<LuaFunction>("get_body_type").is_ok());
        #[cfg(feature = "physics-3d")]
        {
            assert!(engine.get::<LuaFunction>("set_body_type_3d").is_ok());
            assert!(engine.get::<LuaFunction>("get_body_type_3d").is_ok());
        }
        // Shape overlap queries
        assert!(engine.get::<LuaFunction>("point_query").is_ok());
        assert!(engine.get::<LuaFunction>("aabb_query").is_ok());
        assert!(engine.get::<LuaFunction>("overlap_circle").is_ok());
        assert!(engine.get::<LuaFunction>("overlap_box").is_ok());
        #[cfg(feature = "physics-3d")]
        {
            assert!(engine.get::<LuaFunction>("point_query_3d").is_ok());
            assert!(engine.get::<LuaFunction>("aabb_query_3d").is_ok());
            assert!(engine.get::<LuaFunction>("overlap_sphere").is_ok());
            assert!(engine.get::<LuaFunction>("overlap_box_3d").is_ok());
        }
        // Spatial grid queries
        assert!(engine.get::<LuaFunction>("rebuild_spatial_grid").is_ok());
        assert!(engine
            .get::<LuaFunction>("rebuild_spatial_grid_3d")
            .is_ok());
        assert!(engine.get::<LuaFunction>("query_region").is_ok());
        assert!(engine.get::<LuaFunction>("query_radius").is_ok());
        assert!(engine.get::<LuaFunction>("query_region_3d").is_ok());
        assert!(engine.get::<LuaFunction>("query_radius_3d").is_ok());
        // Joints (2D)
        assert!(engine.get::<LuaFunction>("create_revolute_joint").is_ok());
        assert!(engine.get::<LuaFunction>("create_fixed_joint").is_ok());
        assert!(engine.get::<LuaFunction>("create_prismatic_joint").is_ok());
        assert!(engine.get::<LuaFunction>("remove_joint").is_ok());
        // Joints (3D)
        #[cfg(feature = "physics-3d")]
        {
            assert!(engine
                .get::<LuaFunction>("create_revolute_joint_3d")
                .is_ok());
            assert!(engine.get::<LuaFunction>("create_fixed_joint_3d").is_ok());
            assert!(engine.get::<LuaFunction>("create_ball_joint_3d").is_ok());
            assert!(engine
                .get::<LuaFunction>("create_prismatic_joint_3d")
                .is_ok());
            assert!(engine.get::<LuaFunction>("remove_joint_3d").is_ok());
        }
        // Cursor
        assert!(engine.get::<LuaFunction>("set_cursor_mode").is_ok());
        assert!(engine.get::<LuaFunction>("get_cursor_mode").is_ok());
        // Window
        assert!(engine.get::<LuaFunction>("set_window_size").is_ok());
        assert!(engine.get::<LuaFunction>("get_window_size").is_ok());
        assert!(engine.get::<LuaFunction>("set_ui_anchor").is_ok());
        assert!(engine.get::<LuaFunction>("get_ui_anchor").is_ok());

        // Text color
        assert!(engine.get::<LuaFunction>("get_text_color").is_ok());
        assert!(engine.get::<LuaFunction>("set_text_color").is_ok());

        // Runtime settings
        assert!(engine.get::<LuaFunction>("get_vsync").is_ok());
        assert!(engine.get::<LuaFunction>("set_vsync").is_ok());
        assert!(engine.get::<LuaFunction>("get_fullscreen").is_ok());
        assert!(engine.get::<LuaFunction>("set_fullscreen").is_ok());
        assert!(engine.get::<LuaFunction>("get_shadow_quality").is_ok());
        assert!(engine.get::<LuaFunction>("set_shadow_quality").is_ok());
        assert!(engine.get::<LuaFunction>("quit").is_ok());
        assert!(engine.get::<LuaFunction>("load_scene").is_ok());
        assert!(engine.get::<LuaFunction>("get_gui_scale").is_ok());
        assert!(engine.get::<LuaFunction>("set_gui_scale").is_ok());
        // Loading screen
        assert!(engine
            .get::<LuaFunction>("set_loading_screen_color")
            .is_ok());
        assert!(engine
            .get::<LuaFunction>("get_loading_screen_color")
            .is_ok());
        // Component manipulation
        assert!(engine.get::<LuaFunction>("add_component").is_ok());
        assert!(engine.get::<LuaFunction>("remove_component").is_ok());
        // Save/load
        assert!(engine.get::<LuaFunction>("save_data").is_ok());
        assert!(engine.get::<LuaFunction>("load_data").is_ok());
        assert!(engine.get::<LuaFunction>("delete_save").is_ok());
        assert!(engine.get::<LuaFunction>("save_exists").is_ok());
        assert!(engine.get::<LuaFunction>("list_saves").is_ok());
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
    fn math_helpers() {
        let lua = setup();

        // distance
        lua.load("d = Engine.distance(0, 0, 0, 3, 4, 0)")
            .exec()
            .unwrap();
        let d: f32 = lua.globals().get("d").unwrap();
        assert!((d - 5.0).abs() < 0.001);

        // distance_2d
        lua.load("d2 = Engine.distance_2d(0, 0, 3, 4)")
            .exec()
            .unwrap();
        let d2: f32 = lua.globals().get("d2").unwrap();
        assert!((d2 - 5.0).abs() < 0.001);

        // lerp
        lua.load("l = Engine.lerp(0, 10, 0.25)").exec().unwrap();
        let l: f32 = lua.globals().get("l").unwrap();
        assert!((l - 2.5).abs() < 0.001);

        // clamp
        lua.load("c = Engine.clamp(15, 0, 10)").exec().unwrap();
        let c: f32 = lua.globals().get("c").unwrap();
        assert!((c - 10.0).abs() < 0.001);

        // move_toward
        lua.load("m = Engine.move_toward(0, 10, 3)").exec().unwrap();
        let m: f32 = lua.globals().get("m").unwrap();
        assert!((m - 3.0).abs() < 0.001);

        // move_toward (arrives at target)
        lua.load("m2 = Engine.move_toward(8, 10, 5)")
            .exec()
            .unwrap();
        let m2: f32 = lua.globals().get("m2").unwrap();
        assert!((m2 - 10.0).abs() < 0.001);

        // vector_length
        lua.load("vl = Engine.vector_length(3, 4, 0)")
            .exec()
            .unwrap();
        let vl: f32 = lua.globals().get("vl").unwrap();
        assert!((vl - 5.0).abs() < 0.001);
    }

    #[test]
    fn gamepad_button_name_mapping() {
        assert_eq!(
            gamepad_button_name_to_enum("South"),
            Some(GamepadButton::South)
        );
        assert_eq!(gamepad_button_name_to_enum("A"), Some(GamepadButton::South));
        assert_eq!(
            gamepad_button_name_to_enum("North"),
            Some(GamepadButton::North)
        );
        assert_eq!(
            gamepad_button_name_to_enum("DPadUp"),
            Some(GamepadButton::DPadUp)
        );
        assert_eq!(
            gamepad_button_name_to_enum("L1"),
            Some(GamepadButton::LeftBumper)
        );
        assert_eq!(gamepad_button_name_to_enum("bogus"), None);
    }

    #[test]
    fn gamepad_axis_name_mapping() {
        assert_eq!(
            gamepad_axis_name_to_enum("LeftStickX"),
            Some(GamepadAxis::LeftStickX)
        );
        assert_eq!(
            gamepad_axis_name_to_enum("RightTrigger"),
            Some(GamepadAxis::RightTrigger)
        );
        assert_eq!(gamepad_axis_name_to_enum("bogus"), None);
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

    // -- Skeletal animation (no context) --

    #[test]
    fn play_skeletal_animation_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.play_skeletal_animation(12345, "idle")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn play_skeletal_animation_blended_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.play_skeletal_animation_blended(12345, "run", 0.3)"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn stop_skeletal_animation_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.stop_skeletal_animation(12345)")
            .exec()
            .unwrap();
    }

    #[test]
    fn is_skeletal_animation_playing_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.is_skeletal_animation_playing(12345)")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn get_skeletal_animation_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.get_skeletal_animation(12345)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result == LuaValue::Nil);
    }

    #[test]
    fn set_skeletal_animation_speed_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_skeletal_animation_speed(12345, 2.0)")
            .exec()
            .unwrap();
    }

    #[test]
    fn get_skeletal_animation_time_no_context_returns_zero() {
        let lua = setup();
        lua.load("result = Engine.get_skeletal_animation_time(12345)")
            .exec()
            .unwrap();
        let result: f32 = lua.globals().get("result").unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn list_skeletal_animations_no_context_returns_empty() {
        let lua = setup();
        lua.load("result = Engine.list_skeletal_animations(12345)")
            .exec()
            .unwrap();
        let result: mlua::Table = lua.globals().get("result").unwrap();
        assert_eq!(result.len().unwrap(), 0);
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
    fn fade_in_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.fade_in(12345, 1.0)")
            .exec()
            .expect("fade_in should not error without context");
    }

    #[test]
    fn fade_out_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.fade_out(12345, 1.0)")
            .exec()
            .expect("fade_out should not error without context");
    }

    #[test]
    fn master_volume_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_master_volume(0.5)")
            .exec()
            .expect("set_master_volume should not error without context");
        lua.load("result = Engine.get_master_volume()")
            .exec()
            .expect("get_master_volume should not error without context");
    }

    #[test]
    fn category_volume_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.set_category_volume('music', 0.5)")
            .exec()
            .expect("set_category_volume should not error without context");
        lua.load("result = Engine.get_category_volume('sfx')")
            .exec()
            .expect("get_category_volume should not error without context");
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

    #[test]
    fn set_body_type_no_context_no_error() {
        let lua = setup();
        lua.load(r#"Engine.set_body_type(12345, "dynamic")"#)
            .exec()
            .expect("set_body_type should not error without context");
    }

    #[test]
    fn get_body_type_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.get_body_type(12345)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn point_query_no_context_returns_empty() {
        let lua = setup();
        lua.load("result = Engine.point_query(0, 0)")
            .exec()
            .unwrap();
        let result: LuaTable = lua.globals().get("result").unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }

    #[test]
    fn overlap_circle_no_context_returns_empty() {
        let lua = setup();
        lua.load("result = Engine.overlap_circle(0, 0, 1.0)")
            .exec()
            .unwrap();
        let result: LuaTable = lua.globals().get("result").unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }

    #[test]
    fn create_revolute_joint_no_context_returns_nil() {
        let lua = setup();
        lua.load("result = Engine.create_revolute_joint(111, 222, 0, 0, 0, 0)")
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn remove_joint_no_context_no_error() {
        let lua = setup();
        lua.load("Engine.remove_joint(12345)").exec().unwrap();
    }

    #[test]
    fn action_functions_registered() {
        let lua = setup();
        let engine: LuaTable = lua.globals().get("Engine").unwrap();
        assert!(engine.get::<LuaFunction>("is_action_pressed").is_ok());
        assert!(engine.get::<LuaFunction>("is_action_just_pressed").is_ok());
        assert!(engine.get::<LuaFunction>("is_action_just_released").is_ok());
        assert!(engine.get::<LuaFunction>("get_action_value").is_ok());
    }

    #[test]
    fn action_no_context_returns_defaults() {
        let lua = setup();
        lua.load(r#"result = Engine.is_action_pressed("jump")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);

        lua.load(r#"val = Engine.get_action_value("move")"#)
            .exec()
            .unwrap();
        let val: f32 = lua.globals().get("val").unwrap();
        assert!(val.abs() < 0.001);
    }

    // -----------------------------------------------------------------------
    // Save/load tests
    // -----------------------------------------------------------------------

    #[test]
    fn save_data_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.save_data("slot1", {level = 5})"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn load_data_no_context_returns_nil() {
        let lua = setup();
        lua.load(r#"result = Engine.load_data("slot1")"#)
            .exec()
            .unwrap();
        let result: LuaValue = lua.globals().get("result").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn delete_save_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.delete_save("slot1")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn save_exists_no_context_returns_false() {
        let lua = setup();
        lua.load(r#"result = Engine.save_exists("slot1")"#)
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn list_saves_no_context_returns_empty() {
        let lua = setup();
        lua.load("result = Engine.list_saves()")
            .exec()
            .unwrap();
        let result: LuaTable = lua.globals().get("result").unwrap();
        assert_eq!(result.len().unwrap(), 0);
    }

    #[test]
    fn save_slot_validation_rejects_path_traversal() {
        let lua = setup();
        let result = lua
            .load(r#"Engine.save_data("../evil", {x = 1})"#)
            .exec();
        assert!(result.is_err());
    }

    #[test]
    fn save_slot_validation_rejects_slashes() {
        let lua = setup();
        let result = lua
            .load(r#"Engine.save_data("foo/bar", {x = 1})"#)
            .exec();
        assert!(result.is_err());
    }

    #[test]
    fn save_slot_validation_rejects_empty() {
        let lua = setup();
        let result = lua.load(r#"Engine.save_data("", {x = 1})"#).exec();
        assert!(result.is_err());
    }

    #[test]
    fn lua_table_to_json_roundtrip() {
        let lua = setup();

        // Create a Lua table with mixed data types.
        lua.load(
            r#"
            test_table = {
                name = "Player1",
                level = 42,
                health = 99.5,
                alive = true,
                inventory = {"sword", "shield", "potion"},
                stats = { str = 10, dex = 15 },
            }
            "#,
        )
        .exec()
        .unwrap();

        let table: LuaTable = lua.globals().get("test_table").unwrap();
        let json = lua_table_to_json(&table).unwrap();

        // Verify JSON structure.
        assert_eq!(json["name"], "Player1");
        assert_eq!(json["level"], 42);
        assert!((json["health"].as_f64().unwrap() - 99.5).abs() < 0.001);
        assert_eq!(json["alive"], true);
        assert!(json["inventory"].is_array());
        assert_eq!(json["inventory"][0], "sword");
        assert_eq!(json["inventory"].as_array().unwrap().len(), 3);
        assert_eq!(json["stats"]["str"], 10);
        assert_eq!(json["stats"]["dex"], 15);

        // Roundtrip: JSON → Lua table.
        let lua_val = json_to_lua_value(&lua, &json).unwrap();
        let rt_table = match lua_val {
            LuaValue::Table(t) => t,
            _ => panic!("Expected table"),
        };
        let name: String = rt_table.get("name").unwrap();
        assert_eq!(name, "Player1");
        let level: i64 = rt_table.get("level").unwrap();
        assert_eq!(level, 42);
        let alive: bool = rt_table.get("alive").unwrap();
        assert!(alive);
    }

    #[test]
    fn lua_table_to_json_array_detection() {
        let lua = setup();

        // Pure array (consecutive integer keys 1..N).
        lua.load("arr = {10, 20, 30}").exec().unwrap();
        let arr: LuaTable = lua.globals().get("arr").unwrap();
        let json = lua_table_to_json(&arr).unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 3);

        // Mixed table (has string keys) → object.
        lua.load("mixed = {10, 20, foo = 'bar'}").exec().unwrap();
        let mixed: LuaTable = lua.globals().get("mixed").unwrap();
        let json = lua_table_to_json(&mixed).unwrap();
        assert!(json.is_object());
    }

    #[test]
    fn lua_table_to_json_rejects_functions() {
        let lua = setup();
        lua.load("bad = {callback = function() end}").exec().unwrap();
        let table: LuaTable = lua.globals().get("bad").unwrap();
        let result = lua_table_to_json(&table);
        assert!(result.is_err());
    }

    #[test]
    fn save_load_with_context() {
        // Integration test: set up a real Scene with a save directory,
        // inject SceneScriptContext, and do a full save/load cycle.
        let lua = setup();
        let temp_dir = std::env::temp_dir().join("gg_test_saves");
        let _ = std::fs::remove_dir_all(&temp_dir);

        let mut scene = Scene::new();
        scene.set_save_data_directory(temp_dir.clone());

        // Set up the SceneScriptContext.
        let ctx = SceneScriptContext {
            scene: &mut scene as *mut Scene,
            input: std::ptr::null(),
        };
        lua.set_app_data(ctx);

        // save_exists should return false for non-existent slot.
        lua.load(r#"exists_before = Engine.save_exists("test_slot")"#)
            .exec()
            .unwrap();
        let exists: bool = lua.globals().get("exists_before").unwrap();
        assert!(!exists);

        // save_data should succeed.
        lua.load(
            r#"
            save_ok = Engine.save_data("test_slot", {
                level = 7,
                name = "Hero",
                items = {"axe", "bow"},
            })
            "#,
        )
        .exec()
        .unwrap();
        let save_ok: bool = lua.globals().get("save_ok").unwrap();
        assert!(save_ok);

        // save_exists should now return true.
        lua.load(r#"exists_after = Engine.save_exists("test_slot")"#)
            .exec()
            .unwrap();
        let exists: bool = lua.globals().get("exists_after").unwrap();
        assert!(exists);

        // load_data should return the saved table.
        lua.load(
            r#"
            loaded = Engine.load_data("test_slot")
            loaded_level = loaded.level
            loaded_name = loaded.name
            loaded_item1 = loaded.items[1]
            "#,
        )
        .exec()
        .unwrap();
        let level: i64 = lua.globals().get("loaded_level").unwrap();
        assert_eq!(level, 7);
        let name: String = lua.globals().get("loaded_name").unwrap();
        assert_eq!(name, "Hero");
        let item1: String = lua.globals().get("loaded_item1").unwrap();
        assert_eq!(item1, "axe");

        // list_saves should include "test_slot".
        lua.load(
            r#"
            saves = Engine.list_saves()
            num_saves = #saves
            "#,
        )
        .exec()
        .unwrap();
        let num_saves: i64 = lua.globals().get("num_saves").unwrap();
        assert_eq!(num_saves, 1);

        // delete_save should succeed.
        lua.load(r#"deleted = Engine.delete_save("test_slot")"#)
            .exec()
            .unwrap();
        let deleted: bool = lua.globals().get("deleted").unwrap();
        assert!(deleted);

        // save_exists should return false again.
        lua.load(r#"exists_final = Engine.save_exists("test_slot")"#)
            .exec()
            .unwrap();
        let exists: bool = lua.globals().get("exists_final").unwrap();
        assert!(!exists);

        // Cleanup.
        lua.remove_app_data::<SceneScriptContext>();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // -----------------------------------------------------------------------
    // Loading screen color tests
    // -----------------------------------------------------------------------

    #[test]
    fn loading_screen_color_default() {
        let lua = setup();
        lua.load("r, g, b = Engine.get_loading_screen_color()")
            .exec()
            .unwrap();
        let r: f32 = lua.globals().get("r").unwrap();
        let g: f32 = lua.globals().get("g").unwrap();
        let b: f32 = lua.globals().get("b").unwrap();
        // No context → default (0, 0, 0).
        assert_eq!(r, 0.0);
        assert_eq!(g, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn loading_screen_color_roundtrip_with_context() {
        let lua = setup();
        let mut scene = Scene::new();
        let ctx = SceneScriptContext {
            scene: &mut scene as *mut Scene,
            input: std::ptr::null(),
        };
        lua.set_app_data(ctx);

        // Default is black.
        lua.load("r, g, b = Engine.get_loading_screen_color()")
            .exec()
            .unwrap();
        let r: f32 = lua.globals().get("r").unwrap();
        assert_eq!(r, 0.0);

        // Set to a custom color.
        lua.load("Engine.set_loading_screen_color(0.2, 0.4, 0.8)")
            .exec()
            .unwrap();
        lua.load("r2, g2, b2 = Engine.get_loading_screen_color()")
            .exec()
            .unwrap();
        let r2: f32 = lua.globals().get("r2").unwrap();
        let g2: f32 = lua.globals().get("g2").unwrap();
        let b2: f32 = lua.globals().get("b2").unwrap();
        assert!((r2 - 0.2).abs() < 1e-6);
        assert!((g2 - 0.4).abs() < 1e-6);
        assert!((b2 - 0.8).abs() < 1e-6);

        // Verify on the Rust side too.
        let color = scene.loading_screen_color();
        assert!((color[0] - 0.2).abs() < 1e-6);
        assert!((color[1] - 0.4).abs() < 1e-6);
        assert!((color[2] - 0.8).abs() < 1e-6);
        assert_eq!(color[3], 1.0);

        lua.remove_app_data::<SceneScriptContext>();
    }

    // -----------------------------------------------------------------------
    // Animation event no-context tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_animation_event_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.add_animation_event(12345, 'walk', 2, 'footstep')")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn remove_animation_event_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.remove_animation_event(12345, 'walk', 'footstep')")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn add_skeletal_anim_event_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.add_skeletal_anim_event(12345, 'walk', 0.3, 'footstep')")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn remove_skeletal_anim_event_no_context_returns_false() {
        let lua = setup();
        lua.load("result = Engine.remove_skeletal_anim_event(12345, 'walk', 'footstep')")
            .exec()
            .unwrap();
        let result: bool = lua.globals().get("result").unwrap();
        assert!(!result);
    }

    #[test]
    fn start_coroutine_no_context_errors_gracefully() {
        let lua = setup();
        // Without PendingCoroutineOps in app_data, start_coroutine should error.
        let result = lua
            .load("Engine.start_coroutine(function() end)")
            .exec();
        assert!(result.is_err());
    }

    #[test]
    fn stop_all_coroutines_no_context_no_error() {
        let lua = setup();
        // Without PendingCoroutineOps, stop_all_coroutines silently succeeds.
        lua.load("Engine.stop_all_coroutines()")
            .exec()
            .unwrap();
    }

    #[test]
    fn wait_and_wait_frame_exist() {
        let lua = setup();
        // Verify Engine.wait and Engine.wait_frame are callable functions.
        let is_fn: bool = lua
            .load("type(Engine.wait) == 'function'")
            .eval()
            .unwrap();
        assert!(is_fn);
        let is_fn: bool = lua
            .load("type(Engine.wait_frame) == 'function'")
            .eval()
            .unwrap();
        assert!(is_fn);
    }

    #[test]
    fn emit_no_context_errors_gracefully() {
        let lua = setup();
        let result = lua.load("Engine.emit('test')").exec();
        assert!(result.is_err());
    }

    #[test]
    fn on_no_context_errors_gracefully() {
        let lua = setup();
        let result = lua
            .load("Engine.on('test', function() end)")
            .exec();
        assert!(result.is_err());
    }

    #[test]
    fn off_no_context_no_error() {
        let lua = setup();
        // Without PendingEventBusOps, off silently succeeds.
        lua.load("Engine.off('test')").exec().unwrap();
    }

    #[test]
    fn event_bus_functions_exist() {
        let lua = setup();
        let is_fn: bool = lua
            .load("type(Engine.emit) == 'function'")
            .eval()
            .unwrap();
        assert!(is_fn);
        let is_fn: bool = lua
            .load("type(Engine.on) == 'function'")
            .eval()
            .unwrap();
        assert!(is_fn);
        let is_fn: bool = lua
            .load("type(Engine.off) == 'function'")
            .eval()
            .unwrap();
        assert!(is_fn);
    }
}
