-- Lantern Market's director: loads the cast, defines the brains (market/brains/: every
-- person, cart and the portcullis is an AiAgent running a behaviour tree or state
-- machine), fills the market, and runs the camera, the HUD and the keys.
--
-- Steer it with lua_eval on this entity: MARKET holds the state, and RT.status(),
-- RT.gate(), RT.summon(x, y, z, n), RT.shot(n), RT.debug(on), RT.ai(on), RT.theft(),
-- RT.tag() are test hooks. Over MCP, ai_status / ai_inspect show what everyone thinks.

local C = require("market/core")
local People = require("market/people")
local Props = require("market/props")
local Camera = require("market/camera")
local Hud = require("market/hud")
local Townsfolk = require("market/brains/townsfolk")
local Vendor = require("market/brains/vendor")
local Guard = require("market/brains/guard")
local Thief = require("market/brains/thief")
local Child = require("market/brains/child")
local Performer = require("market/brains/performer")

-- Exports keep what scripts name as literals: the cast's folders (filled by
-- tools/fetch_cast.py, listed in cast_data.lua).
local CAST_FOLDERS = { "content/characters/", "content/animations/" }

local IMPORTS_PER_FRAME = 2
-- A game of tag starts this often (seconds), when none is on.
local TAG_EVERY = { 35, 60 }

MARKET = {
    mode = "boot",
    holders = {},
    queue = {},
    importing = {},
    loaded = 0,
    debug = { mesh = false, agents = false, paths = false, obstacles = false, x_ray = false },
    ai_debug = false,
    nav = { polys = 0, carving = 0 },
    frame = 0,
    next_tag = 0,
}
RT = {}

local function boot()
    if not C.load_cast() then
        MARKET.mode = "nocast"
        return
    end
    -- Only the characters someone plays get imported.
    local wanted = {}
    for _, group in ipairs({ "townsfolk", "vendor", "guard", "thief", "child", "performer", "puller" }) do
        for _, char in ipairs(C.by_group[group] or {}) do
            wanted[char.id] = char
        end
    end
    for _, char in ipairs(C.characters) do
        if wanted[char.id] then
            local holder = spawn_entity("Template " .. char.name)
            set_component(holder, "Visible", false)
            set_translation(holder, 0, -60, 0)
            MARKET.holders[char.id] = holder
            table.insert(MARKET.queue, char)
        end
    end
    MARKET.total = #MARKET.queue
    MARKET.mode = "loading"
end

