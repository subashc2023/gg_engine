-- Game Session Manager
-- Attach to any gameplay scene. Press Escape to return to level select.

function on_update(dt)
    if Engine.is_key_pressed("Escape") then
        Engine.load_scene("assets/scenes/level_select.ggscene")
    end
end
