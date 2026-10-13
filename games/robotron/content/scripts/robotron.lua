-- ROBOTRON 2084, a modern take. One controller script drives everything.
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
-- Rules follow the 1982 original: one hit kills you, the last human family members are worth
-- 1000..5000 points each (rising within a wave), hulks cannot be killed (only pushed), brains turn
-- humans into progs and fire homing missiles, spheroids spawn enforcers, quarks spawn tanks,
-- and a wave ends when every killable enemy is gone. An extra life comes every 25,000 points.
--
-- The arena (floor, grid, border), lights and camera are placed in the scene; every actor,
-- projectile and spark is spawned here, lazily, and reused. Everything this script spawns is
-- despawned in on_destroyed, so hot reloads do not leak entities.

local HW, HD = 19.0, 11.0                -- arena half extents inside the border (x, z)
local P_R = 0.34                         -- player hit radius
local LX, LZ = HW - 0.45, HD - 0.45      -- player position limits
local P_SPEED = 10.0
local FIRE_INT = 0.095
local B_SPEED, B_R = 36.0, 0.16
local N_PBULLETS = 26
local START_LIVES = 3
local EXTRA_LIFE = 25000
local SFX_VOLUME, MUSIC_VOLUME = 0.55, 0.16
local FOV = 40.0
local TILT = math.rad(16.0)
local SPARKS_PER_COLOR = 28

local atan, sqrt, sin, cos, abs, floor, min, max = math.atan, math.sqrt, math.sin, math.cos, math.abs, math.floor, math.min, math.max

-- ---------------------------------------------------------------- helpers

local function clamp(v, lo, hi)
    if v < lo then return lo elseif v > hi then return hi end
    return v
end

local function frand(a, b) return a + (b - a) * math.random() end
local function wrap_pi(a)
    while a > math.pi do a = a - 2 * math.pi end
    while a < -math.pi do a = a + 2 * math.pi end
    return a
end

local PAL = {
    red = { 1.0, 0.12, 0.18 }, orange = { 1.0, 0.55, 0.1 }, yellow = { 1.0, 0.95, 0.3 },
    green = { 0.2, 1.0, 0.35 }, cyan = { 0.2, 0.9, 1.0 }, blue = { 0.3, 0.4, 1.0 },
    magenta = { 1.0, 0.2, 0.9 }, white = { 1.0, 1.0, 1.0 }, pink = { 1.0, 0.45, 0.75 },
    purple = { 0.65, 0.3, 1.0 },
}
local FLASH = { "red", "orange", "yellow", "green", "cyan", "blue", "magenta", "white" }

local g = {
    state = "running", demo = true, bot = true, god = false,
    time = 0, score = 0, hi = 0, lives = START_LIVES, wave = 0, next_life = EXTRA_LIFE,
    px = 0.0, pz = 0.0, pvx = 0.0, pvz = 0.0, face = 0.0, fire_cd = 0, invuln = 0, p_alive = true,
    dying_t = 0, over_t = 0, clear_t = 0, banner_t = 0, banner = "", rescue_n = 0,
    wspd = 1.0, grunt_speed = 3.6, shake = 0, nid = 0, pending = 0,
    bmx = 0, bmz = 0, bfx = 0, bfz = 0, bot_t = 0, aim_t = 0, bprev = 0, aprev = 0,
    popups = {}, stats = { kills = 0, rescued = 0, lost = 0, deaths = 0, shots = 0, waves = 0, games = 0, cleared = 0 },
    ready = false, cam = nil, walls = {}, wall_hue = 0, demo_t = 0, paused = false,
}
GAME = g -- global so lua_eval can inspect and poke the game state while testing

local units = {}       -- every actor / hostile projectile, alive or pooled
local pools = {}       -- kind -> list of units
local pbullets = {}    -- player bullets
local sparks = {}      -- color -> { items, i }
local all_entities = {} -- everything this script spawned, for on_destroyed
local player = nil
local sfx_last = {}

