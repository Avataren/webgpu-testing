-- The people of the market: clones of the cast's templates, each an AiAgent running its
-- role's brain (market/brains/), and, unless they stand still for good, a NavAgent the
-- brain walks (the engine steers them round each other and the carts, and feeds their
-- speed to the locomotion tree each template got).
--
-- How many of each comes from CAST_SIZE below, or from market/cast_size_local.lua if it
-- exists (git-ignored; for a small cast when testing:
-- `return { townsfolk = 12, vendors = 2, guards = 2, children = 3 }`).

local C = require("market/core")
local P = require("market/places")
local Townsfolk = require("market/brains/townsfolk")
local Vendor = require("market/brains/vendor")
local Guard = require("market/brains/guard")
local Thief = require("market/brains/thief")
local Child = require("market/brains/child")
local Performer = require("market/brains/performer")

local People = { list = {}, by_role = {}, counts = { total = 0, walking = 0, roles = {} } }

local CAST_SIZE = { townsfolk = 72, vendors = 6, guards = 3, children = 6, thief = 1, performer = 1 }

function People.cast_size()
    local size = {}
    for k, v in pairs(CAST_SIZE) do
        size[k] = v
    end
    local ok, local_size = pcall(require, "market/cast_size_local")
    if ok and type(local_size) == "table" then
        for k, v in pairs(local_size) do
            size[k] = v
        end
    end
    return size
end

-- What each role is like: walking speed (m/s), avoidance priority, brain settings.
local ROLE = {
    townsfolk = {
        speed = { 1.15, 1.6 }, hurry = 0.15, priority = 0.5,
        brain = "townsfolk", tick_rate = 8, run_speed = 3.0, tags = { "townsfolk" },
        perception = { sight_range = 9, sight_angle = 150, near_range = 2, hearing_range = 18,
            memory = 4, interests = { "townsfolk", "cart" }, rate = 2 },
    },
    vendor = {
        static = true, brain = "vendor", tick_rate = 5, tags = { "vendor" },
        perception = { sight_range = 7, sight_angle = 150, near_range = 2, hearing_range = 0,
            interests = { "townsfolk" }, line_of_sight = "none", rate = 2 },
    },
    guard = {
        speed = { 1.2, 1.35 }, priority = 0.85,
        brain = "guard", tick_rate = 10, run_speed = 4.3, tags = { "guard" }, group = "watch",
        perception = { sight_range = 15, sight_angle = 150, near_range = 2.5, hearing_range = 30,
            memory = 6, interests = { "thief", "guard" }, rate = 5 },
    },
    thief = {
        speed = { 1.3, 1.3 }, priority = 0.6,
        brain = "thief", tick_rate = 10, run_speed = 3.9, tags = { "thief" },
        perception = { sight_range = 12, sight_angle = 200, near_range = 3, hearing_range = 20,
            memory = 5, interests = { "guard", "townsfolk" }, rate = 5 },
    },
    child = {
        speed = { 2.6, 3.3 }, priority = 0.3, child = true,
        brain = "child", tick_rate = 10, run_speed = 3.4, tags = { "child" }, group = "kids",
        perception = { sight_range = 10, sight_angle = 220, near_range = 2, hearing_range = 15,
            interests = { "child" }, rate = 4 },
    },
    performer = {
        speed = { 1.1, 1.1 }, priority = 1.0,
        brain = "performer", tick_rate = 5, tags = { "performer" },
        perception = { sight_range = 0 },
    },
}