-- A cart's puller: a townsperson walking in front of the cart, holding its handles.
local function spawn_puller(cart, offset)
    local pullers = C.group("puller")
    local char = pullers[#MARKET.pullers % #pullers + 1]
    local template = C.templates[char.id]
    if not template then
        return nil
    end
    local e = clone_entity(template, char.name .. " (cart)")
    local scale = 1.74 / (char.height or 1.8)
    set_parent(e, cart)
    set_translation(e, 0, 0, offset)
    set_rotation(e, math.pi, 0, 0)
    set_scale(e, scale, scale, scale)
    set_component(e, "Visible", true)
    table.insert(MARKET.pullers, e)
    return e
end

local function define_brains()
    Townsfolk.define()
    Vendor.define()
    Guard.define()
    Thief.define()
    Child.define()
    Performer.define()
    Props.define()
end

local function start_market()
    MARKET.pullers = {}
    define_brains()
    People.spawn_all()
    Props.setup(spawn_puller)
    MARKET.next_tag = get_time() + C.random(TAG_EVERY[1], TAG_EVERY[2])
    MARKET.mode = "running"
end

local function load_step()
    for _ = 1, IMPORTS_PER_FRAME do
        local char = table.remove(MARKET.queue, 1)
        if not char then
            break
        end
        import_gltf(MARKET.holders[char.id], char.path, 1.0)
        MARKET.importing[char.id] = { char = char, frames = 0 }
    end
    -- Once a model is in: its clips, in place (the agent moves it), then a locomotion
    -- tree from those clips (built after the hook, so a frame later), then ready.
    for id, entry in pairs(MARKET.importing) do
        local holder = MARKET.holders[id]
        if not entry.prepared then
            local children = get_children(holder)
            if children and #children > 0 then
                C.add_clips(holder)
                animator_set_root_motion(holder, "in_place")
                entry.prepared = true
            end
        else
            entry.frames = entry.frames + 1
            if entry.frames == 1 then
                animation_tree_generate(holder)
            elseif entry.frames >= 3 then
                C.templates[id] = holder
                MARKET.importing[id] = nil
                MARKET.loaded = MARKET.loaded + 1
            end
        end
    end
    if #MARKET.queue == 0 and next(MARKET.importing) == nil then
        start_market()
    end
end

local function apply_debug()
    nav_debug_draw(MARKET.debug)
end

-- Now and then the children start a game of tag.
local function maybe_start_tag(force)
    if not force and get_time() < MARKET.next_tag then
        return nil
    end
    MARKET.next_tag = get_time() + C.random(TAG_EVERY[1], TAG_EVERY[2])
    return Child.start_tag(People.ids("child"))
end

local function handle_input()
    if is_key_just_pressed("Tab") then
        Camera.auto = true
        Camera.next()
    end
    for i = 1, #Camera.SHOTS do
        if is_key_just_pressed(tostring(i)) then
            Camera.auto = false
            Camera.select(i)
        end
    end
    if is_key_just_pressed("F") then
        Camera.auto = false
        Camera.select(7)
    end
    if is_key_just_pressed("N") then
        MARKET.debug.mesh = not MARKET.debug.mesh
        MARKET.debug.obstacles = MARKET.debug.mesh
        apply_debug()
    end
    if is_key_just_pressed("P") then
        MARKET.debug.paths = not MARKET.debug.paths
        MARKET.debug.agents = MARKET.debug.paths
        apply_debug()
    end
    if is_key_just_pressed("X") then
        MARKET.debug.x_ray = not MARKET.debug.x_ray
        apply_debug()
    end
    if is_key_just_pressed("B") then
        Hud.labels = not Hud.labels
    end
    if is_key_just_pressed("V") then
        MARKET.ai_debug = not MARKET.ai_debug
        ai_debug_draw(MARKET.ai_debug and { senses = true, goals = true, states = true } or false)
    end
    if is_key_just_pressed("G") then
        Props.toggle_gate()
    end
    if is_key_just_pressed("T") and MARKET.mode == "running" then
        maybe_start_tag(true)
    end
    if is_key_just_pressed("H") then
        Hud.hidden = not Hud.hidden
    end
    if MARKET.mode == "running" and is_mouse_button_just_pressed(0) then
        local mouse = get_mouse_position()
        local ray = mouse and screen_to_world_ray(mouse.x, mouse.y)
        local hit = ray and nav_pick(ray.ox, ray.oy, ray.oz, ray.dx, ray.dy, ray.dz)
        if hit then
            People.summon(hit.x, hit.y, hit.z, 14)
        end
    end
end

function on_created(self)
    Camera.setup()
    Hud.camera = Camera.e
end

function update(self, dt)
    MARKET.frame = MARKET.frame + 1
    if MARKET.mode == "boot" then
        boot()
    elseif MARKET.mode == "loading" then
        load_step()
    elseif MARKET.mode == "running" then
        People.resolve()
        maybe_start_tag(false)
        if MARKET.frame % 15 == 0 then
            People.census()
        end
        if MARKET.frame % 30 == 0 then
            local status = nav_status()
            local polys = 0
            for _, surface in ipairs(status.surfaces) do
                polys = polys + surface.polys
            end
            MARKET.nav = { polys = polys, carving = status.carving }
        end
    end
    Camera.update(dt, People)
    Hud.update(dt)
    handle_input()
end

function on_ui(self, ui)
    local info = { shot = Camera.name(), gate = Props.gate_text(), carts = Props.cart_text() }
    if MARKET.mode == "nocast" then
        info.nocast = true
    elseif MARKET.mode ~= "running" then
        info.loading = string.format("Waking the townsfolk... %d of %d", MARKET.loaded, MARKET.total or 0)
    else
        info.counts = People.counts
    end
    info.polys = MARKET.nav.polys
    info.carving = MARKET.nav.carving
    local shot = Camera.SHOTS[Camera.shot]
    Hud.draw(ui, info, People, shot.follow and Camera.follow or nil)
end

function on_destroyed(self, reason)
    People.clear()
    Props.clear()
    for _, holder in pairs(MARKET.holders) do
        despawn_entity(holder)
    end
end

-- Test hooks ---------------------------------------------------------------------

function RT.status()
    local counts = People.counts
    return {
        mode = MARKET.mode,
        loaded = MARKET.loaded,
        people = counts.total,
        walking = counts.walking,
        states = counts.roles,
        gate = Props.gate_text(),
        carts = Props.cart_text(),
        shot = Camera.name(),
        nav = MARKET.nav,
        thefts = ai_global().thefts or 0,
        arrests = ai_global().arrests or 0,
        wanted = ai_global().thief_wanted or false,
        news = C.news,
    }
end

function RT.gate()
    Props.toggle_gate()
    return Props.gate_text()
end

function RT.summon(x, y, z, n)
    return People.summon(x, y, z, n or 14)
end

function RT.shot(n)
    Camera.auto = false
    Camera.select(n)
    return Camera.name()
end

function RT.debug(on)
    MARKET.debug = { mesh = on, agents = on, paths = on, obstacles = on, x_ray = false }
    apply_debug()
end

function RT.ai(on)
    MARKET.ai_debug = on
    ai_debug_draw(on)
    Hud.labels = on
end

-- Make the thief look for a purse to lift now (he still waits for no guard to be near).
function RT.theft()
    if People.thief and People.thief.id then
        return ai_send(People.thief.id, "strike")
    end
    return false
end

function RT.tag()
    return maybe_start_tag(true)
end
