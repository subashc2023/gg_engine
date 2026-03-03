-- camera_controller.lua
-- Arrow-key camera panning via direct translation.
-- Attach to an entity that has a CameraComponent (primary).

fields = {
    speed = 5.0,
}

function on_create()
    Engine.native_log("CameraController created on entity", entity_id)
end

function on_update(dt)
    local x, y, z = Engine.get_translation(entity_id)

    if Engine.is_key_down("Left") then
        x = x - fields.speed * dt
    end
    if Engine.is_key_down("Right") then
        x = x + fields.speed * dt
    end
    if Engine.is_key_down("Up") then
        y = y + fields.speed * dt
    end
    if Engine.is_key_down("Down") then
        y = y - fields.speed * dt
    end

    Engine.set_translation(entity_id, x, y, z)
end

function on_destroy()
    Engine.native_log("CameraController destroyed on entity", entity_id)
end
