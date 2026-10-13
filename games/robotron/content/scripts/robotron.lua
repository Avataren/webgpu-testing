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
-- Every actor is a voxel model of its 1982 sprite (sprites.lua): one cube per same-coloured
-- rectangle of pixels, standing on a lit floor so it casts a real shadow. The same pixels drive
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
-- and effect is spawned here, lazily, and reused. on_destroyed despawns all of it, so hot reloads do
-- not leak entities.

local SPR = require("sprites")
local COLORS = SPR.COLORS

local HW, HD = 19.0, 12.0                -- arena half extents inside the border (x, z)
local P_R = 0.42                         -- player hit radius
local LX, LZ = HW - 0.45, HD - 0.45      -- player position limits
local P_SPEED = 9.0
local FIRE_INT = 0.085
local B_SPEED, B_R = 38.0, 0.16
local N_PBULLETS = 24
local START_LIVES = 3
local EXTRA_LIFE = 25000
local SFX_VOLUME = 0.55
local PLAY_MUSIC = false                 -- the original has no music during play
local MUSIC_VOLUME = 0.12
local VOX = 0.125                        -- one sprite pixel, in meters
local LEAN = math.rad(16)                -- actors lean back toward the camera so the sprite reads
local CAM_PITCH = math.rad(60)           -- camera angle below the horizon
local FOV = 26.0
local MAT_T = 0.75                       -- materialize time
local SPREAD = 10.0                      -- how far apart the pixel lines start
local TRANS_T = 0.95                     -- rectangle zoom

-- Arcade units. The 1982 game runs 60 frames a second and its player covers one pixel a frame, so
-- one pixel is PXM here; x positions there are bytes of two pixels (BYM). Enemy speeds, step sizes
-- and timers below are the original's, converted with these.
local FR = 1 / 60
local PXM = P_SPEED / 60
local BYM = 2 * PXM
local GRUNT_STEP = 4 * PXM
local PROG_SPEED = 4 * PXM / (3 * FR)

local atan, sqrt, sin, cos, abs, floor, min, max = math.atan, math.sqrt, math.sin, math.cos, math.abs, math.floor, math.min, math.max
local CL, SL = cos(LEAN), sin(LEAN)

-- ---------------------------------------------------------------- helpers

local function clamp(v, lo, hi)
    if v < lo then return lo elseif v > hi then return hi end
    return v
end

local function frand(a, b) return a + (b - a) * math.random() end
local function randu(n) return math.random(1, max(1, floor(n))) end -- 1..n, like the original's RMAX
local function srand(n) return math.random(0, 2 * n - 1) - n end    -- -n..n-1
local function sign(v) if v > 0 then return 1 elseif v < 0 then return -1 end return 0 end
local function wrap_pi(a)
    while a > math.pi do a = a - 2 * math.pi end
    while a < -math.pi do a = a + 2 * math.pi end
    return a
end

-- glowing unlit colours for the transition rectangles and the death flash
local FLASH = { "red", "orange", "yellow", "green", "cyan", "blue", "magenta", "white" }
for _, n in ipairs(FLASH) do
    local c = COLORS[n == "green" and "lgreen" or n]
    COLORS["fx_" .. n] = { c[1], c[2], c[3], 1.6, true }
end

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
    ready = false, cam = nil, cam_fit = nil, walls = {}, wall_hue = 0, demo_t = 0, paused = false,
}
GAME = g -- global so lua_eval can inspect and poke the game state while testing

local units = {}        -- every actor / hostile projectile, alive or pooled
local pools = {}        -- model key -> list of units
local pbullets = {}     -- player bullets
local all_entities = {} -- roots this script spawned, for on_destroyed
local player = nil
local sfx_last = {}
local rects = {}        -- transition rectangles, 4 bars each

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

-- ---------------------------------------------------------------- voxel models

-- one hidden template entity per colour; every cube of that colour draws with its material
local mats = {}
local function mat_of(name)
    local m = mats[name]
    if m then return m end
    local c = COLORS[name]
    local e = reg(spawn_primitive("cube", "RT_mat_" .. name))
    set_material(e, { color = { c[1], c[2], c[3] }, emissive = c[4], unlit = c[5] or false, roughness = 0.55, metallic = 0.05 })
    set_component(e, "Visible", false)
    set_component(e, "CanCastShadow", false)
    set_translation(e, 0, -80, 0)
    mats[name] = e
    return e
end

