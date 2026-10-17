-- Marionette's director: loads the cast, runs the show part by part (once per graphics
-- configuration when benchmarking), moves the camera and the stage lights, and draws
-- the HUD. The parts live in show/part_*.lua, shared state in show/core.lua.
--
-- Inspect or steer it with lua_eval on this entity: SHOW holds the state, and
-- RT.start("show" | "benchmark" | "raster"), RT.part(n), RT.scale(n) jump around.

local S = require("show/core")
local B = require("show/bench")
local H = require("show/hud")

local PARTS = {
    require("show/part_rollcall"),
    require("show/part_dance"),
    require("show/part_marathon"),
    require("show/part_battle"),
    require("show/part_stadium"),
    require("show/part_finale"),
}

-- Exports keep what scripts name as literals: the cast's folders (filled by
-- tools/fetch_cast.py, listed in cast_data.lua).
local CAST_FOLDERS = { "content/characters/", "content/animations/" }

local AUTOSTART = 12
local CARD_TIME = 4.5
local PASS_CARD_TIME = 3.5
local IMPORTS_PER_FRAME = 1

SHOW = {
    mode = "boot",
    time = 0,
    loaded = 0,
    holders = {},
    live = { fps = 0, frame_ms = 0, cpu = {}, characters = 0, bones = 0, skinned_vertices = 0, draw_calls = 0 },
}
RT = {}

local function find_scene_entities()
    SHOW.camera = find_entity_by_name("GameCamera")
    SHOW.spots = {}
    for i = 1, 4 do
        local spot = find_entity_by_name("Spot " .. i)
        if spot then
            table.insert(SHOW.spots, spot)
        end
    end
end

local function apply_camera(eye, target, fov)
    if not SHOW.camera then
        return
    end
    set_translation(SHOW.camera, eye[1], eye[2], eye[3])
    look_at(SHOW.camera, target[1], target[2], target[3])
    if fov and fov ~= SHOW.fov then
        SHOW.fov = fov
        set_camera(SHOW.camera, { fov_deg = fov })
    end
end

-- Four coloured spots on a ring above the arena, each sweeping its own figure over the
-- floor; their colours drift with the show.
local function animate_spots(time)
    for i, spot in ipairs(SHOW.spots) do
        local phase = i * 1.57
        local x = math.sin(time * 0.37 + phase) * 9 + math.sin(time * 0.11) * 4
        local z = math.cos(time * 0.29 + phase * 1.3) * 9
        look_at(spot, x, 0, z)
        local hue = (time * 0.02 + i * 0.25) % 1
        local r = 0.5 + 0.5 * math.cos(S.TAU * hue)
        local g = 0.5 + 0.5 * math.cos(S.TAU * (hue - 1 / 3))
        local b = 0.5 + 0.5 * math.cos(S.TAU * (hue - 2 / 3))
        set_light(spot, { color = { r, g, b } })
    end
end

-- Loading ------------------------------------------------------------------------

local function boot()
    find_scene_entities()
    -- The show runs raster (shadow maps, screen-space effects, baked light probes); the
    -- player's own settings come back when it stops.
    if not SHOW.original then
        SHOW.original = graphics_settings()
        SHOW.original.capabilities = nil
        set_graphics_settings(B.SHOW_SETTINGS, { save = false })
    end
    if not S.load_cast() then
        SHOW.mode = "nocast"
        return
    end
    for _, char in ipairs(S.characters) do
        local holder = spawn_entity("Template " .. char.name)
        set_component(holder, "Visible", false)
        SHOW.holders[char.id] = holder
    end
    SHOW.queue = {}
    for _, char in ipairs(S.characters) do
        table.insert(SHOW.queue, char)
    end
    SHOW.importing = {}
    SHOW.mode = "loading"
end

local function load_step()
    -- Start the next imports (the holders exist from the frame after boot).
    for _ = 1, IMPORTS_PER_FRAME do
        local char = table.remove(SHOW.queue, 1)
        if not char then
            break
        end
        import_gltf(SHOW.holders[char.id], char.path, 1.0)
        SHOW.importing[char.id] = char
        SHOW.current = char.name
    end
    -- A character is in once its import has put the model below its holder.
    for id, char in pairs(SHOW.importing) do
        local children = get_children(SHOW.holders[id])
        if children and #children > 0 then
            S.templates[id] = SHOW.holders[id]
            SHOW.importing[id] = nil
            SHOW.loaded = SHOW.loaded + 1
        end
    end
    if #SHOW.queue == 0 and next(SHOW.importing) == nil then
        SHOW.mode = "title"
        SHOW.title_time = 0
        local clips = 0
        for _ in pairs(S.clips) do
            clips = clips + 1
        end
        SHOW.clip_count = clips
    end
