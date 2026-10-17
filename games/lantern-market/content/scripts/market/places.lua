-- The places people go in the market and what they do there, from the level's layout
-- (tools/build_level.py builds it from the same numbers): customer spots at the stalls,
-- the fountain rim and benches, the terrace balustrade, the shop fronts under the arcade,
-- the bridges, the towpaths, the south bank and the crowd before the stage.
--
-- Each spot is { x, y, z, yaw (facing, or nil), kind, actions }, plus `taken` while
-- someone stands there.

local P = { spots = {}, by_kind = {} }

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

local STALL = { "talking", "pointing", "thinking", "shrugging", "happy-idle", "laughing" }
for _, s in ipairs(P.STALLS) do
    local a = rad(s[3])
    local fx, fz = math.sin(a), math.cos(a)
    local rx, rz = math.cos(a), -math.sin(a)
    for _, side in ipairs({ -0.95, 0.0, 0.95 }) do
        local x = s[1] + fx * 1.95 + rx * side
        local z = s[2] + fz * 1.95 + rz * side
        add("stall", x, 0.0, z, math.atan(-fx, -fz), STALL, { 6, 14 })
    end
end

-- The fountain's rim, between the benches, and the benches (sitting, facing it).
local FOUNTAIN = { "look-around", "breathing-idle", "drinking", "bored", "talking" }
for k = 0, 3 do
    local a = rad(90 * k)
    for _, off in ipairs({ -0.35, 0.35 }) do
        local b = a + off
        add("fountain", math.cos(b) * 3.95, 0.0, math.sin(b) * 3.95, math.atan(-math.cos(b), -math.sin(b)), FOUNTAIN, { 5, 12 })
    end
end
for k = 0, 3 do
    local a = rad(45 + 90 * k)
    local cx, cz = math.cos(a) * 4.62, math.sin(a) * 4.62
    local tx, tz = -math.sin(a), math.cos(a)
    for _, side in ipairs({ -0.5, 0.5 }) do
        add("bench", cx + tx * side, 0.0, cz + tz * side, math.atan(-math.cos(a), -math.sin(a)), { "sitting-idle" }, { 14, 30 })
    end
end

-- Along the terrace balustrade, looking down on the square.
local TERRACE = { "look-around", "waving", "pointing", "breathing-idle", "talking" }
for k = 0, 14 do
    add("terrace", -21.0 + 3.0 * k, 3.8, -12.85, 0.0, TERRACE, { 6, 14 })
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

-- Before the stage, watching the performer.
local AUDIENCE = { "clapping", "cheering", "happy-idle", "laughing" }
-- (East of where the jumps off the stage land.)
for row = 0, 1 do
    for k = 0, 3 do
        add("audience", -9.6 + 1.0 * row, 0.0, -0.4 + 1.2 * k + 0.5 * row, -math.pi / 2, AUDIENCE, { 8, 16 })
    end
end

-- On the stage (children run up, then jump off its east edge).
P.STAGE = { { -16.5, 1.1, 1.0 }, { -15.0, 1.1, 2.5 }, { -14.0, 1.1, 4.0 }, { -16.0, 1.1, 5.5 } }
-- Where the performer stands.
P.PERFORMER = { -17.6, 1.1, 3.0 }

-- Guards' patrol: the square, up the grand stair, along the terrace, down the ramp,
-- over the stone bridge, along the south bank and back over the footbridge. Every
-- third waypoint is a stop, so those stand clear of the way.
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

-- How much each kind of place draws people.
P.WEIGHTS = {
    { "stall", 30 },
    { "fountain", 9 },
    { "bench", 7 },
    { "terrace", 12 },
    { "shop", 8 },
    { "bridge", 8 },
    { "towpath", 10 },
    { "bank", 6 },
    { "audience", 8 },
}

-- A free spot of a kind, chosen with `random`.
function P.free(kind, random)
    local list = P.by_kind[kind]
    if not list then
        return nil
    end
    local start = random(#list)
    for i = 0, #list - 1 do
        local spot = list[(start + i - 1) % #list + 1]
        if not spot.taken then
            return spot
        end
    end
    return nil
end

function P.choose(random, exclude_kind)
    local total = 0
    for _, w in ipairs(P.WEIGHTS) do
        if w[1] ~= exclude_kind then
            total = total + w[2]
        end
    end
    for _ = 1, 6 do
        local pick = random() * total
        for _, w in ipairs(P.WEIGHTS) do
            if w[1] ~= exclude_kind then
                pick = pick - w[2]
                if pick <= 0 then
                    local spot = P.free(w[1], random)
                    if spot then
                        return spot
                    end
                    break
                end
            end
        end
    end
    return nil
end

return P
