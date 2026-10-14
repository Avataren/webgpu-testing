-- Collisions: the player's shots against actors, and actors against the player, the family,
-- electrodes and each other.

local C = require("robotron.config")
local HW, HD, P_R, B_R, PXM, BYM = C.HW, C.HD, C.P_R, C.B_R, C.PXM, C.BYM
local U = require("robotron.util")
local clamp, frand, sign, seg_dist2 = U.clamp, U.frand, U.sign, U.seg_dist2
local S = require("robotron.state")
local g, units, pbullets, sfx, set_vis = S.g, S.units, S.pbullets, S.sfx, S.set_vis
local Fx = require("robotron.fx")
local burst, popup = Fx.burst, Fx.popup
local Actors = require("robotron.actors")
local show_unit, add_score, kill_unit = Actors.show_unit, Actors.add_score, Actors.kill_unit
local human_lost, electrode_near = Actors.human_lost, Actors.electrode_near
local spawn_unit = Actors.spawn_unit
local Player = require("robotron.player")
local kill_player = Player.kill_player

local sqrt, abs, min = math.sqrt, math.abs, math.min

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

return {
    update_bullets = update_bullets, update_interactions = update_interactions,
}