end

-- Running the show --------------------------------------------------------------

local run

local function begin_part(index)
    run.part_index = index
    run.part_time = 0
    local part = PARTS[index]
    part.enter()
    if run.benchmark then
        B.begin_part(part.name)
    end
end

local function end_part()
    local part = PARTS[run.part_index]
    if run.benchmark then
        B.end_part()
    end
    local next_part = PARTS[run.part_index + 1]
    if not (part.keep and next_part) then
        S.clear()
    end
    if part.exit then
        part.exit()
    end
end

local function begin_pass(index)
    run.pass_index = index
    run.pass_card = PASS_CARD_TIME
    if run.benchmark then
        B.begin_pass(run.passes[index])
    end
end

local function start(kind)
    S.clear()
    run = { benchmark = kind ~= "show", time = 0 }
    if run.benchmark then
        run.passes = B.passes(kind == "benchmark")
        B.start(run.passes, S.scale)
        begin_pass(1)
    else
        run.passes = { { name = "SHOW" } }
        run.pass_index = 1
        run.pass_card = 0
        begin_part(1)
    end
    SHOW.mode = "running"
    SHOW.run = run
end

local function finish()
    if run.benchmark then
        local size = { width = 0, height = 0 }
        SHOW.report = B.finish({ characters_in_cast = SHOW.loaded, clips_in_cast = SHOW.clip_count })
        SHOW.mode = "results"
    else
        SHOW.mode = "title"
        SHOW.title_time = 0
    end
    run = nil
    SHOW.run = nil
end

