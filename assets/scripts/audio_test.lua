-- audio_test.lua
-- Per-source audio testing with individual toggle keys.
--
-- Controls:
--   1     — toggle Audio Player (looping)
--   2     — toggle Auto Player (one-shot, 1.5x pitch)
--   3     — toggle Spatial Source (3D panning)
--   4     — toggle Streaming Source (from disk)
--   S     — stop all
--   Up    — increase volume (all sources)
--   Down  — decrease volume (all sources)

fields = {
    volume = 1.0,
}

local sources = {}   -- { uuid, playing }
local key_cooldown = 0

function on_create()
    local names = { "Audio Player", "Auto Player", "Spatial Source", "Streaming Source" }
    for _, name in ipairs(names) do
        local uuid = Engine.find_entity_by_name(name)
        if uuid then
            table.insert(sources, { uuid = uuid, playing = false })
        end
    end
    Engine.native_log("[audio_test] Ready. 1-4=toggle, S=stop all, Up/Down=volume", 0)
end

function on_update(dt)
    key_cooldown = key_cooldown - dt

    if key_cooldown <= 0 then
        -- Number keys toggle individual sources
        local keys = { "Key1", "Key2", "Key3", "Key4" }
        for i, key in ipairs(keys) do
            if Engine.is_key_down(key) and sources[i] then
                local src = sources[i]
                if src.playing then
                    Engine.stop_sound(src.uuid)
                    src.playing = false
                    Engine.native_log("[audio_test] Stopped source " .. i, 0)
                else
                    Engine.play_sound(src.uuid)
                    src.playing = true
                    Engine.native_log("[audio_test] Playing source " .. i, 0)
                end
                key_cooldown = 0.3
                return
            end
        end

        -- S = stop all
        if Engine.is_key_down("S") then
            for _, src in ipairs(sources) do
                Engine.stop_sound(src.uuid)
                src.playing = false
            end
            Engine.native_log("[audio_test] Stopped all", 0)
            key_cooldown = 0.3
            return
        end

        -- Volume control
        if Engine.is_key_down("Up") then
            fields.volume = math.min(fields.volume + 0.1, 1.0)
            for _, src in ipairs(sources) do
                Engine.set_volume(src.uuid, fields.volume)
            end
            Engine.native_log("[audio_test] Volume: " .. string.format("%.1f", fields.volume), 0)
            key_cooldown = 0.15
        end

        if Engine.is_key_down("Down") then
            fields.volume = math.max(fields.volume - 0.1, 0.0)
            for _, src in ipairs(sources) do
                Engine.set_volume(src.uuid, fields.volume)
            end
            Engine.native_log("[audio_test] Volume: " .. string.format("%.1f", fields.volume), 0)
            key_cooldown = 0.15
        end
    end
end
