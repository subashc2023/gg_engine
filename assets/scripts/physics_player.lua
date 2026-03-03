-- physics_player.lua
-- WASD velocity-based movement with impulse jump.
-- Attach to an entity that has a RigidBody2D (Dynamic) + BoxCollider2D.

fields = {
    move_speed = 5.0,
    jump_impulse = 5.0,
}

function on_create()
    Engine.native_log("PhysicsPlayer created on entity", entity_id)
end

function on_fixed_update(dt)
    if not Engine.has_component(entity_id, "RigidBody2D") then
        return
    end

    local vx, vy = Engine.get_linear_velocity(entity_id)

    -- Horizontal movement: direct velocity control
    if Engine.is_key_down("A") then
        vx = -fields.move_speed
    elseif Engine.is_key_down("D") then
        vx = fields.move_speed
    else
        vx = 0
    end

    Engine.set_linear_velocity(entity_id, vx, vy)

    -- Jump (only when roughly grounded — vy near zero)
    if Engine.is_key_down("Space") and math.abs(vy) < 0.1 then
        Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
    end
end

function on_destroy()
    Engine.native_log("PhysicsPlayer destroyed on entity", entity_id)
end
