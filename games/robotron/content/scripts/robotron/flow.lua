-- Game flow: the state machine (running, dying, clearing, transition, game over), new games and
-- the keys outside play (start, pause, autopilot, back to the demo).

local C = require("robotron.config")
local START_LIVES, EXTRA_LIFE, FR, GRUNT_STEP = C.START_LIVES, C.EXTRA_LIFE, C.FR, C.GRUNT_STEP
local FLASH = C.FLASH
local S = require("robotron.state")
local g, units, sfx = S.g, S.units, S.sfx
local Fx = require("robotron.fx")
local explode, ring, add_shake = Fx.explode, Fx.ring, Fx.add_shake
local flash_light, hit_flash, burst = Fx.flash_light, Fx.hit_flash, Fx.burst
local update_transition = Fx.update_transition
local Actors = require("robotron.actors")
local show_unit, count_kind, alive_count = Actors.show_unit, Actors.count_kind, Actors.alive_count
local needed, clear_all_units = Actors.needed, Actors.clear_all_units
local render_units = Actors.render_units
local Player = require("robotron.player")
local show_player, flash_player = Player.show_player, Player.flash_player
local clear_bullets, update_player = Player.clear_bullets, Player.update_player
local Ai = require("robotron.ai")
local update_units = Ai.update_units
local Combat = require("robotron.combat")
local update_bullets, update_interactions = Combat.update_bullets, Combat.update_interactions
local Waves = require("robotron.waves")
local start_wave, finish_wave = Waves.start_wave, Waves.finish_wave
local respawn_player = Waves.respawn_player

local floor, max = math.floor, math.max

-- the rectangle zoom; `after` runs when it has filled the arena. keep: hide the actors but keep
-- them for `after` (a death replays the wave with the survivors)
local function begin_transition(after, keep)
    g.state = "trans"
    g.trans_t = 0
    g.trans_after = after
    g.tcx, g.tcz = g.px, g.pz
    if keep then
        for i = 1, #units do show_unit(units[i], false) end
    else
        clear_all_units()
    end
    clear_bullets()
    show_player(false)
    sfx("transition", 1.0, 0.8)
end

local function new_game(demo)
    g.demo, g.bot = demo, demo
    g.score, g.lives, g.next_life, g.wave = 0, START_LIVES, EXTRA_LIFE, 0
    g.p_alive = true
    g.dying_t, g.over_t = 0, 0
    g.stats.games = g.stats.games + 1
    g.popups = {}
    g.paused = false
    g.demo_t = 0
    if S.player.flash then flash_player(nil) end
    if not demo then sfx("start", 1.0, 1.0) end
    begin_transition(function() start_wave(1) end)
end

local function step_game(dt)
    if g.state == "running" then
        g.wave_t = g.wave_t + dt
        -- every 3.75 s (with fewer than 30 grunts) the grunts' top speed rises, twice as fast
        -- when the player has not scored since the last time
        g.speed_t = g.speed_t - dt
        if g.speed_t <= 0 then
            g.speed_t = 15 * 15 * FR
            if count_kind("grunt") < 30 then
                g.rmxspd = max(1, g.rmxspd - (g.scored and 1 or 2))
                g.robspd = max(g.rmxspd, g.robspd - (g.scored and 2 or 4))
            end
            g.scored = false
        end
        g.grunt_speed = GRUNT_STEP * 1.41 / ((g.robspd + 1) * 2 * FR) -- mean, for the autopilot
        update_player(dt)
        update_units(dt)
        update_bullets(dt)
        update_interactions(dt)
        render_units(dt)
        if g.state == "running" and alive_count(needed) == 0 then
            finish_wave()
        end
    elseif g.state == "dying" then
        -- everything freezes while the player flashes through the colours, then bursts
        g.dying_t = g.dying_t + dt
        update_bullets(dt)
        if g.dying_t < 1.3 then
            local i = floor(g.dying_t * 22) % #FLASH + 1
            if i ~= g.flash_i then
                g.flash_i = i
                flash_player("fx_" .. FLASH[i])
            end
        elseif not g.exploded then
            g.exploded = true
            flash_player(nil)
            show_player(false)
            explode(S.player.m, g.px, 0, g.pz, nil, nil, 1.0, 7)
            ring(g.px, g.pz, 6.0, 0.7, "cyan")
            ring(g.px, g.pz, 3.5, 0.45, "white")
            burst("glow_white", g.px, 0.8, g.pz, 60, 14, 0.09)
            flash_light(g.px, 1.2, g.pz, "cyan", 100, 12, 1.0)
            add_shake(0.8)
            hit_flash(1.0)
        end
        if g.dying_t > 2.2 then
            if g.lives > 0 then
                begin_transition(respawn_player, true)
            else
                g.state = "over"
                g.over_t = g.demo and 2.5 or 5.0
                g.banner, g.banner_t = "GAME OVER", g.over_t
                sfx("gameover", 1.0, 1.0)
            end
        end
    elseif g.state == "clearing" then
        update_player(dt)
        update_bullets(dt)
        g.clear_t = g.clear_t - dt
        if g.clear_t <= 0 then begin_transition(function() start_wave(g.wave + 1) end) end
    elseif g.state == "trans" then
        update_transition(dt)
    elseif g.state == "over" then
        render_units(dt)
        g.over_t = g.over_t - dt
        if g.over_t <= 0 then new_game(true) end
    end
end

local function meta_input()
    if g.demo then
        if is_key_just_pressed("Return") or is_key_just_pressed("Space") or is_mouse_button_just_pressed(0) then
            new_game(false)
        end
    else
        if is_key_just_pressed("B") then g.bot = not g.bot end
        if is_key_just_pressed("P") then g.paused = not g.paused end
        if is_key_just_pressed("Escape") then new_game(true) end
    end
end

return {
    begin_transition = begin_transition, new_game = new_game, step_game = step_game, meta_input = meta_input,
}
