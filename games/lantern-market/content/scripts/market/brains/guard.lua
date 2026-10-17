-- Guards patrol both floors and both bridges, saluting each other when they pass. A cry
-- of "thief!" (an `alarm` noise) sends them to look; one who sees the wanted thief gives
-- chase and calls the rest of the watch (a group event); losing sight of him, they search
-- where he was last seen; catching him, they arrest him.

local C = require("market/core")

local G = {}

local function saluted(ctx)
    ctx.bb.saluted = ctx.time
end

local function give_chase(ctx)
    if not ctx.global.chase_by then
        ctx.global.chase_by = ctx.agent
        C.tell(C.name_of(ctx.agent) .. " has spotted the thief!")
    end
end

local function stop_chase(ctx)
    if ctx.global.chase_by == ctx.agent then
        ctx.global.chase_by = nil
    end
end

local function arrest(ctx)
    local thief = ctx.bb.target
    if thief and ctx.global.thief_wanted then
        ai_send(thief, "caught", nil, ctx.agent)
        ctx.global.thief_wanted = false
        ctx.global.arrests = (ctx.global.arrests or 0) + 1
        C.tell(C.name_of(ctx.agent) .. " caught the thief!")
    end
end

local WANTED_IN_VIEW = "global.thief_wanted and sees('thief')"

function G.define()
    ai_machine("guard", {
        initial = "patrol",
        states = {
            patrol = {
                tree = { "sequence",
                    { "patrol", "route", radius = 0.8, name = "walk the beat" },
                    { "selector",
                        { "play", "look-around", time = { 3, 5 }, chance = 0.35 },
                        { "succeed" } } },
                transitions = {
                    { "chase", when = WANTED_IN_VIEW },
                    { "investigate", event = "hear:alarm", store_position = "lead" },
                    { "investigate", event = "thief_spotted", store_position = "lead" },
                    { "salute", event = "see:guard", store_from = "buddy",
                        when = "time() - (saluted or -99) > 40 and dist(nearest('guard')) < 7" },
                },
            },
            salute = {
                enter = saluted,
                tree = { "sequence", { "face", "buddy", wait = 0.4 }, { "play", "salute" } },
                transitions = { { "chase", when = WANTED_IN_VIEW }, { "patrol", on = "done" } },
            },
            investigate = {
                tree = { "sequence",
                    { "move_to", "lead", run = true, radius = 2.0, give_up = 30, name = "run to the trouble" },
                    { "play", "look-around", time = { 3, 5 } } },
                transitions = {
                    { "chase", when = WANTED_IN_VIEW },
                    { "patrol", on = "done" },
                },
            },
            chase = {
                enter = give_chase,
                exit = stop_chase,
                tree = { "sequence",
                    { "set", "target", expr = "nearest('thief') or target" },
                    { "send", "thief_spotted", to = "group" },
                    { "move_to", "target", run = true, radius = 1.3, repath = 0.3, give_up = 45, name = "chase the thief" } },
                transitions = {
                    { "patrol", when = "not global.thief_wanted" },
                    { "arrest", on = "success" },
                    { "search", event = "lose:thief", store_position = "lead" },
                    { "search", on = "failure" },
                },
            },
            search = {
                tree = { "sequence",
                    { "move_to", "lead", run = true, radius = 1.5, give_up = 20, name = "where did he go?" },
                    { "play", "look-around", time = 4 } },
                transitions = {
                    { "chase", when = WANTED_IN_VIEW },
                    { "patrol", when = "not global.thief_wanted" },
                    { "patrol", on = "done" },
                },
            },
            arrest = {
                enter = arrest,
                tree = { "sequence",
                    { "face", "target", wait = 0.4 },
                    { "play", "pointing", time = 2 },
                    { "play", "salute" } },
                transitions = { { "patrol", on = "done" } },
            },
        },
    })
end

return G
