-- hrtf_test.lua
-- 3D HRTF spatial audio test with first-person camera.
--
-- Put on headphones! Press H to toggle HRTF and hear the difference.
--
-- Controls:
--   WASD      — move camera
--   Mouse     — look around
--   Space     — toggle orbit / pause
--   H         — toggle HRTF on/off
--   1/2       — decrease/increase orbit speed
--   3/4       — decrease/increase orbit radius
--   Up/Down   — volume
--   R         — reset
--   Escape    — release mouse cursor

local camera = nil
local orbit_source = nil
local static_source = nil

local yaw = 0.0
local pitch = 0.0
local angle = 0.0
local orbit_speed = 1.0
local orbit_radius = 8.0
local orbiting = true
local hrtf_on = true
local volume = 0.8
local key_cooldown = 0
local move_speed = 5.0
local mouse_sensitivity = 0.003
local cursor_locked = false
local started = false
local orbit_playing = false
local static_playing = false

function on_create()
    camera = Engine.find_entity_by_name("Camera")
    orbit_source = Engine.find_entity_by_name("Orbiting Source")
    static_source = Engine.find_entity_by_name("Static Source")

    -- Lock cursor for FPS controls
    Engine.set_cursor_mode("locked")
    cursor_locked = true

    Engine.log("[HRTF Test] Ready! Use headphones.")
    Engine.log("  WASD = move, Mouse = look")
    Engine.log("  F/G = toggle orbit/static sound")
    Engine.log("  H = toggle HRTF, Space = pause orbit")
    Engine.log("  1/2 = speed, 3/4 = radius, R = reset")
end

