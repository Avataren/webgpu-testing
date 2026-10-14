-- ROBOTRON 2084, in voxels. One controller script drives everything.
-- Attached to the GameController entity; runs in play mode only.
--
-- The game boots into ATTRACT MODE: an AI pilot plays a demo game under the title.
-- ENTER (or SPACE / a mouse click) starts a real game. After a game over it returns to the demo.
--
--   WASD         move (8 directions)
--   Arrow keys   fire (8 directions; hold two for diagonals)   -or-   hold the left mouse button to fire at the cursor
--   B            toggle the autopilot while playing
--   P            pause
--   Escape       back to the demo
--
-- Every actor is a voxel model of its 1982 sprite (sprites.lua), meshed by the engine into one
-- mesh per colour, standing on a lit floor so it casts a real shadow. The same pixels drive
-- the effects from the original: enemies MATERIALIZE out of spread-apart pixel lines that close
-- up into the sprite, shot enemies STRETCH APART along the line of fire, a RECTANGLE ZOOM runs
-- between waves, and on death everything freezes while the player flashes through colours, then
-- the surviving enemies materialize again in new places.
--
-- Rules, enemy behaviour, speeds and the 40-wave table follow the original's source (see the actor
-- AI section): one hit kills you, the family is worth 1000..5000 points each (rising within a wave,
-- reset when you die), hulks cannot be killed (only pushed), grunts that walk into electrodes die,
-- brains reprogram humans into progs and fire cruise missiles, spheroids drop enforcers, quarks
-- drop tanks, and a wave ends when every grunt, brain, spheroid, enforcer, quark and tank is gone.
-- An extra life comes every 25,000 points.
--
-- The arena (floor and border), the sun and the camera are scene entities; every actor, projectile
-- and effect is spawned by the scripts, lazily, and reused. on_destroyed despawns all of it, so hot
-- reloads do not leak entities.
--
-- This file holds the engine hooks and the globals for testing (GAME, RT). The game itself is in
-- robotron/, one module per part:
--   config     tuning, arena size, the arcade's units      util      maths helpers
--   state      GAME (g), the actor and bullet lists         models    sprites -> voxel models
--   fx         particles, popups, shake, wave transition    actors    kinds, spawning, kills, queries
--   ai         enemy and family AI                          player    the player and its shots
--   combat     collisions                                   waves     the wave tables, start and end
--   autopilot  the demo / B pilot                           flow      the game's state machine
--   camera     framing the arena                            hud       on-screen text
--   logo       the voxel title in attract mode

local C = require("robotron.config")
local PLAY_MUSIC, MUSIC_VOLUME, FOV = C.PLAY_MUSIC, C.MUSIC_VOLUME, C.FOV
local S = require("robotron.state")
local g, units, pools, entities = S.g, S.units, S.pools, S.entities
local Fx = require("robotron.fx")
local update_effects, build_rects, update_walls = Fx.update_effects, Fx.build_rects, Fx.update_walls
local build_lights, update_lights, update_hit = Fx.build_lights, Fx.update_lights, Fx.update_hit
local Actors = require("robotron.actors")
local show_unit, spawn_unit = Actors.show_unit, Actors.spawn_unit
local clear_all_units = Actors.clear_all_units
local Player = require("robotron.player")
local build_player, flash_player = Player.build_player, Player.flash_player
local kill_player = Player.kill_player
local Waves = require("robotron.waves")
local electrode_key, start_wave = Waves.electrode_key, Waves.start_wave
local Flow = require("robotron.flow")
local new_game, step_game, meta_input = Flow.new_game, Flow.step_game, Flow.meta_input
local Camera = require("robotron.camera")
local update_camera = Camera.update_camera
local Hud = require("robotron.hud")
local draw_hud = Hud.draw_hud
local Logo = require("robotron.logo")
local update_logo = Logo.update_logo

local max = math.max

GAME = g -- global so lua_eval can inspect and poke the game state while testing

