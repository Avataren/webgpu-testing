-- Neon Rift: a top-down arena shooter. One controller script drives everything.
-- Attached to the GameController entity; runs in play mode only.
--
-- The game boots into ATTRACT MODE: an AI pilot plays a demo game under the title.
-- SPACE (or Return) starts a real game. After a game over it returns to the demo.
--
--   WASD         move
--   Arrow keys   aim and fire (hold two for diagonals)
--   Space/Shift  dash (brief invulnerability, 1.2 s cooldown)
--   P            toggle the autopilot while playing
--
-- The arena, camera, lights and the ship (Player + its parts) are placed in the scene.
-- Bullets, enemies, sparks, pickups and shockwave rings are pools of primitives that this
-- script spawns once (reusing them by name after a hot reload) and then shows/hides.

local HW, HD = 15.0, 10.0              -- arena half extents inside the walls (x, z)
local P_R = 0.5                        -- player radius
local LX, LZ = HW - P_R, HD - P_R      -- player position limits
local P_SPEED = 9.5
local DASH_SPEED, DASH_TIME, DASH_CD = 30.0, 0.17, 1.2
local BULLET_SPEED, BULLET_LIFE, BULLET_R = 30.0, 1.0, 0.2
local FIRE_INTERVAL, SPREAD_INTERVAL = 0.12, 0.085
local SPREAD_ANGLE, SPREAD_TIME = 0.2, 10.0
local COMBO_TIME = 2.2
local START_LIVES = 3
local SFX_VOLUME = 0.55
local CAM = { x = 0.0, y = 29.0, z = 15.5, tx = 0.0, tz = 0.8 }
local POPUP_TIME = 0.9

local N_BULLETS, N_SPARKS, N_PICKUPS, N_RINGS = 64, 240, 4, 4

local KINDS = {
    drone = {
        mesh = "cube", scale = { 0.8, 0.8, 0.8 }, r = 0.5, hp = 1, speed = 3.8, pts = 10,
        color = { 1.0, 0.15, 0.55 }, pool = 52, burst = 12, burst_speed = 7, sound = "kill",
    },
    dasher = {
        mesh = "cone", scale = { 0.55, 0.3, 1.3 }, r = 0.55, hp = 2, speed = 2.4, pts = 25,
        color = { 1.0, 0.6, 0.1 }, pool = 14, burst = 18, burst_speed = 9, sound = "kill",
    },
    brute = {
        mesh = "sphere", scale = { 1.2, 1.0, 1.2 }, r = 1.2, hp = 9, speed = 1.7, pts = 80,
        color = { 0.45, 1.0, 0.3 }, pool = 8, burst = 40, burst_speed = 11, sound = "boom",
    },
}
local KIND_ORDER = { "drone", "dasher", "brute" }

local g = {
    phase = "play", demo = true, auto = true, t = 0, time = 0,
    score = 0, best = 0, lives = START_LIVES, wave = 0,
    px = 0.0, pz = 0.0, vx = 0.0, vz = 0.0, ax = 0.0, az = -1.0,
    fire_cd = 0, dash_t = 0, dash_cd = 0, ddx = 0.0, ddz = -1.0,
    invuln = 0, spread = 0, combo = 0, combo_t = 0, mult = 1,
    shake = 0, flash = 0, trail_cd = 0, orbit = 1,
    wave_state = "banner", wave_t = 0, spawn_cd = 0, queue = {},
    bullets = {}, enemies = {}, sparks = {}, pickups = {}, rings = {}, popups = {},
    bullet_i = 0, spark_i = 0, ring_i = 0,
    ship_vis = true, light_i = nil, kills = 0, history = {},
}

GAME = g -- global so lua_eval can inspect and poke the game state while testing

local ent = {}
local sfx_last = {}

-- ---------------------------------------------------------------- helpers

local function clamp(v, lo, hi)
    if v < lo then return lo elseif v > hi then return hi end
    return v
end

local function frand(a, b) return a + (b - a) * math.random() end

local function sfx(name, pitch, vol, min_gap)
    if min_gap and sfx_last[name] and g.time - sfx_last[name] < min_gap then return end
    sfx_last[name] = g.time
    local v = (vol or 1.0) * SFX_VOLUME * (g.demo and 0.5 or 1.0)
    play_sound("content/audio/" .. name .. ".wav", { volume = v, pitch = pitch or 1.0 })
end

local function show(o, v)
    if o.vis ~= v then
        o.vis = v
        set_component(o.e, "Visible", v)
    end
end

local function find(name)
    local e = find_entity_by_name(name)
    if not e then log_error("neon_rift: entity '" .. name .. "' not found") end
    return e
end

-- Create (or reuse, after a hot reload) a pooled primitive, hidden below the floor.
local function make_prim(mesh, name, scale, material)
    local e = find_entity_by_name(name)
    if not e then
        e = spawn_primitive(mesh, name)
        set_material(e, material)
    end
    set_component(e, "CanCastShadow", false)
    set_component(e, "Visible", false)
    set_scale(e, scale[1], scale[2], scale[3])
    set_translation(e, 0, -50, 0)
    return { e = e, vis = false }
end

-- ---------------------------------------------------------------- pools