local function reg(e)
    all_entities[#all_entities + 1] = e
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

-- ---------------------------------------------------------------- actor definitions

local function P(mesh, px, py, pz, sx, sy, sz, col, em, o)
    o = o or {}
    return { mesh = mesh, pos = { px, py, pz }, scale = { sx, sy, sz }, col = col, em = em or 1.0,
        unlit = o.unlit, rot = o.rot, alpha = o.alpha, metal = o.metal or 0.0, rough = o.rough or 0.5 }
end

local K = {}

K.grunt = {
    r = 0.5, pts = 100, hostile = true, shoot = true, kill = true, sfx = "grunt_die", col = "red",
    burst = 11, walk = true, bw = 1.0, danger = 1.0,
    parts = {
        P("cube", 0, 0.5, 0, 1.1, 0.85, 0.7, { 1.0, 0.06, 0.1 }, 0.9, { metal = 0.3, rough = 0.4 }),
        P("cube", 0, 1.13, 0, 0.52, 0.4, 0.46, { 1.0, 0.5, 0.1 }, 2.4),
    },
}

K.electrode = {
    r = 0.52, pts = 0, hostile = true, shoot = true, sfx = "electrode_die", col = "yellow",
    burst = 14, spin = 1.6, pulse = true, bw = 0.12, danger = 1.1, flash = true,
    parts = {
        P("cube", 0, 0.65, 0, 0.55, 0.55, 0.55, { 1.0, 0.9, 0.2 }, 1.8, { unlit = true }),
        P("cube", 0, 0.65, 0, 0.55, 0.55, 0.55, { 1.0, 0.9, 0.2 }, 1.8, { rot = { 0.785, 0.785, 0.0 }, unlit = true }),
    },
}

K.hulk = {
    r = 1.0, pts = 0, hostile = true, hulk = true, sfx = "hulk_hit", col = "green",
    burst = 16, walk = true, danger = 1.5, margin = 0.45,
    parts = {
        P("cube", 0, 0.95, 0, 1.55, 1.3, 1.1, { 0.04, 0.5, 0.12 }, 0.5, { metal = 0.6, rough = 0.35 }),
        P("cube", 0, 1.85, -0.05, 0.75, 0.55, 0.65, { 0.3, 1.0, 0.4 }, 1.6),
        P("cube", -1.05, 1.0, 0, 0.5, 1.15, 0.5, { 0.1, 0.7, 0.25 }, 0.8, { metal = 0.5 }),
        P("cube", 1.05, 1.0, 0, 0.5, 1.15, 0.5, { 0.1, 0.7, 0.25 }, 0.8, { metal = 0.5 }),
    },
}

local HUMANS = {
    mommy = { 1.0, 0.35, 0.75 }, daddy = { 0.3, 0.6, 1.0 }, mikey = { 0.75, 1.0, 0.3 },
}
for name, c in pairs(HUMANS) do
    K[name] = {
        r = 0.42, human = true, ai = "human", sfx = "human_die", col = "white", burst = 10, walk = true,
        parts = {
            P("cube", 0, 0.62, 0, 0.58, 0.95, 0.36, c, 0.8),
            P("sphere", 0, 1.38, 0, 0.26, 0.26, 0.26, { 1.0, 0.8, 0.6 }, 0.5),
            P("cube", 0, 1.05, 0, 1.25, 0.14, 0.2, c, 1.0),
            P("cylinder", 0, 0.05, 0, 0.62, 0.62, 0.05, c, 2.2, { unlit = true, rot = { 0, math.pi / 2, 0 } }),
        },
    }
end

K.prog = {
    r = 0.46, pts = 100, hostile = true, shoot = true, kill = true, sfx = "grunt_die", col = "red",
    burst = 10, walk = true, bw = 1.5, danger = 1.1,
    parts = {
        P("cube", 0, 0.62, 0, 0.58, 0.95, 0.36, { 0.5, 0.1, 0.16 }, 0.6),
        P("sphere", 0, 1.38, 0, 0.26, 0.26, 0.26, { 1.0, 0.1, 0.1 }, 4.0, { unlit = true }),
        P("cube", 0, 1.05, 0, 1.25, 0.14, 0.2, { 0.7, 0.12, 0.2 }, 0.9),
        P("cylinder", 0, 0.05, 0, 0.62, 0.62, 0.05, { 1.0, 0.1, 0.1 }, 2.2, { unlit = true, rot = { 0, math.pi / 2, 0 } }),
    },
}

K.brain = {
    r = 0.62, pts = 500, hostile = true, shoot = true, kill = true, sfx = "brain_die", col = "pink",
    burst = 22, bob = { 0.25, 3.0 }, bw = 3.0, danger = 1.1,
    parts = {
        P("sphere", -0.24, 1.0, 0, 0.44, 0.4, 0.5, { 1.0, 0.25, 0.6 }, 1.0, { metal = 0.2 }),
        P("sphere", 0.24, 1.0, 0, 0.44, 0.4, 0.5, { 1.0, 0.25, 0.6 }, 1.0, { metal = 0.2 }),
        P("cube", 0, 0.5, 0, 0.22, 0.5, 0.22, { 0.6, 0.2, 0.9 }, 1.4),
        P("cube", -0.17, 0.9, -0.42, 0.14, 0.14, 0.14, { 0.3, 1.0, 0.6 }, 5.0, { unlit = true }),
        P("cube", 0.17, 0.9, -0.42, 0.14, 0.14, 0.14, { 0.3, 1.0, 0.6 }, 5.0, { unlit = true }),
    },
}

K.spheroid = {
    r = 0.72, pts = 1000, hostile = true, shoot = true, kill = true, sfx = "big_boom", col = "cyan",
    burst = 30, spin = 3.0, bob = { 0.3, 2.0 }, bw = 2.2, danger = 1.0, flash = true,
    parts = {
        P("torus", 0, 0.95, 0, 0.52, 0.52, 0.52, { 0.2, 0.9, 1.0 }, 3.0, { unlit = true }),
        P("torus", 0, 0.95, 0, 0.52, 0.52, 0.52, { 1.0, 0.2, 0.9 }, 3.0, { rot = { math.pi / 2, 0, 0 }, unlit = true }),
        P("sphere", 0, 0.95, 0, 0.3, 0.3, 0.3, { 1.0, 1.0, 1.0 }, 4.0, { unlit = true }),
    },
}

K.enforcer = {
    r = 0.5, pts = 150, hostile = true, shoot = true, kill = true, sfx = "grunt_die", col = "blue",
    burst = 14, spin = 4.0, bob = { 0.35, 4.0 }, bw = 1.6, danger = 1.0,
    parts = {
        P("cube", 0, 0.95, 0, 0.5, 0.5, 0.5, { 0.4, 0.4, 1.0 }, 3.0, { rot = { 0.785, 0.785, 0 }, unlit = true }),
        P("sphere", 0, 0.95, 0, 0.22, 0.22, 0.22, { 1.0, 1.0, 1.0 }, 4.0, { unlit = true }),
    },
}

K.quark = {
    r = 0.7, pts = 1000, hostile = true, shoot = true, kill = true, sfx = "big_boom", col = "cyan",
    burst = 30, tumble = true, bob = { 0.3, 2.5 }, bw = 2.2, danger = 1.0,
    parts = {
        P("cube", 0, 1.0, 0, 1.1, 1.1, 1.1, { 0.2, 1.0, 0.9 }, 2.2, { alpha = 0.4 }),
        P("cube", 0, 1.0, 0, 0.55, 0.55, 0.55, { 1.0, 1.0, 1.0 }, 4.0, { unlit = true }),
    },
}

K.tank = {
    r = 0.85, pts = 200, hostile = true, shoot = true, kill = true, sfx = "brain_die", col = "green",
    burst = 20, bw = 1.2, danger = 1.0,
    parts = {
        P("cube", 0, 0.45, 0, 1.35, 0.6, 1.75, { 0.12, 0.65, 0.2 }, 0.9, { metal = 0.5, rough = 0.4 }),
        P("cube", 0, 0.98, 0.1, 0.75, 0.42, 0.75, { 0.4, 1.0, 0.45 }, 2.0),
        P("cylinder", 0, 0.98, -0.85, 0.13, 0.13, 1.1, { 0.8, 1.0, 0.5 }, 2.5),
    },
}

K.missile = {
    r = 0.3, pts = 50, hostile = true, shoot = true, sfx = "grunt_die", col = "orange", burst = 8,
    bw = 2.4, danger = 1.2, margin = 0.05, proj = true,
    parts = {
        P("cube", 0, 0.7, 0, 0.2, 0.2, 0.8, { 1.0, 0.5, 0.1 }, 4.5, { unlit = true }),
        P("cube", 0, 0.7, 0.55, 0.12, 0.12, 0.3, { 1.0, 0.9, 0.5 }, 5.0, { unlit = true }),
    },
}

K.spark = {
    r = 0.22, hostile = true, sfx = "grunt_die", col = "purple", burst = 5, spin = 9.0,
    danger = 1.2, margin = 0.05, proj = true,
    parts = {
        P("cube", 0, 0.7, 0, 0.3, 0.3, 0.3, { 0.9, 0.5, 1.0 }, 5.0, { rot = { 0.78, 0.78, 0 }, unlit = true }),
    },
}

K.shell = {
    r = 0.28, hostile = true, sfx = "bounce", col = "yellow", burst = 6,
    danger = 1.2, margin = 0.05, proj = true,
    parts = {
        P("sphere", 0, 0.7, 0, 0.28, 0.28, 0.28, { 1.0, 0.85, 0.2 }, 5.0, { unlit = true }),
    },
}

-- ---------------------------------------------------------------- pooled objects

local function make_unit(kind)
    local k = K[kind]
    local root = reg(spawn_entity(uname("RT_" .. kind .. "_")))
    local parts = {}
    for i, p in ipairs(k.parts) do
        local e = reg(spawn_primitive(p.mesh, uname("RT_" .. kind .. "_part")))
        set_parent(e, root)
        set_translation(e, p.pos[1], p.pos[2], p.pos[3])
        set_scale(e, p.scale[1], p.scale[2], p.scale[3])
        if p.rot then set_rotation(e, p.rot[1], p.rot[2], p.rot[3]) end
        local col = { p.col[1], p.col[2], p.col[3] }
        if p.alpha then col[4] = p.alpha end
        set_material(e, { color = col, emissive = p.em, unlit = p.unlit or false, metallic = p.metal,
            roughness = p.rough, transparent = p.alpha ~= nil })
        set_component(e, "CanCastShadow", false)
        set_component(e, "Visible", false)
        parts[i] = e
    end
    set_translation(root, 0, -50, 0)
    local u = { kind = kind, k = k, e = root, parts = parts, vis = false, alive = false, x = 0, z = 0,
        vx = 0, vz = 0, kx = 0, kz = 0, spawn_t = 0, spawn_total = 1, age = 0, face = 0, ph = math.random() * 6.28, sc = 1 }
    units[#units + 1] = u
    return u
end

local function show_unit(u, v)
    if u.vis == v then return end
    u.vis = v
    for i = 1, #u.parts do set_vis(u.parts[i], v) end
    if not v then set_translation(u.e, 0, -50, 0) end
end

local function acquire(kind)
    local pool = pools[kind]
    if not pool then
        pool = {}
        pools[kind] = pool
    end
    for i = 1, #pool do
        if not pool[i].alive then return pool[i] end
    end
    local u = make_unit(kind)
    pool[#pool + 1] = u
    return u
end

-- palette-coloured spark cubes: one pool per colour so a spark never needs a material change
local function new_spark(col)
    local e = reg(spawn_primitive("cube", uname("RT_spark")))
    set_material(e, { color = PAL[col], emissive = 3.0, unlit = true })
    set_component(e, "CanCastShadow", false)
    set_component(e, "Visible", false)
    set_translation(e, 0, -50, 0)
    return { e = e, alive = false, vis = false, x = 0, y = 0, z = 0, vx = 0, vy = 0, vz = 0, life = 0, max = 1, size = 0.2 }
end

local function spark(col, x, y, z, vx, vy, vz, life, size, grav)
    local pool = sparks[col]
    if not pool then
        pool = { i = 0 }
        sparks[col] = pool
    end
    local o
    if #pool < SPARKS_PER_COLOR then
        o = new_spark(col)
        pool[#pool + 1] = o
    else
        pool.i = pool.i % #pool + 1
        o = pool[pool.i]
    end
    o.alive, o.x, o.y, o.z, o.vx, o.vy, o.vz, o.life, o.max, o.size = true, x, y, z, vx, vy, vz, life, life, size
    o.grav = grav or 16
    set_translation(o.e, x, y, z)
    if not o.vis then
        o.vis = true
        set_vis(o.e, true)
    end
end

local function burst(col, x, y, z, count, speed, size)
    for _ = 1, count do
        local a = math.random() * 6.2832
        local s = frand(speed * 0.25, speed)
        spark(col, x, y, z, cos(a) * s, frand(1.5, 7.0), sin(a) * s, frand(0.35, 0.85), (size or 0.2) * frand(0.6, 1.4))
    end
end

-- an expanding ring of sparks lying on the floor (a shockwave)
local function ring(x, z, maxr, dur, col)
    local n = floor(10 + maxr * 5)
    local sp = maxr / dur * 0.85
    for i = 1, n do
        local a = (i / n) * 6.2832
        spark(col, x, 0.15, z, cos(a) * sp, 0, sin(a) * sp, dur, 0.22, 0)
    end
end

local function add_shake(a) g.shake = min(1.0, max(g.shake, a)) end

local function popup(x, z, text, col, size)
    if #g.popups >= 16 then table.remove(g.popups, 1) end
    g.popups[#g.popups + 1] = { x = x, z = z, text = text, col = col or "white", t = 0, size = size or 22 }
end

-- ---------------------------------------------------------------- player

local function build_player()
    local root = reg(spawn_entity("RT_player"))
    local parts = {}
    local function part(mesh, x, y, z, sx, sy, sz, col, em, unlit)
        local e = reg(spawn_primitive(mesh, uname("RT_player_part")))
        set_parent(e, root)
        set_translation(e, x, y, z)
        set_scale(e, sx, sy, sz)
        set_material(e, { color = col, emissive = em, unlit = unlit or false })
        set_component(e, "CanCastShadow", false)
        parts[#parts + 1] = e
    end
    part("cube", 0, 0.62, 0, 0.56, 0.95, 0.36, { 0.35, 0.8, 1.0 }, 0.9)
    part("sphere", 0, 1.38, 0, 0.26, 0.26, 0.26, { 1.0, 1.0, 1.0 }, 2.0, true)
    part("cube", -0.42, 0.75, -0.2, 0.13, 0.13, 0.6, { 0.4, 0.9, 1.0 }, 2.5, true)
    part("cube", 0.42, 0.75, -0.2, 0.13, 0.13, 0.6, { 0.4, 0.9, 1.0 }, 2.5, true)
    local disc = reg(spawn_primitive("cylinder", uname("RT_player_part")))
    set_parent(disc, root)
    set_translation(disc, 0, 0.06, 0)
    set_scale(disc, 0.85, 0.85, 0.05)
    set_rotation(disc, 0, math.pi / 2, 0)
    set_material(disc, { color = { 0.2, 0.9, 1.0 }, emissive = 2.5, unlit = true })
    set_component(disc, "CanCastShadow", false)
    parts[#parts + 1] = disc
    local light = reg(spawn_light("point", { color = { 0.5, 0.9, 1.0 }, intensity = 18, range = 8 }, uname("RT_player_light")))
    set_parent(light, root)
    set_translation(light, 0, 2.2, 0)
    player = { e = root, parts = parts, vis = true }
end

local function show_player(v)
    if player.vis == v then return end
    player.vis = v
    for i = 1, #player.parts do set_vis(player.parts[i], v) end
    if not v then set_translation(player.e, 0, -50, 0) end
end

-- ---------------------------------------------------------------- scoring and kills

local function add_score(n)
    g.score = g.score + n
    if not g.demo and g.score > g.hi then g.hi = g.score end
    while g.score >= g.next_life do
        g.next_life = g.next_life + EXTRA_LIFE
        g.lives = g.lives + 1
        sfx("extra_life", 1.0, 1.0)
        popup(g.px, g.pz - 1.5, "EXTRA LIFE", "yellow", 30)
    end
end

local function kill_unit(u, by_player)
    local k = u.k
    u.alive = false
    show_unit(u, false)
    local col = k.col
    burst(col, u.x, 0.7, u.z, k.burst or 10, 8 + (k.r or 0.5) * 4)
    if k.pts and k.pts >= 500 then
        ring(u.x, u.z, 3.4, 0.5, col)
        add_shake(0.5)
    elseif k.hulk then
        add_shake(0.1)
    else
        add_shake(0.1)
    end
    if k.sfx then sfx(k.sfx, frand(0.92, 1.12), (k.pts or 0) >= 500 and 1.0 or 0.75, 0.035) end
    if by_player and k.pts and k.pts > 0 then
        add_score(k.pts)
        popup(u.x, u.z, tostring(k.pts), col)
        g.stats.kills = g.stats.kills + 1
    end
end

-- ---------------------------------------------------------------- spawning

local function spawn_unit(kind, x, z, spawn_t, extra)
    local u = acquire(kind)
    u.alive = true
    u.x, u.z, u.vx, u.vz, u.kx, u.kz = x, z, 0, 0, 0, 0
    u.spawn_t = spawn_t or 0.0
    u.spawn_total = max(u.spawn_t, 0.01)
    u.age = 0
    u.face = frand(-math.pi, math.pi)
    u.sc = -1
    u.st = frand(0, 0.3)
    u.moving = false
    u.turn_t = 0
    u.fire_t = frand(1.5, 3.2)
    u.life = 6
    u.n_left = 0
    u.bounces = 0
    u.flash_i = math.random(1, #FLASH)
    u.flash_t = 0
    u.ang = frand(-math.pi, math.pi)
    u.tid, u.hdg, u.spd, u.target = nil, nil, nil, nil
    if kind == "spheroid" then
        u.n_left, u.fire_t = math.random(3, 5), frand(0.8, 1.4)
    elseif kind == "quark" then
        u.n_left, u.fire_t = math.random(2, 4), frand(1.2, 2.0)
    elseif kind == "brain" then
        u.fire_t = frand(2.0, 4.0)
    elseif kind == "hulk" then
        u.turn_t = frand(0.2, 1.0)
    end
    if extra then
        for key, val in pairs(extra) do u[key] = val end
    end
    show_unit(u, true)
    set_translation(u.e, x, 0, z)
    return u
end

local function alive_count(pred)
    local n = 0
    for i = 1, #units do
        local u = units[i]
        if u.alive and pred(u) then n = n + 1 end
    end
    return n
end

local function killable(u) return u.k.kill end
local function is_human(u) return u.k.human end

local function free_pos(min_center, margin, spacing, others)
    for _ = 1, 40 do
        local x = frand(-HW + margin, HW - margin)
        local z = frand(-HD + margin, HD - margin)
        local ok = (x * x + z * z) >= min_center * min_center
        if ok and spacing and others then
            for i = 1, #others do
                local o = others[i]
                local dx, dz = o[1] - x, o[2] - z
                if dx * dx + dz * dz < spacing * spacing then ok = false break end
            end
        end
        if ok then return x, z end
    end
    return frand(-HW + margin, HW - margin), frand(-HD + margin, HD - margin)
end

-- ---------------------------------------------------------------- waves

local WAVES = {
    { grunt = 15, electrode = 5, family = 1 },
    { grunt = 18, electrode = 10, hulk = 2, family = 2 },
    { grunt = 12, spheroid = 3, electrode = 6, hulk = 2, family = 2 },
    { grunt = 6, brain = 4, electrode = 3, hulk = 1, family = 3 },
    { grunt = 10, quark = 3, electrode = 8, hulk = 3, family = 2 },
    { grunt = 28, electrode = 10, hulk = 5, family = 1 },
    { grunt = 10, spheroid = 5, brain = 2, electrode = 6, hulk = 2, family = 2 },
    { grunt = 8, brain = 7, electrode = 4, hulk = 2, family = 3 },
    { grunt = 14, quark = 4, spheroid = 2, electrode = 8, hulk = 4, family = 2 },
    { grunt = 35, electrode = 14, hulk = 8, brain = 2, family = 1 },
}

local function wave_def(w)
    if w <= #WAVES then return WAVES[w] end
    local base = WAVES[6 + (w - 11) % 5]
    local s = 1 + 0.06 * (w - 10)
    local d = {}
    for key, n in pairs(base) do d[key] = n end
    d.grunt = min(64, floor((d.grunt or 0) * s))
    d.hulk = min(14, floor((d.hulk or 0) * s + 0.5))
    d.brain = min(12, floor((d.brain or 0) * s))
    d.spheroid = min(8, floor((d.spheroid or 0) * s))
    d.quark = min(6, floor((d.quark or 0) * s))
    d.electrode = min(30, floor((d.electrode or 0) * s))
    return d
end

local function clear_all_units()
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            u.alive = false
            show_unit(u, false)
        end
    end
end

-- Hidden pooled entities still cost frame time, so a new game despawns a large surplus left by the last one.
local function trim_pools(d)
    local fam = (d.family or 0)
    local keep = {
        grunt = d.grunt or 0, electrode = d.electrode or 0, hulk = d.hulk or 0, brain = d.brain or 0,
        spheroid = d.spheroid or 0, quark = d.quark or 0, mommy = fam, daddy = fam, mikey = fam,
        enforcer = 4 * (d.spheroid or 0), tank = 3 * (d.quark or 0), prog = 3 * fam,
        missile = 4, spark = 4, shell = 4,
    }
    local removed = false
    for kind, pool in pairs(pools) do
        -- despawned entities leak their materials in this engine, so only trim a big surplus
        local lim = (keep[kind] or 0) + 16
        for i = #pool, lim + 1, -1 do
            local u = pool[i]
            if not u.alive then
                despawn_entity(u.e)
                for _, pe in ipairs(u.parts) do despawn_entity(pe) end
                u.gone = true
                removed = true
                table.remove(pool, i)
            end
        end
    end
    if removed then
        local alive = {}
        for i = 1, #units do
            if not units[i].gone then alive[#alive + 1] = units[i] end
        end
        units = alive
    end
end

local function start_wave(w)
    g.wave = w
    g.rescue_n = 0
    g.stats.waves = max(g.stats.waves, w)
    if w > 1 then g.stats.cleared = g.stats.cleared + 1 end
    g.wspd = min(2.1, 1.0 + 0.045 * (w - 1))
    g.grunt_speed = min(9.0, 4.0 + 0.2 * (w - 1))
    clear_all_units()
    if w == 1 then trim_pools(wave_def(w)) end
    g.px, g.pz, g.pvx, g.pvz = 0.0, 0.0, 0.0, 0.0
    g.invuln = 2.2
    g.fire_cd = 0
    g.state = "running"
    g.banner, g.banner_t = "WAVE " .. w, 2.2
    for i = 1, #pbullets do
        pbullets[i].alive = false
        if pbullets[i].vis then
            pbullets[i].vis = false
            set_vis(pbullets[i].e, false)
        end
    end

    local d = wave_def(w)
    local taken = {}
    local function put(kind, count, min_c, margin, spacing, t0, t1, extra_fn)
        for _ = 1, count do
            local x, z = free_pos(min_c, margin, spacing, spacing and taken or nil)
            if spacing then taken[#taken + 1] = { x, z } end
            local u = spawn_unit(kind, x, z, frand(t0, t1), extra_fn and extra_fn() or nil)
            u.face = atan(-(0 - x), -(0 - z))
        end
    end
    put("hulk", d.hulk or 0, 9.0, 2.5, 4.0, 0.5, 1.4)
    put("electrode", d.electrode or 0, 4.5, 1.5, 2.6, 0.4, 1.6)
    for _ = 1, d.family or 0 do
        for _, name in ipairs({ "mommy", "daddy", "mikey" }) do
            put(name, 1, 4.0, 1.5, nil, 0.5, 1.2)
        end
    end
    put("grunt", d.grunt or 0, 7.0, 1.2, nil, 0.6, 1.6)
    put("brain", d.brain or 0, 8.0, 2.0, nil, 0.8, 1.6)
    put("spheroid", d.spheroid or 0, 8.0, 2.0, nil, 0.8, 1.8)
    put("quark", d.quark or 0, 8.0, 2.0, nil, 0.8, 1.8)
    sfx("wave_start", 1.0, 0.9)
end

local function finish_wave()
    g.state = "clearing"
    g.clear_t = 2.0
    g.banner, g.banner_t = "WAVE " .. g.wave .. " CLEARED", 2.0
    sfx("wave_clear", 1.0, 0.9)
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            if u.k.human then
                u.alive = false
                show_unit(u, false)
                g.stats.left = (g.stats.left or 0) + 1
                burst("white", u.x, 0.7, u.z, 8, 5)
            else
                kill_unit(u, false)
            end
        end
    end
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
    show_player(true)
    if not demo then sfx("start", 1.0, 1.0) end
    start_wave(1)
end

-- ---------------------------------------------------------------- bots helpers (decls)
local bot_think

-- ---------------------------------------------------------------- player update

local function fire_bullet(fx, fz)
    local b
    for i = 1, #pbullets do
        if not pbullets[i].alive then b = pbullets[i] break end
    end
    if not b then
        if #pbullets >= N_PBULLETS then return end
        local e = reg(spawn_primitive("cube", uname("RT_bullet")))
        set_material(e, { color = { 1.0, 0.95, 0.5 }, emissive = 4.0, unlit = true })
        set_scale(e, 0.13, 0.13, 0.8)
        set_component(e, "CanCastShadow", false)
        set_component(e, "Visible", false)
        b = { e = e, alive = false, vis = false, x = 0, z = 0, vx = 0, vz = 0, life = 0 }
        pbullets[#pbullets + 1] = b
    end
    b.alive = true
    b.x, b.z = g.px + fx * 0.55, g.pz + fz * 0.55
    b.vx, b.vz = fx * B_SPEED, fz * B_SPEED
    b.life = 1.4
    set_translation(b.e, b.x, 0.75, b.z)
    set_rotation(b.e, atan(-fx, -fz), 0, 0)
    if not b.vis then
        b.vis = true
        set_vis(b.e, true)
    end
    g.stats.shots = g.stats.shots + 1
    sfx("shoot", frand(0.95, 1.1), 0.55, 0.04)
end

local function read_input(dt)
    local mx, mz, fx, fz = 0, 0, 0, 0
    if g.bot then
        bot_think(dt)
        mx, mz, fx, fz = g.bmx, g.bmz, g.bfx, g.bfz
    else
        if is_key_pressed("D") then mx = mx + 1 end
        if is_key_pressed("A") then mx = mx - 1 end
        if is_key_pressed("S") then mz = mz + 1 end
        if is_key_pressed("W") then mz = mz - 1 end
        if is_key_pressed("ArrowRight") then fx = fx + 1 end
        if is_key_pressed("ArrowLeft") then fx = fx - 1 end
        if is_key_pressed("ArrowDown") then fz = fz + 1 end
        if is_key_pressed("ArrowUp") then fz = fz - 1 end
        if fx == 0 and fz == 0 and is_mouse_button_pressed(0) then
            local m = get_mouse_position()
            local gp = m and screen_to_ground(m.x, m.y, 0.7)
            if gp then
                local dx, dz = gp.x - g.px, gp.z - g.pz
                local d = sqrt(dx * dx + dz * dz)
                if d > 0.3 then fx, fz = dx / d, dz / d end
            end
        end
    end
    local ml = sqrt(mx * mx + mz * mz)
    if ml > 0 then mx, mz = mx / ml, mz / ml end
    local fl = sqrt(fx * fx + fz * fz)
    if fl > 0 then fx, fz = fx / fl, fz / fl end
    return mx, mz, fx, fz
end

local function update_player(dt)
    if g.invuln > 0 then g.invuln = g.invuln - dt end
    if not g.p_alive then return end
    local mx, mz, fx, fz = read_input(dt)
    g.pvx, g.pvz = mx * P_SPEED, mz * P_SPEED
    g.px = clamp(g.px + g.pvx * dt, -LX, LX)
    g.pz = clamp(g.pz + g.pvz * dt, -LZ, LZ)
    if fx ~= 0 or fz ~= 0 then
        g.face = atan(-fx, -fz)
    elseif mx ~= 0 or mz ~= 0 then
        g.face = atan(-mx, -mz)
    end
    g.fire_cd = g.fire_cd - dt
    if (fx ~= 0 or fz ~= 0) and g.fire_cd <= 0 then
        fire_bullet(fx, fz)
        g.fire_cd = FIRE_INT
    end
    -- draw
    local blink = g.invuln > 0 and (floor(g.time * 14) % 2 == 0)
    show_player(not blink)
    local bob = (mx ~= 0 or mz ~= 0) and abs(sin(g.time * 16)) * 0.08 or 0
    set_translation(player.e, g.px, bob, g.pz)
    set_rotation(player.e, g.face, 0, (mx ~= 0 or mz ~= 0) and sin(g.time * 16) * 0.08 or 0)
end

local function kill_player()
    if not g.p_alive or g.invuln > 0 or g.god then return end
    g.p_alive = false
    g.lives = g.lives - 1
    g.stats.deaths = g.stats.deaths + 1
    g.state = "dying"
    g.dying_t = 2.2
    show_player(false)
    burst("white", g.px, 0.8, g.pz, 40, 13, 0.24)
    burst("cyan", g.px, 0.8, g.pz, 30, 9, 0.2)
    ring(g.px, g.pz, 7.0, 0.8, "cyan")
    ring(g.px, g.pz, 4.0, 0.5, "white")
    add_shake(1.0)
    sfx("player_die", 1.0, 1.0)
end

local function respawn_player()
    g.p_alive = true
    g.state = "running"
    g.px, g.pz, g.pvx, g.pvz = 0.0, 0.0, 0.0, 0.0
    g.invuln = 2.5
    -- clear projectiles and push hostile units out of the respawn zone
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            if u.k.proj then
                u.alive = false
                show_unit(u, false)
            elseif u.k.hostile then
                local d = sqrt(u.x * u.x + u.z * u.z)
                if d < 8.0 then
                    local ux, uz = 1.0, 0.0
                    if d > 0.01 then ux, uz = u.x / d, u.z / d end
                    local r = u.k.r
                    u.x = clamp(ux * 8.5, -HW + r, HW - r)
                    u.z = clamp(uz * 8.5, -HD + r, HD - r)
                end
            end
        end
    end
    sfx("wave_start", 1.3, 0.7)
end

-- ---------------------------------------------------------------- actor AI

local AI = {}

local function dir_to_player(u)
    local dx, dz = g.px - u.x, g.pz - u.z
    local d = sqrt(dx * dx + dz * dz) + 1e-6
    return dx / d, dz / d, d
end

local function nearest_human(x, z, max_d)
    local best, bd = nil, (max_d or 1e9) ^ 2
    for i = 1, #units do
        local h = units[i]
        if h.alive and h.k.human and h.spawn_t <= 0 then
            local dx, dz = h.x - x, h.z - z
            local d2 = dx * dx + dz * dz
            if d2 < bd then best, bd = h, d2 end
        end
    end
    return best, sqrt(bd)
end

local function wander(u, dt, speed_lo, speed_hi, t_lo, t_hi)
    u.turn_t = u.turn_t - dt
    if u.turn_t <= 0 then
        u.turn_t = frand(t_lo, t_hi)
        u.ang = frand(-math.pi, math.pi)
        u.spd = (math.random() < 0.2) and 0 or frand(speed_lo, speed_hi)
    end
    u.spd = u.spd or speed_lo
    u.vx, u.vz = cos(u.ang) * u.spd, sin(u.ang) * u.spd
    -- turn away from walls
    local m = 1.6
    if (u.x > HW - m and u.vx > 0) or (u.x < -HW + m and u.vx < 0) then u.ang = math.pi - u.ang end
    if (u.z > HD - m and u.vz > 0) or (u.z < -HD + m and u.vz < 0) then u.ang = -u.ang end
    if u.spd > 0 then u.face = atan(-u.vx, -u.vz) end
end

function AI.grunt(u, dt)
    u.st = u.st - dt
    if u.st <= 0 then
        u.moving = not u.moving
        u.st = u.moving and frand(0.2, 0.42) or frand(0.04, 0.14)
    end
    if u.moving then
        local ux, uz = dir_to_player(u)
        local sp = g.grunt_speed * 1.45
        u.vx, u.vz = ux * sp, uz * sp
        u.face = atan(-ux, -uz)
    else
        u.vx, u.vz = 0, 0
    end
end

function AI.prog(u, dt)
    local ux, uz = dir_to_player(u)
    local sp = min(8.5, 4.6 + 0.12 * g.wave) * (0.8 + 0.3 * sin(u.age * 5 + u.ph))
    u.vx, u.vz = ux * sp, uz * sp
    u.face = atan(-ux, -uz)
end

function AI.human(u, dt)
    wander(u, dt, 1.6, 2.7, 0.6, 2.2)
end

function AI.electrode(u, dt)
    u.flash_t = u.flash_t - dt
    if u.flash_t <= 0 then
        u.flash_t = 0.11
        u.flash_i = u.flash_i % #FLASH + 1
        local c = PAL[FLASH[u.flash_i]]
        for n = 1, #u.parts do set_material(u.parts[n], { color = c }) end
    end
end

function AI.hulk(u, dt)
    u.turn_t = u.turn_t - dt
    if u.turn_t <= 0 then
        u.turn_t = frand(0.8, 2.2)
        local tx, tz = g.px, g.pz
        if math.random() < 0.55 then
            local h = nearest_human(u.x, u.z, 30)
            if h then tx, tz = h.x, h.z end
        end
        u.ang = atan(tz - u.z, tx - u.x) + frand(-0.9, 0.9)
        u.spd = frand(0.6, 1.0)
    end
    local sp = (2.7 + 0.05 * g.wave) * (u.spd or 0.8)
    u.vx, u.vz = cos(u.ang) * sp, sin(u.ang) * sp
    local m = 2.2
    if (u.x > HW - m and u.vx > 0) or (u.x < -HW + m and u.vx < 0) then u.ang = math.pi - u.ang end
    if (u.z > HD - m and u.vz > 0) or (u.z < -HD + m and u.vz < 0) then u.ang = -u.ang end
    u.face = atan(-u.vx, -u.vz)
end

local function fire_missile(u)
    local ux, uz = dir_to_player(u)
    local m = spawn_unit("missile", u.x, u.z, 0.0)
    m.hdg = atan(uz, ux) + frand(-0.5, 0.5)
    m.life = 4.5
    m.spd = 7.5 + 0.25 * g.wave
    sfx("missile", frand(0.95, 1.1), 0.8, 0.12)
end

function AI.brain(u, dt)
    -- hunt the nearest human, otherwise stalk the player
    u.tid = (u.tid or 0) - dt
    if u.tid <= 0 then
        u.tid = 0.4
        u.target = nearest_human(u.x, u.z, 40)
    end
    local tx, tz = g.px, g.pz
    if u.target and u.target.alive and u.target.k.human then
        tx, tz = u.target.x, u.target.z
    end
    local dx, dz = tx - u.x, tz - u.z
    local d = sqrt(dx * dx + dz * dz) + 1e-6
    local sp = min(6.0, 3.0 + 0.1 * g.wave) * (0.8 + 0.3 * sin(u.age * 3 + u.ph))
    u.vx, u.vz = dx / d * sp, dz / d * sp
    u.face = atan(-dx, -dz)
    u.fire_t = u.fire_t - dt
    if u.fire_t <= 0 and g.p_alive then
        u.fire_t = max(1.4, frand(2.8, 4.8) - 0.1 * g.wave)
        if alive_count(function(m) return m.kind == "missile" end) < 3 + floor(g.wave / 4) then
            fire_missile(u)
        end
    end
end

function AI.missile(u, dt)
    u.life = u.life - dt
    local want = atan(g.pz - u.z, g.px - u.x)
    local diff = wrap_pi(want - u.hdg)
    u.hdg = u.hdg + clamp(diff, -1.5 * dt, 1.5 * dt)
    u.vx, u.vz = cos(u.hdg) * u.spd, sin(u.hdg) * u.spd
    u.face = atan(-u.vx, -u.vz)
    if u.age % 0.06 < dt then
        spark("orange", u.x - cos(u.hdg) * 0.5, 0.7, u.z - sin(u.hdg) * 0.5, frand(-0.6, 0.6), frand(0.2, 1.0), frand(-0.6, 0.6), 0.3, 0.14)
    end
    if u.life <= 0 then kill_unit(u, false) end
end

local function wander_spawner(u, dt, speed, child, child_t, gap_lo, gap_hi)
    u.turn_t = u.turn_t - dt
    if u.turn_t <= 0 then
        u.turn_t = frand(1.0, 2.2)
        u.ang = atan(frand(-HD + 2, HD - 2) - u.z, frand(-HW + 3, HW - 3) - u.x)
    end
    u.hdg = u.hdg or u.ang
    u.hdg = u.hdg + clamp(wrap_pi(u.ang - u.hdg), -2.2 * dt, 2.2 * dt)
    u.vx, u.vz = cos(u.hdg) * speed, sin(u.hdg) * speed
    local m = 2.0
    if (u.x > HW - m and u.vx > 0) or (u.x < -HW + m and u.vx < 0) then u.ang = math.pi - u.ang u.hdg = math.pi - u.hdg end
    if (u.z > HD - m and u.vz > 0) or (u.z < -HD + m and u.vz < 0) then u.ang = -u.ang u.hdg = -u.hdg end
    u.face = u.face + dt * 3
    u.fire_t = u.fire_t - dt
    if u.fire_t <= 0 then
        if u.n_left > 0 then
            u.n_left = u.n_left - 1
            spawn_unit(child, u.x, u.z, child_t)
            sfx("spawn", 0.8, 0.8, 0.05)
            ring(u.x, u.z, 2.2, 0.4, K[u.kind].col)
            u.fire_t = frand(gap_lo, gap_hi)
        else
            -- spent: vanishes without a score
            kill_unit(u, false)
        end
    end
end

function AI.spheroid(u, dt)
    wander_spawner(u, dt, 5.5 + 0.05 * g.wave, "enforcer", 0.6, 1.0, 1.8)
end

function AI.quark(u, dt)
    wander_spawner(u, dt, 3.6, "tank", 0.8, 2.4, 3.6)
end

function AI.enforcer(u, dt)
    local ux, uz, d = dir_to_player(u)
    local sp = (3.2 + 0.06 * g.wave) * (0.6 + 0.7 * (0.5 + 0.5 * sin(u.age * 2.3 + u.ph)))
    local want = 0.35
    if d > 9 then want = 1.0 elseif d < 5 then want = -0.7 end
    local side = (u.ph > 3.14) and 1 or -1
    u.vx = (ux * want + -uz * side * 0.55) * sp
    u.vz = (uz * want + ux * side * 0.55) * sp
    u.face = u.face + dt * 4
    u.fire_t = u.fire_t - dt
    if u.fire_t <= 0 and g.p_alive and d > 2.5 then
        u.fire_t = max(0.9, frand(1.6, 3.2) - 0.05 * g.wave)
        local sp2 = 9.0 + 0.1 * g.wave
        local t = d / sp2
        local ax, az = g.px + g.pvx * t * 0.7 - u.x, g.pz + g.pvz * t * 0.7 - u.z
        local al = sqrt(ax * ax + az * az) + 1e-6
        local s = spawn_unit("spark", u.x, u.z, 0.0)
        s.vx, s.vz = ax / al * sp2, az / al * sp2
        s.life = 4.0
        sfx("spark", frand(0.95, 1.1), 0.6, 0.08)
    end
end

function AI.spark(u, dt)
    u.life = u.life - dt
    if u.life <= 0 or abs(u.x) >= HW - 0.2 or abs(u.z) >= HD - 0.2 then
        kill_unit(u, false)
    end
end

function AI.tank(u, dt)
    local ux, uz, d = dir_to_player(u)
    u.st = u.st - dt
    if u.st <= 0 then
        u.moving = not u.moving
        u.st = u.moving and frand(0.8, 1.8) or frand(0.4, 1.0)
    end
    local sp = (u.moving and d > 4) and (2.6 + 0.04 * g.wave) or 0
    u.vx, u.vz = ux * sp, uz * sp
    u.face = u.face + clamp(wrap_pi(atan(-ux, -uz) - u.face), -3 * dt, 3 * dt)
    u.fire_t = u.fire_t - dt
    if u.fire_t <= 0 and g.p_alive and d > 3 then
        u.fire_t = max(1.2, frand(2.2, 3.8) - 0.05 * g.wave)
        local sp2 = 11.0 + 0.1 * g.wave
        local fx, fz = -sin(u.face), -cos(u.face)
        local s = spawn_unit("shell", u.x + fx * 1.2, u.z + fz * 1.2, 0.0)
        s.vx, s.vz = fx * sp2, fz * sp2
        s.life = 5.0
        s.bounces = 0
        sfx("tank_shot", frand(0.95, 1.1), 0.8, 0.1)
    end
end

function AI.shell(u, dt)
    u.life = u.life - dt
    local bounced = false
    if (u.x > HW - 0.3 and u.vx > 0) or (u.x < -HW + 0.3 and u.vx < 0) then u.vx = -u.vx bounced = true end
    if (u.z > HD - 0.3 and u.vz > 0) or (u.z < -HD + 0.3 and u.vz < 0) then u.vz = -u.vz bounced = true end
    if bounced then
        u.bounces = u.bounces + 1
        sfx("bounce", frand(0.9, 1.2), 0.5, 0.05)
        burst("yellow", u.x, 0.7, u.z, 4, 5, 0.12)
    end
    u.face = u.face + dt * 6
    if u.life <= 0 or u.bounces > 3 then kill_unit(u, false) end
end

local function update_units(dt)
    local grunts = nil
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            local k = u.k
            u.age = u.age + dt
            if u.spawn_t > 0 then
                u.spawn_t = u.spawn_t - dt
                u.face = u.face + dt * 10
                if u.spawn_t <= 0 then
                    ring(u.x, u.z, 1.8, 0.35, k.col)
                    burst(k.col, u.x, 0.6, u.z, 8, 5, 0.14)
                    sfx("spawn", frand(0.9, 1.3), 0.5, 0.06)
                end
            else
                local ai = AI[k.ai or u.kind]
                if ai then ai(u, dt) end
                if u.alive then
                    -- move
                    u.x = u.x + (u.vx + u.kx) * dt
                    u.z = u.z + (u.vz + u.kz) * dt
                    local f = max(0, 1 - 7 * dt)
                    u.kx, u.kz = u.kx * f, u.kz * f
                    if not k.proj then
                        local r = k.r
                        local lx, lz = HW - r, HD - r
                        if u.x > lx then u.x = lx elseif u.x < -lx then u.x = -lx end
                        if u.z > lz then u.z = lz elseif u.z < -lz then u.z = -lz end
                    end
                end
            end
        end
    end

    -- soft separation for crowds (every unit pair that overlaps pushes apart)
    for i = 1, #units - 1 do
        local a = units[i]
        if a.alive and a.spawn_t <= 0 and not a.k.proj and a.kind ~= "electrode" then
            for j = i + 1, #units do
                local b = units[j]
                if b.alive and b.spawn_t <= 0 and not b.k.proj and b.kind ~= "electrode" then
                    local dx, dz = b.x - a.x, b.z - a.z
                    if dx < 1.6 and dx > -1.6 and dz < 1.6 and dz > -1.6 then
                        local rr = (a.k.r + b.k.r) * 0.8
                        local d2 = dx * dx + dz * dz
                        if d2 < rr * rr and d2 > 1e-6 then
                            local d = sqrt(d2)
                            local push = (rr - d) * 0.5
                            dx, dz = dx / d, dz / d
                            if not a.k.hulk then a.x, a.z = a.x - dx * push, a.z - dz * push end
                            if not b.k.hulk then b.x, b.z = b.x + dx * push, b.z + dz * push end
                        end
                    end
                end
            end
        end
    end
end

local function render_units()
    local t = g.time
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            local k = u.k
            local y, sc = 0, 1
            if u.spawn_t > 0 then
                local p = 1 - u.spawn_t / u.spawn_total
                sc = (0.15 + 0.85 * p) * (1 + 0.25 * sin(t * 55))
                y = (1 - p) * 2.0
            else
                if k.bob then y = k.bob[1] * (0.5 + 0.5 * sin(t * k.bob[2] + u.ph)) end
                if k.pulse then sc = 1 + 0.14 * sin(t * 9 + u.ph) end
            end
            local roll, pitch = 0, 0
            if k.walk and (u.vx ~= 0 or u.vz ~= 0) then
                roll = sin(t * 17 + u.ph) * (k.hulk and 0.05 or 0.14)
                y = y + abs(sin(t * 17 + u.ph)) * (k.hulk and 0.05 or 0.1)
            end
            if k.spin then u.face = u.face + k.spin * 0.016 end
            if k.tumble then
                pitch = u.age * 1.4 + u.ph
                roll = u.age * 0.9
                u.face = u.face + 0.03
            end
            if k.proj and not k.spin then pitch = 0 end
            set_translation(u.e, u.x, y, u.z)
            set_rotation(u.e, u.face, pitch, roll)
            if sc ~= u.sc then
                u.sc = sc
                set_scale(u.e, sc, sc, sc)
            end
        end
    end
end

-- ---------------------------------------------------------------- collisions

local function seg_dist2(ax, az, bx, bz, cx, cz)
    local dx, dz = bx - ax, bz - az
    local l2 = dx * dx + dz * dz
    local t = 0
    if l2 > 1e-9 then
        t = ((cx - ax) * dx + (cz - az) * dz) / l2
        if t < 0 then t = 0 elseif t > 1 then t = 1 end
    end
    local qx, qz = ax + dx * t - cx, az + dz * t - cz
    return qx * qx + qz * qz, t
end

local function update_bullets(dt)
    for i = 1, #pbullets do
        local b = pbullets[i]
        if b.alive then
            local ox, oz = b.x, b.z
            b.x, b.z = b.x + b.vx * dt, b.z + b.vz * dt
            b.life = b.life - dt
            local best, bt = nil, 2
            for j = 1, #units do
                local u = units[j]
                if u.alive and (u.k.shoot or u.k.hulk) and u.spawn_t <= 0 then
                    local rr = u.k.r + B_R
                    local d2, t = seg_dist2(ox, oz, b.x, b.z, u.x, u.z)
                    if d2 < rr * rr and t < bt then best, bt = u, t end
                end
            end
            local dead = b.life <= 0
            if best then
                dead = true
                if best.k.hulk then
                    local l = sqrt(b.vx * b.vx + b.vz * b.vz)
                    best.kx = best.kx + b.vx / l * 7.0
                    best.kz = best.kz + b.vz / l * 7.0
                    burst("green", b.x, 0.8, b.z, 3, 4, 0.12)
                    sfx("hulk_hit", frand(0.9, 1.1), 0.5, 0.06)
                else
                    kill_unit(best, true)
                end
            elseif abs(b.x) > HW or abs(b.z) > HD then
                dead = true
                burst("yellow", clamp(b.x, -HW, HW), 0.7, clamp(b.z, -HD, HD), 3, 4, 0.1)
            end
            if dead then
                b.alive = false
                b.vis = false
                set_vis(b.e, false)
            else
                set_translation(b.e, b.x, 0.75, b.z)
            end
        end
    end
end

local function update_interactions(dt)
    local px, pz = g.px, g.pz
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 then
            local k = u.k
            if k.hostile then
                if g.p_alive then
                    local dx, dz = u.x - px, u.z - pz
                    local rr = k.r * 0.9 + P_R
                    if dx * dx + dz * dz < rr * rr then kill_player() end
                end
                if k.hulk then
                    -- hulks crush humans
                    for j = 1, #units do
                        local h = units[j]
                        if h.alive and h.k.human and h.spawn_t <= 0 then
                            local dx, dz = h.x - u.x, h.z - u.z
                            local rr = k.r + h.k.r * 0.7
                            if dx * dx + dz * dz < rr * rr then
                                kill_unit(h, false)
                                g.stats.lost = g.stats.lost + 1
                                g.stats.lost_hulk = (g.stats.lost_hulk or 0) + 1
                                popup(h.x, h.z, "X", "red")
                            end
                        end
                    end
                elseif u.kind == "brain" then
                    for j = 1, #units do
                        local h = units[j]
                        if h.alive and h.k.human and h.spawn_t <= 0 then
                            local dx, dz = h.x - u.x, h.z - u.z
                            local rr = k.r + h.k.r * 0.8
                            if dx * dx + dz * dz < rr * rr then
                                h.alive = false
                                show_unit(h, false)
                                spawn_unit("prog", h.x, h.z, 0.5)
                                burst("red", h.x, 0.7, h.z, 14, 6)
                                ring(h.x, h.z, 2.4, 0.5, "red")
                                sfx("convert", 1.0, 0.9, 0.1)
                                g.stats.lost = g.stats.lost + 1
                                g.stats.lost_brain = (g.stats.lost_brain or 0) + 1
                            end
                        end
                    end
                end
            elseif k.human and g.p_alive then
                local dx, dz = u.x - px, u.z - pz
                local rr = k.r + P_R + 0.15
                if dx * dx + dz * dz < rr * rr then
                    g.rescue_n = g.rescue_n + 1
                    local pts = min(5000, 1000 * g.rescue_n)
                    u.alive = false
                    show_unit(u, false)
                    add_score(pts)
                    g.stats.rescued = g.stats.rescued + 1
                    popup(u.x, u.z, tostring(pts), "yellow", 30)
                    burst("white", u.x, 0.9, u.z, 18, 7, 0.16)
                    burst("yellow", u.x, 0.9, u.z, 12, 5, 0.16)
                    ring(u.x, u.z, 3.0, 0.5, "yellow")
                    sfx("rescue", 0.9 + 0.08 * min(g.rescue_n, 5), 1.0)
                end
            end
        end
    end
end

-- ---------------------------------------------------------------- effects

local function update_effects(dt)
    for _, pool in pairs(sparks) do
        for i = 1, #pool do
            local o = pool[i]
            if o.alive then
                o.life = o.life - dt
                if o.life <= 0 then
                    o.alive = false
                    o.vis = false
                    set_vis(o.e, false)
                else
                    o.vy = o.vy - o.grav * dt
                    o.x, o.y, o.z = o.x + o.vx * dt, o.y + o.vy * dt, o.z + o.vz * dt
                    if o.grav > 0 and o.y < 0.08 then
                        o.y = 0.08
                        o.vy = -o.vy * 0.4
                        o.vx, o.vz = o.vx * 0.7, o.vz * 0.7
                    end
                    local s = o.size * (0.25 + 0.75 * o.life / o.max)
                    set_translation(o.e, o.x, o.y, o.z)
                    set_scale(o.e, s, s, s)
                end
            end
        end
    end
    local keep = {}
    for i = 1, #g.popups do
        local p = g.popups[i]
        p.t = p.t + dt
        if p.t < 1.3 then keep[#keep + 1] = p end
    end
    g.popups = keep
    g.shake = max(0, g.shake - dt * 2.4)
    if g.banner_t > 0 then g.banner_t = g.banner_t - dt end
end

local function update_walls(dt)
    g.wall_hue = (g.wall_hue + dt * 0.12) % 1
    local h = g.wall_hue * 6
    local i = floor(h)
    local f = h - i
    local r, gg, b
    local q = 1 - f
    if i == 0 then r, gg, b = 1, f, 0
    elseif i == 1 then r, gg, b = q, 1, 0
    elseif i == 2 then r, gg, b = 0, 1, f
    elseif i == 3 then r, gg, b = 0, q, 1
    elseif i == 4 then r, gg, b = f, 0, 1
    else r, gg, b = 1, 0, q end
    -- keep the border bright but not white: mix toward saturated colours
    local glow = 1.3 + g.shake * 3
    for n = 1, #g.walls do
        set_material(g.walls[n], { color = { r, gg, b }, emissive = glow })
    end
end

local function update_camera(dt)
    if not g.cam then return end
    local vp = get_viewport_size()
    local aspect = (vp and vp.height and vp.height > 0) and (vp.width / vp.height) or 1.78
    local t = math.tan(math.rad(FOV) * 0.5)
    local dist = max((HW + 3.0) / (aspect * t), (HD + 2.6) / t * 0.93)
    local sx, sz = 0, 0
    if g.shake > 0 then
        local s = g.shake * g.shake * 0.9
        sx, sz = frand(-s, s), frand(-s, s)
    end
    -- the view leans a little toward the player, which gives the flat arena some depth
    local lx, lz = g.px * 0.05, g.pz * 0.05
    set_translation(g.cam, sx + lx, dist * cos(TILT), dist * sin(TILT) + sz + lz)
    look_at(g.cam, sx + lx, 0, 0.5 + lz)
end

-- ---------------------------------------------------------------- the autopilot

local DIRS = {
    [0] = { 0, 0 }, { 0, -1 }, { 0.7071, -0.7071 }, { 1, 0 }, { 0.7071, 0.7071 },
    { 0, 1 }, { -0.7071, 0.7071 }, { -1, 0 }, { -0.7071, -0.7071 },
}
local SIM_K, SIM_DT = 8, 0.09

local tx, tz, tvx, tvz, tr, tw, tch = {}, {}, {}, {}, {}, {}, {}
local wx, wz = {}, {}
local nthr = 0
local gx, gz, gvx, gvz, gr, gw = {}, {}, {}, {}, {}, {}   -- shootable targets
local ngt = 0
local bx_, bz_, br_ = {}, {}, {}                            -- blockers (hulks, electrodes)
local nbl = 0

local function gather_threats()
    local n = 0
    local px, pz = g.px, g.pz
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.k.hostile and u.spawn_t < 0.45 then
            local dx, dz = u.x - px, u.z - pz
            if dx * dx + dz * dz < 19 * 19 then
                n = n + 1
                local k = u.k
                tx[n], tz[n] = u.x, u.z
                tr[n] = k.r * 0.9 + (k.margin or 0.15)
                tw[n] = k.danger or 1.0
                tch[n] = 0
                if u.spawn_t > 0 then
                    tvx[n], tvz[n] = 0, 0
                elseif u.kind == "grunt" then
                    tch[n] = g.grunt_speed * 1.1
                    tvx[n], tvz[n] = 0, 0
                elseif u.kind == "prog" then
                    tch[n] = min(8.5, 4.6 + 0.12 * g.wave) * 0.95
                    tvx[n], tvz[n] = 0, 0
                else
                    tvx[n], tvz[n] = u.vx + u.kx, u.vz + u.kz
                end
            end
        end
    end
    nthr = n
end

local function pick_rescue()
    local best, bd = nil, 1e9
    for i = 1, #units do
        local h = units[i]
        if h.alive and h.k.human and h.spawn_t <= 0 then
            local dx, dz = h.x - g.px, h.z - g.pz
            local d = sqrt(dx * dx + dz * dz)
            if d < bd then
                -- skip humans that a hulk or brain is about to get
                local safe = true
                for j = 1, #units do
                    local o = units[j]
                    if o.alive and (o.k.hulk or o.kind == "brain") then
                        local ex, ez = o.x - h.x, o.z - h.z
                        local lim = o.k.hulk and 3.2 or 2.0
                        if ex * ex + ez * ez < lim * lim then safe = false break end
                    end
                end
                if safe then best, bd = h, d end
            end
        end
    end
    return best
end

local function eval_dir(c, hx, hz, hw)
    local d = DIRS[c]
    local dxm, dzm = d[1] * P_SPEED * SIM_DT, d[2] * P_SPEED * SIM_DT
    local px, pz = g.px, g.pz
    for i = 1, nthr do wx[i], wz[i] = tx[i], tz[i] end
    local cost = 0
    for s = 1, SIM_K do
        px = clamp(px + dxm, -LX, LX)
        pz = clamp(pz + dzm, -LZ, LZ)
        local disc = 1.3 - 0.11 * s
        for i = 1, nthr do
            local ch = tch[i]
            local x, z = wx[i], wz[i]
            if ch > 0 then
                local ddx, ddz = px - x, pz - z
                local dd = sqrt(ddx * ddx + ddz * ddz) + 1e-6
                local step = ch * SIM_DT
                if step > dd then step = dd end
                x, z = x + ddx / dd * step, z + ddz / dd * step
            else
                x, z = x + tvx[i] * SIM_DT, z + tvz[i] * SIM_DT
            end
            wx[i], wz[i] = x, z
            local ex, ez = x - px, z - pz
            local d2 = ex * ex + ez * ez
            local rs = tr[i] + P_R
            local zone = rs + 2.3
            if d2 < zone * zone then
                if d2 < rs * rs then
                    cost = cost + 300 * disc * tw[i]
                else
                    local f = 2.3 - (sqrt(d2) - rs)
                    cost = cost + tw[i] * disc * f * f
                end
            end
        end
        local ox = abs(px) - (LX - 1.3)
        if ox > 0 then cost = cost + ox * ox * 6 end
        local oz = abs(pz) - (LZ - 1.3)
        if oz > 0 then cost = cost + oz * oz * 6 end
    end
    if hx then
        local d0 = sqrt((g.px - hx) ^ 2 + (g.pz - hz) ^ 2)
        local d1 = sqrt((px - hx) ^ 2 + (pz - hz) ^ 2)
        cost = cost - hw * (d0 - d1)
    else
        cost = cost + 0.04 * sqrt(px * px + pz * pz)
    end
    if c == g.bprev then cost = cost - 1.2 end
    return cost
end

local function bot_move()
    gather_threats()
    local h = pick_rescue()
    local hx, hz, hw
    if h then
        hx, hz = h.x, h.z
        hw = 3.6 + 0.4 * min(g.rescue_n, 4)
    end
    local best, bc = 0, 1e18
    for c = 0, 8 do
        local cost = eval_dir(c, hx, hz, hw)
        if cost < bc then best, bc = c, cost end
    end
    g.bprev = best
    g.bmx, g.bmz = DIRS[best][1], DIRS[best][2]
end

local function bot_aim()
    local n, nb = 0, 0
    local px, pz = g.px, g.pz
    -- with family left to rescue and the wave nearly over, do not finish it off from afar
    local humans, killers = 0, 0
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            if u.k.human then humans = humans + 1 elseif u.k.kill then killers = killers + 1 end
        end
    end
    local hold = humans > 0 and killers <= 9 and g.state == "running"
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 then
            local k = u.k
            local dx, dz = u.x - px, u.z - pz
            local far = hold and dx * dx + dz * dz > 10 * 10 and u.kind ~= "brain" and u.kind ~= "missile"
            if dx * dx + dz * dz < 34 * 34 then
                if k.shoot and k.bw and not far then
                    n = n + 1
                    gx[n], gz[n], gr[n], gw[n] = u.x, u.z, k.r, k.bw
                    local vx, vz = u.vx + u.kx, u.vz + u.kz
                    if u.kind == "grunt" or u.kind == "prog" then
                        -- chasers close in on the player: use the heading they will have
                        local d = sqrt(dx * dx + dz * dz) + 1e-6
                        local sp = (u.kind == "grunt") and g.grunt_speed * 0.8 or 4.6
                        vx, vz = -dx / d * sp, -dz / d * sp
                    end
                    gvx[n], gvz[n] = vx, vz
                end
                if k.hulk or u.kind == "electrode" then
                    nb = nb + 1
                    bx_[nb], bz_[nb], br_[nb] = u.x, u.z, k.r
                    if k.hulk then
                        local d2 = dx * dx + dz * dz
                        if d2 < 7 * 7 then
                            -- shove an approaching hulk away
                            n = n + 1
                            gx[n], gz[n], gr[n], gw[n] = u.x, u.z, k.r, 1.5 * (1 - sqrt(d2) / 7)
                            gvx[n], gvz[n] = u.vx, u.vz
                            nb = nb - 1 -- a hulk target is not its own blocker
                        end
                    end
                end
            end
        end
    end
    ngt, nbl = n, nb
    local best, bv = 0, 0.05
    for d = 1, 8 do
        local ux, uz = DIRS[d][1], DIRS[d][2]
        local v = 0
        for i = 1, n do
            local rx, rz = gx[i] - px, gz[i] - pz
            local along = rx * ux + rz * uz
            if along > 0.2 and along < 36 then
                local tt = along / B_SPEED
                local lx, lz = rx + gvx[i] * tt, rz + gvz[i] * tt
                local a2 = lx * ux + lz * uz
                local perp = abs(lx * uz - lz * ux)
                if a2 > 0.2 and perp < gr[i] + 0.12 then
                    local blocked = false
                    for b = 1, nb do
                        local brx, brz = bx_[b] - px, bz_[b] - pz
                        local ba = brx * ux + brz * uz
                        if ba > 0 and ba < a2 - 0.3 then
                            local bp = abs(brx * uz - brz * ux)
                            if bp < br_[b] + 0.1 then blocked = true break end
                        end
                    end
                    if not blocked then v = v + gw[i] / (1 + a2 * 0.12) end
                end
            end
        end
        if d == g.aprev then v = v * 1.15 end
        if v > bv then best, bv = d, v end
    end
    g.aprev = best
    if best == 0 then
        g.bfx, g.bfz = 0, 0
    else
        g.bfx, g.bfz = DIRS[best][1], DIRS[best][2]
    end
end

function bot_think(dt)
    g.bot_t = g.bot_t - dt
    if g.bot_t <= 0 then
        g.bot_t = 0.07
        bot_move()
    end
    g.aim_t = g.aim_t - dt
    if g.aim_t <= 0 then
        g.aim_t = 0.05
        bot_aim()
    end
end

-- ---------------------------------------------------------------- game flow

local function step_game(dt)
    if g.state == "running" then
        update_player(dt)
        update_units(dt)
        update_bullets(dt)
        update_interactions(dt)
        render_units()
        if g.state == "running" and alive_count(killable) == 0 then
            finish_wave()
        end
    elseif g.state == "dying" then
        update_player(dt)
        update_units(dt)
        update_bullets(dt)
        render_units()
        g.dying_t = g.dying_t - dt
        if g.dying_t <= 0 then
            if g.lives > 0 then
                respawn_player()
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
        render_units()
        g.clear_t = g.clear_t - dt
        if g.clear_t <= 0 then start_wave(g.wave + 1) end
    elseif g.state == "over" then
        update_units(dt)
        render_units()
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

function on_created(self_entity)
    local ok, t = pcall(os.time)
    math.randomseed(ok and t or 2084)
    g.cam = find_entity_by_name("GameCamera")
    for _, n in ipairs({ "Wall_N", "Wall_S", "Wall_W", "Wall_E" }) do
        local e = find_entity_by_name(n)
        if e then g.walls[#g.walls + 1] = e end
    end
    build_player()
    g.music = play_sound("content/audio/music.wav", { volume = MUSIC_VOLUME, loop = true })
    g.ready = true
    new_game(true)
end

function on_destroyed(self_entity, reason)
    if g.music then stop_sound(g.music) end
    for i = 1, #all_entities do despawn_entity(all_entities[i]) end
    all_entities = {}
    g.ready = false
end

-- GAME.prof: moving average of the milliseconds Lua spends per section of update (about 0.5 ms in total)
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
    local c3 = clock()
    update_walls(dt)
    local c4 = clock()
    local k = 0.05
    prof.step = prof.step + ((c1 - c0) * 1000 - prof.step) * k
    prof.fx = prof.fx + ((c2 - c1) * 1000 - prof.fx) * k
    prof.cam = prof.cam + ((c3 - c2) * 1000 - prof.cam) * k
    prof.walls = prof.walls + ((c4 - c3) * 1000 - prof.walls) * k
    prof.total = prof.step + prof.fx + prof.cam + prof.walls
end

-- ---------------------------------------------------------------- HUD

local function colv(name, a) local c = PAL[name] or PAL.white return { c[1], c[2], c[3], a or 1.0 } end

function on_ui(self_entity, ui)
    if not g.ready then return end
    local score = string.format("%07d", g.score)
    ui:screen_label("top_left", 24, 14, "SCORE " .. score, { size = 30, color = { 1, 0.95, 0.5, 1 } })
    ui:screen_label("top_left", 24, 52, "HIGH  " .. string.format("%07d", g.hi), { size = 18, color = { 0.6, 0.8, 1, 0.9 } })
    ui:screen_label("top_center", 0, 14, "WAVE " .. g.wave, { size = 30, color = { 0.4, 1, 0.9, 1 } })
    local lives = ""
    for _ = 1, min(g.lives, 12) do lives = lives .. "I " end
    ui:screen_label("top_right", 24, 14, "LIVES " .. lives, { size = 30, color = { 0.6, 1, 1, 1 } })
    if g.bot and not g.demo then
        ui:screen_label("top_right", 24, 52, "AUTOPILOT", { size = 18, color = { 1, 0.6, 0.2, 1 } })
    end

    if g.demo then
        if g.demo_t < 7.0 then
            ui:screen_label("center", 0, -150, "ROBOTRON 2084", { size = 64, color = colv(FLASH[floor(g.time * 6) % #FLASH + 1], min(0.9, (7.0 - g.demo_t) * 0.5)) })
        end
        if floor(g.time * 2) % 2 == 0 then
            ui:screen_label("bottom_center", 0, 60, "PRESS ENTER TO PLAY", { size = 34, color = { 1, 1, 1, 1 } })
        end
        ui:screen_label("bottom_center", 0, 24, "DEMO  -  AUTOPILOT   |   WASD move   ARROWS or mouse fire", { size = 16, color = { 0.7, 0.8, 1, 0.9 } })
    end
    if g.paused then
        ui:screen_label("center", 0, -40, "PAUSED", { size = 56, color = { 1, 1, 1, 1 } })
    end
    if g.banner_t > 0 and g.banner ~= "" and not g.paused then
        local a = min(1, g.banner_t * 2)
        ui:screen_label("center", 0, -40, g.banner, { size = 56, color = { 1, 1, 1, a } })
    end
    for i = 1, #g.popups do
        local p = g.popups[i]
        local c = colv(p.col, 1 - p.t / 1.3)
        ui:world_label(p.x, 1.2 + p.t * 1.6, p.z, p.text, { size = p.size, color = c })
    end
end

-- ---------------------------------------------------------------- test hooks
-- Globals for lua_eval {entity = "GameController"}: RT.jump(5), RT.spawn("brain", 3, 2), RT.god(true) ...
RT = {
    jump = function(w)
        g.p_alive, g.state = true, "running"
        show_player(true)
        start_wave(w)
    end,
    spawn = function(kind, x, z) return spawn_unit(kind, x, z, 0.2) end,
    god = function(on) g.god = on end,
    kill = function() g.invuln, g.god = 0, false kill_player() end,
    lives = function(n) g.lives = n end,
    bot = function(on) g.bot = on end,
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
    entities = function() return #all_entities end,
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
}
