-- audio_test.lua
-- Tests the Audio Lua API: play_sound / stop_sound / set_volume
--
-- Controls:
--   P     — play sound
--   S     — stop sound
--   Up    — increase volume
--   Down  — decrease volume

fields = {
    volume = 1.0,
}

local key_cooldown = 0

function on_create()
    Engine.native_log("[audio_test] Ready. P=play, S=stop, Up/Down=volume", 0)
end

function on_update(dt)
    key_cooldown = key_cooldown - dt

    if Engine.is_key_down("P") and key_cooldown <= 0 then
        Engine.play_sound(entity_id)
        Engine.native_log("[audio_test] Playing sound", 0)
        key_cooldown = 0.3
    end

    if Engine.is_key_down("S") and key_cooldown <= 0 then
        Engine.stop_sound(entity_id)
        Engine.native_log("[audio_test] Stopped sound", 0)
        key_cooldown = 0.3
    end

    if Engine.is_key_down("Up") and key_cooldown <= 0 then
        fields.volume = math.min(fields.volume + 0.1, 1.0)
        Engine.set_volume(entity_id, fields.volume)
        Engine.native_log("[audio_test] Volume:", fields.volume)
        key_cooldown = 0.15
    end

    if Engine.is_key_down("Down") and key_cooldown <= 0 then
        fields.volume = math.max(fields.volume - 0.1, 0.0)
        Engine.set_volume(entity_id, fields.volume)
        Engine.native_log("[audio_test] Volume:", fields.volume)
        key_cooldown = 0.15
    end
end
