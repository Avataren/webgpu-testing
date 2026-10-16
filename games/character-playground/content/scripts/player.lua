-- The playground's robot: the shared third-person controller (lib/third_person.lua)
-- plus this game's extras: jump and landing sounds, landing dust, a respawn after
-- falling out of the world, and debug keys.
--
-- Controls (F3 switches between the two schemes):
--   "mmo" (World of Warcraft style, the default)
--     W/S  forward / backpedal     A/D  turn (strafe while holding the right button)
--     Q/E  strafe                  left drag   look around    right drag   steer
--     both buttons  run forward    Space  jump    Shift  walk    wheel  zoom
--   "action"
--     WASD  move relative to the camera    click  capture the mouse (Escape releases)
--     right drag  look around    Space  jump    Shift  walk    wheel  zoom
--   F1 colliders    F2 double jump

local ThirdPerson = require("lib.third_person")

local player
local spawn
-- The controller, also reachable from the console: lua_eval {entity = "Player"} with
-- PLAYER.options.turn_rate = 90, for example.
PLAYER = nil
local debug_draw = false
-- Shown by the HUD (on_ui cannot query the physics itself).
HUD = { grounded = false, speed = 0, clip = "", double_jump = false, scheme = "mmo" }

local HELP = {
    mmo = "W/S move · A/D turn · Q/E strafe · left drag look · right drag steer · both buttons run",
    action = "WASD move · click to look (Esc releases) · right drag look",
}

-- A controller with this game's sounds and dust on top.
local function make_player(self, scheme)
    local controller = ThirdPerson.new(self, { camera = "PlayerCamera", scheme = scheme })

    local landed = controller.landed
    function controller:landed(speed)
        landed(self, speed)
        if speed > 5 then
            local p = get_world_translation(self.entity)
            emit_particles(p.x, p.y + 0.05, p.z, {
                count = 28, shape = "ring", speed = 2 + speed * 0.15, life = 0.45,
                color = "#d8d2c8", size = 0.07, gravity = -2,
            })
            play_sound("content/audio/land.wav", { volume = math.min(1, speed / 14) })
        end
    end

    local jumped = controller.jumped
    function controller:jumped()
        jumped(self)
        play_sound("content/audio/jump.wav", { volume = 0.5, pitch = 0.95 + math.random() * 0.1 })
    end
    return controller
end

function on_created(self)
    player = make_player(self, HUD.scheme)
    PLAYER = player
    spawn = get_world_translation(self)
end

function update(self, dt)
    player:update(dt)

    if is_key_just_pressed("F1") then
        debug_draw = not debug_draw
        physics_debug_draw(debug_draw)
    end
    if is_key_just_pressed("F2") then
        HUD.double_jump = not HUD.double_jump
        set_component(self, "CharacterController", { air_jumps = HUD.double_jump and 1 or 0 })
    end
    if is_key_just_pressed("F3") then
        player:release()
        set_cursor_locked(false)
        HUD.scheme = HUD.scheme == "mmo" and "action" or "mmo"
        player = make_player(self, HUD.scheme)
        PLAYER = player
    end

    local p = get_world_translation(self)
    if p and p.y < -15 then
        set_translation(self, spawn.x, spawn.y + 1, spawn.z)
        set_velocity(self, 0, 0, 0)
    end

    local s = player.state
    if s then
        HUD.grounded = s.grounded
        HUD.speed = s.horizontal_speed
        HUD.clip = player.clip or ""
    end
end

function on_jumped(self) player:jumped() end
function on_landed(self, speed) player:landed(speed) end

function on_destroyed(self, reason)
    if player then player:release() end
end

function on_ui(self, ui)
    ui:anchored_panel("controls", "bottom_left", 14, 14, nil, nil, function(panel)
        panel:label(HELP[HUD.scheme] .. " · Space jump · Shift walk · wheel zoom")
        panel:label(string.format("F1 colliders · F2 double jump (%s) · F3 controls (%s)   %.1f m/s  %s  %s",
            HUD.double_jump and "on" or "off", HUD.scheme, HUD.speed,
            HUD.grounded and "grounded" or "airborne", HUD.clip))
    end)
end