function on_created(self_entity)
    local ok, t = pcall(os.time)
    math.randomseed(ok and t or 2084)
    g.cam = find_entity_by_name("GameCamera")
    if g.cam then set_camera(g.cam, { fov_deg = FOV, near = 1.0, far = 200 }) end
    for _, n in ipairs({ "Wall_N", "Wall_S", "Wall_W", "Wall_E" }) do
        local e = find_entity_by_name(n)
        if e then g.walls[#g.walls + 1] = e end
    end
    for _, n in ipairs({ "Spill_N", "Spill_S", "Spill_W", "Spill_E" }) do
        local e = find_entity_by_name(n)
        if e then g.spills[#g.spills + 1] = e end
    end
    for _, n in ipairs({ "WallBody_N", "WallBody_S", "WallBody_W", "WallBody_E" }) do
        local e = find_entity_by_name(n)
        if e then g.wall_bodies[#g.wall_bodies + 1] = e end
    end
    g.rim = find_entity_by_name("FillWarm")
    g.arena = find_entity_by_name("Arena")
    g.hit_vol = find_entity_by_name("HitVolume")
    g.hi = tonumber(load_data("highscore") or "0") or 0
    build_player()
    build_rects()
    build_lights()
    if PLAY_MUSIC then g.music = play_sound("content/audio/music.wav", { volume = MUSIC_VOLUME, loop = true }) end
    g.ready = true
    new_game(true)
end

function on_destroyed(self_entity, reason)
    if g.music then stop_sound(g.music) end
    for i = #entities, 1, -1 do
        despawn_entity(entities[i])
        entities[i] = nil
    end
    g.ready = false
end

-- GAME.prof: moving average of the milliseconds Lua spends per section of update
local clock = os.clock
local prof = { step = 0, fx = 0, cam = 0, walls = 0, total = 0 }
g.prof = prof

function update(self_entity, dt)
    if not g.ready then return end
    if dt > 0.034 then dt = 0.034 end
    local c0 = clock()
    g.time = g.time + dt
    g.demo_t = g.demo_t + dt
    meta_input()
    if g.paused then
        update_camera(dt)
        return
    end
    step_game(dt)
    local c1 = clock()
    update_effects(dt)
    local c2 = clock()
    update_camera(dt)
    update_logo(dt)
    local c3 = clock()
    update_walls(dt)
    update_lights(dt)
    update_hit(dt)
    local c4 = clock()
    if not g.demo and not g.god and g.score >= g.hi and g.score > 0 and g.time - (g.hi_saved or 0) > 5 then
        g.hi_saved = g.time
        save_data("highscore", tostring(g.hi))
    end
    local k = 0.05
    prof.step = prof.step + ((c1 - c0) * 1000 - prof.step) * k
    prof.fx = prof.fx + ((c2 - c1) * 1000 - prof.fx) * k
    prof.cam = prof.cam + ((c3 - c2) * 1000 - prof.cam) * k
    prof.walls = prof.walls + ((c4 - c3) * 1000 - prof.walls) * k
    prof.total = prof.step + prof.fx + prof.cam + prof.walls
end

function on_ui(self_entity, ui)
    if not g.ready then return end
    draw_hud(ui)
end

-- ---------------------------------------------------------------- test hooks
-- Globals for lua_eval {entity = "GameController"}: RT.jump(5), RT.spawn("brain", 3, 2), RT.god(true) ...
RT = {
    jump = function(w)
        g.p_alive = true
        if S.player.flash then flash_player(nil) end
        start_wave(w)
    end,
    spawn = function(kind, x, z) return spawn_unit(kind, x, z, 0.0, nil, nil, kind == "electrode" and electrode_key(max(1, g.wave)) or nil) end,
    god = function(on) g.god = on end,
    kill = function() g.invuln, g.god, g.p_spawn = 0, false, 0 kill_player() end,
    lives = function(n) g.lives = n end,
    bot = function(on) g.bot = on end,
    botinfo = function() return g.b_humans, g.b_killers, g.hold_t, g.px, g.pz, g.state, g.wave end,
    new = function(demo) new_game(demo) end,
    clear = function() clear_all_units() end,
    count = function()
        local c = {}
        for i = 1, #units do
            local u = units[i]
            if u.alive then c[u.kind] = (c[u.kind] or 0) + 1 end
        end
        return c
    end,
    entities = function() return #entities end,
    pooled = function()
        local c, parts = {}, 0
        for key, pool in pairs(pools) do
            c[key] = #pool
            for _, u in ipairs(pool) do parts = parts + #u.parts end
        end
        c.parts = parts
        return c
    end,
    units = function(kind)
        local out = {}
        for i = 1, #units do
            local u = units[i]
            if u.alive and (not kind or u.kind == kind) then
                out[#out + 1] = string.format("%s %.1f,%.1f", u.kind, u.x, u.z)
            end
        end
        return out
    end,
    -- line up one of everything in front of the camera (for screenshots)
    lineup = function()
        clear_all_units()
        local kinds = { "grunt", "hulk", "brain", "mommy", "daddy", "mikey", "prog", "enforcer", "spheroid", "quark", "tank", "electrode", "skull" }
        for i, kind in ipairs(kinds) do
            local u = RT.spawn(kind, -12 + (i - 1) * 2.0, 2.0)
            u.spawn_t, u.emitted = 0, true
            u.life = 1e9
            u.frozen = true
            show_unit(u, true)
        end
    end,
}
