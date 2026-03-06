-- physics_player.lua
-- WASD force-based movement with jump.
-- Attach to an entity that has a RigidBody2D (Dynamic, FixedRotation)
-- + BoxCollider2D (friction=0, braking handled in code).

fields = {
    move_speed = 5.0,
    move_accel = 50.0,
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

    -- Horizontal movement: force toward target velocity
    local target_vx = 0
    if Engine.is_key_down("A") then target_vx = -fields.move_speed
    elseif Engine.is_key_down("D") then target_vx = fields.move_speed end

    local force_x = (target_vx - vx) * fields.move_accel
    Engine.apply_force(entity_id, force_x, 0)

    -- Ground check: short downward raycast from entity center
    local px, py = Engine.get_translation(entity_id)
    local hit_id = Engine.raycast(px, py, 0, -1, 0.55, entity_id)
    local grounded = hit_id ~= nil

    -- Jump when grounded. Raycast ground check prevents spam — after the
    -- impulse the player rises past the skin distance within one step.
    if Engine.is_key_down("Space") and grounded then
        Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
    end
end

function on_destroy()
    Engine.native_log("PhysicsPlayer destroyed on entity", entity_id)
end
