-- Main Menu System
-- Attach to a "GameManager" entity.
--
-- Features:
--   - Two pages: Main Menu and Settings
--   - Keyboard navigation (Up/Down/Enter/Escape/Left/Right)
--   - Mouse hover + click support
--   - Runtime settings: Resolution, Window Mode, VSync, Shadow Quality, GUI Scale
--   - Software cursor (CursorMode::Confined)

-- ═══════════════════════════════════════════════════════════════
-- Entity references (populated in on_create)
-- ═══════════════════════════════════════════════════════════════

local entities = {}  -- name → entity_id

-- ═══════════════════════════════════════════════════════════════
-- Page & navigation state
-- ═══════════════════════════════════════════════════════════════

local current_page = "main"  -- "main" or "settings"
local selected_index = 1

-- Ordered menu items per page: { {name, entity_name, font_size}, ... }
local main_items = {
    { name = "play",     entity = "PlayBtn",     font_size = 0.45 },
    { name = "settings", entity = "SettingsBtn", font_size = 0.45 },
    { name = "quit",     entity = "QuitBtn",     font_size = 0.45 },
}

local settings_items = {
    { name = "resolution",  entity = "ResolutionVal",  font_size = 0.38 },
    { name = "window_mode", entity = "WindowModeVal",  font_size = 0.38 },
    { name = "vsync",       entity = "VsyncVal",       font_size = 0.38 },
    { name = "shadows",     entity = "ShadowVal",      font_size = 0.38 },
    { name = "gui_scale",   entity = "GuiScaleVal",    font_size = 0.38 },
    { name = "back",        entity = "BackBtn",        font_size = 0.38 },
}

-- ═══════════════════════════════════════════════════════════════
-- Settings state
-- ═══════════════════════════════════════════════════════════════

local resolutions = {
    { 1280,  720  },
    { 1600,  900  },
    { 1920,  1080 },
    { 2560,  1440 },
}
local resolution_index = 1

local shadow_names = { "Low", "Medium", "High", "Ultra" }

local window_modes = { "windowed", "borderless", "exclusive" }
local window_mode_labels = { windowed = "Windowed", borderless = "Borderless", exclusive = "Exclusive" }

local gui_scale_presets = { 0.75, 1.0, 1.25, 1.5 }

-- ═══════════════════════════════════════════════════════════════
-- Colors
-- ═══════════════════════════════════════════════════════════════

local COLOR_NORMAL   = { 0.7, 0.7, 0.7, 1.0 }
local COLOR_HOVER    = { 0.3, 0.65, 1.0, 1.0 }
local COLOR_ACTIVE   = { 1.0, 0.85, 0.25, 1.0 }

-- ═══════════════════════════════════════════════════════════════
-- UI anchor positions
-- ═══════════════════════════════════════════════════════════════

-- Main menu anchors (anchor_x, anchor_y, offset_x, offset_y)
local MAIN_ANCHORS = {
    TitleText    = { 0.0, 0.0, 1.5, -1.2 },
    SubtitleText = { 0.0, 0.0, 1.5, -2.0 },
    AccentBar    = { 0.0, 0.0, 10.5, -2.2 },
    PlayBtn      = { 0.0, 0.0, 1.5, -3.5 },
    SettingsBtn  = { 0.0, 0.0, 1.5, -4.5 },
    QuitBtn      = { 0.0, 0.0, 1.5, -5.5 },
    Selector     = { 0.0, 0.0, 0.8, -3.7 },
}

-- Settings page anchors
local SETTINGS_ANCHORS = {
    SettingsTitle = { 0.0, 0.0, 1.5, -1.2 },
    AccentBar     = { 0.0, 0.0, 10.5, -2.2 },
    ResolutionVal = { 0.0, 0.0, 1.5, -3.0 },
    WindowModeVal = { 0.0, 0.0, 1.5, -3.8 },
    VsyncVal      = { 0.0, 0.0, 1.5, -4.6 },
    ShadowVal     = { 0.0, 0.0, 1.5, -5.4 },
    GuiScaleVal   = { 0.0, 0.0, 1.5, -6.2 },
    BackBtn       = { 0.0, 0.0, 1.5, -7.6 },
    Selector      = { 0.0, 0.0, 0.8, -3.2 },
}

-- Off-screen position for hidden entities
local OFF_SCREEN = { 0.0, 0.0, 200.0, 0.0 }

