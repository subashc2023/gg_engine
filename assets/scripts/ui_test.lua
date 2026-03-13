-- UI Test Script
-- Attach to any entity. Tests UIRect + UIImage + UIInteractable + UILayout.
--
-- Button entities have UIInteractableComponent (auto hover/press color tinting)
-- and TextComponent (auto-centered inside UIRect). This script polls
-- Engine.get_ui_state() to detect clicks and update the counter.

local entities = {}
local click_count = 0

function on_create()
    Engine.log("UI Test: loaded")
    local names = {
        "PlayBtnBg", "ResetBtnBg", "QuitBtnBg",
        "ClickCounter", "StatusText",
    }
    for _, name in ipairs(names) do
        local id = Engine.find_entity_by_name(name)
        if id then
            entities[name] = id
        else
            Engine.log("WARNING: entity '" .. name .. "' not found")
        end
    end
end

local was_pressed = false

function on_update(dt)
    local mouse_down = Engine.is_mouse_button_down("Left")
    local just_released = was_pressed and not mouse_down
    was_pressed = mouse_down

    -- Check each button's UI state.
    for _, btn in ipairs({"PlayBtnBg", "ResetBtnBg", "QuitBtnBg"}) do
        local id = entities[btn]
        if id then
            local state = Engine.get_ui_state(id)
            if state == "hovered" then
                set_status("Hovering: " .. btn)
            end
            -- Detect click (mouse released while hovered or pressed).
            if just_released and (state == "hovered" or state == "pressed") then
                handle_click(btn)
            end
        end
    end
end

function handle_click(btn_name)
    if btn_name == "PlayBtnBg" then
        click_count = click_count + 1
        if entities.ClickCounter then
            Engine.set_text(entities.ClickCounter, "Clicks: " .. tostring(click_count))
        end
        set_status("Clicked! Count: " .. tostring(click_count))
    elseif btn_name == "ResetBtnBg" then
        click_count = 0
        if entities.ClickCounter then
            Engine.set_text(entities.ClickCounter, "Clicks: 0")
        end
        set_status("Counter reset!")
    elseif btn_name == "QuitBtnBg" then
        set_status("Quit pressed")
        Engine.log("Quit pressed")
    end
end

function set_status(text)
    if entities.StatusText then
        Engine.set_text(entities.StatusText, text)
    end
end
