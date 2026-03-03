-- camera_follow.lua
-- Camera that follows a player entity found by name.
-- Demonstrates: Engine.find_entity_by_name, Engine.get_translation,
--               Engine.get_script_field, Engine.set_translation.
-- Attach to an entity with a CameraComponent (primary).

fields = {
    target_name = "Player",
    offset_z = 0.0,
}

local target_id = nil

function on_create()
    -- Look up the target entity once at start.
    target_id = Engine.find_entity_by_name(fields.target_name)
    if target_id then
        Engine.native_log("CameraFollow: found target", target_id)
    else
        Engine.native_log("CameraFollow: target not found!", 0)
    end
end

function on_update(dt)
    if not target_id then
        return
    end

    -- Read the target's position.
    local tx, ty, tz = Engine.get_translation(target_id)

    -- Read the camera's current position (preserve Z).
    local cx, cy, cz = Engine.get_translation(entity_id)

    -- Follow the target's XY, keep our Z offset.
    Engine.set_translation(entity_id, tx, ty, cz)
end

function on_destroy()
    Engine.native_log("CameraFollow destroyed on entity", entity_id)
end
