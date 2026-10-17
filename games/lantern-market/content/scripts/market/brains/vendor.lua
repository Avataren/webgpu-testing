-- Vendors stand behind their counters (no NavAgent: they never walk). They idle, call
-- out to passers-by now and then (a wave, then an `offer` noise: townsfolk who hear it and
-- fancy a purchase come over), and serve whoever comes to their stall (a `customer`
-- event a townsperson sends on arriving).

local C = require("market/core")

local V = {}

local function hawked(ctx)
    ctx.bb.hawked = ctx.time
end

local function served(ctx)
    ctx.bb.served = (ctx.bb.served or 0) + 1
end

function V.define()
    ai_machine("vendor", {
        initial = "idle",
        states = {
            idle = {
                tree = { "random",
                    { "play", "bored", time = { 5, 9 } },
                    { "play", "breathing-idle", time = { 5, 9 } },
                    { "play", "look-around", time = { 4, 6 } },
                    { "play", "happy-idle", time = { 3, 5 } } },
                transitions = {
                    { "hawk", when = "count('townsfolk', 6) > 1 and time() - (hawked or -99) > 14", chance = 0.08 },
                },
            },
            hawk = {
                enter = hawked,
                tree = { "sequence",
                    { "face", "nearest('townsfolk')", wait = 0.3, name = "catch an eye" },
                    { "play", "waving", time = 1.6 },
                    { "noise", "offer", radius = 7, data = { expr = "stall" }, name = "call out" },
                    { "play", "pointing", time = 1.4 },
                    { "face", yaw = "post_yaw" } },
                transitions = { { "idle", on = "done" } },
            },
            serve = {
                enter = served,
                tree = { "sequence",
                    { "face", "customer", wait = 0.3 },
                    { "play", "talking", time = { 3, 5 } },
                    { "random",
                        { "play", "pointing", time = 1.5 },
                        { "play", "happy-idle", time = 1.5 },
                        { "play", "thinking", time = 2 } },
                    { "face", yaw = "post_yaw" } },
                transitions = { { "idle", on = "done" } },
            },
        },
        transitions = {
            { "serve", event = "customer", store_from = "customer", except = { "serve" } },
        },
    })
end

-- The vendor of `stall` stands at `post`.
function V.blackboard(stall, post, name)
    return { stall = stall, post_yaw = post.yaw, name = name }
end

V.C = C
return V
