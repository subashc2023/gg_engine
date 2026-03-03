-- physics_player.lua
-- WASD impulse-based movement with velocity clamping and Space to jump.
-- Attach to an entity that has a RigidBody2D (Dynamic) + BoxCollider2D.

fields = {
    move_speed = 1.0,
    jump_impulse = 5.0,
    max_speed = 10.0,
}

function on_create()
    Engine.native_log("PhysicsPlayer created on entity", entity_id)
end

function on_update(dt)
    if not Engine.has_component(entity_id, "RigidBody2D") then
        return
    end

    -- Horizontal movement via impulses
    if Engine.is_key_down("A") then
        Engine.apply_impulse(entity_id, -fields.move_speed, 0)
    end
    if Engine.is_key_down("D") then
        Engine.apply_impulse(entity_id, fields.move_speed, 0)
    end

    -- Clamp horizontal velocity
    local vx, vy = Engine.get_linear_velocity(entity_id)
    if vx > fields.max_speed then
        Engine.set_linear_velocity(entity_id, fields.max_speed, vy)
    elseif vx < -fields.max_speed then
        Engine.set_linear_velocity(entity_id, -fields.max_speed, vy)
    end

    -- Jump (only when roughly grounded — vy near zero)
    if Engine.is_key_down("Space") then
        local _, cur_vy = Engine.get_linear_velocity(entity_id)
        if math.abs(cur_vy) < 0.1 then
            Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
        end
    end
end

function on_destroy()
    Engine.native_log("PhysicsPlayer destroyed on entity", entity_id)
end
