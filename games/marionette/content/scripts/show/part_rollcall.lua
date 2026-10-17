-- Part 1: every character of the cast once, in a ring around the camera, each with an
-- idle or emote of its own; every few seconds the whole ring changes clip in a ripple
-- that follows the camera round. Names and triangle counts float over their heads.

local S = require("show/core")

local EMOTES = {
    "happy-idle", "waving", "thinking", "salute", "taunt", "laughing", "clapping",
    "thumbs-up", "pointing", "shrugging", "talking", "look-around", "cheering", "bored",
    "angry", "yelling", "victory", "quick-formal-bow", "fighting-idle", "spell-casting",
    "sword-and-shield-idle", "great-sword-idle", "rifle-idle", "pistol-idle",
    "breathing-idle", "drinking", "ual2-idle-fold-arms-loop", "ual2-idle-lantern-loop",
    "ual2-idle-talking-phone-loop", "ual1-idle-torch-loop", "ual1-spell-simple-idle-loop",
    "ual2-idle-shield-loop", "ual2-yes", "boxing", "kneeling-idle", "crouching-idle",
}
local CHANGE_EVERY = 6

local part = {
    name = "ROLL CALL",
    subtitle = "the whole cast, one clip each: Mixamo and Quaternius rigs share every clip",
    duration = 26,
}

local radius = 12
local emotes = {}

function part.enter()
    local random = S.rng(11)
    emotes = S.available(EMOTES)
    local cast = S.characters
    radius = math.max(9, #cast * 1.7 / S.TAU)
    for i, char in ipairs(cast) do
        local angle = (i - 1) / #cast * S.TAU
        local x, z = math.sin(angle) * radius, math.cos(angle) * radius
        local first = emotes[(i - 1) % #emotes + 1]
        local actor = S.spawn(char, x, z, {
            yaw = S.yaw_to(-x, -z),
            clip = first,
            random = random,
        })
        if actor then
            actor.angle = angle
            actor.slot = i
            actor.stage = 0
        end
    end
end

-- The camera turns once round the ring, from the side the first character stands on.
local function camera_angle(t)
    return -0.6 + t / part.duration * S.TAU * 0.92
end

function part.update(t)
    -- Each change sweeps round behind the camera's turn, so the switch is seen happening.
    local wave = math.floor(t / CHANGE_EVERY)
    if wave < 1 then
        return
    end
    local front = camera_angle(t)
    for _, actor in ipairs(S.actors) do
        if actor.stage < wave then
            local behind = (front - actor.angle) % S.TAU
            if behind < 0.9 then
                actor.stage = wave
                local clip = emotes[(actor.slot + wave * 7 - 1) % #emotes + 1]
                S.play(actor, clip, { fade = 0.4 })
            end
        end
    end
end

function part.camera(t)
    local angle = camera_angle(t)
    local intro = 1 - S.smooth(t / 5)
    local height = 1.6 + intro * 9
    local back = -2.5 - intro * 6
    local eye = { math.sin(angle) * back, height, math.cos(angle) * back }
    local target = { math.sin(angle) * radius, 1.1 + intro * -0.6, math.cos(angle) * radius }
    return eye, target, 42
end

function part.labels(ui, t)
    local angle = camera_angle(t)
    for _, actor in ipairs(S.actors) do
        local off = math.abs(((actor.angle - angle + math.pi) % S.TAU) - math.pi)
        if off < 0.32 then
            local fade = 1 - off / 0.32
            local tris = actor.char.triangles or 0
            local text = string.format("%s  %.1fk tris", actor.char.name, tris / 1000)
            -- Neighbours alternate heights so their names do not run into each other.
            local lift = (actor.slot % 2 == 0) and 0.55 or 0.25
            ui:world_label(actor.x * 0.97, (actor.char.height or 1.8) + lift, actor.z * 0.97, text, {
                color = { 1, 1, 1, fade },
                size = 15,
                backdrop = true,
            })
        end
    end
end

return part
