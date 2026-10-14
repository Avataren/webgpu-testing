-- Enemy and family AI, after the 1982 source (see the notes on each kind), and the per-frame
-- actor update that runs it.

local C = require("robotron.config")
local HW, HD, FR, PXM, BYM, GRUNT_STEP = C.HW, C.HD, C.FR, C.PXM, C.BYM, C.GRUNT_STEP
local U = require("robotron.util")
local clamp, frand, randu, srand, sign = U.clamp, U.frand, U.randu, U.srand, U.sign
local S = require("robotron.state")
local g, units, sfx = S.g, S.units, S.sfx
local Fx = require("robotron.fx")
local px_particle, materialize, afterimage = Fx.px_particle, Fx.materialize, Fx.afterimage
local burst = Fx.burst
local Actors = require("robotron.actors")
local show_unit, set_frame, unit_y = Actors.show_unit, Actors.set_frame, Actors.unit_y
local free_human, nearest_human = Actors.free_human, Actors.nearest_human
local human_lost, count_kind = Actors.human_lost, Actors.count_kind
local electrode_near, outside, spawn_unit = Actors.electrode_near, Actors.outside, Actors.spawn_unit

local sqrt, abs, floor, max = math.sqrt, math.abs, math.floor, math.max

-- After the 1982 source: RRP8 (grunts), RRH11 (hulks, the family), RRB10 (brains, progs, cruise
-- missiles), RRC11 (spheroids, enforcers, sparks) and RRTK4 (quarks, tanks, shells). Its timers
-- count frames or an actor's own steps; they are converted to seconds with FR.

local AI = {}

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

return {
    update_units = update_units,
}
