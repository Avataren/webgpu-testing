-- Shared game state: the GAME table (g), the actor and bullet lists, and helpers every module
-- uses. These tables are only ever changed in place, so other modules can keep references to
-- them. S.player is the player's model, set by player.build_player.

local C = require("robotron.config")
local START_LIVES, EXTRA_LIFE, SFX_VOLUME = C.START_LIVES, C.EXTRA_LIFE, C.SFX_VOLUME

local g = {
    state = "running", demo = true, bot = true, god = false,
    time = 0, score = 0, hi = 0, lives = START_LIVES, wave = 0, next_life = EXTRA_LIFE,
    px = 0.0, pz = 0.0, pvx = 0.0, pvz = 0.0, face = 0.0, fire_cd = 0, invuln = 0, p_alive = true,
    p_spawn = 0, p_anim = 0, p_yaw = 0,
    dying_t = 0, over_t = 0, clear_t = 0, banner_t = 0, banner = "", rescue_n = 0,
    wave_t = 0, grunt_speed = 1.0, shake = 0, nid = 0, mat_mode = "v",
    trans_t = 0, trans_after = nil, tcx = 0, tcz = 0,
    bmx = 0, bmz = 0, bfx = 0, bfz = 0, bot_t = 0, aim_t = 0, bprev = 0, aprev = 0, hold_t = 0, b_humans = 0, b_killers = 0,
    popups = {}, stats = { kills = 0, rescued = 0, lost = 0, deaths = 0, shots = 0, waves = 0, games = 0, cleared = 0 },
    lean = C.LEAN, lean_c = C.CL, lean_s = C.SL, -- the actors' current lean (camera.lua)
    ready = false, cam = nil, cam_fit = nil, walls = {}, wall_bodies = {}, spills = {}, wall_hue = 0, demo_t = 0, paused = false,
}

local units = {}    -- every actor / hostile projectile, alive or pooled
local pools = {}    -- model key -> list of units
local pbullets = {} -- player bullets
local entities = {} -- roots this script spawned, for on_destroyed
local sfx_last = {}

local function reg(e)
    entities[#entities + 1] = e
    return e
end

local function uname(prefix)
    g.nid = g.nid + 1
    return prefix .. g.nid
end

local function sfx(name, pitch, vol, min_gap)
    if min_gap and sfx_last[name] and g.time - sfx_last[name] < min_gap then return end
    sfx_last[name] = g.time
    local v = (vol or 1.0) * SFX_VOLUME * (g.demo and 0.55 or 1.0)
    play_sound("content/audio/" .. name .. ".wav", { volume = v, pitch = pitch or 1.0 })
end

local function set_vis(e, v) set_component(e, "Visible", v) end

return {
    g = g, units = units, pools = pools, pbullets = pbullets, entities = entities,
    reg = reg, uname = uname, sfx = sfx, set_vis = set_vis,
}
