-- Shared state and helpers of the show: the cast, the character templates, the actors
-- on stage, a seeded random generator and a few maths helpers. The director and every
-- part require this module and share the one table it returns.

local S = {
    -- From cast_data.lua (written by tools/fetch_cast.py): characters and clips.
    characters = {},
    clips = {},
    by_group = {},
    -- Character id -> the hidden entity the character was imported into; clones of it
    -- are the actors.
    templates = {},
    -- What is on stage now: { e, char, clip, x, y, z, yaw, ... } per actor.
    actors = {},
    -- Crowd size multiplier (title screen: 1, 2, 3 keys).
    scale = 1,
    -- Bones animated on stage right now (the HUD's count).
    bones = 0,
}

S.TAU = math.pi * 2
-- The arena's stands (built in the scene to match): tiers rising outward from the
-- floor's edge, each `depth` deep and `rise` higher than the one before.
S.STANDS = { first = 31.5, depth = 2.1, rise = 0.6, tiers = 10 }
-- The clips were measured on Mixamo's Y Bot and Quaternius's mannequins; travel scales
-- with the character's size (the animator retargets translations by leg length).
S.CLIP_RIG_HEIGHT = 1.8

function S.load_cast()
    local ok, data = pcall(require, "cast_data")
    if not ok or type(data) ~= "table" then
        return false
    end
    S.characters = data.characters or {}
    S.clips = data.clips or {}
    S.by_group = {}
    for _, char in ipairs(S.characters) do
        for _, group in ipairs(char.groups) do
            S.by_group[group] = S.by_group[group] or {}
            table.insert(S.by_group[group], char)
        end
    end
    return #S.characters > 0
end

-- Deterministic random numbers (the same show every run): a xorshift generator.
function S.rng(seed)
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

function S.pick(list, random)
    return list[random(#list)]
end

-- Characters of a group (falling back to everyone), for crowds.
function S.group(name)
    local list = S.by_group[name]
    if list and #list > 0 then
        return list
    end
    return S.characters
end

-- Characters of several groups together, each once.
function S.groups(...)
    local seen, list = {}, {}
    for _, name in ipairs({ ... }) do
        for _, char in ipairs(S.by_group[name] or {}) do
            if not seen[char.id] then
                seen[char.id] = true
                table.insert(list, char)
            end
        end
    end
    if #list == 0 then
        return S.characters
    end
    return list
end

-- Clips that exist in the cast, in the order given (missing ones are skipped).
function S.available(names)
    local list = {}
    for _, name in ipairs(names) do
        if S.clips[name] then
            table.insert(list, name)
        end
    end
    return list
end

-- How far a clip carries this character per second when played at speed 1.
function S.ground_speed(clip, char)
    local info = S.clips[clip]
    if not info then
        return 0
    end
    return info.speed * (char.height or S.CLIP_RIG_HEIGHT) / S.CLIP_RIG_HEIGHT
end

function S.count(base)
    return math.max(1, math.floor(base * S.scale + 0.5))
end

-- Yaw (radians about +Y) that turns a character facing +Z toward (dx, dz).
function S.yaw_to(dx, dz)
    return math.atan(dx, dz)
end

function S.lerp(a, b, t)
    return a + (b - a) * t
end

function S.smooth(t)
    t = math.max(0, math.min(1, t))
    return t * t * (3 - 2 * t)
end

-- Put a clone of `char` on stage. Options: y, yaw, clip (played from a random point
-- unless `start` is given), clips (every clip the actor will play: they bind once, at
-- spawn), speed, random.
function S.spawn(char, x, z, opts)
    opts = opts or {}
    local template = S.templates[char.id]
    if not template then
        return nil
    end
    local e = clone_entity(template, char.name)
    local y = opts.y or 0
    local yaw = opts.yaw or 0
    set_translation(e, x, y, z)
    set_rotation(e, yaw, 0, 0)
    set_component(e, "Visible", true)
    local actor = { e = e, char = char, x = x, y = y, z = z, yaw = yaw, libs = {} }
    for _, clip in ipairs(opts.clips or {}) do
        S.add_clip(actor, clip)
    end
    animator_set_root_motion(e, "in_place")
    if opts.speed then
        animator_set_speed(e, opts.speed)
    end
    if opts.clip then
        local start = opts.start
        if start == nil and opts.random and S.clips[opts.clip] then
            start = opts.random() * S.clips[opts.clip].duration
        end
        S.play(actor, opts.clip, { fade = 0, start = start or 0, loop = opts.loop })
    end
    table.insert(S.actors, actor)
    S.bones = S.bones + (char.bones or 0)
    return actor
end

function S.add_clip(actor, clip)
    local info = S.clips[clip]
    if info and not actor.libs[clip] then
        actor.libs[clip] = true
        animator_add_clips(actor.e, info.path)
    end
end

-- Cross-fade an actor to `clip` (options as animator_play).
function S.play(actor, clip, opts)
    if not S.clips[clip] then
        return
    end
    S.add_clip(actor, clip)
    animator_play(actor.e, clip, opts or { fade = 0.3 })
    actor.clip = clip
end

function S.place(actor, x, y, z, yaw)
    actor.x, actor.y, actor.z = x, y, z
    set_translation(actor.e, x, y, z)
    if yaw then
        actor.yaw = yaw
        set_rotation(actor.e, yaw, 0, 0)
    end
end

function S.clear()
    for _, actor in ipairs(S.actors) do
        despawn_entity(actor.e)
    end
    S.actors = {}
    S.bones = 0
end

-- Characters to fill a crowd of `n` from `pool`, spread evenly over the pool (each
-- character about as often as the others) in a shuffled order.
function S.cast_list(pool, n, random)
    local list = {}
    for i = 1, n do
        list[i] = pool[(i - 1) % #pool + 1]
    end
    for i = n, 2, -1 do
        local j = random(i)
        list[i], list[j] = list[j], list[i]
    end
    return list
end

return S
