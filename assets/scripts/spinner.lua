-- spinner.lua
-- Continuously rotates the entity around the Z axis.
-- Demonstrates: get_rotation, set_rotation

fields = {
    spin_speed = 2.0,  -- radians per second
}

function on_create()
    Engine.native_log("Spinner created on entity", entity_id)
end

function on_update(dt)
    local rx, ry, rz = Engine.get_rotation(entity_id)
    rz = rz + fields.spin_speed * dt
    Engine.set_rotation(entity_id, rx, ry, rz)
end

function on_destroy()
end
