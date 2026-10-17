-- Shared state of the market: the cast (from cast_data.lua, written by
-- tools/fetch_cast.py), the character templates, a seeded random generator and small
-- maths helpers. Every module requires this and shares the table it returns.

local C = {
    characters = {},
    clips = {},
    by_group = {},
    -- Character id -> the hidden holder its model was imported into; the crowd are clones.
    templates = {},
    TAU = math.pi * 2,
    -- The heights the clones are scaled to (the library's models range from 1.3 to 2.4 m).
    HEIGHT = { min = 1.62, max = 1.86 },
    -- Entity id -> the person (filled in once clones exist; brains get ids).
    by_id = {},
    -- What just happened, for the HUD: { text, time }.
    news = {},
    -- Stall index -> its vendor's entity id.
    vendors = {},
}

-- Every clip a character can play: locomotion for the animation tree, the rest for the
-- things people do where they stop.
C.LOCOMOTION = { "idle", "walking", "jogging", "running", "jump" }
C.ACTIONS = {
    "happy-idle", "breathing-idle", "bored", "look-around", "thinking", "talking", "laughing",
    "waving", "clapping", "cheering", "pointing", "shrugging", "drinking", "sitting-idle", "salute",
}

function C.load_cast()
    local ok, data = pcall(require, "cast_data")
    if not ok or type(data) ~= "table" then
        return false
    end
    C.characters = data.characters or {}
    C.clips = data.clips or {}
    C.by_group = {}
    for _, char in ipairs(C.characters) do
        for _, group in ipairs(char.groups) do
            C.by_group[group] = C.by_group[group] or {}
            table.insert(C.by_group[group], char)
        end
    end
    return #C.characters > 0
end

-- Deterministic random numbers (the same market every run): xorshift.
function C.rng(seed)
    local state = (seed or 1) % 4294967296
    if state == 0 then
        state = 2463534242
    end
    return function(a, b)
        state = state ~ ((state << 13) & 0xffffffff)
        state = state ~ (state >> 17)
        state = state ~ ((state << 5) & 0xffffffff)
        local unit = state / 4294967296
        if a == nil then
            return unit
        elseif b == nil then
            return math.floor(unit * a) + 1
        end
        return a + (b - a) * unit
    end
end

C.random = C.rng(20261017)

function C.pick(list)
    return list[C.random(#list)]
end

-- The characters of a group that were loaded (falling back to every loaded one).
function C.group(name)
    local list = {}
    for _, char in ipairs(C.by_group[name] or {}) do
        if C.templates[char.id] then
            table.insert(list, char)
        end
    end
    if #list == 0 then
        for _, char in ipairs(C.characters) do
            if C.templates[char.id] then
                table.insert(list, char)
            end
        end
    end
    return list
end

-- Yaw (radians about +Y) that turns a model facing +Z toward (dx, dz).
function C.yaw_to(dx, dz)
    return math.atan(dx, dz)
end

function C.dist2(ax, az, bx, bz)
    local dx, dz = ax - bx, az - bz
    return dx * dx + dz * dz
end

function C.lerp(a, b, t)
    return a + (b - a) * t
end

function C.smooth(t)
    t = math.max(0, math.min(1, t))
    return t * t * (3 - 2 * t)
end

-- The clips that exist, of those named.
function C.available(names)
    local list = {}
    for _, name in ipairs(names) do
        if C.clips[name] then
            table.insert(list, name)
        end
    end
    return list
end

-- Make every clip the market uses playable on `e` (a template or a clone).
function C.add_clips(e)
    for name, clip in pairs(C.clips) do
        animator_add_clips(e, clip.path)
    end
end

-- Something worth telling (a theft, an arrest, a game of tag).
function C.tell(text)
    table.insert(C.news, 1, { text = text, time = get_time() })
    while #C.news > 5 do
        table.remove(C.news)
    end
    log_info("[market] " .. text)
end

-- The person an entity id belongs to, and their name.
function C.person(id)
    return C.by_id[id]
end

function C.name_of(id)
    local person = C.by_id[id]
    return person and person.name or ("someone " .. tostring(id))
end

function C.clamp01(v)
    return math.max(0, math.min(1, v))
end

return C
