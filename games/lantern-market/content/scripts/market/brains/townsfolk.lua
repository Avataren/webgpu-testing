-- Townsfolk: a state machine around a needs-driven life.
--
-- In `life`, a utility node picks what to do from how hungry, thirsty, tired, curious,
-- sociable and keen to shop each person is (needs grow while they go about, and the
-- activity that answers one brings it down): a meal at a stall, a drink at the fountain,
-- a sit on a bench, the shops, the sights, the show when it is on, a chat, a stroll.
-- Scores are re-weighed every few seconds, so a show starting can pull people away.
--
-- Around that: chatting with someone they invited (or who invited them), being tempted
-- by a vendor's call, gathering where they are called (a click), turning to an alarm or
-- the portcullis, stepping out of a cart's way, and being robbed.

local C = require("market/core")
local P = require("market/places")

local T = {}

T.NEEDS = { "hunger", "thirst", "rest", "shopping", "curiosity", "social" }

-- What each kind of place does for a person.
local SATISFY = {
    stall = { hunger = -0.75, shopping = -0.3 },
    fountain = { thirst = -0.85 },
    bench = { rest = -0.85 },
    shop = { shopping = -0.7, curiosity = -0.15 },
    terrace = { curiosity = -0.55 },
    bridge = { curiosity = -0.45 },
    towpath = { curiosity = -0.3, rest = 0.05 },
    bank = { curiosity = -0.45 },
    audience = { curiosity = -0.55, social = -0.25 },
}

-- A person's starting blackboard: needs part-way up, how fast each grows, a temperament.
function T.blackboard(random, name)
    local bb = { name = name, sociable = random(0.4, 1.3) }
    for _, need in ipairs(T.NEEDS) do
        bb[need] = random(0.05, 0.6)
        bb["rate_" .. need] = random(0.004, 0.009)
    end
    bb.rate_social = bb.rate_social * bb.sociable
    return bb
end

