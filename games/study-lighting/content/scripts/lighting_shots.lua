-- Study Lighting: fixed camera shots of the chess study, for comparing the lighting
-- modes (raster, ray-traced global illumination, path tracing) on the same views.
-- Path tracing is the reference the other two should look like.
--
-- The lighting mode is a player setting, so switch it on the settings page; the
-- overlay names the mode in use. Depth of field is off here so it cannot blur the
-- differences.
--
-- Keys: 1-7 pick a shot, ArrowLeft / ArrowRight step through them, H hides the overlay.
-- Test hooks (lua_eval on this entity): SHOTS.list(), SHOTS.go(3 or "fireplace"),
-- SHOTS.current().

-- The room spans x -2.4..2.4 and z -2.4..2.6 with the floor at y 0: the sash windows
-- (and the sun) on the -x wall, the fireplace on the -z wall, the floor lamp in the
-- +x/-z corner. The board sits on the games table at about (0, 0.77, 0).
local SHOTS_LIST = {
    { name = "board" }, -- the scene's own camera pose
    { name = "overhead", eye = { 0.05, 1.72, 0.2 }, look = { 0, 0.77, 0 } },
    { name = "sunlit windows", eye = { 1.6, 1.35, 1.9 }, look = { -2.4, 0.9, -0.4 } },
    { name = "fireplace", eye = { 0.9, 1.15, 0.9 }, look = { 0, 0.75, -2.3 } },
    -- The wall facing the windows gets little direct light: mostly bounced light.
    { name = "far wall", eye = { -1.6, 1.25, 0.9 }, look = { 2.4, 0.8, -0.4 } },
    { name = "decanter", eye = { -0.8, 0.95, -0.1 }, look = { -1.3, 0.72, -0.58 } },
    { name = "the room", eye = { 2.15, 1.75, 2.35 }, look = { -1.0, 0.7, -1.1 } },
}

local STATE = { index = 1, overlay = true }

local function lighting_mode()
    if path_traced() then return "path traced" end
    if ray_traced_gi() then return "ray-traced GI" end
    return "raster"
end

local function find_shot(which)
    if type(which) == "number" then
        return SHOTS_LIST[which] and which
    end
    for i, shot in ipairs(SHOTS_LIST) do
        if shot.name == which then return i end
    end
end

local function go(which)
    local index = find_shot(which)
    if not index or not STATE.camera then return false end
    local shot = SHOTS_LIST[index]
    local eye, look = shot.eye or STATE.home_eye, shot.look or STATE.home_look
    set_translation(STATE.camera, eye[1], eye[2], eye[3])
    look_at(STATE.camera, look[1], look[2], look[3])
    STATE.index = index
    return true
end

function on_created(self)
    STATE.camera = find_entity_by_name("GameCamera")
    if not STATE.camera then
        log_warn("Study Lighting: no GameCamera in the scene")
        return
    end
    local p = get_world_translation(STATE.camera)
    local f = get_forward(STATE.camera)
    STATE.home_eye = { p.x, p.y, p.z }
    STATE.home_look = { p.x + f.x, p.y + f.y, p.z + f.z }
    go(1)
end

function update(self, dt)
    for i = 1, #SHOTS_LIST do
        if is_key_just_pressed(tostring(i)) then go(i) end
    end
    if is_key_just_pressed("ArrowRight") then go(STATE.index % #SHOTS_LIST + 1) end
    if is_key_just_pressed("ArrowLeft") then go((STATE.index - 2) % #SHOTS_LIST + 1) end
    if is_key_just_pressed("H") then STATE.overlay = not STATE.overlay end
end

function on_ui(self, ui)
    if not STATE.overlay then return end
    local shot = SHOTS_LIST[STATE.index]
    ui:screen_label("top_left", 20, 16,
        ("%d/%d  %s   |   %s"):format(STATE.index, #SHOTS_LIST, shot.name, lighting_mode()),
        { color = { 0.95, 0.93, 0.88, 0.9 }, size = 14 })
    ui:screen_label("bottom_left", 20, 14, "1-7 / arrows: shot   H: hide",
        { color = { 0.85, 0.83, 0.78, 0.55 }, size = 11 })
end

SHOTS = {
    list = function()
        local names = {}
        for i, shot in ipairs(SHOTS_LIST) do names[i] = shot.name end
        return names
    end,
    go = go,
    current = function()
        return { index = STATE.index, name = SHOTS_LIST[STATE.index].name, mode = lighting_mode() }
    end,
}
