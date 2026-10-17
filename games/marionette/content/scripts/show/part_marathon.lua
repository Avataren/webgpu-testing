-- Part 3: a running track round the arena, one gait per lane, from crawling on the
-- inside to sprinting on the outside. Clips play in place and the script moves each
-- runner at the speed its clip travels (scaled to its size), so feet do not slide.

local S = require("show/core")

local LANES = {
    { clip = "crawling", radius = 17.6 },
    { clip = "injured-walk", radius = 18.9 },
    { clip = "walking", radius = 20.2 },
    { clip = "jogging", radius = 21.5 },
    { clip = "running", radius = 22.8 },
    { clip = "sprint", radius = 24.1 },
}

local part = {
    name = "MARATHON",
    subtitle = "six gaits, each moved at the speed its clip travels: no foot sliding",
    duration = 26,
    base = 300,
}

local lanes = {}

function part.enter()
    local random = S.rng(37)
    lanes = {}
    for _, lane in ipairs(LANES) do
        if S.clips[lane.clip] then
            table.insert(lanes, lane)
        end
    end
    if #lanes == 0 then
        return
    end
    local n = S.count(part.base)
    local cast = S.cast_list(S.characters, n, random)
    local per_lane = math.ceil(n / #lanes)
    for i = 1, n do
        local lane = lanes[(i - 1) % #lanes + 1]
        local index = (i - 1) // #lanes
        local angle = index / per_lane * S.TAU + random(-0.04, 0.04)
        local char = cast[i]
        local actor = S.spawn(char, 0, 0, {
            clip = lane.clip,
            random = random,
        })
        if actor then
            actor.angle = angle
            actor.radius = lane.radius + random(-0.25, 0.25)
            actor.speed = S.ground_speed(lane.clip, char) / actor.radius
        end
    end
    part.update(0, 0)
end

function part.update(t, dt)
    for _, actor in ipairs(S.actors) do
        actor.angle = actor.angle + actor.speed * dt
        local s, c = math.sin(actor.angle), math.cos(actor.angle)
        -- Counter-clockwise seen from above: the tangent is (c, -s) in (x, z).
        S.place(actor, s * actor.radius, 0, c * actor.radius, S.yaw_to(c, -s))
    end
end

function part.camera(t)
    -- Running alongside the outer lanes, then rising to see the whole track.
    local follow = 0.2 + t * (5.4 / 23)
    local rise = S.smooth((t - 15) / 9)
    local radius = S.lerp(27.5, 40, rise)
    local height = S.lerp(1.4, 24, rise)
    local s, c = math.sin(follow), math.cos(follow)
    local eye = { s * radius, height, c * radius }
    local ahead = follow + S.lerp(0.32, 0, rise)
    local look_radius = S.lerp(21, 0, rise)
    local target = { math.sin(ahead) * look_radius, S.lerp(1.0, 0, rise), math.cos(ahead) * look_radius }
    return eye, target, S.lerp(48, 55, rise)
end

return part
