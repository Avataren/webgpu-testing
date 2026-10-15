-- The camera: the closest view that fits the whole arena at the window's shape, plus shake.
-- While the autopilot plays the attract-mode demo (after the title), a director cuts between
-- cinematic shots instead.

local C = require("robotron.config")
local HW, HD, CAM_PITCH, FOV, TITLE_T, LEAN = C.HW, C.HD, C.CAM_PITCH, C.FOV, C.TITLE_T, C.LEAN
local U = require("robotron.util")
local frand = U.frand
local S = require("robotron.state")
local g = S.g

local sqrt, sin, cos, abs, exp, random, pi = math.sqrt, math.sin, math.cos, math.abs, math.exp, math.random, math.pi
local atan, rad, min, max = math.atan, math.rad, math.min, math.max

-- The actors lean back toward the camera so their sprites read from the game's steep view; from
-- a low camera that looks like they are falling over, so the lean follows the camera's pitch:
-- all of it from LEAN_FULL down, none below LEAN_NONE.
local LEAN_FULL, LEAN_NONE = rad(55), rad(22)
local function set_lean(cx, cy, cz, tx, ty, tz)
    local dx, dz = tx - cx, tz - cz
    local pitch = atan(cy - ty, sqrt(dx * dx + dz * dz))
    local k = max(0, min(1, (pitch - LEAN_NONE) / (LEAN_FULL - LEAN_NONE)))
    local lean = LEAN * k
    if lean ~= g.lean then
        g.lean, g.lean_c, g.lean_s = lean, cos(lean), sin(lean)
    end
end

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

-- ATTRACT-MODE DIRECTOR. Each shot lasts SHOT_T seconds and is a function of its own time t
-- (0..SHOT_T), its random variation v and the followed point (the player, smoothed), returning
-- the camera position, the point it looks at and the field of view. Shots cut from one to the
-- next in a shuffled order, every shot once before any repeats, overview included.
local SHOT_T = 7.0
local function smooth(k) return k * k * (3 - 2 * k) end
local SHOTS = {
    -- the game's own view
    overview = false,
    -- a slow circle around the arena from above the rail
    orbit = function(t, v)
        local a = v.a + v.dir * t * 0.11
        return sin(a) * 27, 8.5, cos(a) * 20, 0, 0.3, 0, 42
    end,
    -- behind and above the player, following
    chase = function(t, v, fx, fz)
        return fx + v.dir * 3.5, 5.0, fz + 9.0, fx, 0.7, fz, 40
    end,
    -- low over the floor just inside one wall, tracking along it and looking across the arena
    skim = function(t, v)
        local x = v.dir * (-14 + 28 * smooth(t / SHOT_T))
        return x, 2.0, v.side * (HD - 0.9), x * 0.4, 0.0, -v.side * 3.0, 50
    end,
    -- rising out of a corner
    crane = function(t, v)
        local h = 1.8 + 11 * smooth(t / SHOT_T)
        return v.dir * (HW + 3.5), h, v.side * (HD + 3.5), -v.dir * 3, 0, -v.side * 2, 44
    end,
    -- close over the player's shoulder, steep
    close = function(t, v, fx, fz)
        return fx + v.dir * 2.0, 12.5, fz + 6.5, fx, 0.4, fz, 30
    end,
}
local ORDER = { "overview", "orbit", "chase", "skim", "crane", "close" }

local cine = nil   -- { kind, t, v, fx, fz, order, n } while the director runs

local function next_shot()
    if cine.n >= #cine.order then
        -- reshuffle, never starting with the shot that just ran
        local last = cine.order[#cine.order]
        for i = #cine.order, 2, -1 do
            local j = random(i)
            cine.order[i], cine.order[j] = cine.order[j], cine.order[i]
        end
        if cine.order[1] == last then cine.order[1], cine.order[2] = cine.order[2], cine.order[1] end
        cine.n = 0
    end
    cine.n = cine.n + 1
    cine.kind = cine.order[cine.n]
    cine.t = 0
    cine.v = { a = random() * 2 * pi, dir = random() < 0.5 and -1 or 1, side = random() < 0.5 and -1 or 1 }
    cine.fx, cine.fz = g.px, g.pz
end

local function start_director()
    local order = {}
    for i, k in ipairs(ORDER) do order[i] = k end
    cine = { order = order, n = #order, fov = FOV }
    next_shot()
end

-- for testing: run `kind` now (in the demo, after the title), from the start of the shot
local function force_shot(kind)
    if SHOTS[kind] == nil then error("no shot " .. tostring(kind)) end
    if not cine then start_director() end
    cine.kind, cine.t = kind, 0
end

-- the director's camera; false when the current shot is the overview
local function direct(dt)
    if not cine then start_director() end
    cine.t = cine.t + dt
    if cine.t >= SHOT_T then next_shot() end
    -- the followed point trails the player a little, so shots that follow do not jitter
    local k = 1 - exp(-dt * 3.0)
    cine.fx = cine.fx + (g.px - cine.fx) * k
    cine.fz = cine.fz + (g.pz - cine.fz) * k
    local shot = SHOTS[cine.kind]
    local fov = FOV
    if shot then
        local cx, cy, cz, tx, ty, tz
        cx, cy, cz, tx, ty, tz, fov = shot(cine.t, cine.v, cine.fx, cine.fz)
        local sx, sz = 0, 0
        if g.shake > 0 then
            local s = g.shake * g.shake * 0.3
            sx, sz = frand(-s, s), frand(-s, s)
        end
        set_translation(g.cam, cx + sx, cy, cz + sz)
        look_at(g.cam, tx + sx, ty, tz + sz)
        set_lean(cx, cy, cz, tx, ty, tz)
    end
    if fov ~= cine.fov then
        set_camera(g.cam, { fov_deg = fov })
        cine.fov = fov
    end
    return shot ~= false
end

local function update_camera(dt)
    if not g.cam then return end
    if g.demo and g.demo_t > TITLE_T then
        if direct(dt) then return end
    elseif cine then
        -- back to the game's view
        if cine.fov ~= FOV then set_camera(g.cam, { fov_deg = FOV }) end
        cine = nil
    end
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
    set_lean(0, f.dist * sin(CAM_PITCH), f.dist * cos(CAM_PITCH), 0, 0, 0)
end

return {
    update_camera = update_camera, force_shot = force_shot,
}