-- Needs grow while people go about their day (every state's `update`).
local function grow(ctx)
    local bb = ctx.bb
    for _, need in ipairs(T.NEEDS) do
        local value = bb[need] or 0
        bb[need] = C.clamp01(value + (bb["rate_" .. need] or 0.005) * ctx.dt)
    end
end

-- Choose a place for the clip and how long to stay.
local function settle(ctx, spot)
    ctx.bb.spot = spot
    ctx.bb.doing = spot.kind
    local actions = spot.actions and C.available(spot.actions) or {}
    ctx.bb.clip = #actions > 0 and C.pick(actions) or nil
    local linger = spot.linger or { 3, 6 }
    ctx.bb.linger = C.random(linger[1], linger[2])
end

-- Claim a free spot of one of `kinds` (near where they are).
local function claim(kinds)
    return function(ctx)
        local here = get_world_translation(ctx.agent)
        local kind = kinds[C.random(#kinds)]
        local spot = P.claim(kind, ctx.agent, here and here.x, here and here.z, C.random)
        if not spot then
            return false
        end
        settle(ctx, spot)
        return true
    end
end

-- At the spot: face its way and tell the stall's vendor there is a customer.
local function arrive(ctx)
    local spot = ctx.bb.spot
    if not spot then
        return false
    end
    if spot.yaw then
        nav_face(ctx.agent, spot.yaw)
    end
    local vendor = spot.stall and C.vendors[spot.stall]
    if vendor then
        ai_send(vendor, "customer", nil, ctx.agent)
    end
    return true
end

-- Done: the need it answered goes down, the spot is free again.
local function satisfy(ctx)
    local spot = ctx.bb.spot
    local effect = spot and SATISFY[spot.kind] or {}
    for need, delta in pairs(effect) do
        ctx.bb[need] = C.clamp01((ctx.bb[need] or 0) + delta)
    end
    P.release(ctx.agent)
    ctx.bb.spot = nil
    ctx.bb.doing = nil
    return true
end

local function leave_spot(ctx)
    P.release(ctx.agent)
    ctx.bb.spot = nil
    ctx.bb.doing = nil
end

-- Go somewhere, stay a while (playing the place's clip), feel better.
local function visit(name, kinds, score)
    return { "sequence", name = name, score = score,
        { "lua", claim(kinds), name = "find a place" },
        { "move_to", "spot", give_up = 60, accept_partial = 1.5, name = "walk there" },
        { "lua", arrive, name = "arrive" },
        { "wait", 0.4 },
        { "selector",
            { "play", { expr = "clip" }, time = "linger", name = "linger", when = "exists(clip)" },
            { "wait", "linger", name = "linger" } },
        { "lua", satisfy, name = "feel better" },
    }
end

local function has_tag(tags, tag)
    for _, t in ipairs(tags or {}) do
        if t == tag then
            return true
        end
    end
    return false
end

-- Someone to talk to: a person in view, close, free and in the mood. Both walk to
-- either side of the point between them.
local function find_partner(ctx)
    local senses = ai_senses(ctx.agent)
    local here = get_world_translation(ctx.agent)
    if not senses or not here then
        return false
    end
    local best, best_d
    for _, p in ipairs(senses.percepts or {}) do
        if p.visible and has_tag(p.tags, "townsfolk") then
            local other = p.entity
            local d = math.sqrt(C.dist2(here.x, here.z, p.position[1], p.position[3]))
            if d < 9 and (not best or d < best_d)
                and ai_state(other) == "life"
                and (ai_get(other, "social") or 0) > 0.25
                and not ai_get(other, "partner") then
                best, best_d = other, d
            end
        end
    end
    if not best then
        return false
    end
    local there = get_world_translation(best)
    if not there then
        return false
    end
    local dx, dz = there.x - here.x, there.z - here.z
    local len = math.max(0.01, math.sqrt(dx * dx + dz * dz))
    local mx, mz = (here.x + there.x) / 2, (here.z + there.z) / 2
    local mine = nav_closest_point(mx - dx / len * 0.6, here.y, mz - dz / len * 0.6, 1.5)
    local theirs = nav_closest_point(mx + dx / len * 0.6, there.y, mz + dz / len * 0.6, 1.5)
    if not mine or not theirs then
        return false
    end
    leave_spot(ctx)
    ctx.bb.partner = best
    ctx.bb.meet = mine
    ai_send(best, "chat_invite", { at = theirs, with = ctx.agent }, ctx.agent)
    return true
end

local function begin_chat(ctx)
    local invite = ctx.bb.invite
    if invite then
        leave_spot(ctx)
        ctx.bb.partner = invite.with
        ctx.bb.meet = invite.at
        ctx.bb.invite = nil
    end
    ctx.bb.doing = "chat"
end

local function end_chat(ctx)
    ctx.bb.social = C.clamp01((ctx.bb.social or 0) - 0.7)
    ctx.bb.partner = nil
    ctx.bb.meet = nil
    ctx.bb.doing = nil
end

-- A vendor's call: a spot at that stall.
local function claim_offered(ctx)
    local stall = ctx.bb.offer
    local spot = stall and P.claim_at_stall(stall, ctx.agent)
    ctx.bb.offer = nil
    if not spot then
        return false
    end
    settle(ctx, spot)
    return true
end

local function robbed(ctx)
    leave_spot(ctx)
    ctx.bb.shopping = C.clamp01((ctx.bb.shopping or 0) + 0.3)
    ctx.bb.robbed = (ctx.bb.robbed or 0) + 1
    C.tell(C.name_of(ctx.agent) .. " has had a purse lifted!")
end

local function dodged(ctx)
    ctx.bb.dodged = ctx.time
end

local LIFE = { "utility", interval = 3, hysteresis = 0.3,
    visit("eat at a stall", { "stall" }, "hunger * 1.1"),
    visit("drink at the fountain", { "fountain" }, "thirst"),
    visit("sit on a bench", { "bench" }, "rest * 0.9"),
    visit("browse the shops", { "shop" }, "shopping * 0.85"),
    visit("see the sights", { "terrace", "bridge", "towpath", "bank" }, "curiosity * 0.8"),
    visit("watch the show", { "audience" }, "global.show_on and curiosity * 1.2 + 0.25 or -1"),
    { "lua", find_partner, name = "look for company", score = "social * sociable", cooldown = 6 },
    { "sequence", name = "stroll", score = 0.12,
        { "wander", 10, min_distance = 4 },
        { "play", "look-around", time = { 2, 4 } } },
}

local CHAT = { "sequence",
    { "move_to", "meet", radius = 0.5, give_up = 25, name = "go to meet them" },
    { "wait_until", "dist(partner) < 2.6", timeout = 20, name = "wait for them" },
    { "face", "partner", wait = 0.4 },
    { "repeat", count = 2,
        { "sequence",
            { "play", "talking", time = { 3, 5 } },
            { "random",
                { "play", "laughing", time = { 2, 3 } },
                { "play", "happy-idle", time = 2 },
                { "play", "shrugging" } } } },
    { "send", "chat_over", to = "partner" },
}

local TEMPTED = { "sequence",
    { "lua", claim_offered, name = "answer the call" },
    { "move_to", "spot", give_up = 40 },
    { "lua", arrive, name = "arrive" },
    { "play", { expr = "clip" }, time = "linger", name = "haggle" },
    { "lua", satisfy, name = "buy something" },
}

local GATHER = { "sequence",
    { "pick_point", "gather_at", 2.2, around = "summon_at" },
    { "move_to", "gather_at", run = true, give_up = 40 },
    { "face", "summon_at", wait = 0.4 },
    { "random",
        { "play", "cheering", time = { 4, 8 } },
        { "play", "clapping", time = { 4, 8 } },
        { "play", "waving", time = { 4, 8 } } },
}

local STARTLED = { "sequence",
    { "face", "alarm_at", wait = 0.5 },
    { "random",
        { "play", "pointing", time = { 1.5, 2.5 } },
        { "play", "look-around", time = { 2, 3 } } },
}

local ROBBED = { "sequence",
    { "play", "shrugging", name = "where is my purse?" },
    { "noise", "alarm", radius = 16, data = { expr = "thief" }, name = "cry thief!" },
    { "face", "thief", wait = 0.3 },
    { "play", "pointing", time = 3 },
}

local DODGE = { "flee", "nearest('cart')", 3, run = false, speed = 2.0, name = "out of the cart's way" }

-- States that are interrupted by nothing but a theft.
local BUSY = { "robbed", "gather" }

function T.define()
    ai_machine("townsfolk", {
        initial = "life",
        states = {
            life = {
                update = grow,
                tree = LIFE,
                transitions = {
                    { "chat", when = "exists(partner)" },
                    { "tempted", event = "hear:offer", when = "shopping + hunger > 0.7", chance = 0.5, store = "offer" },
                },
            },
            chat = {
                update = grow,
                enter = begin_chat,
                exit = end_chat,
                tree = CHAT,
                transitions = { { "life", on = "done" }, { "life", event = "chat_over" } },
            },
            tempted = { update = grow, exit = leave_spot, tree = TEMPTED, transitions = { { "life", on = "done" } } },
            gather = { update = grow, exit = leave_spot, tree = GATHER, transitions = { { "life", on = "done" } } },
            startled = { update = grow, exit = leave_spot, tree = STARTLED, transitions = { { "life", on = "done" } } },
            robbed = { enter = robbed, tree = ROBBED, transitions = { { "life", on = "done" } } },
            dodge = { enter = dodged, exit = leave_spot, tree = DODGE, transitions = { { "life", on = "done" } } },
        },
        transitions = {
            { "robbed", event = "robbed", store_from = "thief", except = { "robbed" } },
            { "gather", event = "summon", store = "summon_at", except = { "robbed" } },
            { "chat", event = "chat_invite", store = "invite", when = "social > 0.2 and not exists(partner)", except = { "chat", "robbed", "gather" } },
            { "startled", event = "hear:alarm", store_position = "alarm_at", chance = 0.8, except = { "startled", "robbed", "gather" } },
            { "startled", event = "hear:portcullis", store_position = "alarm_at", chance = 0.4, except = { "startled", "robbed", "gather", "chat" } },
            { "dodge", when = "count('cart', 2.8) > 0 and time() - (dodged or -9) > 5", except = { "dodge", "robbed", "gather", "chat" } },
        },
    })
    T.BUSY = BUSY
end

return T
