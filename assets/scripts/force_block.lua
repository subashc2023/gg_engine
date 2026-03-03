-- force_block.lua
-- Demonstrates: apply_force, apply_impulse_at_point, get/set_angular_velocity, get/set_scale
-- Press E/Q to apply torque-like impulses (impulse at offset point).
-- Press Z/X to scale the entity up/down.
-- Press F to apply upward force (sustained while held).

fields = {
    scale_speed = 1.0,
}

function on_create()
    Engine.native_log("ForceBlock created on entity", entity_id)
end

function on_fixed_update(dt)
    if not Engine.has_component(entity_id, "RigidBody2D") then
        return
    end

    local tx, ty, tz = Engine.get_translation(entity_id)

    -- Apply sustained upward force while F is held
    if Engine.is_key_down("F") then
        Engine.apply_force(entity_id, 0, 20.0)
    end

    -- Apply impulse at offset point to create spin (Q = left spin, E = right spin)
    if Engine.is_key_down("Q") then
        Engine.apply_impulse_at_point(entity_id, 0, 1.0, tx + 0.5, ty)
    end
    if Engine.is_key_down("E") then
        Engine.apply_impulse_at_point(entity_id, 0, 1.0, tx - 0.5, ty)
    end

    -- Clamp angular velocity
    local omega = Engine.get_angular_velocity(entity_id)
    if omega > 10.0 then
        Engine.set_angular_velocity(entity_id, 10.0)
    elseif omega < -10.0 then
        Engine.set_angular_velocity(entity_id, -10.0)
    end
end

function on_update(dt)
    -- Scale up/down with Z/X (visual, dt-scaled — stays in on_update)
    local sx, sy, sz = Engine.get_scale(entity_id)
    if Engine.is_key_down("Z") then
        sx = sx + fields.scale_speed * dt
        sy = sy + fields.scale_speed * dt
    end
    if Engine.is_key_down("X") then
        sx = math.max(0.2, sx - fields.scale_speed * dt)
        sy = math.max(0.2, sy - fields.scale_speed * dt)
    end
    Engine.set_scale(entity_id, sx, sy, sz)
end

function on_destroy()
end