-- merge same-coloured pixels into rectangles (grid[r][c] = colour or nil)
local function greedy(grid, W, H)
    local used, out = {}, {}
    for r = 1, H do used[r] = {} end
    for r = 1, H do
        for c = 1, W do
            local col = grid[r][c]
            if col and not used[r][c] then
                local w = 1
                while c + w <= W and grid[r][c + w] == col and not used[r][c + w] do w = w + 1 end
                local h, ok = 1, true
                while ok and r + h <= H do
                    for i = 0, w - 1 do
                        if grid[r + h][c + i] ~= col or used[r + h][c + i] then ok = false break end
                    end
                    if ok then h = h + 1 end
                end
                for rr = r, r + h - 1 do
                    for cc = c, c + w - 1 do used[rr][cc] = true end
                end
                out[#out + 1] = { c = c, r = r, w = w, h = h, col = col }
            end
        end
    end
    return out
end

local function prep_model(key, def)
    local frames = def.frames
    local H, W = #frames[1], #frames[1][1]
    local grids = {}
    for f, rows in ipairs(frames) do
        if #rows ~= H then error(string.format("sprite %s frame %d has %d rows, expected %d", key, f, #rows, H)) end
        local gr = {}
        for r, line in ipairs(rows) do
            if #line ~= W then error(string.format("sprite %s frame %d row %d is %d wide, expected %d", key, f, r, #line, W)) end
            gr[r] = {}
            for c = 1, W do
                local ch = line:sub(c, c)
                if ch ~= "." then
                    local col = def.legend[ch]
                    if not col or not COLORS[col] then error(string.format("sprite %s: no colour for '%s'", key, ch)) end
                    gr[r][c] = col
                end
            end
        end
        grids[f] = gr
    end
    local nf = #frames
    local static, per = {}, {}
    for f = 1, nf do per[f] = {} end
    for r = 1, H do
        static[r] = {}
        for f = 1, nf do per[f][r] = {} end
        for c = 1, W do
            local v = grids[1][r][c]
            local same = v ~= nil
            for f = 2, nf do
                if grids[f][r][c] ~= v then same = false break end
            end
            if same then
                static[r][c] = v
            else
                for f = 1, nf do per[f][r][c] = grids[f][r][c] end
            end
        end
    end
    local m = { key = key, W = W, H = H, depth = def.depth or 3, nf = nf, static = greedy(static, W, H), frames = {}, px = {} }
    if nf > 1 then
        for f = 1, nf do m.frames[f] = greedy(per[f], W, H) end
    end
    -- frame-1 pixels in model space (x, height, colour) for the particle effects
    for r = 1, H do
        for c = 1, W do
            local col = grids[1][r][c]
            if col then m.px[#m.px + 1] = { (c - 0.5 - W / 2) * VOX, (H - r + 0.5) * VOX, col } end
        end
    end
    m.height = H * VOX
    return m
end

local models = {}
local function model(key)
    local m = models[key]
    if m then return m end
    local def
    local v = key:match("^electrode(%d+)$")
    if v then def = SPR.electrode(tonumber(v)) else def = SPR[key] end
    m = prep_model(key, def)
    models[key] = m
    return m
end

-- builds the cubes of model m under root; returns the frame nodes and every cube with its colour
local function build_model(root, m, cast)
    local cubes = {}
    local function add(parent, list)
        for _, rc in ipairs(list) do
            local e = spawn_primitive("cube", "RT_v", { material_of = mat_of(rc.col) })
            set_parent(e, parent)
            set_translation(e, (rc.c - 1 + rc.w / 2 - m.W / 2) * VOX, (m.H - (rc.r - 1) - rc.h / 2) * VOX, 0)
            set_scale(e, rc.w * VOX, rc.h * VOX, m.depth * VOX)
            if not cast then set_component(e, "CanCastShadow", false) end
            cubes[#cubes + 1] = { e, rc.col }
        end
    end
    add(root, m.static)
    local fnodes = {}
    if m.nf > 1 then
        for f = 1, m.nf do
            local n = spawn_entity("RT_frame")
            set_parent(n, root)
            if f ~= 1 then set_component(n, "Visible", false) end
            add(n, m.frames[f])
            fnodes[f] = n
        end
    end
    return fnodes, cubes
end

-- model-space point (x, height) of an upright, leaning, camera-facing actor at (wx, wy, wz) -> world
local function lean_pt(wx, wy, wz, x, h)
    return wx + x, wy + h * CL, wz - h * SL
end

-- one pixel as a GPU particle: no drag, so it travels in a straight line for `life` seconds
local function px_particle(x, y, z, col, vx, vy, vz, life, inten, fade)
    local c = COLORS[col]
    local cs = { c[1], c[2], c[3], 1 }
    emit_particles(x, y, z, {
        count = 1, color = cs, color_end = fade and { c[1] * 0.4, c[2] * 0.4, c[3] * 0.4, 0 } or cs,
        intensity = inten or 1.6, speed = 0, life = life, size = VOX * 1.1, size_end = fade and VOX * 0.7 or VOX * 1.1,
        velocity = { vx, vy, vz }, drag = 0,
    })
end

-- MATERIALIZE: the sprite's pixel rows (mode "v") or columns ("h") start far apart and close up;
-- "r" spreads both ways a little (an enforcer growing out of a spheroid)
local function materialize(m, wx, wy, wz, mode, t)
    local hc = m.height * 0.5
    for i = 1, #m.px do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        local ox, oz = 0, 0
        if mode == "h" then
            ox = p[1] * SPREAD * 1.6
        elseif mode == "v" then
            oz = -(p[2] - hc) * SPREAD
        else
            ox, oz = p[1] * 3.5, -(p[2] - hc) * 3.5
        end
        px_particle(fx + ox, fy, fz + oz, p[3], -ox / t, 0, -oz / t, t, 1.7, false)
    end
end

-- EXPLODE: the sprite stretches apart along the line of fire (dx, dz), or every way (no direction)
local function explode(m, wx, wy, wz, dx, dz, life, k)
    life = life or 0.55
    k = k or 9.0
    local hc = m.height * 0.5
    local horiz = dx and abs(dx) >= abs(dz)
    local vert = dx and not horiz
    for i = 1, #m.px do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        local vx, vz = 0, 0
        if horiz then
            vx = p[1] * k * 1.6 + dx * 3.0
        elseif vert then
            vz = -(p[2] - hc) * k + dz * 3.0
        else
            vx, vz = p[1] * k, -(p[2] - hc) * k
        end
        px_particle(fx, fy, fz, p[3], vx, 0, vz, life * frand(0.8, 1.1), 2.0, true)
    end
end

-- a prog's afterimage: every pixel in one colour, left behind for a moment
local function afterimage(m, wx, wy, wz, col)
    for i = 1, #m.px, 2 do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        px_particle(fx, fy, fz - 0.05, col, 0, 0, 0, 0.22, 1.1, true)
    end
end

local function burst(col, x, y, z, count, speed, size)
    local c = COLORS[col] or COLORS.white
    emit_particles(x, y, z, { count = count, color = { c[1], c[2], c[3], 1 }, intensity = 2.2, speed = { speed * 0.25, speed },
        life = { 0.25, 0.6 }, size = (size or 0.12), size_end = 0.0, gravity = -12, drag = 1.5 })
end

-- an expanding ring of glowing particles on the floor (a shockwave)
local function ring(x, z, maxr, dur, col)
    local c = COLORS[col] or COLORS.white
    emit_particles(x, 0.15, z, { count = floor(24 + maxr * 10), shape = "ring", color = { c[1], c[2], c[3], 1 }, intensity = 2.5,
        speed = maxr / dur, life = dur, size = 0.14, size_end = 0.04, drag = 0 })
end

local function add_shake(a) g.shake = min(1.0, max(g.shake, a)) end

local function popup(x, z, text, col, size)
    if #g.popups >= 16 then table.remove(g.popups, 1) end
    g.popups[#g.popups + 1] = { x = x, z = z, text = text, col = col or "white", t = 0, size = size or 22 }
end

-- ---------------------------------------------------------------- actor definitions
-- r: hit radius; pts: score; y: hover height; flags drive AI, collisions and the autopilot.
-- need: the wave lasts while one is alive (the original's grunt, brain, spheroid, enforcer, quark
-- and tank counts); progs, projectiles, hulks and electrodes vanish with the wave.

local K = {}
K.grunt = { r = 0.6, pts = 100, hostile = true, shoot = true, need = true, sfx = "grunt_die", col = "red", walk = true, stepper = true, bw = 1.0, danger = 1.0 }
K.electrode = { r = 0.58, pts = 0, hostile = true, shoot = true, sfx = "electrode_die", col = "yellow", bw = 0.12, danger = 1.1 }
K.hulk = { r = 0.91, pts = 0, hostile = true, hulk = true, sfx = "hulk_hit", col = "green", walk = true, danger = 1.5, margin = 0.45 }
for _, name in ipairs({ "mommy", "daddy", "mikey" }) do
    K[name] = { r = name == "mikey" and 0.46 or 0.54, human = true, ai = "human", sfx = "human_die", col = "white", walk = true }
end
K.prog = { r = 0.54, pts = 100, hostile = true, shoot = true, sfx = "grunt_die", col = "magenta", walk = true, trail = true, bw = 1.5, danger = 1.2 }
K.brain = { r = 0.74, pts = 500, hostile = true, shoot = true, need = true, sfx = "brain_die", col = "pink", walk = true, bw = 3.0, danger = 1.1 }
K.spheroid = { r = 0.67, pts = 1000, hostile = true, shoot = true, need = true, sfx = "big_boom", col = "orange", y = 0.35, bob = { 0.2, 2.0 }, cycle = 9, bw = 2.2, danger = 1.0 }
K.enforcer = { r = 0.6, pts = 150, hostile = true, shoot = true, need = true, sfx = "grunt_die", col = "white", y = 0.5, bob = { 0.3, 4.0 }, cycle = 6, bw = 1.6, danger = 1.0 }
K.quark = { r = 0.67, pts = 1000, hostile = true, shoot = true, need = true, sfx = "big_boom", col = "cyan", y = 0.35, bob = { 0.2, 2.5 }, cycle = 8, spin = 7.0, bw = 2.2, danger = 1.0 }
K.tank = { r = 0.82, pts = 200, hostile = true, shoot = true, need = true, sfx = "brain_die", col = "blue", cycle = 0, bw = 1.2, danger = 1.0 }
K.missile = { r = 0.26, pts = 25, hostile = true, shoot = true, sfx = "grunt_die", col = "magenta", y = 0.45, bw = 2.4, danger = 1.3, margin = 0.1, proj = true }
K.spark = { r = 0.26, pts = 25, hostile = true, shoot = true, sfx = "spark", col = "purple", y = 0.45, cycle = 14, spin = 9.0, bw = 0.8, danger = 1.2, margin = 0.05, proj = true }
K.shell = { r = 0.24, pts = 25, hostile = true, shoot = true, sfx = "bounce", col = "yellow", y = 0.45, bw = 0.8, danger = 1.3, margin = 0.1, proj = true }
K.skull = { r = 0.36, deco = true, ai = "skull", col = "white" }

-- ---------------------------------------------------------------- pooled actors

local function make_unit(kind, mkey)
    local m = model(mkey)
    local root = reg(spawn_entity(uname("RT_" .. kind .. "_")))
    set_component(root, "Visible", false)
    set_translation(root, 0, -50, 0)
    local fnodes, cubes = build_model(root, m, not K[kind].proj)
    local u = { kind = kind, mkey = mkey, k = K[kind], m = m, e = root, fnodes = fnodes, cubes = cubes, frame = 1,
        vis = false, alive = false, x = 0, z = 0, vx = 0, vz = 0, ox = 0, oz = 0, spawn_t = 0, age = 0, face = 0,
        yaw = 0, ph = math.random() * 6.28, sc = 1, anim = 0 }
    units[#units + 1] = u
    return u
end

local function show_unit(u, v)
    if u.vis == v then return end
    u.vis = v
    set_vis(u.e, v)
    if not v then set_translation(u.e, 0, -50, 0) end
end

local function set_frame(u, f)
    if u.frame == f or not u.fnodes[f] then return end
    set_vis(u.fnodes[u.frame], false)
    set_vis(u.fnodes[f], true)
    u.frame = f
end

local function acquire(kind, mkey)
    mkey = mkey or kind
    local pool = pools[mkey]
    if not pool then
        pool = {}
        pools[mkey] = pool
    end
    for i = 1, #pool do
        if not pool[i].alive then return pool[i] end
    end
    local u = make_unit(kind, mkey)
    pool[#pool + 1] = u
    return u
end

-- ---------------------------------------------------------------- player

local function build_player()
    local root = reg(spawn_entity("RT_player"))
    local m = model("player")
    local fnodes, cubes = build_model(root, m, true)
    player = { e = root, m = m, fnodes = fnodes, cubes = cubes, frame = 1, vis = true, flash = false }
end

local function show_player(v)
    if player.vis == v then return end
    player.vis = v
    set_vis(player.e, v)
    if not v then set_translation(player.e, 0, -50, 0) end
end

local function flash_player(col)
    for _, c in ipairs(player.cubes) do share_material(c[1], mat_of(col or c[2])) end
    player.flash = col ~= nil
end

-- ---------------------------------------------------------------- scoring and kills

local function add_score(n)
    g.score = g.score + n
    g.scored = true
    if not g.demo and g.score > g.hi then g.hi = g.score end
    while g.score >= g.next_life do
        g.next_life = g.next_life + EXTRA_LIFE
        g.lives = g.lives + 1
        sfx("extra_life", 1.0, 1.0)
        popup(g.px, g.pz - 1.5, "EXTRA MAN", "yellow", 30)
    end
end

local function unit_y(u) return u.k.y or 0 end

-- dx, dz: direction of the shot that killed it (nil explodes every way)
local function kill_unit(u, by_player, dx, dz)
    local k = u.k
    u.alive = false
    show_unit(u, false)
    if k.proj then
        burst(k.col, u.x, unit_y(u) + 0.2, u.z, 10, 5, 0.1)
    else
        explode(u.m, u.x, unit_y(u), u.z, dx, dz)
    end
    if k.pts and k.pts >= 500 then
        ring(u.x, u.z, 3.4, 0.5, k.col)
        add_shake(0.45)
    end
    if k.sfx then sfx(k.sfx, frand(0.92, 1.12), (k.pts or 0) >= 500 and 1.0 or 0.75, 0.035) end
    if u.kind == "grunt" then
        -- every grunt down speeds the rest up by an eighth, as far as the wave's limit
        local v = floor(g.robspd * 224 / 256)
        if v >= g.rmxspd then g.robspd = v end
    end
    if by_player and k.pts and k.pts > 0 then
        add_score(k.pts)
        if k.pts >= 500 then popup(u.x, u.z, tostring(k.pts), k.col) end
        g.stats.kills = g.stats.kills + 1
    end
end

-- ---------------------------------------------------------------- spawning

local reset_ai -- per-kind AI state (actor AI section)

-- spawn_t: delay + materialize time; the actor is drawn (and dangerous) once it reaches 0
local function spawn_unit(kind, x, z, delay, extra, mode, mkey, mat_t)
    local u = acquire(kind, mkey)
    u.alive = true
    u.x, u.z, u.vx, u.vz, u.ox, u.oz = x, z, 0, 0, x, z
    u.mat_t = mat_t or MAT_T
    u.mode = mode or g.mat_mode
    u.spawn_t = (delay or 0) + u.mat_t
    if delay == false then u.spawn_t = 0 end
    u.emitted = false
    u.age = 0
    u.face = frand(-math.pi, math.pi)
    u.yaw = 0
    u.sc = -1
    u.at = 0
    u.dx, u.dz = 0, 0
    u.trail_t = 0
    u.target, u.mut, u.busy, u.victim = nil, nil, nil, nil
    reset_ai(u)
    if extra then
        for key, val in pairs(extra) do u[key] = val end
    end
    set_frame(u, 1)
    if u.spawn_t <= 0 then
        show_unit(u, true)
        set_translation(u.e, x, unit_y(u), z)
    else
        show_unit(u, false)
    end
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

local function needed(u) return u.k.need end

local function free_pos(min_center, margin, spacing, others, cx, cz)
    cx, cz = cx or 0, cz or 0
    for _ = 1, 40 do
        local x = frand(-HW + margin, HW - margin)
        local z = frand(-HD + margin, HD - margin)
        local ok = ((x - cx) ^ 2 + (z - cz) ^ 2) >= min_center * min_center
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

-- ---------------------------------------------------------------- waves (the original's tables)

local function row(str)
    local t = {}
    for n in str:gmatch("%d+") do t[#t + 1] = tonumber(n) end
    return t
end
local function runs(...) -- runs(v1, n1, v2, n2, ...): n1 copies of v1, then n2 of v2 ...
    local t, a = {}, { ... }
    for i = 1, #a, 2 do
        for _ = 1, a[i + 1] do t[#t + 1] = a[i] end
    end
    return t
end

-- enemies per wave, waves 1-40 (WVCNT); after 40 the game repeats waves 21-40
local WAVE_COUNT = {
    grunt = row("15 17 22 34 20 32 0 35 60 25 35 0 35 27 25 35 0 35 70 25 35 0 35 0 25 35 0 35 75 25 35 0 35 30 27 35 0 35 80 30"),
    electrode = row("5 15 25 25 20 25 0 25 0 20 25 0 25 5 20 25 0 25 0 20 25 0 25 0 20 25 0 25 0 20 25 0 25 0 15 25 0 25 0 15"),
    mommy = row("1 1 2 2 15 3 4 3 3 0 3 3 3 5 0 3 3 3 3 8 3 3 3 3 25 3 3 3 3 0 3 3 3 3 0 3 3 3 3 10"),
    daddy = row("1 1 2 2 0 3 4 3 3 22 3 3 3 5 0 3 3 3 3 8 3 3 3 3 0 3 3 3 3 25 3 3 3 3 0 3 3 3 3 10"),
    mikey = row("0 1 2 2 1 3 4 3 3 0 3 3 3 5 22 3 3 3 3 8 3 3 3 3 1 3 3 3 3 0 3 3 3 3 25 3 3 3 3 10"),
    hulk = row("0 5 6 7 0 7 12 8 4 0 8 13 8 20 2 3 14 8 3 2 8 15 8 13 1 8 16 8 4 1 8 16 8 25 2 8 16 8 6 2"),
    brain = row("0 0 0 0 15 0 0 0 0 20 0 0 0 0 20 0 0 0 0 20 0 0 0 0 21 0 0 0 0 22 0 0 0 0 23 0 0 0 0 25"),
    spheroid = row("0 1 3 4 1 4 0 5 5 1 5 0 5 2 1 5 0 5 5 2 5 0 5 6 1 5 0 5 5 1 5 0 5 2 1 5 0 5 5 1"),
    quark = row("0 0 0 0 0 0 10 0 0 0 0 12 0 0 0 0 12 0 0 0 0 12 0 7 0 0 12 1 1 1 1 13 1 2 2 2 14 2 1 1"),
}

-- per-wave tuning (WVTAB). Timers count in the original's frames or steps, see the actor AI.
local WAVE_PARAM = {
    robspd = row("20 15 15 15 15 15 15 15 15 15 14 14 14 14 14 13 13 13 13 13 14 14 14 14 14 14 13 13 13 13 13 13 12 12 12 12 12 12 15 12"),
    rmxspd = row("9 7 6 5 5 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 3 3 4 3 3 3 3 3 3 3 3 3 4 3"),
    enfnum = runs(10, 20, 11, 20),
    enstim = row("30 28 26 24 22 20 18 18 16 14 14 14 14 14 14 14 14 14 14 14 15 15 15 15 15 15 15 15 15 14 14 14 14 14 14 14 14 14 14 14"),
    cdptim = row("30 28 26 24 30 20 18 16 18 25 12 12 12 25 25 12 12 12 18 20 14 14 14 14 14 25 14 14 18 25 12 12 12 12 25 12 12 12 18 20"),
    hlkspd = row("8 8 7 7 7 7 7 6 6 6 6 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5"),
    bshtim = row("64 64 64 64 64 40 40 38 38 38 38 38 38 38 38 38 36 36 36 36 32 32 32 32 32 32 32 30 30 30 30 30 25 25 25 25 25 25 25 25"),
    brnspd = runs(8, 5, 7, 10, 6, 25),
    thksht = row("32 32 32 32 32 32 32 30 30 30 30 30 30 28 28 28 28 28 28 28 30 30 30 30 30 30 28 28 28 28 28 26 26 26 26 26 24 24 24 24"),
    shlspd = runs(176, 20, 184, 10, 192, 10),
    tdptim = runs(16, 14, 15, 6, 14, 20),
    sqspd = runs(50, 12, 56, 16, 60, 12),
}

local function table_wave(w)
    while w > 40 do w = w - 20 end
    return w
end

local function wave_def(w)
    local i, d = table_wave(w), {}
    for key, list in pairs(WAVE_COUNT) do d[key] = list[i] end
    return d
end

local function wave_params(w)
    local i, p = table_wave(w), {}
    for key, list in pairs(WAVE_PARAM) do p[key] = list[i] end
    return p
end
g.p = wave_params(1)
g.robspd, g.rmxspd, g.rmx0, g.speed_t = g.p.robspd, g.p.rmxspd, g.p.rmxspd, 0

local function electrode_key(w) return "electrode" .. ((w - 1) % SPR.ELECTRODE_VARIANTS + 1) end

local function clear_all_units()
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            u.alive = false
            show_unit(u, false)
        end
    end
end

local function clear_bullets()
    for i = 1, #pbullets do
        local b = pbullets[i]
        b.alive = false
        if b.vis then
            b.vis = false
            set_vis(b.e, false)
        end
    end
end

-- Hidden pooled models still cost frame time, so drop a large surplus (and last wave's electrodes).
local function trim_pools(d, ekey)
    local fam = max(d.mommy or 0, d.daddy or 0, d.mikey or 0)
    local keep = {
        grunt = d.grunt or 0, hulk = d.hulk or 0, brain = d.brain or 0, spheroid = d.spheroid or 0, quark = d.quark or 0,
        mommy = d.mommy or 0, daddy = d.daddy or 0, mikey = d.mikey or 0,
        enforcer = (d.spheroid or 0) > 0 and 8 or 0, tank = min(20, 3 * (d.quark or 0)),
        prog = (d.brain or 0) > 0 and (d.mommy or 0) + (d.daddy or 0) + (d.mikey or 0) or 0, missile = (d.brain or 0) > 0 and 8 or 0,
        spark = (d.spheroid or 0) > 0 and 12 or 0, shell = (d.quark or 0) > 0 and 12 or 0, skull = 2,
    }
    keep[ekey] = d.electrode or 0
    local removed = false
    for mkey, pool in pairs(pools) do
        local lim = keep[mkey] and keep[mkey] + 12 or 0
        for i = #pool, lim + 1, -1 do
            local u = pool[i]
            if not u.alive then
                despawn_entity(u.e)
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

local function materialize_player(delay)
    g.p_spawn = delay + MAT_T * 0.8
    g.p_emitted = false
    show_player(false)
end

-- where an actor starts: away from the player in the middle, except brains (near an edge),
-- spheroids (at the left or right wall) and quarks (at the top or bottom wall)
local function place(kind, taken)
    if kind == "brain" then
        local x, z = frand(-HW + 1.5, HW - 1.5), frand(-HD + 1.5, HD - 1.5)
        if math.random() < 0.5 then
            x = (math.random() < 0.5 and -1 or 1) * (HW - frand(1.0, 16 * BYM))
        else
            z = (math.random() < 0.5 and -1 or 1) * (HD - frand(1.0, 32 * PXM))
        end
        return x, z
    elseif kind == "spheroid" then
        return (math.random() < 0.5 and -1 or 1) * (HW - 1.0), frand(-HD + 1.5, HD - 1.5)
    elseif kind == "quark" then
        return frand(-HW + 1.5, HW - 1.5), (math.random() < 0.5 and -1 or 1) * (HD - 0.9)
    elseif kind == "electrode" then
        local x, z = free_pos(4.5, 1.5, 2.2, taken)
        taken[#taken + 1] = { x, z }
        return x, z
    elseif kind == "hulk" then
        local x, z = free_pos(8.0, 2.0, 3.0, taken)
        taken[#taken + 1] = { x, z }
        return x, z
    elseif K[kind].human then
        return free_pos(3.5, 1.5)
    end
    return free_pos(7.0, 1.0)
end

local function start_wave(w)
    g.wave = w
    g.wave_t = 0
    g.rescue_n = 0
    g.stats.waves = max(g.stats.waves, w)
    g.mat_mode = (w % 2 == 1) and "v" or "h"
    clear_all_units()
    local d = wave_def(w)
    g.p = wave_params(w)
    -- grunts step every 1..robspd ticks; robspd falls toward rmxspd as grunts die, and rmxspd
    -- itself falls every few seconds (faster when the player is not scoring)
    g.robspd, g.rmxspd, g.rmx0 = g.p.robspd, g.p.rmxspd, g.p.rmxspd
    g.speed_t, g.scored = 18 * 15 * FR, false
    local ekey = electrode_key(w)
    trim_pools(d, ekey)
    g.px, g.pz, g.pvx, g.pvz = 0.0, 0.0, 0.0, 0.0
    g.p_alive = true
    g.invuln = 1.6
    g.fire_cd = 0
    g.state = "running"
    g.banner, g.banner_t = "", 0
    clear_bullets()
    materialize_player(0.0)

    local taken = {}
    local function put(kind, count, mkey)
        for _ = 1, count do
            local x, z = place(kind, taken)
            spawn_unit(kind, x, z, frand(0.0, 0.55), nil, nil, mkey)
        end
    end
    put("electrode", d.electrode, ekey)
    put("hulk", d.hulk)
    put("mommy", d.mommy)
    put("daddy", d.daddy)
    put("mikey", d.mikey)
    put("grunt", d.grunt)
    put("brain", d.brain)
    put("spheroid", d.spheroid)
    put("quark", d.quark)
    sfx("materialize", 1.0, 0.9)
end

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

local function finish_wave()
    g.state = "clearing"
    g.clear_t = 0.7
    g.stats.cleared = g.stats.cleared + 1
    sfx("wave_clear", 1.0, 0.7)
    -- the hulks, electrodes and any family left vanish with the wave
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 and not u.k.proj and not u.k.deco then
            if u.k.human then g.stats.left = (g.stats.left or 0) + 1 end
            u.alive = false
            show_unit(u, false)
            explode(u.m, u.x, unit_y(u), u.z, nil, nil, 0.45, 6)
        elseif u.alive then
            u.alive = false
            show_unit(u, false)
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
    if player.flash then flash_player(nil) end
    if not demo then sfx("start", 1.0, 1.0) end
    begin_transition(function() start_wave(1) end)
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
        local e = reg(spawn_primitive("cube", uname("RT_bullet"), { material_of = mat_of("glow_white") }))
        set_scale(e, 0.07, 0.07, 1.1)
        set_component(e, "CanCastShadow", false)
        set_component(e, "Visible", false)
        b = { e = e, alive = false, vis = false, x = 0, z = 0, vx = 0, vz = 0, life = 0 }
        pbullets[#pbullets + 1] = b
    end
    b.alive = true
    b.x, b.z = g.px + fx * 0.5, g.pz + fz * 0.5
    b.vx, b.vz = fx * B_SPEED, fz * B_SPEED
    b.life = 1.4
    set_translation(b.e, b.x, 0.8, b.z)
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
            local gp = m and screen_to_ground(m.x, m.y, 0.8)
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

local function draw_player(mx, mz, dt)
    local moving = mx ~= 0 or mz ~= 0
    if moving then
        g.p_anim = g.p_anim + dt * 9
        local f = 2 + floor(g.p_anim) % 2
        if player.frame ~= f then
            set_vis(player.fnodes[player.frame], false)
            set_vis(player.fnodes[f], true)
            player.frame = f
        end
    elseif player.frame ~= 1 then
        set_vis(player.fnodes[player.frame], false)
        set_vis(player.fnodes[1], true)
        player.frame = 1
    end
    -- turn a little toward the way the player moves (or fires)
    local want = 0
    local hx = (g.fx_last ~= 0 and g.fx_last) or mx
    if hx ~= 0 then want = 0.55 * hx end
    g.p_yaw = g.p_yaw + (want - g.p_yaw) * min(1, dt * 12)
    set_translation(player.e, g.px, 0, g.pz)
    set_rotation(player.e, g.p_yaw, -LEAN, 0)
end

local function update_player(dt)
    if g.invuln > 0 then g.invuln = g.invuln - dt end
    if not g.p_alive then return end
    if g.p_spawn > 0 then
        g.p_spawn = g.p_spawn - dt
        if not g.p_emitted and g.p_spawn <= MAT_T * 0.8 then
            g.p_emitted = true
            materialize(player.m, g.px, 0, g.pz, g.mat_mode, MAT_T * 0.8)
        end
        if g.p_spawn <= 0 then
            show_player(true)
            g.p_yaw = 0
            draw_player(0, 0, dt)
        end
        return
    end
    local mx, mz, fx, fz = read_input(dt)
    g.fx_last = fx
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
    show_player(true)
    draw_player(mx, mz, dt)
end

local function kill_player()
    if g.state ~= "running" or not g.p_alive or g.invuln > 0 or g.god or g.p_spawn > 0 then return end
    g.p_alive = false
    g.lives = g.lives - 1
    g.stats.deaths = g.stats.deaths + 1
    g.state = "dying"
    g.dying_t = 0
    g.exploded = false
    g.flash_i = 0
    sfx("player_die", 1.0, 1.0)
end

-- After a death the wave restarts with what is left, everything in new places: progs and
-- projectiles are gone, every four enforcers left become a spheroid again (at least one if no
-- spheroid is left, never more than the wave started with), and grunts keep most of their speed.
local function respawn_player()
    g.p_alive = true
    g.state = "running"
    g.px, g.pz, g.pvx, g.pvz = 0.0, 0.0, 0.0, 0.0
    g.invuln = 1.6
    g.rescue_n = 0
    g.wave_t = 0
    g.rmxspd = g.rmx0
    g.robspd = max(g.robspd, g.rmxspd)
    g.speed_t, g.scored = 18 * 15 * FR, false
    materialize_player(0.0)
    local enforcers, spheroids = 0, 0
    local taken = {}
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            if u.kind == "enforcer" then enforcers = enforcers + 1 end
            if u.kind == "spheroid" then spheroids = spheroids + 1 end
            if u.k.proj or u.k.deco or u.kind == "prog" or u.kind == "enforcer" or u.mut then
                u.alive = false
                show_unit(u, false)
            else
                u.x, u.z = place(u.kind, taken)
                u.ox, u.oz = u.x, u.z
                u.mode = g.mat_mode
                u.spawn_t = frand(0.0, 0.5) + u.mat_t
                u.emitted = false
                u.mut, u.busy, u.victim, u.target = nil, nil, nil, nil
                reset_ai(u)
                show_unit(u, false)
            end
        end
    end
    local back = floor(enforcers / 4)
    if back == 0 and enforcers > 0 and spheroids == 0 then back = 1 end
    back = min(back, max(0, (wave_def(g.wave).spheroid or 0) - spheroids))
    for _ = 1, back do
        local x, z = place("spheroid")
        spawn_unit("spheroid", x, z, frand(0.0, 0.5))
    end
    sfx("materialize", 1.0, 0.8)
end

-- ---------------------------------------------------------------- actor AI
-- After the 1982 source: RRP8 (grunts), RRH11 (hulks, the family), RRB10 (brains, progs, cruise
-- missiles), RRC11 (spheroids, enforcers, sparks) and RRTK4 (quarks, tanks, shells). Its timers
-- count frames or an actor's own steps; they are converted to seconds with FR.

local AI = {}

local function dir_to_player(u)
    local dx, dz = g.px - u.x, g.pz - u.z
    local d = sqrt(dx * dx + dz * dz) + 1e-6
    return dx / d, dz / d, d
end

-- a human a brain or hulk can go after (not one being reprogrammed)
local function free_human(h)
    return h and h.alive and h.k.human and h.spawn_t <= 0 and not h.mut
end

local function nearest_human(x, z)
    local best, bd = nil, 1e18
    for i = 1, #units do
        local h = units[i]
        if free_human(h) then
            local d = abs(h.x - x) + abs(h.z - z) -- the original measures |dx| + |dy|
            if d < bd then best, bd = h, d end
        end
    end
    return best
end

local function random_human()
    local list = {}
    for i = 1, #units do
        local h = units[i]
        if h.alive and h.k.human then list[#list + 1] = h end
    end
    return list[math.random(1, max(1, #list))]
end

local function human_lost(h, by)
    h.alive = false
    show_unit(h, false)
    g.stats.lost = g.stats.lost + 1
    g.stats["lost_" .. by] = (g.stats["lost_" .. by] or 0) + 1
end

local function count_kind(kind)
    local n = 0
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.kind == kind then n = n + 1 end
    end
    return n
end

local function electrode_near(x, z, r)
    for i = 1, #units do
        local e = units[i]
        if e.alive and e.kind == "electrode" and e.spawn_t <= 0 then
            local dx, dz = e.x - x, e.z - z
            local rr = e.k.r + r
            if dx * dx + dz * dz < rr * rr then return e end
        end
    end
    return nil
end

local function outside(x, z, m)
    return x < -HW + m or x > HW - m or z < -HD + m or z > HD - m
end

reset_ai = function(u)
    local p, kind = g.p, u.kind
    u.st, u.nd, u.fire_t, u.phase = 0, 0, 0, nil
    u.offx, u.nv, u.esc_dir, u.vt, u.ux, u.uz = nil, nil, nil, nil, nil, nil
    if kind == "grunt" then
        u.st = randu(g.robspd) * 4 * FR
    elseif kind == "hulk" then
        u.axis = math.random() < 0.5 and "x" or "z"
        u.target = math.random() < 0.75 and random_human() or nil -- the rest go for the player
    elseif kind == "brain" then
        u.shots = randu(p.bshtim)
    elseif kind == "spheroid" or kind == "quark" then
        u.phase = "wait"
        u.n_left = floor((randu(p.enfnum) + 1) / 2)
        u.drop = randu(kind == "spheroid" and p.cdptim or p.tdptim)
        u.cyc, u.acc_t, u.ax, u.az, u.vx, u.vz = 0, 0, 0, 0, 0, 0
    elseif kind == "enforcer" then
        u.fire_t = randu(p.enstim) * 3 * FR
    elseif kind == "tank" then
        u.shots = math.random(0, 31) + p.thksht
    end
end

-- Grunts step every 1..robspd ticks of four frames: four pixels toward the player on each axis.
function AI.grunt(u, dt)
    u.st = u.st - dt
    if u.st > 0 then return end
    u.st = randu(g.robspd) * 4 * FR
    local dx, dz = g.px - u.x, g.pz - u.z
    local sx = (dx > 0.5 * BYM and 1) or (dx < -0.5 * BYM and -1) or 0
    local sz = (dz > 1.5 * PXM and 1) or (dz < -1.5 * PXM and -1) or 0
    u.x, u.z = u.x + sx * GRUNT_STEP, u.z + sz * GRUNT_STEP
    u.vx, u.vz = sx, sz
    u.anim = ((u.anim or 0) % 2) + 1
    set_frame(u, 1 + u.anim)
end

-- Hulks step every hlkspd frames along one axis, three or four pixels sideways or two up and down,
-- swapping axes each time they pick a direction (every 1..32 steps or at a wall), toward a family
-- member (a quarter of them toward the player) with a random offset.
local function hulk_dir(u)
    u.nd = randu(32)
    local t = u.target
    if not free_human(t) then
        t = nil
        u.target = nil
    end
    local tx, tz = g.px, g.pz
    if t then tx, tz = t.x, t.z end
    if u.axis == "x" then
        u.axis = "z"
        u.dx, u.dz = 0, (tz + srand(16) * PXM > u.z) and 1 or -1
    else
        u.axis = "x"
        u.dx, u.dz = (tx + srand(16) * BYM > u.x) and 1 or -1, 0
    end
end

function AI.hulk(u, dt)
    local hs = g.p.hlkspd * FR
    u.st = u.st - dt
    if u.st <= 0 then
        u.st = hs
        u.nd = u.nd - 1
        if u.nd <= 0 then hulk_dir(u) end
    end
    local m = u.k.r + 0.05
    if (u.dx == 0 and u.dz == 0) or outside(u.x + u.dx * 0.3, u.z + u.dz * 0.3, m) then hulk_dir(u) end
    u.vx, u.vz = u.dx * 3.5 * PXM / hs, u.dz * 2 * PXM / hs
end

function AI.skull(u, dt)
    u.life = u.life - dt
    if u.life <= 0 then
        u.alive = false
        show_unit(u, false)
    end
end

-- The family steps every 8 frames in one of eight directions (a pixel and a half sideways, one
-- up or down) and turns after 1..128 steps, or at a wall or an electrode.
local function human_dir(u)
    u.nd = randu(128)
    repeat
        u.dx, u.dz = math.random(-1, 1), math.random(-1, 1)
    until u.dx ~= 0 or u.dz ~= 0
end

-- a human being reprogrammed: pinned beside the brain and flickering, then a prog
local function mutate(u, dt)
    local b = u.mut
    if not (b.alive and b.victim == u) then
        -- the brain was shot mid-way: the human dies
        human_lost(u, "brain")
        local s = spawn_unit("skull", u.x, u.bz, false)
        s.life = 1.5
        return
    end
    u.vx, u.vz = 0, 0
    u.mut_t = u.mut_t - dt
    u.flick = u.flick - dt
    if u.flick <= 0 then
        u.flick = 2 * FR
        u.fl = not u.fl
        u.z = u.bz + (u.fl and -1 or 1) * math.random(0, 7) * PXM * 0.5
        show_unit(u, u.fl)
    end
    if u.mut_t <= 0 then
        human_lost(u, "brain")
        local p = spawn_unit("prog", u.x, u.bz, false)
        b.busy, b.victim, b.target = nil, nil, nil
        sfx("convert", 1.15, 0.8, 0.05)
    end
end

function AI.human(u, dt)
    if u.mut then return mutate(u, dt) end
    local hs = 8 * FR
    u.st = u.st - dt
    if u.st <= 0 then
        u.st = hs
        u.nd = u.nd - 1
        if u.nd <= 0 then human_dir(u) end
    end
    local nx, nz = u.x + u.dx * 0.25, u.z + u.dz * 0.25
    if outside(nx, nz, u.k.r + 0.1) or electrode_near(nx, nz, u.k.r) then
        human_dir(u)
        u.vx, u.vz = 0, 0
        return
    end
    u.vx, u.vz = u.dx * 1.5 * PXM / hs, u.dz * PXM / hs
end

-- Cruise missiles take one step a frame: a byte sideways and/or a pixel up or down toward the
-- player (give or take a few), re-aimed every 1..7 double frames, so they zig-zag in; they bounce
-- off the walls and live until shot. A brain keeps up to eight of them in the air.
local function missile_dir(m)
    m.dx, m.dz = 0, 0
    local function toward(a, b) return (a >= b) and 1 or -1 end
    if math.random() < 0.5 then
        m.dx = toward(g.px + math.random(-6, 9) * BYM, m.x)
        if math.random() < 0.5 then m.dz = toward(g.pz + math.random(-6, 9) * PXM, m.z) end
    else
        m.dz = toward(g.pz + math.random(-6, 9) * PXM, m.z)
    end
    m.nv = randu(7) * 2 * FR
end

local function fire_missile(u)
    local m = spawn_unit("missile", u.x, u.z + 0.3, false)
    missile_dir(m)
    sfx("missile", frand(0.95, 1.1), 0.8, 0.08)
end

function AI.missile(u, dt)
    u.nv = u.nv - dt
    if u.nv <= 0 then missile_dir(u) end
    local m = 0.3
    if (u.x > HW - m and u.dx > 0) or (u.x < -HW + m and u.dx < 0) then u.dx = -u.dx end
    if (u.z > HD - m and u.dz > 0) or (u.z < -HD + m and u.dz < 0) then u.dz = -u.dz end
    u.vx, u.vz = u.dx * BYM / FR, u.dz * PXM / FR
    u.trail_t = u.trail_t - dt
    if u.trail_t <= 0 then
        u.trail_t = 0.025
        px_particle(u.x, 0.55, u.z, (floor(u.age * 30) % 2 == 0) and "magenta" or "white", 0, 0, 0, 0.14, 1.8, true)
    end
end

-- Brains step every brnspd frames, a byte sideways and a pixel up or down, toward the nearest
-- family member (the player once the family is gone). On reaching one they stop and reprogram it
-- into a prog. Every 1..bshtim steps they fire a cruise missile.
local function start_mutation(b, h)
    b.busy, b.victim = true, h
    h.mut, h.mut_t, h.flick, h.fl = b, 20 * 4 * FR, 0, true
    h.vx, h.vz = 0, 0
    local side = (b.x - 1.1 < -HW + 0.6) and 1 or -1
    h.x = b.x + side * 1.1
    h.bz = b.z + 2 * PXM
    h.z = h.bz
    sfx("convert", 1.0, 0.9, 0.1)
end

function AI.brain(u, dt)
    if u.busy then
        u.vx, u.vz = 0, 0
        return
    end
    local bs = g.p.brnspd * FR
    local t = u.target
    if not free_human(t) then
        t = nearest_human(u.x, u.z)
        u.target = t
    end
    local tx, tz = g.px, g.pz
    if t then tx, tz = t.x, t.z end
    local dx, dz = tx - u.x, tz - u.z
    local sx = (abs(dx) > 2 * BYM) and sign(dx) or 0
    local sz = (abs(dz) > 0.5 * PXM) and sign(dz) or 0
    u.vx, u.vz = sx * BYM / bs, sz * PXM / bs
    u.st = u.st - dt
    if u.st <= 0 then
        u.st = bs
        u.shots = u.shots - 1
        if u.shots <= 0 then
            u.shots = randu(g.p.bshtim)
            if count_kind("missile") < 8 then fire_missile(u) end
        end
    end
    if t and abs(dx) <= 3 * BYM and abs(dz) <= 3 * PXM then start_mutation(u, t) end
end

-- Progs run four pixels every three frames along one axis toward a point near the player, keep
-- going until they pick again (about one step in ten, or at a wall), and leave afterimages.
local function prog_dir(u)
    if math.random() < 0.5 then
        local tx = g.px + u.offx
        u.dx, u.dz = (tx > u.x) and 1 or -1, 0
    else
        local tz = g.pz + u.offz
        u.dx, u.dz = 0, (tz > u.z) and 1 or -1
    end
end

local function prog_offset(u)
    u.offx = math.random(-28, 28) * BYM
    u.offz = math.random(-16, 18) * PXM
end

function AI.prog(u, dt)
    if not u.offx then
        prog_offset(u)
        prog_dir(u)
    end
    u.st = u.st - dt
    if u.st <= 0 then
        u.st = 3 * FR
        if math.random() < 7 / 256 then prog_offset(u) end
        if math.random() < 27 / 256 then prog_dir(u) end
    end
    if outside(u.x + u.dx * 0.4, u.z + u.dz * 0.4, u.k.r) then prog_dir(u) end
    u.vx, u.vz = u.dx * 2 * BYM / (3 * FR), u.dz * 4 * PXM / (3 * FR)
    u.trail_t = u.trail_t - dt
    if u.trail_t <= 0 then
        u.trail_t = 0.06
        u.trail_i = ((u.trail_i or 0) % 4) + 1
        afterimage(u.m, u.x, 0, u.z, ({ "cyan", "magenta", "yellow", "red" })[u.trail_i])
    end
end

-- Spheroids and quarks drift on random accelerations, then drop their enforcers or tanks (one to
-- five, a few seconds apart) and leave through the nearest wall.
local function spawner(u, dt, child, tick, cycle_wait, cycle_drop, gap, cap, ax_r, az_r, vx_max, vz_max)
    u.cyc = u.cyc + dt
    local cyc = (u.phase == "wait") and cycle_wait or cycle_drop
    while u.cyc >= cyc and u.phase ~= "esc" do
        u.cyc = u.cyc - cyc
        u.drop = u.drop - 1
        if u.drop <= 0 then
            if u.phase == "wait" then
                u.phase = "drop"
            elseif count_kind(child) < cap then
                spawn_unit(child, u.x, u.z, 0, nil, "r", nil, child == "tank" and 0.8 or 0.6)
                sfx("spawn", child == "tank" and 0.7 or 0.85, 0.8, 0.05)
                u.n_left = u.n_left - 1
                if u.n_left <= 0 then u.phase = "esc" end
            end
            u.drop = randu(gap)
        end
    end
    if u.phase == "esc" then return true end
    -- new acceleration (spheroids) or velocity (quarks) every so many ticks
    u.acc_t = u.acc_t - dt
    if u.acc_t <= 0 then
        u.acc_t = tick
        u.ax, u.az = ax_r(u), az_r(u)
    end
    return false
end

function AI.spheroid(u, dt)
    local p = g.p
    local esc = spawner(u, dt, "enforcer", randu(15) * 2 * FR, 10 * FR, 16 * FR, floor(p.cdptim / 4), 8,
        function() return srand(16) / 256 end, function() return srand(32) / 256 end)
    if esc then
        if not u.esc_dir then u.esc_dir = (math.random() < 0.5) and -1 or 1 end
        u.vx, u.vz = u.esc_dir * BYM / FR, 0
        if abs(u.x) > HW - 0.8 then
            u.alive = false -- gone through the wall, no score
            show_unit(u, false)
        end
        return
    end
    -- velocity in bytes / pixels per frame, damped, integrated every two frames
    u.vt, u.ux, u.uz = (u.vt or 0) + dt, u.ux or 0, u.uz or 0
    while u.vt >= 2 * FR do
        u.vt = u.vt - 2 * FR
        u.ux = clamp(u.ux + u.ax, -1, 1)
        u.ux = u.ux - u.ux / 64
        u.uz = clamp(u.uz + u.az, -2, 2)
        u.uz = u.uz - u.uz / 128
    end
    local m = u.k.r + 0.1
    if (u.x > HW - m and u.ux > 0) or (u.x < -HW + m and u.ux < 0) then u.ux, u.ax = -u.ux, -u.ax end
    if (u.z > HD - m and u.uz > 0) or (u.z < -HD + m and u.uz < 0) then u.uz, u.az = -u.uz, -u.az end
    u.vx, u.vz = u.ux * BYM / FR, u.uz * PXM / FR
end

function AI.quark(u, dt)
    local p = g.p
    local function vel(edge_lo, edge_hi, pos, scale)
        local v = randu(p.sqspd) * scale / 256
        if pos <= edge_lo then return v end
        if pos >= edge_hi then return -v end
        return (math.random() < 0.5) and v or -v
    end
    local esc = spawner(u, dt, "tank", randu(32) * 3 * FR, 15 * FR, 27 * FR, floor(p.tdptim / 2) + 1, 20,
        function(q) return vel(-HW + 5 * BYM, HW - 12 * BYM, q.x, 4) end,
        function(q) return vel(-HD + 5 * PXM, HD - 20 * PXM, q.z, 8) end)
    if esc then
        if not u.esc_dir then u.esc_dir = (u.z > 0) and 1 or -1 end
        u.vx, u.vz = 0, u.esc_dir * 2 * PXM / FR
        if abs(u.z) > HD - 0.8 then
            u.alive = false
            show_unit(u, false)
        end
        return
    end
    local m = u.k.r + 0.1
    if (u.x > HW - m and u.ax > 0) or (u.x < -HW + m and u.ax < 0) then u.ax = -u.ax end
    if (u.z > HD - m and u.az > 0) or (u.z < -HD + m and u.az < 0) then u.az = -u.az end
    u.vx, u.vz = u.ax * BYM / FR, u.az * PXM / FR
end

-- Enforcers glide toward a point near the player at a speed proportional to the distance,
-- re-aimed every 1..32 ticks of three frames, and fire a spark every 1..enstim ticks.
local function spark_shot(u)
    local ox = (g.px < -HW + 16 * BYM) and 0 or srand(16) * BYM
    local tx, tz = g.px + ox, g.pz + srand(16) * PXM
    local s = spawn_unit("spark", u.x, u.z, false)
    -- reaches the aim point in about a second, bending on a random acceleration
    s.vx, s.vz = (tx - u.x) * 4 / 256 / FR, (tz - u.z) * 4 / 256 / FR
    s.ax, s.az = srand(16) / 256 * BYM / FR / (4 * FR), srand(16) / 256 * PXM / FR / (4 * FR)
    s.life = (20 + math.random(0, 15)) * 4 * FR
    sfx("spark", frand(0.95, 1.1), 0.5, 0.06)
end

function AI.enforcer(u, dt)
    u.nv = (u.nv or 0) - dt
    if u.nv <= 0 then
        u.nv = randu(32) * 3 * FR
        local tx = clamp(g.px + srand(16) * BYM, -HW, HW)
        local tz = clamp(g.pz + srand(16) * PXM, -HD, HD)
        u.vx, u.vz = (tx - u.x) * 2 / 256 / FR, (tz - u.z) * 2 / 256 / FR
    end
    u.fire_t = u.fire_t - dt
    if u.fire_t <= 0 then
        u.fire_t = randu(g.p.enstim) * 3 * FR
        if count_kind("spark") < 20 then spark_shot(u) end
    end
end

-- sparks slide along the walls they hit
function AI.spark(u, dt)
    u.life = u.life - dt
    u.vx, u.vz = u.vx + u.ax * dt, u.vz + u.az * dt
    local m = 0.3
    if (u.x > HW - m and u.vx > 0) or (u.x < -HW + m and u.vx < 0) then u.vx, u.ax = 0, 0 end
    if (u.z > HD - m and u.vz > 0) or (u.z < -HD + m and u.vz < 0) then u.vz, u.az = 0, 0 end
    if u.life <= 0 then
        u.alive = false
        show_unit(u, false)
    end
end

-- Tanks step a pixel each way every two frames toward the player (or a random spot), and every
-- thksht steps fire a shell, half of them banked off the wall nearest the player.
local function tank_dir(u)
    local tx, tz
    if math.random() < 97 / 256 then
        tx, tz = g.px, g.pz
    else
        tx, tz = frand(-HW, HW), frand(-HD, HD)
    end
    u.dx = (tx >= u.x) and 1 or -1
    u.dz = (abs(tz - u.z) < 16 * PXM) and 0 or ((tz > u.z) and 1 or -1)
    u.nd = randu(32)
end

local function tank_fire(u)
    local p = g.p
    local s = spawn_unit("shell", u.x, u.z, false)
    s.life = (48 + math.random(0, 31)) * 2 * FR
    local k = p.shlspd
    if math.random() < 0.5 then
        -- straight at the player, give or take
        local ox = (g.px < -HW + 10 * BYM) and 0 or srand(16) * BYM
        local tx, tz = g.px + ox, g.pz + srand(16) * PXM
        s.vx, s.vz = (tx - u.x) * k * 8 / 65536 / FR, (tz - u.z) * k * 8 / 65536 / FR
    else
        -- a bank shot: at the wall beside the player, halfway along, so it bounces at them
        local near = math.random(0, 7) ~= 0
        local tx, tz
        if math.random() < 0.5 then
            local right = (g.px >= 0) == near
            tx = right and HW or -HW
            tz = (u.z + g.pz + srand(16) * PXM) / 2
        else
            local low = (g.pz >= 0) == near
            tz = low and HD or -HD
            tx = (u.x + g.px + srand(8) * BYM) / 2
        end
        -- the original doubles the aim vector until one component passes a speed limit
        local vx, vz = (tx - u.x) / BYM / 256, (tz - u.z) / PXM / 256
        local a = floor(k * 64 / 256) * 4 / 256
        if abs(vx) < 1e-4 and abs(vz) < 1e-4 then vx = a end
        while abs(vx) <= a and abs(vz) <= 2 * a do
            vx, vz = vx * 2, vz * 2
        end
        s.vx, s.vz = vx * BYM / FR, vz * PXM / FR
    end
    sfx("tank_shot", frand(0.95, 1.1), 0.8, 0.08)
end

function AI.tank(u, dt)
    local ts = 2 * FR
    u.st = u.st - dt
    if u.st <= 0 then
        u.st = ts
        u.nd = u.nd - 1
        if u.nd <= 0 then tank_dir(u) end
        u.shots = u.shots - 1
        if u.shots <= 0 then
            u.shots = g.p.thksht
            if count_kind("shell") <= 20 then tank_fire(u) end
        end
    end
    if (u.dx == 0 and u.dz == 0) or outside(u.x + u.dx * 0.3, u.z + u.dz * 0.3, u.k.r) then tank_dir(u) end
    u.vx, u.vz = u.dx * PXM / ts, u.dz * PXM / ts
end

-- shells bounce off the walls until their time is up
function AI.shell(u, dt)
    u.life = u.life - dt
    local bounced = false
    if (u.x > HW - 0.3 and u.vx > 0) or (u.x < -HW + 0.3 and u.vx < 0) then u.vx = -u.vx bounced = true end
    if (u.z > HD - 0.3 and u.vz > 0) or (u.z < -HD + 0.3 and u.vz < 0) then u.vz = -u.vz bounced = true end
    if bounced then
        sfx("bounce", frand(0.9, 1.2), 0.5, 0.05)
        burst("yellow", u.x, 0.5, u.z, 6, 5, 0.08)
    end
    if u.life <= 0 then
        u.alive = false
        show_unit(u, false)
    end
end

local function update_units(dt)
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            local k = u.k
            u.age = u.age + dt
            u.ox, u.oz = u.x, u.z
            if u.spawn_t > 0 then
                u.spawn_t = u.spawn_t - dt
                if not u.emitted and u.spawn_t <= u.mat_t then
                    u.emitted = true
                    materialize(u.m, u.x, unit_y(u), u.z, u.mode, max(0.05, u.spawn_t))
                end
                if u.spawn_t <= 0 then
                    u.yaw = 0
                    show_unit(u, true)
                end
            else
                local ai = AI[k.ai or u.kind]
                if ai then ai(u, dt) end
                if u.alive then
                    if not k.stepper then
                        u.x = u.x + u.vx * dt
                        u.z = u.z + u.vz * dt
                    end
                    local r = k.proj and 0.25 or k.r
                    local lx, lz = HW - r, HD - r
                    if u.x > lx then u.x = lx elseif u.x < -lx then u.x = -lx end
                    if u.z > lz then u.z = lz elseif u.z < -lz then u.z = -lz end
                end
            end
        end
    end

    -- soft separation for crowds (every unit pair that overlaps pushes apart)
    for i = 1, #units - 1 do
        local a = units[i]
        if a.alive and a.spawn_t <= 0 and not a.k.proj and not a.k.deco and a.kind ~= "electrode" and not a.mut then
            for j = i + 1, #units do
                local b = units[j]
                if b.alive and b.spawn_t <= 0 and not b.k.proj and not b.k.deco and b.kind ~= "electrode" and not b.mut then
                    local dx, dz = b.x - a.x, b.z - a.z
                    if dx < 1.4 and dx > -1.4 and dz < 1.4 and dz > -1.4 then
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

local function render_units(dt)
    local t = g.time
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 then
            local k = u.k
            local y = k.y or 0
            if k.bob then y = y + k.bob[1] * (0.5 + 0.5 * sin(t * k.bob[2] + u.ph)) end
            local moving = u.vx ~= 0 or u.vz ~= 0
            -- animation frames
            if k.walk and not k.stepper then
                if moving then
                    u.at = u.at + dt * (k.hulk and 5 or 8)
                    set_frame(u, 2 + floor(u.at) % 2)
                else
                    set_frame(u, 1)
                end
            elseif k.cycle then
                if k.cycle > 0 then
                    u.at = u.at + dt * k.cycle
                    local n = #u.fnodes
                    if n > 1 then
                        if u.kind == "spheroid" then
                            local p = floor(u.at) % 4 -- 1 2 3 2: the rings pulse in and out
                            set_frame(u, p == 3 and 2 or p + 1)
                        else
                            set_frame(u, floor(u.at) % n + 1)
                        end
                    end
                elseif moving then
                    u.at = u.at + dt * 10
                    set_frame(u, floor(u.at) % 2 + 1)
                end
            end
            -- facing: sprites face the camera and turn a little toward the way they move
            if k.spin then
                u.yaw = u.yaw + k.spin * dt
            else
                local want = 0
                local sp = sqrt(u.vx * u.vx + u.vz * u.vz)
                if sp > 0.1 then want = 0.5 * u.vx / sp end
                u.yaw = u.yaw + (want - u.yaw) * min(1, dt * 8)
            end
            set_translation(u.e, u.x, y, u.z)
            set_rotation(u.e, u.yaw, -LEAN, 0)
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
                local l = sqrt(b.vx * b.vx + b.vz * b.vz)
                if best.k.hulk then
                    -- a shot knocks a hulk back a byte or two (a few pixels up or down)
                    local fx, fz = b.vx / l, b.vz / l
                    local lx, lz = HW - best.k.r, HD - best.k.r
                    best.x = clamp(best.x + sign(fx) * (abs(fx) > 0.3 and 1 or 0) * BYM * math.random(1, 2), -lx, lx)
                    best.z = clamp(best.z + sign(fz) * (abs(fz) > 0.3 and 1 or 0) * 2 * PXM * math.random(1, 2), -lz, lz)
                    burst("lgreen", b.x, 0.8, b.z, 4, 4, 0.08)
                    sfx("hulk_hit", frand(0.9, 1.1), 0.5, 0.06)
                else
                    kill_unit(best, true, b.vx / l, b.vz / l)
                end
            elseif abs(b.x) > HW or abs(b.z) > HD then
                dead = true
                burst("white", clamp(b.x, -HW, HW), 0.7, clamp(b.z, -HD, HD), 4, 4, 0.07)
            end
            if dead then
                b.alive = false
                b.vis = false
                set_vis(b.e, false)
            else
                set_translation(b.e, b.x, 0.8, b.z)
            end
        end
    end
end

local function update_interactions(dt)
    local px, pz = g.px, g.pz
    local pready = g.p_alive and g.p_spawn <= 0
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 then
            local k = u.k
            if k.hostile then
                if pready then
                    local rr = k.r * 0.85 + P_R
                    local d2
                    if k.proj then
                        -- fast shots: test the path covered this frame
                        d2 = seg_dist2(u.ox, u.oz, u.x, u.z, px, pz)
                    else
                        local dx, dz = u.x - px, u.z - pz
                        d2 = dx * dx + dz * dz
                    end
                    if d2 < rr * rr then kill_player() end
                end
                if k.hulk then
                    -- hulks crush the family (leaving a skull) and electrodes
                    for j = 1, #units do
                        local h = units[j]
                        if h.alive and h.spawn_t <= 0 and (h.k.human and not h.mut or h.kind == "electrode") then
                            local dx, dz = h.x - u.x, h.z - u.z
                            local rr = (k.r + h.k.r) * 0.6
                            if dx * dx + dz * dz < rr * rr then
                                if h.kind == "electrode" then
                                    kill_unit(h, false)
                                else
                                    human_lost(h, "hulk")
                                    local s = spawn_unit("skull", h.x, h.z, false)
                                    s.life = 1.6
                                    sfx("human_die", frand(0.95, 1.05), 0.9, 0.1)
                                end
                            end
                        end
                    end
                elseif u.kind == "grunt" then
                    -- grunts that blunder into an electrode die (and score, as in the original)
                    local el = electrode_near(u.x, u.z, k.r * 0.7)
                    if el then kill_unit(u, true) end
                end
            elseif k.human and pready and not u.mut then
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
                    sfx("rescue", 0.9 + 0.08 * min(g.rescue_n, 5), 1.0)
                end
            end
        end
    end
end

-- ---------------------------------------------------------------- effects

local function update_effects(dt)
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

local N_RECTS = 9
local function build_rects()
    for i = 1, N_RECTS do
        local r = {}
        for s = 1, 4 do
            local e = reg(spawn_primitive("cube", uname("RT_rect"), { material_of = mat_of("fx_red") }))
            set_component(e, "CanCastShadow", false)
            set_component(e, "Visible", false)
            r[s] = e
        end
        r.vis = false
        r.col = 0
        rects[i] = r
    end
end

local function show_rect(r, v)
    if r.vis == v then return end
    r.vis = v
    for s = 1, 4 do set_vis(r[s], v) end
end

-- concentric rectangles in cycling colours zoom out from the player to the border
local function update_transition(dt)
    g.trans_t = g.trans_t + dt
    local t = g.trans_t
    for i = 1, N_RECTS do
        local r = rects[i]
        local ti = t - (i - 1) * 0.055
        local p = ti / 0.42
        if p > 0 and p < 1.15 then
            p = min(1, p)
            local e = p * p * (3 - 2 * p)
            local cx, cz = g.tcx * (1 - e), g.tcz * (1 - e)
            local hx, hz = 0.3 + (HW + 0.15 - 0.3) * e, 0.25 + (HD + 0.15 - 0.25) * e
            local th, y = 0.14, 0.08
            set_translation(r[1], cx, y, cz - hz)
            set_scale(r[1], hx * 2 + th, 0.1, th)
            set_translation(r[2], cx, y, cz + hz)
            set_scale(r[2], hx * 2 + th, 0.1, th)
            set_translation(r[3], cx - hx, y, cz)
            set_scale(r[3], th, 0.1, hz * 2)
            set_translation(r[4], cx + hx, y, cz)
            set_scale(r[4], th, 0.1, hz * 2)
            local col = (i + floor(t * 26)) % #FLASH + 1
            if r.col ~= col then
                r.col = col
                for s = 1, 4 do share_material(r[s], mat_of("fx_" .. FLASH[col])) end
            end
            show_rect(r, true)
        else
            show_rect(r, false)
        end
    end
    if t >= TRANS_T then
        for i = 1, N_RECTS do show_rect(rects[i], false) end
        local f = g.trans_after
        g.trans_after = nil
        if f then f() end
    end
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
    local glow = 1.4 + g.shake * 3
    for n = 1, #g.walls do
        set_material(g.walls[n], { color = { r, gg, b }, emissive = glow })
    end
end

-- ---------------------------------------------------------------- camera

-- normalized device coordinates of a world point for a camera at c looking at t
local function project(cx, cy, cz, tx, ty, tz, px, py, pz, tanf, aspect)
    local fx, fy, fz = tx - cx, ty - cy, tz - cz
    local fl = sqrt(fx * fx + fy * fy + fz * fz)
    fx, fy, fz = fx / fl, fy / fl, fz / fl
    local rx, rz = -fz, fx
    local rl = sqrt(rx * rx + rz * rz)
    rx, rz = rx / rl, rz / rl
    local ux, uy, uz = -rz * fy, rz * fx - rx * fz, rx * fy
    local dx, dy, dz = px - cx, py - cy, pz - cz
    local x = dx * rx + dz * rz
    local y = dx * ux + dy * uy + dz * uz
    local z = dx * fx + dy * fy + dz * fz
    return x / (z * tanf * aspect), y / (z * tanf)
end

-- the closest camera (and its aim point) that shows the whole arena with room for the HUD
local function fit_camera(aspect)
    local tanf = math.tan(math.rad(FOV) * 0.5)
    local ex, ez = HW + 0.7, HD + 0.7
    local function fits(dist, tz)
        local cx, cy, cz = 0, dist * sin(CAM_PITCH), tz + dist * cos(CAM_PITCH)
        for _, p in ipairs({ { -ex, 0, -ez }, { ex, 0, -ez }, { -ex, 0, ez }, { ex, 0, ez }, { ex, 1.6, -ez }, { -ex, 1.6, -ez } }) do
            local nx, ny = project(cx, cy, cz, 0, 0, tz, p[1], p[2], p[3], tanf, aspect)
            if abs(nx) > 0.97 or ny > 0.86 or ny < -0.88 then return false end
        end
        return true
    end
    local best_d, best_z = 1e9, 0
    for tz = -3.0, 3.0, 0.25 do
        local lo, hi = 5, 200
        if fits(hi, tz) then
            for _ = 1, 28 do
                local mid = (lo + hi) * 0.5
                if fits(mid, tz) then hi = mid else lo = mid end
            end
            if hi < best_d then best_d, best_z = hi, tz end
        end
    end
    return { aspect = aspect, dist = best_d, tz = best_z }
end

local function update_camera(dt)
    if not g.cam then return end
    local vp = get_viewport_size()
    local aspect = (vp and vp.height and vp.height > 0) and (vp.width / vp.height) or 1.78
    if not g.cam_fit or abs(g.cam_fit.aspect - aspect) > 0.01 then g.cam_fit = fit_camera(aspect) end
    local f = g.cam_fit
    local sx, sz = 0, 0
    if g.shake > 0 then
        local s = g.shake * g.shake * 0.5
        sx, sz = frand(-s, s), frand(-s, s)
    end
    set_translation(g.cam, sx, f.dist * sin(CAM_PITCH), f.tz + f.dist * cos(CAM_PITCH) + sz)
    look_at(g.cam, sx, 0, f.tz + sz)
end

-- ---------------------------------------------------------------- the autopilot

local DIRS = {
    [0] = { 0, 0 }, { 0, -1 }, { 0.7071, -0.7071 }, { 1, 0 }, { 0.7071, 0.7071 },
    { 0, 1 }, { -0.7071, 0.7071 }, { -1, 0 }, { -0.7071, -0.7071 },
}
local SIM_K, SIM_DT = 8, 0.09
-- with the family on screen, the last KEEP enemies are left alive while it collects them, unless
-- they get within SPARE_R (or, for a brain, near a human); after HOLD_MAX seconds it finishes the wave
local KEEP, SPARE_R, HOLD_MAX = 2, 1.8, 25

local tx, tz, tvx, tvz, tr, tw, tch = {}, {}, {}, {}, {}, {}, {}
local wx, wz = {}, {}
local nthr = 0
local gx, gz, gvx, gvz, gr, gw = {}, {}, {}, {}, {}, {}   -- shootable targets
local bx_, bz_, br_ = {}, {}, {}                            -- blockers (hulks, electrodes)
local sx_, sz_, sr_ = {}, {}, {}                            -- spared enemies: keep out of the line of fire

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
                    tch[n] = g.grunt_speed * 1.15
                    tvx[n], tvz[n] = 0, 0
                elseif u.kind == "prog" then
                    tch[n] = PROG_SPEED * 0.85
                    tvx[n], tvz[n] = 0, 0
                else
                    tvx[n], tvz[n] = u.vx, u.vz
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
        if h.alive and h.k.human and h.spawn_t <= 0 and not h.mut then
            local dx, dz = h.x - g.px, h.z - g.pz
            local d = sqrt(dx * dx + dz * dz)
            if d < bd then
                -- skip humans that a hulk or brain is about to get
                local safe = true
                for j = 1, #units do
                    local o = units[j]
                    if o.alive and (o.k.hulk or o.kind == "brain") then
                        local ex, ez = o.x - h.x, o.z - h.z
                        local lim = o.k.hulk and 1.6 or 2.0
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
    -- closest approach to the human and the step that reaches them (the end point alone would
    -- overshoot one nearby and make standing still look best)
    local hmin, hstep = 1e9, nil
    for s = 1, SIM_K do
        px = clamp(px + dxm, -LX, LX)
        pz = clamp(pz + dzm, -LZ, LZ)
        if hx and not hstep then
            local hd = sqrt((px - hx) ^ 2 + (pz - hz) ^ 2)
            if hd < hmin then hmin = hd end
            if hd < 0.9 then hstep = s end
        end
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
        cost = cost - hw * (d0 - hmin)
        if hstep then cost = cost - hw * 0.5 * (SIM_K + 1 - hstep) end
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
        if g.b_killers <= KEEP then hw = hw + 3 end
    end
    local best, bc = 0, 1e18
    for c = 0, 8 do
        local cost = eval_dir(c, hx, hz, hw)
        if cost < bc then best, bc = c, cost end
    end
    g.bprev = best
    g.bmx, g.bmz = DIRS[best][1], DIRS[best][2]
end

local function human_within(x, z, r)
    for i = 1, #units do
        local h = units[i]
        if free_human(h) and (h.x - x) ^ 2 + (h.z - z) ^ 2 < r * r then return true end
    end
    return false
end

local function bot_aim()
    local n, nb, ns = 0, 0, 0
    local px, pz = g.px, g.pz
    -- with family left to rescue and the wave nearly over, do not finish it off from afar,
    -- and spare the last few so the wave stays open until they are collected
    local hold = g.b_humans > 0 and g.b_killers <= 9 and g.state == "running" and g.hold_t < HOLD_MAX
    local keep = hold and g.b_killers <= KEEP
    for i = 1, #units do
        local u = units[i]
        if u.alive and u.spawn_t <= 0 then
            local k = u.k
            local dx, dz = u.x - px, u.z - pz
            local d2 = dx * dx + dz * dz
            local far = hold and d2 > 10 * 10 and u.kind ~= "brain" and u.kind ~= "missile"
            local spare = keep and k.need and d2 > SPARE_R * SPARE_R
                and not (u.kind == "brain" and human_within(u.x, u.z, 5.0))
            if spare then
                ns = ns + 1
                sx_[ns], sz_[ns], sr_[ns] = u.x, u.z, k.r
            end
            if d2 < 34 * 34 then
                if k.shoot and k.bw and not far and not spare then
                    n = n + 1
                    gx[n], gz[n], gr[n], gw[n] = u.x, u.z, k.r, k.bw
                    local vx, vz = u.vx, u.vz
                    if u.kind == "grunt" or u.kind == "prog" then
                        -- chasers close in on the player: use the heading they will have
                        local d = sqrt(dx * dx + dz * dz) + 1e-6
                        local sp = (u.kind == "grunt") and g.grunt_speed * 0.8 or PROG_SPEED * 0.5
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
        for i = 1, ns do
            local rx, rz = sx_[i] - px, sz_[i] - pz
            if rx * ux + rz * uz > 0 and abs(rx * uz - rz * ux) < sr_[i] + 0.6 then v = 0 break end
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
    local humans, killers = 0, 0
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            if u.k.human and not u.mut then humans = humans + 1 elseif u.k.need then killers = killers + 1 end
        end
    end
    g.b_humans, g.b_killers = humans, killers
    if humans > 0 and killers <= KEEP then g.hold_t = g.hold_t + dt else g.hold_t = 0 end
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
            explode(player.m, g.px, 0, g.pz, nil, nil, 1.0, 7)
            ring(g.px, g.pz, 6.0, 0.7, "cyan")
            add_shake(0.8)
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

function on_created(self_entity)
    local ok, t = pcall(os.time)
    math.randomseed(ok and t or 2084)
    g.cam = find_entity_by_name("GameCamera")
    if g.cam then set_camera(g.cam, { fov_deg = FOV, near = 1.0, far = 200 }) end
    for _, n in ipairs({ "Wall_N", "Wall_S", "Wall_W", "Wall_E" }) do
        local e = find_entity_by_name(n)
        if e then g.walls[#g.walls + 1] = e end
    end
    g.hi = tonumber(load_data("highscore") or "0") or 0
    build_player()
    build_rects()
    if PLAY_MUSIC then g.music = play_sound("content/audio/music.wav", { volume = MUSIC_VOLUME, loop = true }) end
    g.ready = true
    new_game(true)
end

function on_destroyed(self_entity, reason)
    if g.music then stop_sound(g.music) end
    for i = 1, #all_entities do despawn_entity(all_entities[i]) end
    all_entities = {}
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
    local c3 = clock()
    update_walls(dt)
    local c4 = clock()
    if not g.demo and g.score >= g.hi and g.score > 0 and g.time - (g.hi_saved or 0) > 5 then
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

-- ---------------------------------------------------------------- HUD

local function colv(name, a)
    local c = COLORS[name] or COLORS.white
    return { min(1, c[1] * 1.1), min(1, c[2] * 1.1), min(1, c[3] * 1.1), a or 1.0 }
end
local HUD_RED = { 1.0, 0.25, 0.2, 1 }

function on_ui(self_entity, ui)
    if not g.ready then return end
    ui:screen_label("top_left", 24, 12, string.format("%d", g.score), { size = 34, color = HUD_RED })
    local men = ""
    for _ = 1, min(max(g.lives - (g.p_alive and 1 or 0), 0), 10) do men = men .. "I" end
    ui:screen_label("top_left", 24, 52, men ~= "" and ("MEN " .. men) or "", { size = 18, color = { 1, 0.85, 0.3, 1 } })
    ui:screen_label("top_right", 24, 12, "HIGH " .. g.hi, { size = 22, color = { 0.6, 0.8, 1, 0.9 } })
    if g.wave > 0 then
        ui:screen_label("bottom_center", 0, 8, g.wave .. " WAVE", { size = 26, color = HUD_RED })
    end
    if g.bot and not g.demo then
        ui:screen_label("top_right", 24, 42, "AUTOPILOT", { size = 18, color = { 1, 0.6, 0.2, 1 } })
    end

    if g.demo then
        if g.demo_t < 8.0 then
            local a = min(1, (8.0 - g.demo_t) * 0.5)
            ui:screen_label("center", 0, -170, "ROBOTRON: 2084", { size = 72, color = colv(FLASH[floor(g.time * 8) % #FLASH + 1], a) })
            ui:screen_label("center", 0, -105, "SAVE THE LAST HUMAN FAMILY", { size = 24, color = { 1, 1, 1, a * 0.9 } })
        end
        if floor(g.time * 2) % 2 == 0 then
            ui:screen_label("bottom_center", 0, 70, "PRESS ENTER TO PLAY", { size = 34, color = { 1, 1, 1, 1 } })
        end
        ui:screen_label("bottom_center", 0, 44, "WASD move   ARROWS or mouse fire", { size = 16, color = { 0.7, 0.8, 1, 0.9 } })
    end
    if g.paused then
        ui:screen_label("center", 0, -40, "PAUSED", { size = 56, color = { 1, 1, 1, 1 } })
    end
    if g.banner_t > 0 and g.banner ~= "" and not g.paused then
        local a = min(1, g.banner_t * 2)
        ui:screen_label("center", 0, -40, g.banner, { size = 60, color = { 1, 0.2, 0.15, a } })
    end
    for i = 1, #g.popups do
        local p = g.popups[i]
        local c = colv(p.col, 1 - p.t / 1.3)
        ui:world_label(p.x, 1.4 + p.t * 1.2, p.z, p.text, { size = p.size, color = c })
    end
end

-- ---------------------------------------------------------------- test hooks
-- Globals for lua_eval {entity = "GameController"}: RT.jump(5), RT.spawn("brain", 3, 2), RT.god(true) ...
RT = {
    jump = function(w)
        g.p_alive = true
        if player.flash then flash_player(nil) end
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
    entities = function() return #all_entities end,
    pooled = function()
        local c, cubes = {}, 0
        for key, pool in pairs(pools) do
            c[key] = #pool
            for _, u in ipairs(pool) do cubes = cubes + #u.cubes end
        end
        c.cubes = cubes
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
