-- Part 4: two armies. They taunt, charge row by row, and the front rows fight in pairs:
-- attacks answered by blocks, dodges and hit reactions; some fall and get up again
-- while the ranks behind them cheer their side on.

local S = require("show/core")

local SIDES = {
    {
        groups = { "good" },
        sign = -1,
        idle = { "sword-and-shield-idle", "fighting-idle", "great-sword-idle", "ual2-idle-shield-loop" },
        run = "sword-and-shield-run",
        attacks = { "sword-and-shield-slash", "great-sword-slash", "stabbing", "ual2-sword-regular-combo", "roundhouse-kick" },
    },
    {
        groups = { "evil" },
        sign = 1,
        idle = { "ual2-zombie-idle-loop", "yelling", "taunt", "angry" },
        run = "running",
        attacks = { "ual2-zombie-scratch", "hook-punch", "cross-punch", "mma-kick", "ual2-melee-hook", "punching" },
    },
}
local REACTIONS = { "sword-and-shield-block", "body-block", "dodging", "hit-reaction", "head-hit", "stomach-hit" }
local DEATHS = { "dying", "falling-back-death" }
local RALLY = { "fighting-idle", "yelling", "cheering", "taunt" }
local WIDTH = 15 -- soldiers per row
local SPACING = 1.6
local CHARGE_AT = 5

local part = {
    name = "BATTLE",
    subtitle = "two armies, a clip for every blow and every fall",
    duration = 28,
    base = 300,
}

local random
local pairs_front = {}

local function clips_of(side)
    local list = { side.run }
    for _, set in ipairs({ side.idle, side.attacks, REACTIONS, DEATHS, RALLY, { "getting-up" } }) do
        for _, clip in ipairs(set) do
            table.insert(list, clip)
        end
    end
    return S.available(list)
end

function part.enter()
    random = S.rng(53)
    pairs_front = {}
    local per_side = math.max(1, S.count(part.base) // 2)
    for side_index, side in ipairs(SIDES) do
        side.idle_clips = S.available(side.idle)
        side.attack_clips = S.available(side.attacks)
        local clips = clips_of(side)
        local cast = S.cast_list(S.groups(table.unpack(side.groups)), per_side, random)
        for i = 1, per_side do
            local row, col = (i - 1) // WIDTH, (i - 1) % WIDTH
            local z = (col - (WIDTH - 1) / 2) * SPACING + random(-0.15, 0.15)
            local x = side.sign * (14 + row * SPACING) + random(-0.2, 0.2)
            local actor = S.spawn(cast[i], x, z, {
                yaw = S.yaw_to(-side.sign, 0),
                clips = clips,
                clip = S.pick(side.idle_clips, random),
                random = random,
            })
            if actor then
                actor.side = side
                actor.row, actor.col = row, col
                actor.target = side.sign * (0.75 + row * 1.45)
                actor.start = CHARGE_AT + row * 0.35 + random(0, 0.3)
                actor.state = "waiting"
                actor.run = S.ground_speed(side.run, actor.char)
                if row == 0 then
                    pairs_front[side_index * 1000 + col] = actor
                end
            end
        end
    end
    -- Front-row soldiers face the one across from them.
    for col = 0, WIDTH - 1 do
        local a, b = pairs_front[1000 + col], pairs_front[2000 + col]
        if a and b then
            a.foe, b.foe = b, a
            a.next_blow = 0
            b.next_blow = 0
        end
    end
end

local function arrive(actor, t)
    actor.state = "arrived"
    if actor.foe then
        S.play(actor, "fighting-idle", { fade = 0.25 })
        actor.next_blow = t + random(0.2, 1.4)
    else
        S.play(actor, S.pick(S.available(RALLY), random), { fade = 0.3, start = random(0, 1) })
    end
end

local function fight(actor, t, dt)
    local foe = actor.foe
    if not foe or actor.dead or foe.state ~= "arrived" or foe.dead then
        return
    end
    if actor.pending_reaction and t >= actor.pending_reaction then
        actor.pending_reaction = nil
        S.play(actor, S.pick(S.available(REACTIONS), random), { fade = 0.15, loop = false, start = 0 })
        actor.recover = t + 1.2
    end
    if actor.recover and t >= actor.recover then
        actor.recover = nil
        S.play(actor, "fighting-idle", { fade = 0.35 })
    end
    if t >= actor.next_blow and not foe.pending_reaction and not actor.recover then
        S.play(actor, S.pick(actor.side.attack_clips, random), { fade = 0.15, loop = false, start = 0 })
        actor.recover = t + 1.5
        foe.pending_reaction = t + 0.35
        actor.next_blow = t + random(2.4, 3.6)
        foe.next_blow = math.max(foe.next_blow, t + 1.6)
    end
    -- Some fall, and get up again before the end.
    if not actor.fallen and t > 17 and random() < 0.25 * dt then
        actor.fallen, actor.dead = true, true
        actor.pending_reaction, actor.recover = nil, nil
        S.play(actor, S.pick(S.available(DEATHS), random), { fade = 0.2, loop = false, start = 0 })
        actor.rise = t + random(3.5, 5.5)
    end
    if actor.rise and t >= actor.rise then
        actor.rise = nil
        S.play(actor, "getting-up", { fade = 0.3, loop = false, start = 3.0 })
        actor.up = t + 4.5
    end
    if actor.up and t >= actor.up then
        actor.up, actor.dead = nil, false
        S.play(actor, "fighting-idle", { fade = 0.4 })
    end
end

function part.update(t, dt)
    for _, actor in ipairs(S.actors) do
        if actor.state == "waiting" and t >= actor.start then
            actor.state = "charging"
            S.play(actor, actor.side.run, { fade = 0.25, start = random(0, 0.5) })
        elseif actor.state == "charging" then
            local step = actor.run * dt
            local remaining = actor.target - actor.x
            if math.abs(remaining) <= step then
                S.place(actor, actor.target, 0, actor.z)
                arrive(actor, t)
            else
                S.place(actor, actor.x + step * (remaining > 0 and 1 or -1), 0, actor.z)
            end
        elseif actor.state == "arrived" then
            fight(actor, t, dt)
        end
    end
end

function part.camera(t)
    if t < 6.5 then
        -- Down the front of the knights' line, low, while they wait.
        local k = t / 6.5
        return { -8.5, 1.5, S.lerp(-13, 9, k) }, { -15, 1.3, S.lerp(-9, 13, k) }, 40
    end
    local k = S.smooth((t - 6.5) / 6)
    local angle = S.lerp(-1.9, -0.6, k) + math.max(0, t - 12.5) * 0.09
    local radius = S.lerp(30, 14, k)
    local height = S.lerp(11, 3.2, k)
    return { math.sin(angle) * radius, height, math.cos(angle) * radius }, { 0, 1.0, 0 }, 44
end

return part