-- ═══════════════════════════════════════════════════════════════
-- Input cooldown to prevent key repeat stutter
-- ═══════════════════════════════════════════════════════════════

local input_cooldown = 0.0
local INPUT_DELAY = 0.15  -- seconds between accepted inputs

-- ═══════════════════════════════════════════════════════════════
-- Lifecycle
-- ═══════════════════════════════════════════════════════════════

function on_create()
    Engine.log("Main Menu loaded")

    -- Find all UI entities by name.
    local names = {
        "TitleText", "SubtitleText", "AccentBar", "Selector", "StatusText",
        "PlayBtn", "SettingsBtn", "QuitBtn",
        "SettingsTitle", "ResolutionVal", "WindowModeVal", "VsyncVal", "ShadowVal",
        "GuiScaleVal", "BackBtn",
    }
    for _, name in ipairs(names) do
        local id = Engine.find_entity_by_name(name)
        if id then
            entities[name] = id
        else
            Engine.log("WARNING: entity '" .. name .. "' not found")
        end
    end

    -- Detect current resolution.
    local w, h = Engine.get_window_size()
    for i, res in ipairs(resolutions) do
        if res[1] == w and res[2] == h then
            resolution_index = i
            break
        end
    end

    -- Show main menu.
    switch_to_main()
    update_settings_display()
    update_status()
end

function on_update(dt)
    -- Tick input cooldown.
    if input_cooldown > 0 then
        input_cooldown = input_cooldown - dt
    end

    local items = get_current_items()

    -- Keyboard navigation.
    handle_keyboard(items)

    -- Mouse hover + click.
    handle_mouse(items)

    -- Update visual state.
    update_colors(items)
    update_selector(items)
end

function on_destroy()
    -- Cursor mode is carried forward by the player on scene transitions.
end

-- ═══════════════════════════════════════════════════════════════
-- Input handling
-- ═══════════════════════════════════════════════════════════════

function handle_keyboard(items)
    if input_cooldown > 0 then return end

    local consumed = false

    -- Navigation: Up / Down / W / S
    if Engine.is_key_pressed("Up") or Engine.is_key_pressed("W") then
        selected_index = selected_index - 1
        if selected_index < 1 then selected_index = #items end
        consumed = true
    elseif Engine.is_key_pressed("Down") or Engine.is_key_pressed("S") then
        selected_index = selected_index + 1
        if selected_index > #items then selected_index = 1 end
        consumed = true
    end

    -- Activate: Enter / Space
    if Engine.is_key_pressed("Return") or Engine.is_key_pressed("Space") then
        activate_item(items[selected_index])
        consumed = true
    end

    -- Settings: Left / Right to cycle values
    if current_page == "settings" then
        if Engine.is_key_pressed("Left") or Engine.is_key_pressed("A") then
            cycle_setting(items[selected_index], -1)
            consumed = true
        elseif Engine.is_key_pressed("Right") or Engine.is_key_pressed("D") then
            cycle_setting(items[selected_index], 1)
            consumed = true
        end
    end

    -- Escape: go back
    if Engine.is_key_pressed("Escape") then
        if current_page == "settings" then
            switch_to_main()
            consumed = true
        end
    end

    if consumed then
        input_cooldown = INPUT_DELAY
    end
end

function handle_mouse(items)
    local mx, my = Engine.get_mouse_position()

    -- Check hover over each menu item.
    local hovered = nil
    for i, item in ipairs(items) do
        local eid = entities[item.entity]
        if eid then
            if is_point_over_text(mx, my, eid, item.font_size) then
                hovered = i
                break
            end
        end
    end

    -- Update selection on hover.
    if hovered and hovered ~= selected_index then
        selected_index = hovered
    end

    -- Click to activate.
    if hovered and Engine.is_mouse_button_pressed("Left") then
        activate_item(items[hovered])
        input_cooldown = INPUT_DELAY
    end
end

--- Approximate hit test for a text entity using monospace character width.
function is_point_over_text(mx, my, entity_id, font_size)
    local tx, ty = Engine.get_translation(entity_id)
    local text = Engine.get_text(entity_id)
    if not text or #text == 0 then return false end

    -- Approximate bounds. Monospace: each char ~ font_size * 0.55 wide.
    -- Text renders UPWARD from anchor (ty = baseline).
    -- Scale by GUI scale (engine scales rendering, we must match hit bounds).
    local scale = Engine.get_gui_scale()
    local scaled_font = font_size * scale
    local char_width = scaled_font * 0.55
    local text_width = #text * char_width

    return mx >= tx and mx <= tx + text_width
       and my >= ty and my <= ty + scaled_font
