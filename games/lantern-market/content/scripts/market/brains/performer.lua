-- The performer dances on the stage (the show is on: `global.show_on`, which townsfolk
-- and children weigh when choosing what to do, and a `show` noise heard across the
-- square), bows, then takes a break for a drink at the fountain and comes back.

local C = require("market/core")
local P = require("market/places")

local Pf = {}

local function show_starts(ctx)
    ctx.global.show_on = true
    ctx.global.shows = (ctx.global.shows or 0) + 1
    C.tell("The show begins on the stage")
end

local function show_ends(ctx)
    ctx.global.show_on = false
end

local function claim_fountain(ctx)
    local here = get_world_translation(ctx.agent)
    local spot = P.claim("fountain", ctx.agent, here and here.x, here and here.z, C.random)
    ctx.bb.spot = spot
    return spot ~= nil
end

local function face_spot(ctx)
    local spot = ctx.bb.spot
    if spot and spot.yaw then
        nav_face(ctx.agent, spot.yaw)
    end
    return true
end

local function release(ctx)
    P.release(ctx.agent)
    ctx.bb.spot = nil
    return true
end

function Pf.define()
    ai_machine("performer", {
        initial = "perform",
        states = {
            perform = {
                exit = show_ends,
                tree = { "sequence",
                    { "move_to", "stage", give_up = 40, name = "to the stage" },
                    { "face", yaw = "stage_yaw", wait = 0.6 },
                    { "lua", show_starts, name = "curtain up" },
                    { "noise", "show", radius = 30 },
                    { "shuffle_sequence",
                        { "play", "samba-dancing", time = { 9, 13 } },
                        { "play", "hip-hop-dancing", time = { 9, 13 } },
                        { "play", "cheering", time = 3 } },
                    { "play", "quick-formal-bow" } },
                transitions = { { "rest", on = "done" } },
            },
            rest = {
                exit = release,
                tree = { "sequence",
                    { "lua", claim_fountain, name = "a drink" },
                    { "move_to", "spot", give_up = 40 },
                    { "lua", face_spot },
                    { "play", "drinking", time = { 6, 9 } },
                    { "lua", release } },
                transitions = { { "perform", on = "done" } },
            },
        },
    })
end

function Pf.blackboard(name)
    local s = P.PERFORMER
    return { name = name, stage = { x = s.x, y = s.y, z = s.z }, stage_yaw = s.yaw }
end

return Pf
