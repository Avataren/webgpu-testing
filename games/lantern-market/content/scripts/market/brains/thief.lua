-- The pickpocket: prowls like any shopper; when no guard is near, stalks someone standing
-- at a stall or a shop, lifts their purse and runs: away from the victim, then to a
-- hideout (under the bridges, the ends of the arcade), dodging any guard who comes close.
-- He lies low until the hue and cry dies down. Caught, he shrugs and is walked off over
-- the canal.

local C = require("market/core")
local P = require("market/places")

local Th = {}

local function has_tag(tags, tag)
    for _, t in ipairs(tags or {}) do
        if t == tag then
            return true
        end
    end
    return false
end

-- Where the marks are: somewhere by the stalls, the shops or the fountain.
local function case_the_market(ctx)
    local kinds = { "stall", "stall", "shop", "fountain", "bench" }
    local list = P.by_kind[kinds[C.random(#kinds)]]
    local spot = list[C.random(#list)]
    ctx.bb.casing = { x = spot.x, y = spot.y, z = spot.z }
    return true
end

-- Places where people stand about with their purses within reach.
local STANDING = { stall = true, shop = true, fountain = true, bench = true, audience = true }

-- Someone in view standing about at a stall, a shop, the fountain, a bench or the show.
local function pick_victim(ctx)
    ctx.bb.victim = nil
    local senses = ai_senses(ctx.agent)
    local here = get_world_translation(ctx.agent)
    if not senses or not here then
        return
    end
    local best, best_d
    for _, p in ipairs(senses.percepts or {}) do
        if p.visible and has_tag(p.tags, "townsfolk") then
            local other = p.entity
            local doing = ai_get(other, "doing")
            local action = ai_action(other)
            if STANDING[doing] and (action == "linger" or action == "arrive") then
                local d = C.dist2(here.x, here.z, p.position[1], p.position[3])
                if not best or d < best_d then
                    best, best_d = other, d
                end
            end
        end
    end
    ctx.bb.victim = best
    return best ~= nil
end

local function rob(ctx)
    local victim = ctx.bb.victim
    if not victim then
        return false
    end
    ai_send(victim, "robbed", nil, ctx.agent)
    ctx.global.thief_wanted = true
    ctx.global.thefts = (ctx.global.thefts or 0) + 1
    ctx.bb.loot = (ctx.bb.loot or 0) + 1
    ctx.bb.last_job = ctx.time
    return true
end

-- The hideout farthest from the nearest guard he knows of (among the nearer ones).
local function choose_hideout(ctx)
    local here = get_world_translation(ctx.agent)
    if not here then
        return false
    end
    local guard
    local senses = ai_senses(ctx.agent)
    for _, p in ipairs(senses and senses.percepts or {}) do
        if has_tag(p.tags, "guard") then
            guard = p.position
        end
    end
    local best, best_score
    for _, h in ipairs(P.HIDEOUTS) do
        local d = math.sqrt(C.dist2(here.x, here.z, h[1], h[3]))
        local score = -d
        if guard then
            score = score + 1.5 * math.sqrt(C.dist2(guard[1], guard[3], h[1], h[3]))
        end
        if not best or score > best_score then
            best, best_score = h, score
        end
    end
    ctx.bb.hideout = { x = best[1], y = best[2], z = best[3] }
    return true
end

local function evaded(ctx)
    ctx.bb.evaded = ctx.time
end

local function lie_low_over(ctx)
    ctx.bb.last_job = ctx.time
    if ctx.global.thief_wanted then
        ctx.global.thief_wanted = false
        C.tell("The thief has given the watch the slip")
    end
end

local function caught(ctx)
    ctx.bb.caught = (ctx.bb.caught or 0) + 1
    ctx.bb.last_job = ctx.time
    ctx.global.thief_wanted = false
end

local GUARD_CLOSE = "sees('guard') and dist(nearest('guard')) < 8 and time() - (evaded or -9) > 4"

function Th.define()
    ai_machine("thief", {
        initial = "prowl",
        states = {
            prowl = {
                tree = { "sequence",
                    { "lua", case_the_market, name = "pick somewhere busy" },
                    { "wander", 4, center = "casing", name = "case the stalls" },
                    { "play", "look-around", time = { 2, 4 } } },
                transitions = {
                    { "stalk", after = { 20, 35 }, when = "not sees('guard')" },
                    { "stalk", event = "strike" },
                },
            },
            stalk = {
                tree = { "sequence",
                    -- A mark in view, or on to another busy spot to look again.
                    { "retry", count = 4,
                        { "selector",
                            { "lua", pick_victim, name = "a mark in view?" },
                            { "sequence",
                                { "lua", case_the_market, name = "somewhere busier" },
                                { "wander", 3, center = "casing", name = "look for a mark" },
                                { "fail" } } } },
                    { "move_to", "victim", radius = 1.0, speed = 1.2, give_up = 25, name = "sidle up" } },
                transitions = {
                    { "prowl", when = "count('guard', 9) > 0" },
                    { "steal", on = "success" },
                    { "prowl", on = "failure" },
                },
            },
            steal = {
                tree = { "sequence",
                    { "face", "victim", wait = 0.2 },
                    { "play", "look-around", time = 0.8, name = "nobody looking?" },
                    { "lua", rob, name = "lift the purse" } },
                transitions = { { "escape", on = "success" }, { "prowl", on = "failure" } },
            },
            escape = {
                tree = { "sequence",
                    { "flee", "victim", 8, name = "get away" },
                    { "lua", choose_hideout, name = "pick a hideout" },
                    { "move_to", "hideout", run = true, give_up = 50, name = "run to the hideout" } },
                transitions = {
                    { "evade", when = GUARD_CLOSE },
                    { "hide", on = "success" },
                    { "hide", on = "failure" },
                },
            },
            evade = {
                enter = evaded,
                tree = { "flee", "nearest('guard')", 10, name = "dodge the guard" },
                transitions = { { "escape", on = "done" } },
            },
            hide = {
                exit = lie_low_over,
                tree = { "play", "breathing-idle", time = { 18, 26 }, name = "lie low" },
                transitions = {
                    { "evade", when = GUARD_CLOSE },
                    { "prowl", on = "done" },
                },
            },
            caught = {
                enter = caught,
                tree = { "sequence",
                    { "face", "captor", wait = 0.4 },
                    { "play", "shrugging" },
                    { "play", "breathing-idle", time = 2 },
                    { "move_to", "release_at", give_up = 60, name = "walked off" } },
                transitions = { { "prowl", on = "done" } },
            },
        },
        transitions = {
            { "caught", event = "caught", store_from = "captor", except = { "caught" } },
        },
    })
end

function Th.blackboard(name)
    return {
        name = name,
        -- The open square (the fountain's basin itself, at the origin, is not walkable).
        home = { x = 0, y = 0, z = 6.5 },
        release_at = { x = P.RELEASE[1], y = P.RELEASE[2], z = P.RELEASE[3] },
        last_job = 0,
    }
end

return Th
