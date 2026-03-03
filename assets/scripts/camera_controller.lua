-- camera_controller.lua
-- Smooth camera follow on the "Lua Player" entity.
-- Falls back to arrow-key panning if no player is found.

fields = {
    speed = 5.0,
    follow_speed = 5.0,
}

local player_id = nil

function on_create()
    Engine.native_log("CameraController created on entity", entity_id)
    player_id = Engine.find_entity_by_name("Lua Player")
end

function on_update(dt)
    local cx, cy, cz = Engine.get_translation(entity_id)

    if player_id then
        -- Smooth follow: lerp camera toward player position.
        local px, py, _ = Engine.get_translation(player_id)
        local t = math.min(1.0, fields.follow_speed * dt)
        cx = cx + (px - cx) * t
        cy = cy + (py - cy) * t
    else
        -- Fallback: arrow-key panning.
        if Engine.is_key_down("Left") then
            cx = cx - fields.speed * dt
        end
        if Engine.is_key_down("Right") then
            cx = cx + fields.speed * dt
        end
        if Engine.is_key_down("Up") then
            cy = cy + fields.speed * dt
        end
        if Engine.is_key_down("Down") then
            cy = cy - fields.speed * dt
        end
    end

    Engine.set_translation(entity_id, cx, cy, cz)
end

function on_destroy()
    Engine.native_log("CameraController destroyed on entity", entity_id)
end