end

-- ═══════════════════════════════════════════════════════════════
-- Item activation
-- ═══════════════════════════════════════════════════════════════

function activate_item(item)
    if not item then return end

    if item.name == "play" then
        Engine.load_scene("assets/scenes/level_select.ggscene")
    elseif item.name == "settings" then
        switch_to_settings()
    elseif item.name == "quit" then
        Engine.quit()
    elseif item.name == "back" then
        switch_to_main()
    else
        -- Settings items: toggle on Enter
        cycle_setting(item, 1)
    end
end

function cycle_setting(item, direction)
    if not item then return end

    if item.name == "resolution" then
        resolution_index = ((resolution_index - 1 + direction) % #resolutions) + 1
        local r = resolutions[resolution_index]
        Engine.set_window_size(r[1], r[2])
    elseif item.name == "window_mode" then
        local current = Engine.get_fullscreen()
        local current_index = 1
        for i, m in ipairs(window_modes) do
            if m == current then current_index = i break end
        end
        local next_index = ((current_index - 1 + direction) % #window_modes) + 1
        Engine.set_fullscreen(window_modes[next_index])
    elseif item.name == "vsync" then
        local current = Engine.get_vsync()
        Engine.set_vsync(not current)
    elseif item.name == "shadows" then
        local current = Engine.get_shadow_quality()
        local next = (current + direction) % 4
        if next < 0 then next = 3 end
        Engine.set_shadow_quality(next)
    elseif item.name == "gui_scale" then
        local current = Engine.get_gui_scale()
        local current_index = 1
        for i, s in ipairs(gui_scale_presets) do
            if math.abs(s - current) < 0.01 then current_index = i break end
        end
        local next_index = ((current_index - 1 + direction) % #gui_scale_presets) + 1
        Engine.set_gui_scale(gui_scale_presets[next_index])
    end

    update_settings_display()
    update_status()
end

-- ═══════════════════════════════════════════════════════════════
-- Page switching
-- ═══════════════════════════════════════════════════════════════

function switch_to_main()
    current_page = "main"
    selected_index = 1

    -- Show main menu entities.
    set_anchor("TitleText",    MAIN_ANCHORS.TitleText)
    set_anchor("SubtitleText", MAIN_ANCHORS.SubtitleText)
    set_anchor("AccentBar",    MAIN_ANCHORS.AccentBar)
    set_anchor("PlayBtn",      MAIN_ANCHORS.PlayBtn)
    set_anchor("SettingsBtn",  MAIN_ANCHORS.SettingsBtn)
    set_anchor("QuitBtn",      MAIN_ANCHORS.QuitBtn)
    set_anchor("Selector",     MAIN_ANCHORS.Selector)

    -- Hide settings entities.
    set_anchor("SettingsTitle", OFF_SCREEN)
    set_anchor("ResolutionVal", OFF_SCREEN)
    set_anchor("WindowModeVal", OFF_SCREEN)
    set_anchor("VsyncVal",      OFF_SCREEN)
    set_anchor("ShadowVal",     OFF_SCREEN)
    set_anchor("GuiScaleVal",   OFF_SCREEN)
    set_anchor("BackBtn",       OFF_SCREEN)

    update_status()
end

function switch_to_settings()
    current_page = "settings"
    selected_index = 1

    -- Hide main menu entities.
    set_anchor("TitleText",    OFF_SCREEN)
    set_anchor("SubtitleText", OFF_SCREEN)
    set_anchor("PlayBtn",      OFF_SCREEN)
    set_anchor("SettingsBtn",  OFF_SCREEN)
    set_anchor("QuitBtn",      OFF_SCREEN)

    -- Show settings entities.
    set_anchor("AccentBar",     SETTINGS_ANCHORS.AccentBar)
    set_anchor("SettingsTitle", SETTINGS_ANCHORS.SettingsTitle)
    set_anchor("ResolutionVal", SETTINGS_ANCHORS.ResolutionVal)
    set_anchor("WindowModeVal", SETTINGS_ANCHORS.WindowModeVal)
    set_anchor("VsyncVal",      SETTINGS_ANCHORS.VsyncVal)
    set_anchor("ShadowVal",     SETTINGS_ANCHORS.ShadowVal)
    set_anchor("GuiScaleVal",   SETTINGS_ANCHORS.GuiScaleVal)
    set_anchor("BackBtn",       SETTINGS_ANCHORS.BackBtn)
    set_anchor("Selector",      SETTINGS_ANCHORS.Selector)

    update_settings_display()
    update_status()
end

-- ═══════════════════════════════════════════════════════════════
-- Display updates
-- ═══════════════════════════════════════════════════════════════

function update_settings_display()
    -- Resolution
    if entities.ResolutionVal then
        local r = resolutions[resolution_index]
        Engine.set_text(entities.ResolutionVal,
            string.format("Resolution:  %d x %d", r[1], r[2]))
    end

    -- Window mode
    if entities.WindowModeVal then
        local mode = Engine.get_fullscreen()
        Engine.set_text(entities.WindowModeVal,
            "Window Mode:  " .. (window_mode_labels[mode] or mode))
    end

    -- VSync
    if entities.VsyncVal then
        local on = Engine.get_vsync()
        Engine.set_text(entities.VsyncVal, "VSync:  " .. (on and "ON" or "OFF"))
    end

    -- Shadows
    if entities.ShadowVal then
        local q = Engine.get_shadow_quality()
        Engine.set_text(entities.ShadowVal, "Shadows:  " .. shadow_names[q + 1])
    end

    -- GUI Scale
    if entities.GuiScaleVal then
        local s = Engine.get_gui_scale()
        Engine.set_text(entities.GuiScaleVal,
            string.format("GUI Scale:  %.0f%%", s * 100))
    end
end

function update_status()
    if not entities.StatusText then return end

    -- Use the selected preset resolution, not live window size
    -- (live size can differ due to DPI scaling or window chrome).
    local r = resolutions[resolution_index]
    local mode = Engine.get_fullscreen()
    local vsync = Engine.get_vsync() and "ON" or "OFF"
    local sq = shadow_names[Engine.get_shadow_quality() + 1]
    local gs = string.format("%.0f%%", Engine.get_gui_scale() * 100)

    local text = string.format(
        "%dx%d  |  %s  |  VSync: %s  |  Shadows: %s  |  GUI: %s",
        r[1], r[2], window_mode_labels[mode] or mode, vsync, sq, gs
    )
    Engine.set_text(entities.StatusText, text)
end

function update_colors(items)
    for i, item in ipairs(items) do
        local eid = entities[item.entity]
        if eid then
            if i == selected_index then
                Engine.set_text_color(eid, COLOR_HOVER[1], COLOR_HOVER[2], COLOR_HOVER[3], COLOR_HOVER[4])
            else
                Engine.set_text_color(eid, COLOR_NORMAL[1], COLOR_NORMAL[2], COLOR_NORMAL[3], COLOR_NORMAL[4])
            end
        end
    end
end

function update_selector(items)
    local item = items[selected_index]
    if not item or not entities.Selector or not entities[item.entity] then return end

    -- Must use set_ui_anchor (not set_translation) because apply_ui_anchors()
    -- overwrites translation every frame for entities with a UIAnchorComponent.
    local anchors = current_page == "main" and MAIN_ANCHORS or SETTINGS_ANCHORS
    local item_anchor = anchors[item.entity]
    if item_anchor then
        local scale = Engine.get_gui_scale()
        local ox = item_anchor[3] - 0.55
        -- Text renders UPWARD from anchor (anchor = baseline).
        -- Center the selector on the text by going UP by half the scaled font size.
        local oy = item_anchor[4] + item.font_size * scale * 0.5
        Engine.set_ui_anchor(entities.Selector,
            item_anchor[1], item_anchor[2], ox, oy)
    else
        Engine.log("WARNING: no anchor found for " .. item.entity)
    end
end

-- ═══════════════════════════════════════════════════════════════
-- Helpers
-- ═══════════════════════════════════════════════════════════════

function get_current_items()
    return current_page == "main" and main_items or settings_items
end

function set_anchor(name, pos)
    local eid = entities[name]
    if eid and pos then
        Engine.set_ui_anchor(eid, pos[1], pos[2], pos[3], pos[4])
    end
end
