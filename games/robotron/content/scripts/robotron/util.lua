-- Small maths helpers.

local floor, max = math.floor, math.max

local function clamp(v, lo, hi)
    if v < lo then return lo elseif v > hi then return hi end
    return v
end

local function frand(a, b) return a + (b - a) * math.random() end
local function randu(n) return math.random(1, max(1, floor(n))) end -- 1..n, like the original's RMAX
local function srand(n) return math.random(0, 2 * n - 1) - n end    -- -n..n-1
local function sign(v) if v > 0 then return 1 elseif v < 0 then return -1 end return 0 end

-- squared distance from c to the segment a-b, and how far along it the closest point is
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

return {
    clamp = clamp, frand = frand, randu = randu, srand = srand, sign = sign,
    seg_dist2 = seg_dist2,
}
