-- The camera: the closest view that fits the whole arena at the window's shape, plus shake.

local C = require("robotron.config")
local HW, HD, CAM_PITCH, FOV = C.HW, C.HD, C.CAM_PITCH, C.FOV
local U = require("robotron.util")
local frand = U.frand
local S = require("robotron.state")
local g = S.g

local sqrt, sin, cos, abs = math.sqrt, math.sin, math.cos, math.abs

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

return {
    update_camera = update_camera,
}
