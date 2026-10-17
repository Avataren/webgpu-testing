-- The people of the market: clones of the cast's templates, each a NavAgent (the
-- engine walks them, steers them round each other and the carts, and feeds their speed
-- to the locomotion AnimationTree each template got). This module only decides where
-- they go and what they do when they get there:
--
-- * townsfolk walk to a spot (a stall, the fountain, a bench, the terrace, a shop, a
--   bridge, the towpath, the south bank, the stage's audience), turn to face it, play a
--   clip there on animation layer 1 (talking, pointing, sitting, waving...), linger, and
--   move on;
-- * children run about, up onto the stage and off its east edge (NavLinks: they jump);
-- * guards patrol a loop over both floors and both bridges;
-- * the performer dances on the stage.

local C = require("market/core")
local P = require("market/places")

local People = { list = {}, by_entity = {}, cursor = 1, counts = {} }

local ROLE = {
    townsfolk = { speed = { 1.15, 1.6 }, hurry = 0.15, priority = 0.5 },
    child = { speed = { 2.6, 3.3 }, priority = 0.3 },
    guard = { speed = { 1.2, 1.35 }, priority = 0.85 },
    performer = { speed = { 1.0, 1.0 }, priority = 1.0 },
}

local CHILD_ACTIONS = { "cheering", "laughing", "waving", "happy-idle" }
local GUARD_ACTIONS = { "look-around", "salute", "breathing-idle" }
local DANCES = { "samba-dancing", "hip-hop-dancing", "quick-formal-bow", "cheering" }

local function spawn(char, role, x, y, z)
    local template = C.templates[char.id]
    if not template then
        return nil
    end
    local e = clone_entity(template, char.name .. " " .. (#People.list + 1))
    local height = C.random(C.HEIGHT.min, C.HEIGHT.max)
    if role == "child" then
        height = C.random(1.2, 1.4)
    end
    local scale = height / (char.height or 1.8)
    set_translation(e, x, y, z)
    set_rotation(e, C.random() * C.TAU, 0, 0)
    set_scale(e, scale, scale, scale)
    set_component(e, "Visible", true)
    local r = ROLE[role]
    local speed = C.random(r.speed[1], r.speed[2])
    if r.hurry and C.random() < r.hurry then
        speed = C.random(2.1, 2.6)
    end
    set_component(e, "NavAgent", {
        radius = role == "child" and 0.26 or 0.32,
        height = height,
        speed = speed,
        acceleration = role == "child" and 9 or 6,
        turn_speed = 420,
        stopping_distance = 0.12,
        priority = r.priority,
        separation = 1.0,
    })
    local person = {
        e = e,
        char = char,
        role = role,
        speed = speed,
        state = "idle",
        timer = C.random(0.3, 3.0),
        waited = 0,
    }
    table.insert(People.list, person)
    People.by_entity[e] = person
    return person
end

-- Fill the market: `count` townsfolk plus children, guards and the performer.
function People.spawn_all(count)
    local folk = C.group("townsfolk")
    -- Random points spread out from a start on the open plaza (the origin is the
    -- fountain's basin).
    for i = 1, count do
        local p = nav_random_point(0, 0, 6.5, 22) or { x = C.random(-12, 12), y = 0, z = C.random(-8, 8) }
        spawn(folk[(i - 1) % #folk + 1], "townsfolk", p.x, p.y, p.z)
    end
    local children = C.group("child")
    for i = 1, 6 do
        local p = nav_random_point(-6, 0, 0, 8) or { x = 0, y = 0, z = 0 }
        spawn(children[(i - 1) % #children + 1], "child", p.x, p.y, p.z)
    end
    local guards = C.group("guard")
    for i = 1, 3 do
        local wp = P.PATROL[(i - 1) * 3 + 1]
        local g = spawn(guards[(i - 1) % #guards + 1], "guard", wp[1], wp[2], wp[3])
        if g then
            g.waypoint = (i - 1) * 3 + 1
        end
    end
    local performers = C.group("performer")
    local spot = P.PERFORMER
    People.performer = spawn(performers[1], "performer", spot[1], spot[2], spot[3])
end

function People.clear()
    for _, person in ipairs(People.list) do
        despawn_entity(person.e)
    end
    People.list = {}
    People.by_entity = {}
    for _, spot in ipairs(P.spots) do
        spot.taken = nil
    end
end

local function release(person)
    if person.spot and person.spot.taken == person then
        person.spot.taken = nil
    end
    person.spot = nil
end

local function stop_acting(person, fade)
    if person.acting then
        animator_stop(person.e, { layer = 1, fade = fade or 0.5 })
        person.acting = nil
    end
end

local function walk_to(person, x, y, z, spot)
    release(person)
    person.spot = spot
    if spot and spot.taken == nil and spot.kind then
        spot.taken = person
    end
    if nav_set_destination(person.e, x, y, z) then
        person.state = "walking"
        person.waited = 0
    else
        person.state = "idle"
        person.timer = 1.0
    end
end

-- Pick where this person goes next.
local function go(person)
    stop_acting(person)
    local role = person.role
    if role == "townsfolk" then
        local spot = P.choose(C.random, person.last_kind)
        if spot then
            person.last_kind = spot.kind
            walk_to(person, spot.x, spot.y, spot.z, spot)
        else
            person.timer = 2.0
        end
    elseif role == "child" then
        if not person.on_stage and C.random() < 0.4 then
            local s = P.STAGE[C.random(#P.STAGE)]
            person.on_stage = true
            walk_to(person, s[1], s[2], s[3], { x = s[1], y = s[2], z = s[3], actions = { "waving", "cheering" }, linger = { 0.5, 1.5 } })
        else
            person.on_stage = false
            -- Off the stage eastward (the way down is the jump), or anywhere about.
            local p = nav_random_point(-5.5, 0, 0, 14)
            if p then
                walk_to(person, p.x, p.y, p.z, { x = p.x, y = p.y, z = p.z, actions = CHILD_ACTIONS, linger = { 0.5, 3.0 } })
            else
                person.timer = 1.0
            end
        end
    elseif role == "guard" then
        person.waypoint = person.waypoint % #P.PATROL + 1
        local wp = P.PATROL[person.waypoint]
        local stops = person.waypoint % 3 == 0
        walk_to(person, wp[1], wp[2], wp[3], { x = wp[1], y = wp[2], z = wp[3], actions = stops and GUARD_ACTIONS or nil, linger = stops and { 3, 6 } or { 0, 0 } })
    elseif role == "performer" then
        nav_face(person.e, math.pi / 2)
        local dance = DANCES[(person.dance or 0) % #DANCES + 1]
        person.dance = (person.dance or 0) + 1
        if C.clips[dance] then
            animator_play(person.e, dance, { layer = 1, fade = 0.6, loop = dance ~= "quick-formal-bow" })
            person.acting = true
        end
        person.state = "lingering"
        person.timer = dance == "quick-formal-bow" and 2.6 or C.random(9, 14)
    end
end

local function arrive(person)
    local spot = person.spot
    if spot and spot.yaw then
        nav_face(person.e, spot.yaw)
    end
    person.state = "turning"
    person.timer = spot and spot.yaw and 0.7 or 0.15
end

local function begin_lingering(person)
    local spot = person.spot
    local actions = spot and spot.actions
    if actions and #actions > 0 then
        local available = C.available(actions)
        if #available > 0 then
            animator_play(person.e, C.pick(available), { layer = 1, fade = 0.6 })
            person.acting = true
        end
    end
    local range = spot and spot.linger or { 2, 5 }
    person.state = "lingering"
    person.timer = C.random(range[1], range[2])
end

-- One person's decisions (called for a slice of the crowd each frame).
local function think(person, dt)
    local state = person.state
    if state == "idle" or state == "leaving" then
        if person.timer <= 0 then
            go(person)
        end
    elseif state == "walking" then
        person.waited = person.waited + dt
        local s = nav_agent_state(person.e)
        if not s then
            return
        end
        if s.state == "arrived" then
            arrive(person)
        elseif s.state == "failed" or s.state == "lost" then
            release(person)
            person.state = "idle"
            person.timer = 0.5
        elseif s.partial and (s.remaining_distance or 99) < 1.0 then
            -- The way is shut (the gate, a parked cart): do something else meanwhile.
            begin_lingering(person)
            person.timer = math.min(person.timer, 3)
        elseif person.waited > 90 then
            release(person)
            person.state = "idle"
            person.timer = 0.2
        end
    elseif state == "turning" then
        if person.timer <= 0 then
            begin_lingering(person)
        end
    elseif state == "lingering" then
        if person.timer <= 0 then
            if person.role == "performer" then
                go(person)
            else
                stop_acting(person)
                release(person)
                person.state = "leaving"
                person.timer = 0.6
            end
        end
    end
end

-- Send the townsfolk nearest a point there (a click on the navmesh).
function People.summon(x, y, z, count)
    local near = {}
    for _, person in ipairs(People.list) do
        if person.role == "townsfolk" then
            local w = get_world_translation(person.e)
            if w and math.abs(w.y - y) < 4 then
                table.insert(near, { person = person, d = C.dist2(w.x, w.z, x, z) })
            end
        end
    end
    table.sort(near, function(a, b)
        return a.d < b.d
    end)
    for i = 1, math.min(count, #near) do
        local person = near[i].person
        local p = nav_random_point(x, y, z, 2.2) or { x = x, y = y, z = z }
        stop_acting(person, 0.3)
        walk_to(person, p.x, p.y, p.z, {
            x = p.x,
            y = p.y,
            z = p.z,
            yaw = C.yaw_to(x - p.x, z - p.z),
            actions = { "cheering", "waving", "clapping", "pointing" },
            linger = { 4, 8 },
        })
    end
    return math.min(count, #near)
end

function People.update(dt)
    local list = People.list
    local n = #list
    if n == 0 then
        return
    end
    for _, person in ipairs(list) do
        person.timer = person.timer - dt
    end
    -- A quarter of the crowd thinks each frame (plenty for walking pace).
    local slice = math.max(8, math.ceil(n / 4))
    for _ = 1, slice do
        if People.cursor > n then
            People.cursor = 1
        end
        think(list[People.cursor], dt * (n / slice))
        People.cursor = People.cursor + 1
    end
end

-- How many are walking, lingering, waiting (for the HUD).
function People.census()
    local walking, lingering = 0, 0
    for _, person in ipairs(People.list) do
        if person.state == "walking" then
            walking = walking + 1
        elseif person.state == "lingering" or person.state == "turning" then
            lingering = lingering + 1
        end
    end
    return #People.list, walking, lingering
end

-- Someone walking, for the follow camera.
function People.walker(after)
    local n = #People.list
    if n == 0 then
        return nil
    end
    local start = C.random(n)
    for i = 0, n - 1 do
        local person = People.list[(start + i - 1) % n + 1]
        if person.state == "walking" and person ~= after and person.role ~= "performer" then
            return person
        end
    end
    return People.list[start]
end

return People