local function run_step(dt, stats)
    run.time = run.time + dt
    if run.pass_card > 0 then
        -- Between passes: let the new settings settle before anything is measured.
        run.pass_card = run.pass_card - dt
        apply_camera({ 0, 7, 26 }, { 0, 1, 0 }, 45)
        if run.pass_card <= 0 then
            begin_part(1)
        end
        return
    end
    local part = PARTS[run.part_index]
    run.part_time = run.part_time + dt
    part.update(run.part_time, dt)
    apply_camera(part.camera(run.part_time))
    if run.benchmark then
        B.sample(run.part_time, stats, #S.actors, S.bones)
    end
    if run.part_time >= part.duration then
        end_part()
        if PARTS[run.part_index + 1] then
            begin_part(run.part_index + 1)
        elseif run.pass_index < #run.passes then
            begin_pass(run.pass_index + 1)
        else
            finish()
        end
    end
end

-- Hooks --------------------------------------------------------------------------

local function sample_live(dt, stats)
    local live = SHOW.live
    local k = math.min(1, dt * 4)
    live.fps = live.fps + ((stats.fps or 0) - live.fps) * k
    live.frame_ms = live.frame_ms + ((stats.frame_ms or 0) - live.frame_ms) * k
    if stats.gpu_ms then
        live.gpu_ms = (live.gpu_ms or stats.gpu_ms) + (stats.gpu_ms - (live.gpu_ms or stats.gpu_ms)) * k
    end
    for stage, ms in pairs(stats.cpu) do
        live.cpu[stage] = (live.cpu[stage] or ms) + (ms - (live.cpu[stage] or ms)) * k
    end
    live.characters = #S.actors
    live.bones = S.bones
    live.skinned_vertices = stats.skinned_vertices or 0
    live.draw_calls = stats.draw_calls or 0
end

function on_created(self)
    SHOW.mode = "boot"
end

function update(self, dt)
    SHOW.time = SHOW.time + dt
    local stats = frame_stats()
    sample_live(dt, stats)
    if SHOW.camera and SHOW.spots then
        animate_spots(SHOW.time)
    end
    if is_key_just_pressed("F1") then
        SHOW.show_stats = not SHOW.show_stats
    end

    if SHOW.mode == "boot" then
        boot()
    elseif SHOW.mode == "loading" then
        load_step()
        apply_camera({ 0, 9, 30 }, { 0, 1, 0 }, 45)
    elseif SHOW.mode == "title" then
        SHOW.title_time = SHOW.title_time + dt
        apply_camera({ math.sin(SHOW.time * 0.05) * 30, 9, math.cos(SHOW.time * 0.05) * 30 }, { 0, 1, 0 }, 45)
        for key, scale in pairs({ ["1"] = 1, ["2"] = 2, ["3"] = 3 }) do
            if is_key_just_pressed(key) then
                S.scale = scale
                SHOW.title_time = 0
            end
        end
        if is_key_just_pressed("Return") or is_key_just_pressed("Space") or SHOW.title_time >= AUTOSTART then
            start("show")
        elseif is_key_just_pressed("B") then
            start("benchmark")
        elseif is_key_just_pressed("R") then
            start("raster")
        end
    elseif SHOW.mode == "running" then
        run_step(dt, stats)
    elseif SHOW.mode == "results" then
        apply_camera({ math.sin(SHOW.time * 0.05) * 30, 9, math.cos(SHOW.time * 0.05) * 30 }, { 0, 1, 0 }, 45)
        if is_key_just_pressed("Return") then
            SHOW.mode = "title"
            SHOW.title_time = 0
        end
    end
end

function on_ui(self, ui)
    if SHOW.mode == "nocast" then
        H.message(ui, {
            "The cast is missing",
            "Copy it from the asset library first:",
            "games/marionette/tools/fetch_cast.py",
            "(see the README; the library is filled with tools/library/library.py)",
        })
        return
    elseif SHOW.mode == "loading" or SHOW.mode == "boot" then
        H.loading(ui, SHOW.loaded, #S.characters, SHOW.current)
        return
    elseif SHOW.mode == "title" then
        H.title(ui, {
            time = SHOW.time,
            scale = S.scale,
            autostart = AUTOSTART - SHOW.title_time,
            subtitle = string.format("a skeletal animation show and crowd benchmark   *   %d characters   *   %d clips",
                SHOW.loaded, SHOW.clip_count or 0),
        })
        return
    elseif SHOW.mode == "results" then
        H.results(ui, SHOW.report or {})
        return
    end

    local r = SHOW.run
    if not r then
        return
    end
    if r.pass_card > 0 then
        local pass = r.passes[r.pass_index]
        H.message(ui, {
            string.format("PASS %d / %d:  %s", r.pass_index, #r.passes, pass.name),
            pass.note or "",
        })
        return
    end
    local part = PARTS[r.part_index]
    local t = r.part_time
    if part.labels then
        part.labels(ui, t)
    end
    local alpha = math.min(1, t / 0.4) * math.min(1, math.max(0, (CARD_TIME - t) / 0.8))
    H.card(ui, r.part_index, part.name, part.subtitle, alpha,
        string.format("%s characters", H.thousands(#S.actors)))
    if r.part_index == #PARTS then
        local k = math.max(0, math.min(1, (t - part.duration + 6) / 2))
        if k > 0 then
            ui:screen_label("center", 0, -40, "M A R I O N E T T E", { size = 64, color = { 0.35, 0.95, 1.0, k } })
            ui:screen_label("center", 0, 20, "thanks for watching", { size = 20, color = { 1, 1, 1, k } })
        end
    end
    if r.benchmark or SHOW.show_stats then
        local live = SHOW.live
        H.stats(ui, {
            pass = r.benchmark and r.passes[r.pass_index].name or nil,
            pass_index = r.pass_index,
            pass_count = #r.passes,
            fps = live.fps,
            frame_ms = live.frame_ms,
            gpu_ms = live.gpu_ms,
            cpu = live.cpu,
            characters = live.characters,
            bones = live.bones,
            skinned_vertices = live.skinned_vertices,
            draw_calls = live.draw_calls,
        })
    end
end

function on_destroyed(self, reason)
    B.abort()
    if SHOW.original then
        set_graphics_settings(SHOW.original, { save = false })
        SHOW.original = nil
    end
    S.clear()
    for _, holder in pairs(SHOW.holders) do
        despawn_entity(holder)
    end
    SHOW.holders = {}
    S.templates = {}
end

-- Test hooks (lua_eval on the Director) ---------------------------------------------

function RT.start(kind)
    if SHOW.mode == "title" or SHOW.mode == "results" or SHOW.mode == "running" then
        if run then
            B.abort()
        end
        start(kind or "show")
        return true
    end
    return false
end

-- Jump to part n of the running show.
function RT.part(n)
    if not run or not PARTS[n] then
        return false
    end
    if run.part_index and run.pass_card <= 0 then
        end_part()
    end
    S.clear()
    run.pass_card = 0
    begin_part(n)
    return true
end

function RT.scale(n)
    S.scale = n
    return S.scale
end

function RT.status()
    return {
        mode = SHOW.mode,
        loaded = SHOW.loaded,
        part = run and PARTS[run.part_index] and PARTS[run.part_index].name,
        part_time = run and run.part_time,
        actors = #S.actors,
        bones = S.bones,
        fps = SHOW.live.fps,
    }
end