local function build_pools()
    g.bullets, g.enemies, g.sparks, g.pickups, g.rings = {}, {}, {}, {}, {}
    for i = 1, N_BULLETS do
        local o = make_prim("cube", "NR_Bullet" .. i, { 0.14, 0.14, 0.8 },
            { color = { 1.0, 0.95, 0.45 }, emissive = 1.0, unlit = true })
        o.alive, o.x, o.z, o.vx, o.vz, o.life = false, 0, 0, 0, 0, 0
        g.bullets[i] = o
    end
    for i = 1, N_SPARKS do
        local o = make_prim("cube", "NR_Spark" .. i, { 0.15, 0.15, 0.15 },
            { color = { 1, 1, 1 }, emissive = 1.0, unlit = true })
        o.alive, o.x, o.y, o.z, o.vx, o.vy, o.vz = false, 0, 0, 0, 0, 0, 0
        o.life, o.max, o.size = 0, 1, 0.15
        g.sparks[i] = o
    end
    for _, kind in ipairs(KIND_ORDER) do
        local k = KINDS[kind]
        for i = 1, k.pool do
            local o = make_prim(k.mesh, "NR_" .. kind .. i, k.scale,
                { color = k.color, emissive = 1.0, metallic = 0.2, roughness = 0.35 })
            o.kind, o.k, o.r = kind, k, k.r
            o.alive, o.x, o.z, o.vx, o.vz, o.hp = false, 0, 0, 0, 0, 1
            o.spawn_t, o.spawn_total, o.state, o.st = 0, 1, "approach", 0
            o.dirx, o.dirz, o.flash, o.white, o.spin = 0, 1, 0, false, math.random() * 6
            g.enemies[#g.enemies + 1] = o
        end
    end
    for i = 1, N_PICKUPS do
        local o = make_prim("sphere", "NR_Pickup" .. i, { 0.38, 0.38, 0.38 },
            { color = { 1.0, 0.9, 0.2 }, emissive = 1.0, unlit = true })
        o.alive, o.x, o.z, o.t = false, 0, 0, 0
        g.pickups[i] = o
    end
    for i = 1, N_RINGS do
        local o = make_prim("cylinder", "NR_Ring" .. i, { 1, 1, 0.03 },
            { color = { 0.3, 0.9, 1.0, 0.5 }, emissive = 1.0, unlit = true, transparent = true })
        set_rotation(o.e, 0, math.pi / 2, 0)
        o.alive, o.x, o.z, o.t, o.dur, o.maxr, o.col = false, 0, 0, 0, 1, 1, { 0.3, 0.9, 1.0 }
        g.rings[i] = o
    end
end

-- ---------------------------------------------------------------- effects

local function spark(x, y, z, vx, vy, vz, life, size, color)
    g.spark_i = g.spark_i % N_SPARKS + 1
    local o = g.sparks[g.spark_i]
    o.alive, o.x, o.y, o.z, o.vx, o.vy, o.vz = true, x, y, z, vx, vy, vz
    o.life, o.max, o.size = life, life, size
    set_material(o.e, { color = color })
    set_translation(o.e, x, y, z)
    show(o, true)
end

local function burst(x, y, z, color, count, speed, size)
    for _ = 1, count do
        local a = math.random() * 6.2832
        local s = frand(speed * 0.25, speed)
        spark(x, y, z, math.cos(a) * s, frand(1.5, 6.0), math.sin(a) * s,
            frand(0.35, 0.8), (size or 0.16) * frand(0.6, 1.3), color)
    end
end

local function ring(x, z, maxr, dur, color)
    g.ring_i = g.ring_i % N_RINGS + 1
    local o = g.rings[g.ring_i]
    o.alive, o.x, o.z, o.t, o.dur, o.maxr, o.col = true, x, z, 0, dur, maxr, color
    set_translation(o.e, x, 0.12, z)
    show(o, true)
end

local function popup(x, z, text, color)
    if #g.popups >= 14 then table.remove(g.popups, 1) end
    g.popups[#g.popups + 1] = { x = x, z = z, text = text, color = color, t = 0 }
end

local function add_shake(a) g.shake = math.min(1.0, math.max(g.shake, a)) end

-- ---------------------------------------------------------------- enemies

local function alive_count()
    local n = 0
    for _, en in ipairs(g.enemies) do
        if en.alive then n = n + 1 end
    end
    return n
end

local function spawn_enemy(kind, x, z, spawn_time)
    for _, en in ipairs(g.enemies) do
        if not en.alive and en.kind == kind then
            en.alive, en.x, en.z, en.vx, en.vz = true, x, z, 0, 0
            en.hp = en.k.hp
            en.spawn_t = spawn_time or 0.7
            en.spawn_total = math.max(en.spawn_t, 0.01)
            en.state, en.st, en.flash = "approach", 0, 0
            en.dirx, en.dirz = 0, 1
            if en.white then
                en.white = false
                set_material(en.e, { color = en.k.color })
            end
            show(en, true)
            return en
        end
    end
    return nil
end

local function drop_pickup(x, z)
    for _, p in ipairs(g.pickups) do
        if not p.alive then
            p.alive, p.x, p.z, p.t = true, x, z, 0
            show(p, true)
            return
        end
    end
end

local function kill_enemy(en)
    local k = en.k
    en.alive = false
    show(en, false)
    set_translation(en.e, 0, -50, 0)

    g.kills = g.kills + 1
    g.combo = g.combo + 1
    g.combo_t = COMBO_TIME
    g.mult = math.min(8, 1 + math.floor(g.combo / 6))
    local pts = k.pts * g.mult
    g.score = g.score + pts
    if not g.demo and g.score > g.best then g.best = g.score end

    popup(en.x, en.z, "+" .. pts, k.color)
    burst(en.x, 0.5, en.z, k.color, k.burst, k.burst_speed)
    if en.kind == "brute" then
        ring(en.x, en.z, 4.0, 0.5, k.color)
        add_shake(0.55)
    else
        add_shake(0.12)
    end
    sfx(k.sound, frand(0.9, 1.15), en.kind == "brute" and 1.0 or 0.7, 0.04)

    if en.kind == "brute" then
        -- Splits into a pair of drones and always drops a power-up.
        for i = 1, 2 do
            local a = math.random() * 6.2832
            local d = spawn_enemy("drone", clamp(en.x + math.cos(a) * 1.2, -HW + 0.6, HW - 0.6),
                clamp(en.z + math.sin(a) * 1.2, -HD + 0.6, HD - 0.6), 0.25)
            if d then d.vx, d.vz = math.cos(a) * 7, math.sin(a) * 7 end
        end
        drop_pickup(en.x, en.z)
    elseif math.random() < 0.06 then
        drop_pickup(en.x, en.z)
    end
end

local function damage_enemy(en, amount)
    en.hp = en.hp - amount
    if en.hp <= 0 then
        kill_enemy(en)
        return true
    end
    en.flash = 0.07
    sfx("hit", frand(0.9, 1.2), 0.5, 0.05)
    return false
end

local function update_enemies(dt)
    local speed_mul = 1.0 + 0.05 * math.min(g.wave, 20)
    for _, en in ipairs(g.enemies) do
        if en.alive then
            local k = en.k
            if en.spawn_t > 0 then
                en.spawn_t = en.spawn_t - dt
            else
                local dx, dz = g.px - en.x, g.pz - en.z
                local d = math.sqrt(dx * dx + dz * dz) + 1e-4
                local ux, uz = dx / d, dz / d
                local speed = k.speed * speed_mul
                if en.kind == "dasher" then
                    en.st = en.st + dt
                    if en.state == "approach" then
                        en.vx = en.vx + (ux * speed - en.vx) * math.min(1, 4 * dt)
                        en.vz = en.vz + (uz * speed - en.vz) * math.min(1, 4 * dt)
                        en.dirx, en.dirz = ux, uz
                        if en.st > 1.2 and d < 11 then
                            en.state, en.st = "wind", 0
                        end
                    elseif en.state == "wind" then
                        en.vx, en.vz = en.vx * (1 - math.min(1, 8 * dt)), en.vz * (1 - math.min(1, 8 * dt))
                        if en.st < 0.35 then en.dirx, en.dirz = ux, uz end -- tracks, then locks
                        if en.st > 0.65 then en.state, en.st = "dash", 0 end
                    elseif en.state == "dash" then
                        en.vx, en.vz = en.dirx * 17.0, en.dirz * 17.0
                        if en.st > 0.42 then en.state, en.st = "recover", 0 end
                    else -- recover
                        en.vx, en.vz = en.vx * (1 - math.min(1, 5 * dt)), en.vz * (1 - math.min(1, 5 * dt))
                        if en.st > 0.7 then en.state, en.st = "approach", 0 end
                    end
                else
                    local f = math.min(1, 4 * dt)
                    en.vx = en.vx + (ux * speed - en.vx) * f
                    en.vz = en.vz + (uz * speed - en.vz) * f
                    en.dirx, en.dirz = ux, uz
                end
                en.x = en.x + en.vx * dt
                en.z = en.z + en.vz * dt
                local lx, lz = HW - en.r, HD - en.r
                if en.x > lx then en.x, en.vx = lx, 0 elseif en.x < -lx then en.x, en.vx = -lx, 0 end
                if en.z > lz then en.z, en.vz = lz, 0 elseif en.z < -lz then en.z, en.vz = -lz, 0 end
            end
            if en.flash > 0 then en.flash = en.flash - dt end
        end
    end

    -- Soft separation so a swarm spreads out instead of stacking.
    local list = g.enemies
    for i = 1, #list - 1 do
        local a = list[i]
        if a.alive and a.spawn_t <= 0 then
            for j = i + 1, #list do
                local b = list[j]
                if b.alive and b.spawn_t <= 0 then
                    local dx, dz = b.x - a.x, b.z - a.z
                    local rr = (a.r + b.r) * 0.9
                    local d2 = dx * dx + dz * dz
                    if d2 < rr * rr and d2 > 1e-6 then
                        local d = math.sqrt(d2)
                        local push = (rr - d) * 0.5 / d
                        a.x, a.z = a.x - dx * push, a.z - dz * push
                        b.x, b.z = b.x + dx * push, b.z + dz * push
                    end
                end
            end
        end
    end
end

local function render_enemies(dt)
    for _, en in ipairs(g.enemies) do
        if en.alive then
            local k = en.k
            local sk = 1.0
            if en.spawn_t > 0 then
                local p = 1 - en.spawn_t / en.spawn_total
                sk = 0.15 + 0.85 * p * p
                sk = sk * (1 + 0.15 * math.sin(g.time * 40))
            end
            local want_white = en.flash > 0 or en.state == "wind"
            if want_white ~= en.white then
                en.white = want_white
                set_material(en.e, { color = want_white and { 1.0, 1.0, 0.9 } or k.color })
            end
            if en.kind == "dasher" then
                local half = k.scale[3] * 0.5
                set_translation(en.e, en.x + en.dirx * half, 0.5, en.z + en.dirz * half)
                set_rotation(en.e, math.atan(en.dirx, en.dirz), 0, 0)
                set_scale(en.e, k.scale[1] * sk, k.scale[2] * sk, k.scale[3] * sk)
            elseif en.kind == "drone" then
                en.spin = en.spin + dt * 3.2
                set_translation(en.e, en.x, 0.5, en.z)
                set_rotation(en.e, en.spin, en.spin * 0.6, 0)
                set_scale(en.e, k.scale[1] * sk, k.scale[2] * sk, k.scale[3] * sk)
            else
                local pulse = 1 + 0.06 * math.sin(g.time * 5 + en.spin)
                set_translation(en.e, en.x, 0.9, en.z)
                set_scale(en.e, k.scale[1] * sk * pulse, k.scale[2] * sk * pulse, k.scale[3] * sk * pulse)
            end
        end
    end
end

-- ---------------------------------------------------------------- bullets

local function fire_bullet(x, z, dx, dz)
    g.bullet_i = g.bullet_i % N_BULLETS + 1
    local b = g.bullets[g.bullet_i]
    b.alive, b.x, b.z = true, x, z
    b.vx, b.vz = dx * BULLET_SPEED, dz * BULLET_SPEED
    b.life = BULLET_LIFE
    set_rotation(b.e, math.atan(dx, dz), 0, 0)
    set_translation(b.e, x, 0.5, z)
    show(b, true)
end

local function kill_bullet(b)
    b.alive = false
    show(b, false)
    set_translation(b.e, 0, -50, 0)
end

local function update_bullets(dt)
    for _, b in ipairs(g.bullets) do
        if b.alive then
            b.x = b.x + b.vx * dt
            b.z = b.z + b.vz * dt
            b.life = b.life - dt
            if b.life <= 0 or b.x > HW or b.x < -HW or b.z > HD or b.z < -HD then
                if b.life > 0 then
                    burst(clamp(b.x, -HW, HW), 0.5, clamp(b.z, -HD, HD), { 1.0, 0.9, 0.4 }, 3, 3, 0.1)
                end
                kill_bullet(b)
            else
                local hit = false
                for _, en in ipairs(g.enemies) do
                    if en.alive and en.spawn_t <= 0 then
                        local dx, dz = en.x - b.x, en.z - b.z
                        local rr = en.r + BULLET_R
                        if dx * dx + dz * dz < rr * rr then
                            burst(b.x, 0.5, b.z, { 1.0, 0.95, 0.6 }, 2, 4, 0.1)
                            damage_enemy(en, 1)
                            hit = true
                            break
                        end
                    end
                end
                if hit then
                    kill_bullet(b)
                else
                    set_translation(b.e, b.x, 0.5, b.z)
                end
            end
        end
    end
end

-- ---------------------------------------------------------------- player

local function hurt_player()
    g.lives = g.lives - 1
    g.invuln = 2.2
    g.flash = 1.0
    g.combo, g.mult = 0, 1
    add_shake(1.0)
    sfx("player_hit")
    sfx("nova", 1.0, 0.8)
    ring(g.px, g.pz, 6.5, 0.55, { 0.3, 0.9, 1.0 })
    burst(g.px, 0.5, g.pz, { 0.4, 1.0, 1.0 }, 26, 10)

    -- Shockwave: wipes small enemies, wounds brutes and shoves everything away.
    for _, en in ipairs(g.enemies) do
        if en.alive then
            local dx, dz = en.x - g.px, en.z - g.pz
            local d = math.sqrt(dx * dx + dz * dz) + 1e-4
            if d < 6.5 then
                if en.kind == "brute" then
                    if not damage_enemy(en, 3) then
                        en.vx, en.vz = dx / d * 16, dz / d * 16
                    end
                else
                    kill_enemy(en)
                end
            end
        end
    end

    if g.lives <= 0 then
        g.phase, g.t = "over", 0
        g.history[#g.history + 1] = { score = g.score, wave = g.wave, kills = g.kills, demo = g.demo }
        burst(g.px, 0.5, g.pz, { 0.4, 1.0, 1.0 }, 60, 13, 0.2)
        burst(g.px, 0.5, g.pz, { 1.0, 0.7, 0.3 }, 30, 8, 0.2)
        sfx("gameover")
    end
end

local function update_player(dt, inp)
    g.invuln = math.max(0, g.invuln - dt)
    g.dash_cd = math.max(0, g.dash_cd - dt)

    if inp.dash and g.dash_cd <= 0 and g.dash_t <= 0 then
        g.dash_t, g.dash_cd = DASH_TIME, DASH_CD
        local dx, dz = inp.mx, inp.mz
        if dx == 0 and dz == 0 then dx, dz = g.ax, g.az end
        local l = math.sqrt(dx * dx + dz * dz) + 1e-6
        g.ddx, g.ddz = dx / l, dz / l
        sfx("dash")
        burst(g.px, 0.5, g.pz, { 0.5, 0.9, 1.0 }, 8, 5, 0.12)
    end

    if g.dash_t > 0 then
        g.dash_t = g.dash_t - dt
        g.vx, g.vz = g.ddx * DASH_SPEED, g.ddz * DASH_SPEED
        if g.dash_t <= 0 then g.invuln = math.max(g.invuln, 0.12) end
        spark(g.px, 0.5, g.pz, 0, 0.5, 0, 0.35, 0.3, { 0.4, 0.9, 1.0 })
    else
        local f = math.min(1, 16 * dt)
        g.vx = g.vx + (inp.mx * P_SPEED - g.vx) * f
        g.vz = g.vz + (inp.mz * P_SPEED - g.vz) * f
    end

    g.px = g.px + g.vx * dt
    g.pz = g.pz + g.vz * dt
    if g.px > LX then g.px, g.vx = LX, 0 elseif g.px < -LX then g.px, g.vx = -LX, 0 end
    if g.pz > LZ then g.pz, g.vz = LZ, 0 elseif g.pz < -LZ then g.pz, g.vz = -LZ, 0 end

    -- Facing: follows the aim while firing, otherwise eases toward the move direction.
    if inp.fire then
        g.ax, g.az = inp.ax, inp.az
    elseif inp.mx ~= 0 or inp.mz ~= 0 then
        local f = math.min(1, 10 * dt)
        local nx, nz = g.ax + (inp.mx - g.ax) * f, g.az + (inp.mz - g.az) * f
        local l = math.sqrt(nx * nx + nz * nz)
        if l > 1e-4 then g.ax, g.az = nx / l, nz / l end
    end

    -- Engine exhaust.
    g.trail_cd = g.trail_cd - dt
    if g.trail_cd <= 0 then
        g.trail_cd = 0.035
        local spd = math.sqrt(g.vx * g.vx + g.vz * g.vz)
        if spd > 2 then
            spark(g.px - g.ax * 0.8, 0.5, g.pz - g.az * 0.8, -g.vx * 0.1, 0.3, -g.vz * 0.1,
                0.3, 0.14, { 1.0, 0.65, 0.25 })
        end
    end

    -- Shooting.
    g.fire_cd = g.fire_cd - dt
    g.spread = math.max(0, g.spread - dt)
    if inp.fire and g.fire_cd <= 0 then
        local spread = g.spread > 0
        g.fire_cd = spread and SPREAD_INTERVAL or FIRE_INTERVAL
        local angles = spread and { -SPREAD_ANGLE, 0, SPREAD_ANGLE } or { 0 }
        for _, a in ipairs(angles) do
            a = a + frand(-0.015, 0.015)
            local c, s = math.cos(a), math.sin(a)
            fire_bullet(g.px + g.ax * 0.9, g.pz + g.az * 0.9,
                g.ax * c - g.az * s, g.ax * s + g.az * c)
        end
        sfx("shoot", frand(0.92, 1.1), 0.45)
    end

    -- Enemy contact.
    if g.invuln <= 0 and g.dash_t <= 0 then
        for _, en in ipairs(g.enemies) do
            if en.alive and en.spawn_t <= 0 then
                local dx, dz = en.x - g.px, en.z - g.pz
                local rr = en.r + P_R * 0.8
                if dx * dx + dz * dz < rr * rr then
                    hurt_player()
                    break
                end
            end
        end
    end
end

local function update_pickups(dt)
    for _, p in ipairs(g.pickups) do
        if p.alive then
            p.t = p.t + dt
            local dx, dz = g.px - p.x, g.pz - p.z
            if dx * dx + dz * dz < 1.3 * 1.3 then
                p.alive = false
                show(p, false)
                set_translation(p.e, 0, -50, 0)
                g.spread = SPREAD_TIME
                sfx("pickup")
                popup(p.x, p.z, "SPREAD SHOT", { 1.0, 0.9, 0.2 })
                burst(p.x, 0.5, p.z, { 1.0, 0.9, 0.2 }, 14, 6)
            elseif p.t > 10 then
                p.alive = false
                show(p, false)
                set_translation(p.e, 0, -50, 0)
            else
                set_translation(p.e, p.x, 0.6 + 0.15 * math.sin(g.time * 5 + p.x), p.z)
                local blink = p.t > 8 and math.floor(g.time * 10) % 2 == 0
                show(p, not blink)
            end
        end
    end
end

-- ---------------------------------------------------------------- input

local function key(name) return is_key_pressed(name) and 1 or 0 end

local function read_human()
    local mx = key("D") - key("A")
    local mz = key("S") - key("W")
    local l = math.sqrt(mx * mx + mz * mz)
    if l > 0 then mx, mz = mx / l, mz / l end
    local fx = key("ArrowRight") - key("ArrowLeft")
    local fz = key("ArrowDown") - key("ArrowUp")
    local fl = math.sqrt(fx * fx + fz * fz)
    local fire = fl > 0
    local ax, az = g.ax, g.az
    if fire then ax, az = fx / fl, fz / fl end
    return {
        mx = mx, mz = mz, ax = ax, az = az, fire = fire,
        dash = is_key_just_pressed("Space") or is_key_just_pressed("Shift"),
    }
end

-- The autopilot: steers away from threats while orbiting, grabs power-ups, shoots the
-- nearest enemy with a little lead and dashes out when something gets too close.
local function ai_compute(jit)
    local mx, mz, threat = 0.0, 0.0, 0.0
    local best, bd2 = nil, 1e9
    local closest = 1e9
    for _, en in ipairs(g.enemies) do
        if en.alive and en.spawn_t <= 0 then
            local dx, dz = g.px - en.x, g.pz - en.z
            local d2 = dx * dx + dz * dz
            local d = math.sqrt(d2) + 1e-4
            local reach = 6.5 + en.r
            if en.state == "dash" then reach = 11.0 end
            if d < reach then
                local w = (reach - d) / reach
                w = w * w * (en.state == "dash" and 10 or 5)
                mx, mz = mx + dx / d * w, mz + dz / d * w
                threat = threat + w
                if en.state == "dash" or en.state == "wind" then
                    -- sidestep the lunge line instead of backing straight down it
                    local qx, qz = -en.dirz, en.dirx
                    local side = (dx * qx + dz * qz) >= 0 and 1 or -1
                    mx, mz = mx + qx * side * w, mz + qz * side * w
                end
            end
            if d < closest then closest = d end
            if d2 < bd2 then bd2, best = d2, en end
        end
    end

    -- keep off the walls
    local m = 3.5
    if g.px > LX - m then mx = mx - (g.px - (LX - m)) / m * 4 end
    if g.px < -LX + m then mx = mx + ((-LX + m) - g.px) / m * 4 end
    if g.pz > LZ - m then mz = mz - (g.pz - (LZ - m)) / m * 4 end
    if g.pz < -LZ + m then mz = mz + ((-LZ + m) - g.pz) / m * 4 end

    -- circle around the swarm, toward whichever side has more room
    if threat > 0.05 then
        local l = math.sqrt(mx * mx + mz * mz) + 1e-6
        local tx, tz = -mz / l * g.orbit, mx / l * g.orbit
        if tx * -g.px + tz * -g.pz < -2.0 then g.orbit = -g.orbit; tx, tz = -tx, -tz end
        mx, mz = mx + tx * threat * 0.8, mz + tz * threat * 0.8
    end

    -- fetch power-ups when it is safe enough
    if threat < 1.5 then
        local pbest, pd2 = nil, 1e9
        for _, p in ipairs(g.pickups) do
            if p.alive then
                local dx, dz = p.x - g.px, p.z - g.pz
                local d2 = dx * dx + dz * dz
                if d2 < pd2 then pd2, pbest = d2, p end
            end
        end
        if pbest then
            local d = math.sqrt(pd2) + 1e-4
            mx, mz = mx + (pbest.x - g.px) / d * 2.0, mz + (pbest.z - g.pz) / d * 2.0
        elseif threat < 0.05 then
            -- idle: drift back toward the middle
            mx, mz = mx - g.px * 0.05, mz - g.pz * 0.05
        end
    end

    local ml = math.sqrt(mx * mx + mz * mz)
    if ml > 1 then mx, mz = mx / ml, mz / ml end
    if ml < 0.12 then mx, mz = 0, 0 end

    local ax, az, fire = g.ax, g.az, false
    if best then
        local d = math.sqrt(bd2) + 1e-4
        local t = d / BULLET_SPEED
        local dx, dz = best.x + best.vx * t - g.px, best.z + best.vz * t - g.pz
        local l = math.sqrt(dx * dx + dz * dz) + 1e-6
        local j = jit or 0
        local c, s = math.cos(j), math.sin(j)
        local nx, nz = dx / l, dz / l
        ax, az = nx * c - nz * s, nx * s + nz * c
        fire = d < 24
    end

    local dash = closest < 1.5 and (mx ~= 0 or mz ~= 0)
    return { mx = mx, mz = mz, ax = ax, az = az, fire = fire, dash = dash }
end

-- The pilot is deliberately fallible so the demo ends in game overs now and then: it only
-- re-plans its steering about five times a second, holds a random aim error between
-- re-plans and fires in bursts with short pauses.
local ai = { cd = 0, mx = 0.0, mz = 0.0, dash = false, jit = 0.0, fire_on = true, fire_t = 1.2 }

local function ai_input(dt)
    local inp = ai_compute(ai.jit)
    ai.cd = ai.cd - dt
    if ai.cd <= 0 then
        ai.cd = frand(0.14, 0.24)
        ai.jit = (math.random() - 0.5) * 0.36
        ai.mx, ai.mz, ai.dash = inp.mx, inp.mz, inp.dash
    end
    inp.mx, inp.mz, inp.dash = ai.mx, ai.mz, ai.dash
    ai.dash = false
    ai.fire_t = ai.fire_t - dt
    if ai.fire_t <= 0 then
        ai.fire_on = not ai.fire_on
        ai.fire_t = ai.fire_on and frand(0.9, 1.7) or frand(0.3, 0.6)
    end
    inp.fire = inp.fire and ai.fire_on
    return inp
end

-- ---------------------------------------------------------------- waves

local function build_wave(n)
    local q = {}
    for _ = 1, 6 + 3 * n do q[#q + 1] = "drone" end
    if n >= 2 then for _ = 1, math.min(n - 1, 10) do q[#q + 1] = "dasher" end end
    if n >= 3 then for _ = 1, math.min(math.floor((n - 1) / 2), 6) do q[#q + 1] = "brute" end end
    for i = #q, 2, -1 do
        local j = math.random(i)
        q[i], q[j] = q[j], q[i]
    end
    g.queue = q
end

local function begin_wave(n)
    g.wave = n
    build_wave(n)
    g.wave_state, g.wave_t, g.spawn_cd = "banner", 0, 0
    sfx("wave")
end

local function pick_spawn_point()
    local x, z
    for _ = 1, 16 do
        x, z = frand(-HW + 1.5, HW - 1.5), frand(-HD + 1.5, HD - 1.5)
        local dx, dz = x - g.px, z - g.pz
        if dx * dx + dz * dz > 8 * 8 then return x, z end
    end
    return x, z
end

local function update_director(dt)
    g.wave_t = g.wave_t + dt
    if g.wave_state == "banner" then
        if g.wave_t >= 1.8 then g.wave_state, g.wave_t = "fight", 0 end
    elseif g.wave_state == "fight" then
        if #g.queue > 0 then
            g.spawn_cd = g.spawn_cd - dt
            local max_alive = math.min(14 + 2 * g.wave, 44)
            if g.spawn_cd <= 0 and alive_count() < max_alive then
                local kind = g.queue[#g.queue]
                local x, z = pick_spawn_point()
                if spawn_enemy(kind, x, z, 0.7) then
                    g.queue[#g.queue] = nil
                    g.spawn_cd = math.max(0.12, 0.55 - 0.035 * g.wave)
                end
            end
        elseif alive_count() == 0 then
            g.wave_state, g.wave_t = "clear", 0
            local bonus = 100 * g.wave
            g.score = g.score + bonus
            if not g.demo and g.score > g.best then g.best = g.score end
            popup(g.px, g.pz, "WAVE CLEAR +" .. bonus, { 0.4, 1.0, 1.0 })
            sfx("clear")
        end
    elseif g.wave_state == "clear" then
        if g.wave_t >= 1.6 then begin_wave(g.wave + 1) end
    end
end

-- ---------------------------------------------------------------- game flow

local function clear_world()
    for _, b in ipairs(g.bullets) do
        if b.alive then kill_bullet(b) end
    end
    for _, en in ipairs(g.enemies) do
        if en.alive then
            en.alive = false
            show(en, false)
            set_translation(en.e, 0, -50, 0)
        end
    end
    for _, s in ipairs(g.sparks) do
        if s.alive then s.alive = false; show(s, false) end
    end
    for _, p in ipairs(g.pickups) do
        if p.alive then p.alive = false; show(p, false) end
    end
    for _, r in ipairs(g.rings) do
        if r.alive then r.alive = false; show(r, false) end
    end
    g.popups = {}
end

local function start_game(demo)
    clear_world()
    g.demo, g.auto, g.phase, g.t = demo, demo, "play", 0
    g.score, g.lives, g.kills = 0, START_LIVES, 0
    g.px, g.pz, g.vx, g.vz = 0.0, 4.0, 0.0, 0.0
    g.ax, g.az = 0.0, -1.0
    g.fire_cd, g.dash_t, g.dash_cd, g.invuln, g.spread = 0, 0, 0.35, 0, 0 -- cooldown: the Space that started the game is not a dash
    g.combo, g.combo_t, g.mult, g.shake, g.flash = 0, 0, 1, 0, 0
    g.orbit = math.random() < 0.5 and 1 or -1
    begin_wave(1)
    if not demo then sfx("start") end
end

local function update_input_phase()
    if g.phase == "play" and g.demo then
        if is_key_just_pressed("Space") or is_key_just_pressed("Return") then start_game(false) end
    elseif g.phase == "play" then
        if is_key_just_pressed("P") then g.auto = not g.auto end
    elseif g.phase == "over" then
        if g.demo then
            if g.t > 3.0 then start_game(true) end
            if is_key_just_pressed("Space") or is_key_just_pressed("Return") then start_game(false) end
        else
            if g.t > 1.0 and (is_key_just_pressed("Space") or is_key_just_pressed("Return")) then
                start_game(false)
            elseif g.t > 14.0 then
                start_game(true) -- nobody there: back to the demo
            end
        end
    end
end

-- ---------------------------------------------------------------- per-frame visuals

local function update_fx(dt)
    for _, s in ipairs(g.sparks) do
        if s.alive then
            s.life = s.life - dt
            if s.life <= 0 then
                s.alive = false
                show(s, false)
                set_translation(s.e, 0, -50, 0)
            else
                s.vy = s.vy - 20 * dt
                s.x, s.y, s.z = s.x + s.vx * dt, s.y + s.vy * dt, s.z + s.vz * dt
                if s.y < 0.08 then s.y, s.vy = 0.08, -s.vy * 0.3 end
                local drag = 1 - math.min(1, 1.5 * dt)
                s.vx, s.vz = s.vx * drag, s.vz * drag
                local k = (s.life / s.max) * s.size
                set_translation(s.e, s.x, s.y, s.z)
                set_scale(s.e, k, k, k)
            end
        end
    end

    for _, r in ipairs(g.rings) do
        if r.alive then
            r.t = r.t + dt
            local k = r.t / r.dur
            if k >= 1 then
                r.alive = false
                show(r, false)
            else
                local rad = r.maxr * (1 - (1 - k) * (1 - k))
                set_scale(r.e, rad, rad, 0.03)
                set_material(r.e, { color = { r.col[1], r.col[2], r.col[3], (1 - k) * 0.55 } })
            end
        end
    end

    for i = #g.popups, 1, -1 do
        local p = g.popups[i]
        p.t = p.t + dt
        if p.t >= POPUP_TIME then table.remove(g.popups, i) end
    end

    -- combo decay
    if g.combo_t > 0 then
        g.combo_t = g.combo_t - dt
        if g.combo_t <= 0 then g.combo, g.mult = 0, 1 end
    end
end

local function sync_player(dt)
    local alive = g.phase == "play"
    local vis = alive
    if alive and g.invuln > 0 and g.dash_t <= 0 then
        vis = math.floor(g.time * 14) % 2 == 0
    end
    if vis ~= g.ship_vis then
        g.ship_vis = vis
        for _, e in ipairs(ent.ship_parts) do set_component(e, "Visible", vis) end
    end
    set_translation(ent.player, g.px, 0.5, g.pz)
    set_rotation(ent.player, math.atan(-g.ax, -g.az), 0, 0)

    g.flash = math.max(0, g.flash - dt * 2.5)
    local li = math.floor((alive and 25 + g.flash * 140 or 0) / 2)
    if li ~= g.light_i then
        g.light_i = li
        set_light(ent.light, { intensity = li * 2 })
    end
end

local function sync_camera(dt)
    g.shake = math.max(0, g.shake - dt * 2.2)
    local a = g.shake * g.shake * 0.9
    local jx = a > 0.001 and (math.random() - 0.5) * 2 * a or 0
    local jz = a > 0.001 and (math.random() - 0.5) * 2 * a or 0
    local par = g.px * 0.06
    set_translation(ent.cam, CAM.x + par + jx, CAM.y, CAM.z + jz)
    look_at(ent.cam, CAM.tx + par * 0.5 + jx, 0, CAM.tz + jz)
end

local function publish()
    set_f64("score", g.score)
    set_i32("lives", g.lives)
    set_i32("wave", g.wave)
    set_string("phase", g.phase)
    set_bool("demo", g.demo)
end

-- ---------------------------------------------------------------- hooks

function on_created(self_entity)
    ent.player = find("Player")
    ent.light = find("PlayerLight")
    ent.cam = find("GameCamera")
    ent.ship_parts = {}
    for _, n in ipairs({ "PlayerHull", "PlayerWingL", "PlayerWingR", "PlayerEngine" }) do
        ent.ship_parts[#ent.ship_parts + 1] = find(n)
    end
    set_scale(ent.player, 1.25, 1.25, 1.25)
    build_pools()
    g.time, g.best = 0, 0
    g.ship_vis, g.light_i = true, nil
    start_game(true)
    log_info(string.format("neon_rift: ready (%d enemies pooled), attract mode", #g.enemies))
end

function update(self_entity, dt)
    dt = math.min(dt, 1 / 30)
    g.time = g.time + dt
    g.t = g.t + dt

    update_input_phase()

    if g.phase == "play" then
        local inp = g.auto and ai_input(dt) or read_human()
        update_player(dt, inp)
        if g.phase == "play" then
            update_bullets(dt)
            update_enemies(dt)
            update_pickups(dt)
            update_director(dt)
        end
    end

    update_fx(dt)
    render_enemies(dt)
    sync_player(dt)
    sync_camera(dt)
    publish()
end

-- ---------------------------------------------------------------- UI

local function banner(ui, x, z, text, size, color)
    ui:world_label(x, 0.6, z, text, { color = color, size = size })
end

function on_ui(self_entity, ui)
    for _, p in ipairs(g.popups) do
        local k = p.t / POPUP_TIME
        ui:world_label(p.x, 1.0 + k * 1.6, p.z, p.text,
            { color = { p.color[1], p.color[2], p.color[3], 1.0 - k * k }, size = 20 })
    end

    ui:anchored_panel("nr_hud", "top_left", 18, 12, nil, nil, function(p)
        p:heading(string.format("SCORE  %07d", g.score))
        p:label(g.demo and string.format("WAVE %d", g.wave) or string.format("WAVE %d     BEST %d", g.wave, g.best))
        if g.mult > 1 then p:label(string.format("MULTIPLIER x%d   (chain %d)", g.mult, g.combo)) end
    end)
    ui:anchored_panel("nr_status", "top_right", 18, 12, nil, nil, function(p)
        p:heading(string.format("SHIPS  %d", math.max(g.lives, 0)))
        if g.dash_cd > 0 then
            p:label(string.format("DASH  %.1fs", g.dash_cd))
        else
            p:label("DASH  READY")
        end
        if g.spread > 0 then p:label(string.format("SPREAD SHOT  %.1fs", g.spread)) end
        if not g.demo and g.auto then p:label("AUTOPILOT (P to take over)") end
    end)

    if g.demo then
        local blink = 0.55 + 0.45 * math.sin(g.time * 4)
        banner(ui, 0, -6.6, "NEON RIFT", 64, { 0.3, 0.95, 1.0, 0.95 })
        banner(ui, 0, 7.4, "PRESS SPACE TO PLAY", 26, { 1.0, 0.4, 0.8, blink })
        banner(ui, 0, 8.7, "WASD move   ARROWS aim + fire   SPACE dash", 15, { 0.8, 0.85, 1.0, 0.8 })
        ui:anchored_panel("nr_demo", "top_center", 0, 12, nil, nil, function(p)
            p:heading("DEMO")
        end)
        if g.phase == "over" then banner(ui, 0, -1.5, "DEMO OVER", 40, { 1.0, 0.5, 0.5, 0.9 }) end
    elseif g.phase == "over" then
        banner(ui, 0, -2.5, "GAME OVER", 64, { 1.0, 0.3, 0.45, 1.0 })
        banner(ui, 0, 0.5, string.format("SCORE %d", g.score), 30, { 1.0, 1.0, 1.0, 1.0 })
        if g.t > 1.0 then
            banner(ui, 0, 3.5, "PRESS SPACE TO PLAY AGAIN", 24,
                { 1.0, 0.4, 0.8, 0.55 + 0.45 * math.sin(g.time * 4) })
        end
    elseif g.wave_state == "banner" then
        local k = clamp(g.wave_t / 1.8, 0, 1)
        banner(ui, 0, -3.0, "WAVE " .. g.wave, 64, { 0.3, 0.95, 1.0, 1.0 - k * k })
    end
end
