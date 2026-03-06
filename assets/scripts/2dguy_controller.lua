-- 2dguy_controller.lua
-- Full character controller with multiple animation states.
-- Attach to an entity with:
--   SpriteRenderer (2dguy_atlas.png), SpriteAnimator (see scene),
--   RigidBody2D (Dynamic, FixedRotation), BoxCollider2D.

fields = {
    move_speed = 5.0,
    run_speed = 9.0,
    move_accel = 50.0,
    jump_impulse = 5.0,
}

local current_anim = ""
local facing_right = true

function play(name)
    if current_anim ~= name then
        Engine.play_animation(entity_id, name)
        current_anim = name
    end
end

function is_grounded()
    local px, py = Engine.get_translation(entity_id)
    local hit_id = Engine.raycast(px, py, 0, -1, 0.55, entity_id)
    return hit_id ~= nil
end

function on_create()
    play("walking")
end

function on_fixed_update(dt)
    if not Engine.has_component(entity_id, "RigidBody2D") then return end

    local vx, vy = Engine.get_linear_velocity(entity_id)
    local grounded = is_grounded()

    local shift = Engine.is_key_down("Shift")
    local ctrl = Engine.is_key_down("Ctrl")
    local move_left = Engine.is_key_down("A")
    local move_right = Engine.is_key_down("D")
    local crouch = Engine.is_key_down("S")
    local jump = Engine.is_key_pressed("Space")
    local jump_up = Engine.is_key_pressed("W")

    -- Determine target velocity
    local target_vx = 0
    local speed = fields.move_speed

    if shift then
        speed = fields.run_speed
    end

    if move_left then target_vx = -speed end
    if move_right then target_vx = speed end

    -- Apply movement force
    local force_x = (target_vx - vx) * fields.move_accel
    Engine.apply_force(entity_id, force_x, 0)

    -- Flip sprite
    if target_vx < 0 and facing_right then
        facing_right = false
        Engine.set_scale(entity_id, -1, 1, 1)
    elseif target_vx > 0 and not facing_right then
        facing_right = true
        Engine.set_scale(entity_id, 1, 1, 1)
    end

    local moving = target_vx ~= 0

    -- Jumping (Space = side/forward jump, W = upward jump)
    if jump and grounded then
        play("side_jump")
        Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
        return
    end

    if jump_up and grounded then
        play("upward_jump")
        Engine.apply_impulse(entity_id, 0, fields.jump_impulse)
        return
    end

    -- Airborne states
    if not grounded then
        if vy > 0.1 then
            play("jumping")
        elseif vy < -0.1 then
            play("falling")
        end
        return
    end

    -- Ground states
    if ctrl and moving then
        play("roll")
    elseif crouch and not moving then
        play("crouch")
    elseif moving and shift then
        play("running")
    elseif moving then
        play("walking")
    else
        if current_anim == "running" then
            play("stop_running")
        else
            Engine.stop_animation(entity_id)
            current_anim = ""
        end
    end
end
