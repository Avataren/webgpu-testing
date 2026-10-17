-- Part 6: the cast takes a bow in front of the full stadium, and the camera rises into
-- the night as the title comes up.

local S = require("show/core")

local HEROES = 11

local part = {
    name = "CURTAIN CALL",
    subtitle = "",
    duration = 14,
}

local heroes = {}

function part.enter()
    local random = S.rng(97)
    -- The crowd from the stadium claps them on.
    for _, actor in ipairs(S.actors) do
        if actor.waving ~= nil or actor.base then
            actor.waving = nil
            S.play(actor, (random() < 0.5) and "clapping" or "cheering", { fade = 0.6, start = random(0, 1) })
        end
    end
    heroes = {}
    local cast = S.cast_list(S.groups("people", "good", "evil"), HEROES, random)
    for i = 1, HEROES do
        local x = (i - (HEROES + 1) / 2) * 1.3
        local actor = S.spawn(cast[i], x, 3 + math.abs(x) * 0.12, {
            yaw = 0,
            clips = S.available({ "quick-formal-bow", "victory", "salute" }),
            clip = "salute",
            start = 0,
        })
        if actor then
            actor.bow = 1.8 + math.abs(x) * 0.08
            table.insert(heroes, actor)
        end
    end
end

function part.update(t)
    for _, actor in ipairs(heroes) do
        if actor.bow and t >= actor.bow then
            actor.bow = nil
            actor.cheer = t + 2.6
            S.play(actor, "quick-formal-bow", { fade = 0.3, start = 0, loop = false })
        elseif actor.cheer and t >= actor.cheer then
            actor.cheer = nil
            S.play(actor, "victory", { fade = 0.4, start = 0 })
        end
    end
end

function part.camera(t)
    local k = S.smooth((t - 4.5) / 9.5)
    local eye = { 0, S.lerp(1.5, 48, k), S.lerp(11.5, 70, k) }
    local target = { 0, S.lerp(1.1, 0, k), S.lerp(3, -6, k) }
    return eye, target, S.lerp(40, 50, k)
end

return part
