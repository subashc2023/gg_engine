-- action_test.lua
-- Test script for input action mapping.
-- Attach to any entity to verify actions are working.
-- Logs action state and moves the entity using action values.

fields = {
    move_speed = 3.0,
}

function on_create()
    Engine.log("action_test: on_create — action mapping test active")
end

function on_update(dt)
    -- Query axis actions
    local move_h = Engine.get_action_value("move_horizontal")
    local move_v = Engine.get_action_value("move_vertical")

    -- Move entity using action values
    if math.abs(move_h) > 0.01 or math.abs(move_v) > 0.01 then
        local px, py, pz = Engine.get_translation(entity_id)
        local speed = fields.move_speed
        if Engine.is_action_pressed("sprint") then
            speed = speed * 2
        end
        px = px + move_h * speed * dt
        py = py + move_v * speed * dt
        Engine.set_translation(entity_id, px, py, pz)
    end

    -- Log button actions on transitions
    if Engine.is_action_just_pressed("jump") then
        Engine.log("ACTION: jump PRESSED")
    end
    if Engine.is_action_just_released("jump") then
        Engine.log("ACTION: jump RELEASED")
    end
    if Engine.is_action_just_pressed("sprint") then
        Engine.log("ACTION: sprint PRESSED")
    end
    if Engine.is_action_just_released("sprint") then
        Engine.log("ACTION: sprint RELEASED")
    end

    -- Log axis values when non-zero (throttled: only when they change significantly)
    if math.abs(move_h) > 0.01 or math.abs(move_v) > 0.01 then
        -- Only log periodically to avoid spam
        if not self_log_timer then self_log_timer = 0 end
        self_log_timer = self_log_timer + dt
        if self_log_timer > 0.5 then
            Engine.log(string.format("ACTION AXES: h=%.2f v=%.2f", move_h, move_v))
            self_log_timer = 0
        end
    end
end
