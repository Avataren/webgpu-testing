-- Part 5: the stands fill up. A thousand people (at crowd size 1x) cheer, clap and wave
-- while a Mexican wave runs round the stadium and dancers perform on the floor. This is
-- the heaviest part of the show and the core of the benchmark.

local S = require("show/core")

local CHEERS = { "clapping", "cheering", "waving", "ual2-yes", "thumbs-up", "happy-idle", "victory", "jump" }
local WAVE_CLIP = "cheering"
local WAVE_SPEED = 0.85 -- radians per second round the stadium
local WAVE_WIDTH = 0.45 -- radians behind the front that are standing up
local FLOOR_DANCERS = 40

local part = {
    name = "STADIUM",
    subtitle = "a full house: the crowd test",
    duration = 28,
    base = 1000,
    keep = true, -- the finale plays on with this crowd
}

local function seats(n)
    local stands = S.STANDS
    local length = 0
    for tier = 0, stands.tiers - 1 do
        length = length + S.TAU * (stands.first + (tier + 0.5) * stands.depth)
    end
    local spacing = math.max(0.6, math.min(2.5, length / (n * 1.1)))
    local list = {}
    for tier = 0, stands.tiers - 1 do
        local radius = stands.first + (tier + 0.5) * stands.depth
        local count = math.floor(S.TAU * radius / spacing)
        for i = 0, count - 1 do
            local angle = (i + (tier % 2) * 0.5) / count * S.TAU
            table.insert(list, { angle = angle, radius = radius, y = (tier + 1) * stands.rise })
        end
    end
    return list
end

function part.enter()
    local random = S.rng(71)
    local cheers = S.available(CHEERS)
    local clips = S.available({ WAVE_CLIP })
    for _, clip in ipairs(cheers) do
        table.insert(clips, clip)
    end
    local n = S.count(part.base)
    local spots = seats(n)
    for i = #spots, 2, -1 do
        local j = random(i)
        spots[i], spots[j] = spots[j], spots[i]
    end
    n = math.min(n, #spots)
    local cast = S.cast_list(S.characters, n, random)
    part.crowd = {}
    for i = 1, n do
        local spot = spots[i]
        local angle = spot.angle + random(-0.004, 0.004)
        local radius = spot.radius + random(-0.25, 0.25)
        local x, z = math.sin(angle) * radius, math.cos(angle) * radius
        local base = S.pick(cheers, random)
        local actor = S.spawn(cast[i], x, z, {
            y = spot.y,
            yaw = S.yaw_to(-x, -z) + random(-0.2, 0.2),
            clips = clips,
            clip = base,
            random = random,
            speed = random(0.9, 1.1),
        })
        if actor then
            actor.angle = angle % S.TAU
            actor.base = base
            table.insert(part.crowd, actor)
        end
    end

    local dances = S.available({ "samba-dancing", "hip-hop-dancing", "ual1-dance-loop" })
    local dancers = S.cast_list(S.groups("dancers"), FLOOR_DANCERS, random)
    for i = 1, FLOOR_DANCERS do
        local angle = i / FLOOR_DANCERS * S.TAU
        local radius = 4 + (i % 3) * 2.2
        local x, z = math.sin(angle) * radius, math.cos(angle) * radius
        S.spawn(dancers[i], x, z, {
            yaw = S.yaw_to(x, z),
            clips = dances,
            clip = dances[i % #dances + 1],
            random = random,
        })
    end
end

function part.update(t)
    local front = (t * WAVE_SPEED) % S.TAU
    for _, actor in ipairs(part.crowd or {}) do
        local behind = (front - actor.angle) % S.TAU
        local up = t > 4 and behind < WAVE_WIDTH
        if up and not actor.waving then
            actor.waving = true
            S.play(actor, WAVE_CLIP, { fade = 0.2, start = 0.3 })
        elseif not up and actor.waving then
            actor.waving = false
            S.play(actor, actor.base, { fade = 0.45 })
        end
    end
end

function part.camera(t)
    local turn = 0.4 + t * 0.07
    local lift = S.smooth((t - 7) / 11)
    local radius = S.lerp(6, 64, lift)
    local height = S.lerp(1.7, 36, lift)
    local eye = { math.sin(turn) * -radius, height, math.cos(turn) * -radius }
    local target_radius = S.lerp(36, 0, lift)
    local target = { math.sin(turn) * target_radius, S.lerp(4.5, 2, lift), math.cos(turn) * target_radius }
    return eye, target, S.lerp(50, 52, lift)
end

return part
