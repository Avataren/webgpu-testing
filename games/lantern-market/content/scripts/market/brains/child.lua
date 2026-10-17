-- Children run about the square, climb the stage and jump off its east edge (the
-- NavLinks), and watch the show. Now and then a game of tag starts: whoever is "it"
-- (`group.it` on the kids' shared blackboard) counts, then chases the nearest child they
-- can see; a tag passes it on (a `tagged` event), and the others run from it when it
-- comes close.

local C = require("market/core")
local P = require("market/places")

local K = {}

-- A tag game lasts this many tags at most.
local TAGS_PER_GAME = 8

local function on_stage(ctx)
    local s = P.STAGE[C.random(#P.STAGE)]
    ctx.bb.spot = { x = s[1], y = s[2], z = s[3] }
    ctx.bb.clip = C.pick(C.available({ "waving", "cheering" }))
    return true
end

local function counted(ctx)
    C.tell(C.name_of(ctx.agent) .. " is it!")
end

-- Tag the one caught: they are it now.
local function tag(ctx)
    local prey = ctx.bb.prey
    if not prey then
        return false
    end
    local tags = (ctx.group.tags or 0) + 1
    ctx.group.tags = tags
    ctx.group.last_it = ctx.agent
    ai_send(prey, "tagged", nil, ctx.agent)
    if tags >= TAGS_PER_GAME then
        ctx.group.it = nil
        C.tell("The game of tag is over")
    else
        ctx.group.it = prey
    end
    return true
end

-- Nobody to catch: the game peters out.
local function give_up(ctx)
    if ctx.group.it == ctx.agent then
        ctx.group.it = nil
    end
end

local PLAY = { "utility", interval = 4, hysteresis = 0.2,
    { "sequence", name = "climb the stage", score = "random(0.2, 0.6)",
        { "lua", on_stage, name = "pick a place on the stage" },
        { "move_to", "spot", run = true, give_up = 30 },
        { "play", { expr = "clip" }, time = { 0.5, 1.5 } },
        -- The way down to the square is the jump off the front.
        { "wander", 6, center = "playground", min_distance = 3, name = "jump down" } },
    { "sequence", name = "run about", score = "random(0.2, 0.5)",
        { "wander", 9, center = "playground", min_distance = 4 },
        { "random",
            { "play", "cheering", time = { 1, 2 } },
            { "play", "laughing", time = { 1, 2.5 } },
            { "play", "waving", time = { 1, 2 } },
            { "play", "happy-idle", time = { 1, 3 } } } },
    { "sequence", name = "watch the show", score = "global.show_on and 0.75 or -1",
        { "pick_point", "seat", 2.0, around = "front_row" },
        { "move_to", "seat", run = true, give_up = 30 },
        { "face", "stage", wait = 0.3 },
        { "play", "clapping", time = { 4, 7 } } },
}

function K.define()
    ai_machine("child", {
        initial = "play",
        states = {
            play = {
                tree = PLAY,
                transitions = {
                    { "it", when = "group.it == self" },
                    { "run_away", when = "exists(group.it) and dist(group.it) < 6" },
                },
            },
            it = {
                enter = counted,
                exit = give_up,
                tree = { "sequence",
                    { "play", "cheering", time = 1.5, name = "count to three" },
                    { "retry", count = 3,
                        { "sequence",
                            { "find", "child", store = "prey", filter = "it ~= group.last_it or count('child') < 2" },
                            { "move_to", "prey", run = true, radius = 0.9, repath = 0.25, give_up = 12, name = "chase" } } },
                    { "lua", tag, name = "tag!" } },
                transitions = {
                    { "play", when = "group.it ~= self" },
                    { "play", on = "done" },
                },
            },
            run_away = {
                tree = { "flee", "group.it", 7, name = "run from it" },
                transitions = {
                    { "it", when = "group.it == self" },
                    { "play", on = "done" },
                },
            },
            tagged = {
                tree = { "play", "laughing", time = 1.2 },
                transitions = {
                    { "it", when = "group.it == self", on = "done" },
                    { "play", on = "done" },
                },
            },
        },
        transitions = {
            { "tagged", event = "tagged", except = { "tagged" } },
        },
    })
end

function K.blackboard(name)
    return {
        name = name,
        playground = { x = P.PLAYGROUND[1], y = P.PLAYGROUND[2], z = P.PLAYGROUND[3] },
        front_row = { x = -8.5, y = 0, z = 2.0 },
        stage = { x = P.PERFORMER.x, y = P.PERFORMER.y, z = P.PERFORMER.z },
    }
end

-- Start a game of tag with a random child as it (the director does this now and then).
function K.start_tag(kids)
    if #kids == 0 then
        return nil
    end
    local board = ai_group_board("kids")
    if board.it then
        return nil
    end
    local child = kids[C.random(#kids)]
    board.tags = 0
    board.last_it = nil
    board.it = child
    return child
end

return K
