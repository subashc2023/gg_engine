-- Main Menu / Feature Test Script
-- Attach to a "GameManager" entity. Tests cursor modes, resolution, etc.
--
-- Controls:
--   1       Cursor mode: Normal
--   2       Cursor mode: Confined (software cursor)
--   3       Cursor mode: Locked (FPS raw deltas)
--   Escape  Back to Normal from any mode
--   F1      Resolution: 1280x720
--   F2      Resolution: 1920x1080
--   F3      Resolution: 2560x1440

local status_entity = nil
local cursor_info_entity = nil
local mouse_info_entity = nil
local resolution_info_entity = nil

local current_cursor_mode = "normal"

function on_create()
    Engine.log("Main Menu loaded — press 1/2/3 to change cursor mode, F1-F3 for resolution")

    -- Find the display entities by name.
    status_entity = Engine.find_entity_by_name("StatusText")
    cursor_info_entity = Engine.find_entity_by_name("CursorInfoText")
    mouse_info_entity = Engine.find_entity_by_name("MouseInfoText")
    resolution_info_entity = Engine.find_entity_by_name("ResolutionInfoText")

    -- Start in confined mode to show the software cursor.
    Engine.set_cursor_mode("confined")
    current_cursor_mode = "confined"
    update_status()
    update_resolution_display()
end

function on_update(dt)
    -- Cursor mode switching.
    if Engine.is_key_pressed("1") or Engine.is_key_pressed("Escape") then
        Engine.set_cursor_mode("normal")
        current_cursor_mode = "normal"
        update_status()
    elseif Engine.is_key_pressed("2") then
        Engine.set_cursor_mode("confined")
        current_cursor_mode = "confined"
        update_status()
    elseif Engine.is_key_pressed("3") then
        Engine.set_cursor_mode("locked")
        current_cursor_mode = "locked"
        update_status()
    end

    -- Resolution switching.
    if Engine.is_key_pressed("F1") then
        Engine.set_window_size(1280, 720)
        update_resolution_display()
    elseif Engine.is_key_pressed("F2") then
        Engine.set_window_size(1920, 1080)
        update_resolution_display()
    elseif Engine.is_key_pressed("F3") then
        Engine.set_window_size(2560, 1440)
        update_resolution_display()
    end

    -- Update live mouse telemetry.
    if mouse_info_entity then
        local mx, my = Engine.get_mouse_position()
        local text = string.format("Mouse pos: (%.2f, %.2f)", mx, my)
        Engine.set_text(mouse_info_entity, text)
    end

    -- Show current cursor mode readback from engine.
    if cursor_info_entity then
        local mode = Engine.get_cursor_mode()
        local desc = ""
        if mode == "normal" then
            desc = "OS cursor visible, no grab"
        elseif mode == "confined" then
            desc = "OS cursor hidden, software cursor, confined"
        elseif mode == "locked" then
            desc = "OS cursor hidden + locked, raw deltas only"
        end
        Engine.set_text(cursor_info_entity, "Engine reports: " .. mode .. "\n" .. desc)
    end
end

function update_status()
    if status_entity then
        local lines = {
            "CURSOR MODE: " .. string.upper(current_cursor_mode),
            "",
            "[1/Esc] Normal   [2] Confined   [3] Locked",
            "[F1] 1280x720   [F2] 1920x1080   [F3] 2560x1440",
        }
        Engine.set_text(status_entity, table.concat(lines, "\n"))
    end
    Engine.log("Cursor mode -> " .. current_cursor_mode)
end

function update_resolution_display()
    if resolution_info_entity then
        local w, h = Engine.get_window_size()
        Engine.set_text(resolution_info_entity, string.format("Window: %dx%d", w, h))
    end
end

function on_destroy()
    -- Restore normal cursor when scene unloads.
    Engine.set_cursor_mode("normal")
end