function on_update(dt)
    -- Deferred start: audio engine isn't ready during on_create
    if not started then
        started = true
        if orbit_source then Engine.play_sound(orbit_source); orbit_playing = true end
        if static_source then Engine.play_sound(static_source); static_playing = true end
    end

    key_cooldown = key_cooldown - dt

    -- ── FPS Camera ──────────────────────────────────────────
    if camera and cursor_locked then
        local dx, dy = Engine.get_mouse_delta()
        yaw = yaw + dx * mouse_sensitivity
        pitch = pitch + dy * mouse_sensitivity
        -- Clamp pitch to avoid gimbal lock
        if pitch > 1.5 then pitch = 1.5 end
        if pitch < -1.5 then pitch = -1.5 end

        -- Build quaternion: yaw around Y, then pitch around local X
        local half_yaw = yaw * 0.5
        local half_pitch = pitch * 0.5
        local sy = math.sin(half_yaw)
        local cy = math.cos(half_yaw)
        local sp = math.sin(half_pitch)
        local cp = math.cos(half_pitch)

        -- Quat = yaw_quat * pitch_quat
        local qx = cy * sp
        local qy = sy * cp
        local qz = -sy * sp
        local qw = cy * cp

        Engine.set_rotation_quat(camera, qx, qy, qz, qw)

        -- WASD movement relative to camera facing
        local cx, cy_pos, cz = Engine.get_translation(camera)
        local forward_x = -math.sin(yaw)
        local forward_z = -math.cos(yaw)
        local right_x = math.cos(yaw)
        local right_z = -math.sin(yaw)

        local mx, mz = 0, 0
        if Engine.is_key_down("W") then mx = mx + forward_x; mz = mz + forward_z end
        if Engine.is_key_down("S") then mx = mx - forward_x; mz = mz - forward_z end
        if Engine.is_key_down("D") then mx = mx + right_x; mz = mz + right_z end
        if Engine.is_key_down("A") then mx = mx - right_x; mz = mz - right_z end

        local len = math.sqrt(mx * mx + mz * mz)
        if len > 0.001 then
            mx = mx / len * move_speed * dt
            mz = mz / len * move_speed * dt
            Engine.set_translation(camera, cx + mx, cy_pos, cz + mz)
        end
    end

    -- Escape to toggle cursor lock
    if Engine.is_key_pressed("Escape") then
        cursor_locked = not cursor_locked
        if cursor_locked then
            Engine.set_cursor_mode("locked")
        else
            Engine.set_cursor_mode("normal")
        end
    end

    -- ── Orbit source ────────────────────────────────────────
    if orbiting and orbit_source then
        angle = angle + orbit_speed * dt
        if angle > math.pi * 2 then
            angle = angle - math.pi * 2
        end
        local x = math.sin(angle) * orbit_radius
        local z = -math.cos(angle) * orbit_radius
        Engine.set_translation(orbit_source, x, 1.0, z)
    end

    if key_cooldown > 0 then return end

    -- Space: toggle orbit
    if Engine.is_key_pressed("Space") then
        orbiting = not orbiting
        Engine.log("[HRTF] Orbit " .. (orbiting and "ON" or "PAUSED"))
        key_cooldown = 0.3
    end

    -- F: toggle orbiting source on/off
    if Engine.is_key_pressed("F") then
        if orbit_source then
            local playing = Engine.is_sound_playing and Engine.is_sound_playing(orbit_source)
            -- Simple toggle: stop or play
            if orbit_playing then
                Engine.stop_sound(orbit_source)
                orbit_playing = false
                Engine.log("[HRTF] Orbiting source: OFF")
            else
                Engine.play_sound(orbit_source)
                orbit_playing = true
                Engine.log("[HRTF] Orbiting source: ON")
            end
        end
        key_cooldown = 0.3
    end

    -- G: toggle static source on/off
    if Engine.is_key_pressed("G") then
        if static_source then
            if static_playing then
                Engine.stop_sound(static_source)
                static_playing = false
                Engine.log("[HRTF] Static source: OFF")
            else
                Engine.play_sound(static_source)
                static_playing = true
                Engine.log("[HRTF] Static source: ON")
            end
        end
        key_cooldown = 0.3
    end

    -- H: toggle HRTF
    if Engine.is_key_pressed("H") then
        hrtf_on = not hrtf_on
        if orbit_source then Engine.set_hrtf(orbit_source, hrtf_on) end
        if static_source then Engine.set_hrtf(static_source, hrtf_on) end
        Engine.log("[HRTF] Binaural: " .. (hrtf_on and "ON" or "OFF (simple panning)"))
        key_cooldown = 0.3
    end

    -- 1/2: orbit speed
    if Engine.is_key_pressed("Key1") then
        orbit_speed = math.max(0.1, orbit_speed - 0.2)
        Engine.log("[HRTF] Speed: " .. string.format("%.1f", orbit_speed))
        key_cooldown = 0.15
    end
    if Engine.is_key_pressed("Key2") then
        orbit_speed = math.min(5.0, orbit_speed + 0.2)
        Engine.log("[HRTF] Speed: " .. string.format("%.1f", orbit_speed))
        key_cooldown = 0.15
    end

    -- 3/4: radius
    if Engine.is_key_pressed("Key3") then
        orbit_radius = math.max(2.0, orbit_radius - 1.0)
        Engine.log("[HRTF] Radius: " .. string.format("%.0f", orbit_radius))
        key_cooldown = 0.15
    end
    if Engine.is_key_pressed("Key4") then
        orbit_radius = math.min(30.0, orbit_radius + 1.0)
        Engine.log("[HRTF] Radius: " .. string.format("%.0f", orbit_radius))
        key_cooldown = 0.15
    end

    -- Volume
    if Engine.is_key_down("Up") then
        volume = math.min(1.0, volume + 0.1)
        if orbit_source then Engine.set_volume(orbit_source, volume) end
        if static_source then Engine.set_volume(static_source, volume) end
        Engine.log("[HRTF] Volume: " .. string.format("%.1f", volume))
        key_cooldown = 0.15
    end
    if Engine.is_key_down("Down") then
        volume = math.max(0.0, volume - 0.1)
        if orbit_source then Engine.set_volume(orbit_source, volume) end
        if static_source then Engine.set_volume(static_source, volume) end
        Engine.log("[HRTF] Volume: " .. string.format("%.1f", volume))
        key_cooldown = 0.15
    end

    -- R: reset
    if Engine.is_key_pressed("R") then
        orbit_speed = 1.0
        orbit_radius = 8.0
        orbiting = true
        volume = 0.8
        hrtf_on = true
        angle = 0
        yaw = 0
        pitch = 0
        if camera then
            Engine.set_translation(camera, 0, 1.5, 0)
            Engine.set_rotation_quat(camera, 0, 0, 0, 1)
        end
        if orbit_source then
            Engine.set_hrtf(orbit_source, true)
            Engine.set_volume(orbit_source, volume)
            if not orbit_playing then Engine.play_sound(orbit_source); orbit_playing = true end
        end
        if static_source then
            Engine.set_hrtf(static_source, true)
            Engine.set_volume(static_source, volume)
            if not static_playing then Engine.play_sound(static_source); static_playing = true end
        end
        Engine.log("[HRTF] Reset to defaults")
        key_cooldown = 0.3
    end
end
