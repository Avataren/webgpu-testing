-- The places people go in the market and what they do there, from the level's layout
-- (tools/build_level.py builds it from the same numbers): customer spots at the stalls,
-- the fountain rim and benches, the terrace balustrade, the shop fronts under the arcade,
-- the bridges, the towpaths, the south bank and the crowd before the stage; where the
-- vendors stand behind their counters, where a thief lies low, where children play.
--
-- Each spot is { x, y, z, yaw (facing, or nil), kind, actions, linger, stall }, and
-- `taken` (the id of the agent holding it) while someone has claimed it. Brains claim and
-- release spots through P.claim / P.release (a spot is a Lua table on their blackboard).

local P = { spots = {}, by_kind = {}, held = {} }

local function add(kind, x, y, z, yaw, actions, linger)
    local spot = { x = x, y = y, z = z, yaw = yaw, kind = kind, actions = actions, linger = linger }
    table.insert(P.spots, spot)
    P.by_kind[kind] = P.by_kind[kind] or {}
    table.insert(P.by_kind[kind], spot)
    return spot
end

local rad = math.rad

-- Stalls: centre, yaw (degrees; the stall faces its local +z).
P.STALLS = {
    { -7.5, -6.0, 90 },
    { -7.5, 5.5, 90 },
    { 7.5, -6.0, -90 },
    { 7.5, 5.5, -90 },
    { -4.5, 9.0, 180 },
    { 4.5, 9.0, 180 },
}

-- Where each stall's vendor stands: between the counter and the back table, facing the
-- customers.
P.VENDOR_POSTS = {}

local STALL = { "talking", "pointing", "thinking", "shrugging", "happy-idle", "laughing" }
for i, s in ipairs(P.STALLS) do
    local a = rad(s[3])
    local fx, fz = math.sin(a), math.cos(a)
    local rx, rz = math.cos(a), -math.sin(a)
    for _, side in ipairs({ -0.95, 0.0, 0.95 }) do
        local x = s[1] + fx * 1.95 + rx * side
        local z = s[2] + fz * 1.95 + rz * side
        local spot = add("stall", x, 0.0, z, math.atan(-fx, -fz), STALL, { 6, 12 })
        spot.stall = i
    end
    P.VENDOR_POSTS[i] = { x = s[1] - fx * 0.1, y = 0.0, z = s[2] - fz * 0.1, yaw = math.atan(fx, fz) }
end

-- The fountain's rim, between the benches, and the benches (sitting, facing it).
local FOUNTAIN = { "drinking", "drinking", "look-around", "breathing-idle" }
for k = 0, 3 do
    local a = rad(90 * k)
    for _, off in ipairs({ -0.35, 0.35 }) do
        local b = a + off
        add("fountain", math.cos(b) * 3.95, 0.0, math.sin(b) * 3.95, math.atan(-math.cos(b), -math.sin(b)), FOUNTAIN, { 5, 9 })
    end
end
for k = 0, 3 do
    local a = rad(45 + 90 * k)
    local cx, cz = math.cos(a) * 4.62, math.sin(a) * 4.62
    local tx, tz = -math.sin(a), math.cos(a)
    for _, side in ipairs({ -0.5, 0.5 }) do
        add("bench", cx + tx * side, 0.0, cz + tz * side, math.atan(-math.cos(a), -math.sin(a)), { "sitting-idle" }, { 14, 26 })
    end
end

-- Along the terrace balustrade, looking down on the square.
local TERRACE = { "look-around", "waving", "pointing", "breathing-idle", "talking" }
for k = 0, 14 do
    add("terrace", -21.0 + 3.0 * k, 3.8, -12.85, 0.0, TERRACE, { 6, 12 })
end

-- Shop fronts under the arcade: doors and windows.
local SHOP = { "thinking", "look-around", "idle", "pointing" }
for k = 0, 5 do
    local x = -20.0 + 8.0 * k
    add("shop", x, 0.0, -15.35, math.pi, SHOP, { 5, 10 })
    add("shop", x + 2.8, 0.0, -15.35, math.pi, SHOP, { 5, 10 })
end

-- On the stone bridge, looking at the water (the footbridge has room to pass, not to
-- stand about).
local BRIDGE = { "look-around", "breathing-idle", "idle", "pointing" }
for _, z in ipairs({ 15.2, 16.8 }) do
    add("bridge", -1.85, 0.0, z, -math.pi / 2, BRIDGE, { 5, 10 })
    add("bridge", 1.85, 0.0, z, math.pi / 2, BRIDGE, { 5, 10 })
