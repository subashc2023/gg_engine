-- Level Select Menu
-- Attach to a "GameManager" entity.
--
-- Lists playable scenes. Keyboard (Up/Down/Enter/Escape) + mouse navigation.
-- Selecting a level loads it via Engine.load_scene().
-- Back returns to main_menu.

-- ═══════════════════════════════════════════════════════════════
-- Entity references (populated in on_create)
-- ═══════════════════════════════════════════════════════════════

local entities = {}  -- name → entity_id

-- ═══════════════════════════════════════════════════════════════
-- Navigation state
-- ═══════════════════════════════════════════════════════════════

local selected_index = 1

-- Ordered level entries: { display_name, entity_name, scene_path }
local levels = {
    { name = "Physics Playground",  entity = "Level1Btn",  scene = "assets/scenes/lua_camera_follow.ggscene" },
    { name = "PBR Showcase",        entity = "Level2Btn",  scene = "assets/scenes/pbr_showcase.ggscene" },
    { name = "Particle Demo",       entity = "Level3Btn",  scene = "assets/scenes/particle_test.ggscene" },
    { name = "Audio Test",          entity = "Level4Btn",  scene = "assets/scenes/audio_test.ggscene" },
    { name = "Input Actions",       entity = "Level5Btn",  scene = "assets/scenes/action_test.ggscene" },
}

-- Back button appended as last item
local back_item = { name = "back", entity = "BackBtn", scene = nil }

local FONT_SIZE = 0.40

-- ═══════════════════════════════════════════════════════════════
-- Colors
-- ═══════════════════════════════════════════════════════════════

local COLOR_NORMAL   = { 0.7, 0.7, 0.7, 1.0 }
local COLOR_HOVER    = { 0.3, 0.65, 1.0, 1.0 }

-- ═══════════════════════════════════════════════════════════════
-- UI anchor positions
-- ═══════════════════════════════════════════════════════════════

local ANCHORS = {
    TitleText  = { 0.0, 0.0, 1.5, -1.2 },
    AccentBar  = { 0.0, 0.0, 10.5, -1.8 },
    Level1Btn  = { 0.0, 0.0, 1.5, -2.8 },
    Level2Btn  = { 0.0, 0.0, 1.5, -3.6 },
    Level3Btn  = { 0.0, 0.0, 1.5, -4.4 },
    Level4Btn  = { 0.0, 0.0, 1.5, -5.2 },
    Level5Btn  = { 0.0, 0.0, 1.5, -6.0 },
    BackBtn    = { 0.0, 0.0, 1.5, -7.4 },
    Selector   = { 0.0, 0.0, 0.8, -3.0 },
}

-- ═══════════════════════════════════════════════════════════════
-- Input cooldown
-- ═══════════════════════════════════════════════════════════════

local input_cooldown = 0.0
local INPUT_DELAY = 0.15

-- ═══════════════════════════════════════════════════════════════
-- Build items list (levels + back)
-- ═══════════════════════════════════════════════════════════════

local function get_items()
    local items = {}
    for _, lvl in ipairs(levels) do
        items[#items + 1] = lvl
    end
    items[#items + 1] = back_item
    return items
end

local all_items = nil  -- populated in on_create

-- ═══════════════════════════════════════════════════════════════
-- Lifecycle
-- ═══════════════════════════════════════════════════════════════

function on_create()
    Engine.log("Level Select loaded")

    all_items = get_items()

    -- Find all UI entities by name.
    local names = {
        "TitleText", "AccentBar", "Selector",
        "Level1Btn", "Level2Btn", "Level3Btn", "Level4Btn", "Level5Btn",
        "BackBtn",
    }
    for _, name in ipairs(names) do
        local id = Engine.find_entity_by_name(name)
        if id then
            entities[name] = id
        else
            Engine.log("WARNING: entity '" .. name .. "' not found")
        end
    end

    -- Set level button text.
    for _, lvl in ipairs(levels) do
        if entities[lvl.entity] then
            Engine.set_text(entities[lvl.entity], lvl.name)
        end
    end

    update_colors()
    update_selector()
end

function on_update(dt)
    if input_cooldown > 0 then
        input_cooldown = input_cooldown - dt
    end

    handle_keyboard()
    handle_mouse()
    update_colors()
    update_selector()
end

-- ═══════════════════════════════════════════════════════════════
-- Input handling
-- ═══════════════════════════════════════════════════════════════

function handle_keyboard()
    if input_cooldown > 0 then return end

    local consumed = false

    if Engine.is_key_pressed("Up") or Engine.is_key_pressed("W") then
        selected_index = selected_index - 1
        if selected_index < 1 then selected_index = #all_items end
        consumed = true
    elseif Engine.is_key_pressed("Down") or Engine.is_key_pressed("S") then
        selected_index = selected_index + 1
        if selected_index > #all_items then selected_index = 1 end
        consumed = true
    end

    if Engine.is_key_pressed("Return") or Engine.is_key_pressed("Space") then
        activate_item(all_items[selected_index])
        consumed = true
    end

    if Engine.is_key_pressed("Escape") then
        Engine.load_scene("assets/scenes/main_menu.ggscene")
        consumed = true
    end

    if consumed then
        input_cooldown = INPUT_DELAY
    end
end

function handle_mouse()
    local mx, my = Engine.get_mouse_position()

    local hovered = nil
    for i, item in ipairs(all_items) do
        local eid = entities[item.entity]
        if eid then
            if is_point_over_text(mx, my, eid) then
                hovered = i
                break
            end
        end
    end

    if hovered and hovered ~= selected_index then
        selected_index = hovered
    end

    if hovered and Engine.is_mouse_button_pressed("Left") then
        activate_item(all_items[hovered])
        input_cooldown = INPUT_DELAY
    end
end

function is_point_over_text(mx, my, entity_id)
    local tx, ty = Engine.get_translation(entity_id)
    local text = Engine.get_text(entity_id)
    if not text or #text == 0 then return false end

    local scale = Engine.get_gui_scale()
    local scaled_font = FONT_SIZE * scale
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

    if item.name == "back" then
        Engine.load_scene("assets/scenes/main_menu.ggscene")
    elseif item.scene then
        Engine.log("Loading level: " .. item.name)
        Engine.load_scene(item.scene)
    end
end

-- ═══════════════════════════════════════════════════════════════
-- Display updates
-- ═══════════════════════════════════════════════════════════════

function update_colors()
    for i, item in ipairs(all_items) do
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

function update_selector()
    local item = all_items[selected_index]
    if not item or not entities.Selector or not entities[item.entity] then return end

    local item_anchor = ANCHORS[item.entity]
    if item_anchor then
        local scale = Engine.get_gui_scale()
        local ox = item_anchor[3] - 0.55
        local oy = item_anchor[4] + FONT_SIZE * scale * 0.5
        Engine.set_ui_anchor(entities.Selector,
            item_anchor[1], item_anchor[2], ox, oy)
    end
end

-- ═══════════════════════════════════════════════════════════════
-- Helpers
-- ═══════════════════════════════════════════════════════════════

function set_anchor(name, pos)
    local eid = entities[name]
    if eid and pos then
        Engine.set_ui_anchor(eid, pos[1], pos[2], pos[3], pos[4])
    end
end
