local speed = 5.0

function on_create()
    local x, y, z = Engine.get_translation(entity_id)
    print("[Lua] Entity " .. entity_id .. " spawned at (" .. x .. ", " .. y .. ")")
end

function on_update(dt)
    local x, y, z = Engine.get_translation(entity_id)
    if Engine.is_key_down("W") then y = y + speed * dt end
    if Engine.is_key_down("S") then y = y - speed * dt end
    if Engine.is_key_down("A") then x = x - speed * dt end
    if Engine.is_key_down("D") then x = x + speed * dt end
    Engine.set_translation(entity_id, x, y, z)
end

function on_destroy()
    print("[Lua] Entity " .. entity_id .. " destroyed")
end
