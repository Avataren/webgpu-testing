-- Waves: the original's 40-wave tables, where actors start, starting and finishing a wave, and
-- the restart after a death.

local C = require("robotron.config")
local SPR, HW, HD, FR, PXM, BYM = C.SPR, C.HW, C.HD, C.FR, C.PXM, C.BYM
local U = require("robotron.util")
local frand = U.frand
local S = require("robotron.state")
local g, units, pools, sfx = S.g, S.units, S.pools, S.sfx
local Fx = require("robotron.fx")
local explode = Fx.explode
local Actors = require("robotron.actors")
local K, show_unit, unit_y, reset_ai = Actors.K, Actors.show_unit, Actors.unit_y, Actors.reset_ai
local spawn_unit, clear_all_units = Actors.spawn_unit, Actors.clear_all_units
local Player = require("robotron.player")
local materialize_player, clear_bullets = Player.materialize_player, Player.clear_bullets

local floor, min, max = math.floor, math.min, math.max

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
        -- compact the shared list in place (other modules hold on to it)
        local n = 0
        for i = 1, #units do
            local u = units[i]
            if not u.gone then
                n = n + 1
                units[n] = u
            end
        end
        for i = #units, n + 1, -1 do units[i] = nil end
    end
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

return {
    wave_def = wave_def, wave_params = wave_params, electrode_key = electrode_key, start_wave = start_wave, finish_wave = finish_wave,
    respawn_player = respawn_player,
}
