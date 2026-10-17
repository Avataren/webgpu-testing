-- Lantern Market's director: loads the cast, fills the market with people walking the
-- navmesh, runs the carts and the gate, the camera and the HUD.
--
-- Steer it with lua_eval on this entity: MARKET holds the state, and RT.status(),
-- RT.gate(), RT.summon(x, y, z, n), RT.shot(n), RT.debug(on) are test hooks.

local C = require("market/core")
local People = require("market/people")
local Props = require("market/props")
local Camera = require("market/camera")
local Hud = require("market/hud")

-- Exports keep what scripts name as literals: the cast's folders (filled by
-- tools/fetch_cast.py, listed in cast_data.lua).
local CAST_FOLDERS = { "content/characters/", "content/animations/" }

local CROWD = 80
local IMPORTS_PER_FRAME = 2

MARKET = {
    mode = "boot",
    holders = {},
    queue = {},
    importing = {},
    loaded = 0,
    debug = { mesh = false, agents = false, paths = false, obstacles = false, x_ray = false },
    nav = { polys = 0, carving = 0 },
    frame = 0,
}
RT = {}

local function boot()
    if not C.load_cast() then
        MARKET.mode = "nocast"
        return
    end
    for _, char in ipairs(C.characters) do
        local holder = spawn_entity("Template " .. char.name)
        set_component(holder, "Visible", false)
        set_translation(holder, 0, -60, 0)
        MARKET.holders[char.id] = holder
        table.insert(MARKET.queue, char)
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

local function start_market()
    MARKET.pullers = {}
    People.spawn_all(CROWD)
    Props.setup(spawn_puller)
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
    if is_key_just_pressed("G") then
        Props.toggle_gate()
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
end

function update(self, dt)
    MARKET.frame = MARKET.frame + 1
    if MARKET.mode == "boot" then
        boot()
    elseif MARKET.mode == "loading" then
        load_step()
    elseif MARKET.mode == "running" then
        People.update(dt)
        Props.update(dt)
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
    end
    info.people, info.walking, info.lingering = People.census()
    info.polys = MARKET.nav.polys
    info.carving = MARKET.nav.carving
    Hud.draw(ui, info)
end

function on_destroyed(self, reason)
    People.clear()
    for _, holder in pairs(MARKET.holders) do
        despawn_entity(holder)
    end
    for _, e in ipairs(MARKET.pullers or {}) do
        despawn_entity(e)
    end
end

-- Test hooks ---------------------------------------------------------------------

function RT.status()
    local people, walking, lingering = People.census()
    return {
        mode = MARKET.mode,
        loaded = MARKET.loaded,
        people = people,
        walking = walking,
        lingering = lingering,
        gate = Props.gate_text(),
        carts = Props.cart_text(),
        shot = Camera.name(),
        nav = MARKET.nav,
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
