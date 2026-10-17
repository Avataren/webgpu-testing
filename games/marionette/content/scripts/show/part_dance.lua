-- Part 2: a dance floor in a grid. Every few seconds a new dance spreads from the middle
-- outward, each dancer cross-fading into it as the ripple reaches them.

local S = require("show/core")

local DANCES = { "samba-dancing", "hip-hop-dancing", "ual1-dance-loop", "boxing" }
local RIPPLE_EVERY = 6
local RIPPLE_SPEED = 7 -- metres per second
local SPACING = 1.45

local part = {
    name = "DANCE FLOOR",
    subtitle = "cross-faded clips rippling through the crowd",
    duration = 26,
    base = 196,
}

local dances = {}

function part.enter()
    local random = S.rng(23)
    dances = S.available(DANCES)
    local n = S.count(part.base)
    local side = math.ceil(math.sqrt(n))
    local pool = S.groups("dancers", "people")
    local cast = S.cast_list(pool, n, random)
    local half = (side - 1) * SPACING / 2
    for i = 1, n do
        local row, col = (i - 1) // side, (i - 1) % side
        local x = col * SPACING - half + random(-0.2, 0.2)
        local z = row * SPACING - half + random(-0.2, 0.2)
        local actor = S.spawn(cast[i], x, z, {
            yaw = random(-0.35, 0.35),
            clips = dances,
            clip = dances[1],
            random = random,
            speed = random(0.94, 1.06),
        })
        if actor then
            actor.distance = math.sqrt(x * x + z * z)
            actor.dance = 1
        end
    end
end

function part.update(t)
    local wave = math.floor(t / RIPPLE_EVERY)
    if wave < 1 then
        return
    end
    local front = (t - wave * RIPPLE_EVERY) * RIPPLE_SPEED
    local dance = wave % #dances + 1
    for _, actor in ipairs(S.actors) do
        if actor.dance ~= dance and actor.distance <= front then
            actor.dance = dance
            S.play(actor, dances[dance], { fade = 0.5, start = 0 })
        end
    end
end

function part.camera(t)
    local angle = 0.7 + t * 0.16
    local radius = 17 - 4 * math.sin(t * 0.21)
    local height = 3.2 + 3.5 * (0.5 - 0.5 * math.cos(t * 0.3))
    return { math.sin(angle) * radius, height, math.cos(angle) * radius }, { 0, 0.9, 0 }, 46
end

return part