local function spawn(char, role_name, x, y, z, blackboard)
    local template = C.templates[char.id]
    if not template then
        return nil
    end
    local role = ROLE[role_name]
    local name = char.name .. " " .. (#People.list + 1)
    local e = clone_entity(template, name)
    local height = role.child and C.random(1.2, 1.4) or C.random(C.HEIGHT.min, C.HEIGHT.max)
    local scale = height / (char.height or 1.8)
    set_translation(e, x, y, z)
    set_rotation(e, C.random() * C.TAU, 0, 0)
    set_scale(e, scale, scale, scale)
    set_component(e, "Visible", true)
    local speed = 0
    if not role.static then
        speed = C.random(role.speed[1], role.speed[2])
        if role.hurry and C.random() < role.hurry then
            speed = C.random(2.1, 2.6)
        end
        set_component(e, "NavAgent", {
            radius = role.child and 0.26 or 0.32,
            height = height,
            speed = speed,
            acceleration = role.child and 9 or 6,
            turn_speed = 420,
            stopping_distance = 0.12,
            priority = role.priority,
            separation = 1.0,
        })
    end
    blackboard = blackboard or {}
    blackboard.name = blackboard.name or name
    ai_set_brain(e, role.brain, {
        tags = role.tags,
        group = role.group or "",
        tick_rate = role.tick_rate,
        run_speed = role.run_speed or 3.2,
        perception = role.perception,
        blackboard = blackboard,
    })
    local person = { e = e, char = char, role = role_name, name = name, speed = speed,
        run_speed = role.run_speed or 3.2 }
    table.insert(People.list, person)
    People.by_role[role_name] = People.by_role[role_name] or {}
    table.insert(People.by_role[role_name], person)
    return person
end

-- Fill the market.
function People.spawn_all()
    local size = People.cast_size()

    local folk = C.group("townsfolk")
    for i = 1, size.townsfolk do
        -- Random points spread out from the open plaza (the origin is the fountain).
        local p = nav_random_point(0, 0, 6.5, 22) or { x = C.random(-12, 12), y = 0, z = C.random(-8, 8) }
        local char = folk[(i - 1) % #folk + 1]
        spawn(char, "townsfolk", p.x, p.y, p.z, Townsfolk.blackboard(C.random, char.name .. " " .. (#People.list + 1)))
    end

    local vendors = C.group("vendor")
    for i = 1, math.min(size.vendors, #P.STALLS) do
        local post = P.VENDOR_POSTS[i]
        local char = vendors[(i - 1) % #vendors + 1]
        local person = spawn(char, "vendor", post.x, post.y, post.z, Vendor.blackboard(i, post))
        if person then
            set_rotation(person.e, post.yaw, 0, 0)
            person.stall = i
        end
    end

    local guards = C.group("guard")
    for i = 1, size.guards do
        local start = (i - 1) * 3 % #P.PATROL + 1
        local wp = P.PATROL[start]
        spawn(guards[(i - 1) % #guards + 1], "guard", wp[1], wp[2], wp[3], {
            route = P.PATROL,
            patrol_index = start % #P.PATROL + 1,
        })
    end

    if size.thief > 0 then
        local thieves = C.group("thief")
        -- (Not around the origin: the navmesh nearest the fountain's basin is a pocket
        -- under the square.)
        local p = nav_random_point(0, 0, 6.5, 12) or { x = 3, y = 0, z = 6 }
        People.thief = spawn(thieves[1], "thief", p.x, p.y, p.z, Thief.blackboard())
    end

    local children = C.group("child")
    for i = 1, size.children do
        local p = nav_random_point(P.PLAYGROUND[1], 0, P.PLAYGROUND[3], 8) or { x = -5, y = 0, z = 0 }
        spawn(children[(i - 1) % #children + 1], "child", p.x, p.y, p.z, Child.blackboard())
    end

    if size.performer > 0 then
        local performers = C.group("performer")
        local s = P.PERFORMER
        People.performer = spawn(performers[1], "performer", s.x, s.y, s.z, Performer.blackboard())
    end
end

-- Once the clones exist (the frame after spawning): their entity ids, which brains use.
function People.resolve()
    for _, person in ipairs(People.list) do
        if not person.id then
            person.id = entity_id(person.e)
            if person.id then
                C.by_id[person.id] = person
                if person.stall then
                    C.vendors[person.stall] = person.id
                end
            end
        end
    end
end

function People.ids(role)
    local ids = {}
    for _, person in ipairs(People.by_role[role] or {}) do
        if person.id then
            table.insert(ids, person.id)
        end
    end
    return ids
end

function People.clear()
    for _, person in ipairs(People.list) do
        despawn_entity(person.e)
    end
    People.list = {}
    People.by_role = {}
    C.by_id = {}
    C.vendors = {}
    P.clear()
end

-- Call the townsfolk nearest a point there (a click on the navmesh).
function People.summon(x, y, z, count)
    local near = {}
    for _, person in ipairs(People.by_role.townsfolk or {}) do
        local w = get_world_translation(person.e)
        if w and math.abs(w.y - y) < 4 then
            table.insert(near, { person = person, d = C.dist2(w.x, w.z, x, z) })
        end
    end
    table.sort(near, function(a, b)
        return a.d < b.d
    end)
    local called = math.min(count, #near)
    for i = 1, called do
        ai_send(near[i].person.e, "summon", { x = x, y = y, z = z })
    end
    return called
end

-- People by state: { total, walking, roles = { [role] = { [state] = n } } }.
function People.census()
    local counts = { total = #People.list, walking = 0, roles = {} }
    for _, person in ipairs(People.list) do
        local state = ai_state(person.e) or "?"
        local role = counts.roles[person.role] or {}
        counts.roles[person.role] = role
        role[state] = (role[state] or 0) + 1
        local s = person.speed > 0 and nav_agent_state(person.e)
        if s and s.speed and s.speed > 0.3 then
            counts.walking = counts.walking + 1
        end
    end
    People.counts = counts
    return counts
end

-- Someone worth following: a person on the move (not `after`), preferring an
-- interesting role now and then.
function People.walker(after)
    local n = #People.list
    if n == 0 then
        return nil
    end
    local start = C.random(n)
    for i = 0, n - 1 do
        local person = People.list[(start + i - 1) % n + 1]
        local s = person ~= after and person.speed > 0 and nav_agent_state(person.e)
        if s and s.speed and s.speed > 0.5 then
            return person
        end
    end
    return People.list[start]
end

return People