end

-- Along the towpaths: walked to, then on (no stop).
for x = -16, 26, 6 do
    add("towpath", x, -2.3, 13.5, nil, nil, { 0, 1 })
end
for x = -26, 16, 6 do
    add("towpath", x, -2.3, 18.5, nil, nil, { 0, 1 })
end

-- The street south of the canal, looking back over it (not in front of the bridges).
local BANK = { "look-around", "idle", "talking", "breathing-idle" }
for x = -22, 22, 4 do
    if math.abs(x + 14) > 2 and math.abs(x) > 3 then
        add("bank", x, 0.0, 21.2, math.pi, BANK, { 4, 9 })
    end
end

-- Before the stage, watching the performer (east of where the jumps off the stage land).
local AUDIENCE = { "clapping", "cheering", "happy-idle", "laughing" }
for row = 0, 2 do
    for k = 0, 3 do
        add("audience", -9.6 + 1.0 * row, 0.0, -0.4 + 1.2 * k + 0.5 * row, -math.pi / 2, AUDIENCE, { 8, 16 })
    end
end

-- On the stage (children run up, then jump off its east edge).
P.STAGE = { { -16.5, 1.1, 1.0 }, { -15.0, 1.1, 2.5 }, { -14.0, 1.1, 4.0 }, { -16.0, 1.1, 5.5 } }
-- Where the performer stands, facing the square.
P.PERFORMER = { x = -17.6, y = 1.1, z = 3.0, yaw = math.pi / 2 }

-- Guards' patrol: the square, up the grand stair, along the terrace, down the ramp,
-- over the stone bridge, along the south bank and back over the footbridge.
P.PATROL = {
    { -18.0, 0.0, -7.0 },
    { -26.0, 0.0, 0.5 },
    { -26.0, 3.8, -13.5 },
    { 0.0, 3.8, -14.5 },
    { 26.0, 3.8, -13.5 },
    { 26.0, 0.0, 6.0 },
    { 12.0, 0.0, 11.0 },
    { 0.0, 0.0, 21.5 },
    { -16.5, 0.0, 22.2 },
    { -14.0, 0.0, 11.0 },
}

-- Where a thief lies low: under the bridges on the towpaths, the far ends of the arcade.
P.HIDEOUTS = {
    { -3.2, -2.3, 13.6 },
    { 3.2, -2.3, 18.4 },
    { -17.5, -2.3, 18.4 },
    { 24.0, -2.3, 13.6 },
    { -24.5, 0.0, -15.6 },
    { 25.0, 0.0, -15.6 },
}

-- Where a caught thief is let go: the far side of the canal.
P.RELEASE = { 16.0, 0.0, 22.0 }

-- Where children run about (the open square west of the fountain).
P.PLAYGROUND = { -5.5, 0.0, 0.0 }

-- A free spot of `kind` for `agent` (an entity id), near `x, z` if given: of a few free
-- ones picked at random, the nearest. Releases what the agent held before.
function P.claim(kind, agent, x, z, random)
    P.release(agent)
    local list = P.by_kind[kind]
    if not list then
        return nil
    end
    local best, best_d
    for _ = 1, 4 do
        local spot = list[random(#list)]
        if not spot.taken then
            local d = x and ((spot.x - x) ^ 2 + (spot.z - z) ^ 2) or 0
            if not best or d < best_d then
                best, best_d = spot, d
            end
        end
    end
    if not best then
        for _, spot in ipairs(list) do
            if not spot.taken then
                best = spot
                break
            end
        end
    end
    if best then
        best.taken = agent
        P.held[agent] = best
    end
    return best
end

-- A free spot at stall `stall` for `agent`.
function P.claim_at_stall(stall, agent)
    P.release(agent)
    for _, spot in ipairs(P.by_kind.stall) do
        if spot.stall == stall and not spot.taken then
            spot.taken = agent
            P.held[agent] = spot
            return spot
        end
    end
    return nil
end

function P.release(agent)
    local spot = P.held[agent]
    if spot then
        if spot.taken == agent then
            spot.taken = nil
        end
        P.held[agent] = nil
    end
end

function P.clear()
    for _, spot in ipairs(P.spots) do
        spot.taken = nil
    end
    P.held = {}
end

function P.count_taken(kind)
    local n = 0
    for _, spot in ipairs(P.by_kind[kind] or {}) do
        if spot.taken then
            n = n + 1
        end
    end
    return n
end

return P
