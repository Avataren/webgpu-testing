-- Actors: the kinds (K), pooled voxel units, spawning, kills and scoring, the queries the AI
-- and the autopilot share, and drawing them each frame.

local C = require("robotron.config")
local HW, HD, EXTRA_LIFE, LEAN, MAT_T, FR = C.HW, C.HD, C.EXTRA_LIFE, C.LEAN, C.MAT_T, C.FR
local U = require("robotron.util")
local frand, randu = U.frand, U.randu
local S = require("robotron.state")
local g, units, pools, reg, uname, sfx = S.g, S.units, S.pools, S.reg, S.uname, S.sfx
local set_vis = S.set_vis
local Models = require("robotron.models")
local model, build_model = Models.model, Models.build_model
local Fx = require("robotron.fx")
local explode, burst, ring, add_shake, popup = Fx.explode, Fx.burst, Fx.ring, Fx.add_shake, Fx.popup

local sqrt, sin, abs, floor, min, max = math.sqrt, math.sin, math.abs, math.floor, math.min, math.max

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

-- ---------------------------------------------------------------- queries

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

-- per-kind AI state, set when an actor spawns or respawns (the AI itself is in ai.lua)
local function reset_ai(u)
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

-- ---------------------------------------------------------------- spawning

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

local function clear_all_units()
    for i = 1, #units do
        local u = units[i]
        if u.alive then
            u.alive = false
            show_unit(u, false)
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

return {
    K = K, show_unit = show_unit, set_frame = set_frame, add_score = add_score, unit_y = unit_y,
    kill_unit = kill_unit, free_human = free_human, nearest_human = nearest_human, random_human = random_human, human_lost = human_lost,
    count_kind = count_kind, electrode_near = electrode_near, outside = outside, reset_ai = reset_ai, spawn_unit = spawn_unit,
    alive_count = alive_count, needed = needed, clear_all_units = clear_all_units, render_units = render_units,
}
