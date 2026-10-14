-- The autopilot (attract mode, and B during a game): every 70 ms it simulates the player's eight
-- moves 0.7 s ahead and picks the safest one, leaning toward the family; every 50 ms it picks a
-- firing direction. It writes g.bmx/bmz (move) and g.bfx/bfz (fire).

local C = require("robotron.config")
local P_R, LX, LZ, P_SPEED, B_SPEED = C.P_R, C.LX, C.LZ, C.P_SPEED, C.B_SPEED
local PROG_SPEED = C.PROG_SPEED
local U = require("robotron.util")
local clamp = U.clamp
local S = require("robotron.state")
local g, units = S.g, S.units
local Actors = require("robotron.actors")
local free_human = Actors.free_human

local sqrt, abs, min = math.sqrt, math.abs, math.min

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

local function bot_think(dt)
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

return {
    bot_think = bot_think,
}
