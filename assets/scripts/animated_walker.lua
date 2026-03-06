-- animated_walker.lua
-- WASD movement with animated walking spritesheet.
-- Attach to an entity with RigidBody2D (Dynamic, FixedRotation),
-- BoxCollider2D, SpriteRenderer (Walking.png), and SpriteAnimator.

fields = {
    move_speed = 5.0,
    move_accel = 50.0,
    jump_impulse = 5.0,
}

function on_create()
    Engine.play_animation(entity_id, "walk")
end

function on_fixed_update(dt)
    if not Engine.has_component(entity_id, "RigidBody2D") then
        return
    end

    local vx, vy = Engine.get_linear_velocity(entity_id)

    local target_vx = 0
    if Engine.is_key_down("A") then target_vx = -fields.move_speed
    elseif Engine.is_key_down("D") then target_vx = fields.move_speed end

    local force_x = (target_vx - vx) * fields.move_accel
    Engine.apply_force(entity_id, force_x, 0)

    -- Flip sprite based on direction
    if target_vx < 0 then
        Engine.set_scale(entity_id, -1, 1, 1)
    elseif target_vx > 0 then
        Engine.set_scale(entity_id, 1, 1, 1)
    end

    -- Play/stop animation based on movement
    if target_vx ~= 0 then
        if not Engine.is_animation_playing(entity_id) then
            Engine.play_animation(entity_id, "walk")
        end
    else
        Engine.stop_animation(entity_id)
    end

    -- Ground check + jump
    local px, py = Engine.get_translation(entity_id)
    local hit_id = Engine.raycast(px, py, 0, -1, 0.55, entity_id)
    local grounded = hit_id ~= nil

    if Engine.is_key_down("Space") and grounded then
        Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
    end
end
