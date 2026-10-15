-- The Long Game: XavChess plays itself in the study. Press Play in the editor (or run
-- the project in the player) to watch.
--
-- Every move comes from the UCI engine (chess_uci.lua); chess_rules.lua only keeps
-- the position. The board is read from the scene: the square size from the white
-- rooks, the centre from the 32 starting pieces (Chess.<White|Black>.<kind>.<square>),
-- capture rails from Chess.Captures.White / .Black, last-move markers Move.Inlay.0-7.
-- A camera director cycles through slow shots around the board.
--
-- Keys: Space pause, R new game, C camera director on/off, M sound.
-- Test hooks (lua_eval on this entity): RT.status(), RT.speed(4), RT.search(200).

local rules = require("chess_rules")
local uci = require("chess_uci")

local MOVE_SECONDS = 0.9
local AFTER_MOVE = 0.55
local AFTER_GAME = 7.0
local PLY_LIMIT = 400
local SOUND_VOLUME = 0.28
local THINK_MILLIS = 2500   -- the engine's search time per move

GAME = nil

-- ---------------------------------------------------------------------------
-- Scene mapping
-- ---------------------------------------------------------------------------

local function transform_of(entity)
    local t = get_component(entity, "Transform")
    return t and { x = t.translation[1], y = t.translation[2], z = t.translation[3] }
end

local function map_scene()
    local found, named = {}, {}
    for _, e in ipairs(query_entities_with_component("Name")) do
        local name = get_component(e, "Name")
        if type(name) == "string" then
            named[name] = e
            local color, kind = name:match("^Chess%.(%a+)%.(%a+)%.")
            if (color == "White" or color == "Black") and has_component(e, "Mesh") then
                found[#found + 1] = { entity = e, white = color == "White", kind = kind, pos = transform_of(e) }
            end
        end
    end
    if #found ~= 32 then
        return nil, ("Expected 32 pieces, found %d"):format(#found)
    end
    -- Board frame: centre from the symmetric start, square size from the white rooks.
    local c = { x = 0, y = 0, z = 0 }
    local rook_x = {}
    for _, p in ipairs(found) do
        c.x, c.y, c.z = c.x + p.pos.x / 32, c.y + p.pos.y / 32, c.z + p.pos.z / 32
        if p.white and p.kind == "rook" then rook_x[#rook_x + 1] = p.pos.x end
    end
    if #rook_x ~= 2 then return nil, "Cannot find the two white rooks" end
    local s = math.abs(rook_x[1] - rook_x[2]) / 7
    local frame = { center = c, square = s }
    local board = rules.start_board()
    local templates = {}
    for _, p in ipairs(found) do
        local file = math.floor((p.pos.x - c.x) / s + 3.5 + 0.5)
        local rank = math.floor(3.5 - (p.pos.z - c.z) / s + 0.5)
        local sq = rank * 8 + file
        local slot = board[sq]
        if file < 0 or file > 7 or rank < 0 or rank > 7 or not slot or slot.white ~= p.white
            or slot.kind ~= p.kind or slot.entity then
            return nil, "Piece does not match the starting position: " .. get_component(p.entity, "Name")
        end
        slot.entity = p.entity
        p.square = sq
        local rot = get_component(p.entity, "Transform").rotation
        p.home = { x = p.pos.x, y = p.pos.y, z = p.pos.z }
        p.rotation = rot
        templates[p.kind] = templates[p.kind] or p.entity
    end
    frame.board = board
    frame.pieces = found
    frame.templates = templates
    -- Captured pieces: lines of four from each anchor, stepping away from the board.
    frame.rails = {}
    for _, white in ipairs({ true, false }) do
        local anchor = named[white and "Chess.Captures.White" or "Chess.Captures.Black"]
        local origin = anchor and transform_of(anchor)
        if origin then
            local ax = origin.x >= c.x and 1 or -1
            local az = origin.z >= c.z and 1 or -1
            frame.rails[white] = { origin = origin, along = ax * s * 0.82, across = az * s * 0.9 }
        else
            local side = white and -1 or 1
            frame.rails[white] = { origin = { x = c.x + side * 5.2 * s, y = c.y, z = c.z + 3.9 * s },
                                   along = side * s * 0.82, across = -s * 0.9 }
        end
    end
    frame.markers = {}
    for i = 0, 7 do
        local m = named["Move.Inlay." .. i]
        if m then
            frame.markers[#frame.markers + 1] = m
            frame.marker_lift = frame.marker_lift or (transform_of(m).y - c.y)
        end
    end
    frame.marker_lift = frame.marker_lift or s * 0.01
    -- World centre: pieces sit under a parent that only moves them.
    local any = found[1]
    local world = get_world_translation(any.entity)
    frame.world_center = { x = world.x - any.pos.x + c.x, y = world.y - any.pos.y + c.y,
                           z = world.z - any.pos.z + c.z }
    frame.camera = named["GameCamera"]
    return frame
end

local function square_position(frame, sq)
    local s, c = frame.square, frame.center
    return { x = c.x + (sq % 8 - 3.5) * s, y = c.y, z = c.z + (3.5 - sq // 8) * s }
end

local function rail_slot(frame, white, index)
    local r = frame.rails[white]
    return { x = r.origin.x + r.along * (index % 4), y = r.origin.y, z = r.origin.z + r.across * (index // 4) }
end

-- ---------------------------------------------------------------------------
-- Game flow
-- ---------------------------------------------------------------------------

-- Recorded takes per sound (tools/study/sounds.py); a random one plays each time.
local TAKES = { chess_move = 6, chess_capture = 4, chess_check = 2, chess_end = 1, chess_clock = 4 }

local function sound(name, volume)
    if GAME.sound then
        local take = math.random(TAKES[name] or 1)
        play_sound(("content/audio/%s_%d.wav"):format(name, take), { volume = volume or SOUND_VOLUME })
    end
end

local function show_markers(visible)
    for _, m in ipairs(GAME.frame.markers) do
        set_component(m, "Visible", visible)
    end
end

local function reset_board()
    local f = GAME.frame
    for _, p in ipairs(f.pieces) do
        set_translation(p.entity, p.home.x, p.home.y, p.home.z)
        set_component(p.entity, "Visible", true)
        if p.kind == "pawn" and GAME.promoted[p.entity] then
            set_mesh(p.entity, GAME.pawn_template)
        end
    end
    GAME.promoted = {}
    f.board = rules.start_board()
    for _, p in ipairs(f.pieces) do f.board[p.square].entity = p.entity end
    show_markers(false)
end

local function new_game()
    GAME.number = GAME.number + 1
    GAME.position = rules.new_game()
    GAME.position.board = GAME.frame.board
    GAME.captures = { [true] = 0, [false] = 0 }
    GAME.result = nil
    GAME.animation = nil
    GAME.delay = 1.2
    GAME.eval = 0
    GAME.status = "Setting up"
    GAME.uci:new_game()
end

local function restart()
    reset_board()
    new_game()
end

local function animate(delta)
    local f = GAME.frame
    local s = f.square
    local motions = {}
    local function motion(entity, target, lift)
        local from = transform_of(entity)
        motions[#motions + 1] = { entity = entity, from = from, to = target, lift = lift }
    end
    if delta.captured then
        local white = delta.captured.white
        local index = GAME.captures[white]
        GAME.captures[white] = index + 1
        motion(delta.captured.entity, rail_slot(f, white, index), 0.83 * s)
    end
    motion(delta.piece.entity, square_position(f, delta.to), 0.4 * s)
    if delta.rook then
        motion(delta.rook.piece.entity, square_position(f, delta.rook.to), 0.27 * s)
    end
    if delta.promotion then
        set_mesh(delta.piece.entity, f.templates[delta.promotion])
        GAME.promoted[delta.piece.entity] = true
    end
    -- Edges of the from and to squares: 0/1 along files, 2/3 along ranks.
    for i, m in ipairs(f.markers) do
        local index = i - 1
        local sq = index < 4 and delta.from or delta.to
        local p = square_position(f, sq)
        local edge = index % 4
        local o = 0.4667 * s
        local dx = edge == 2 and -o or (edge == 3 and o or 0)
        local dz = edge == 0 and -o or (edge == 1 and o or 0)
        set_translation(m, p.x + dx, p.y + f.marker_lift, p.z + dz)
        set_component(m, "Visible", true)
    end
    GAME.animation = { motions = motions, elapsed = 0, capture = delta.captured ~= nil }
    GAME.focus = square_position(f, delta.to)
end

local function finish(result)
    GAME.result = result
    GAME.status = result
    GAME.delay = AFTER_GAME
    local score = result:find("^Ivory") and 1 or (result:find("^Ebony") and 2 or 3)
    GAME.score[score] = GAME.score[score] + 1
    sound("chess_end")
    log_info(("Game %d finished after %d plies: %s"):format(GAME.number, #GAME.position.history, result))
end

local function on_best(best)
    if best == nil or best == "(none)" or best == "0000" then
        finish(rules.no_move_result(GAME.position))
        return
    end
    local delta, err = rules.apply(GAME.position, best)
    if not delta then
        GAME.error = err
        GAME.paused = true
        log_error("Chess: " .. err)
        return
    end
    animate(delta)
end

local function tick_animation(dt)
    local a = GAME.animation
    a.elapsed = a.elapsed + dt
    local t = math.min(1, a.elapsed / MOVE_SECONDS)
    local e = t * t * (3 - 2 * t)
    local arc = math.sin(math.pi * t)
    for _, m in ipairs(a.motions) do
        set_translation(m.entity,
            m.from.x + (m.to.x - m.from.x) * e,
            m.from.y + (m.to.y - m.from.y) * e + arc * m.lift,
            m.from.z + (m.to.z - m.from.z) * e)
    end
    if t >= 1 then
        GAME.animation = nil
        GAME.delay = AFTER_MOVE
        sound(a.capture and "chess_capture" or "chess_move")
        -- The player who moved presses the clock a moment later.
        GAME.clock_press = 0.32
        local draw = rules.draw_result(GAME.position, PLY_LIMIT)
        if draw then
            finish(draw)
        elseif rules.in_check(GAME.position, GAME.position.white_to_move) then
            sound("chess_check")
        end
    end
end

-- ---------------------------------------------------------------------------
-- Camera director
-- ---------------------------------------------------------------------------

local HOLD = 13.0
local GLIDE = 3.2

local function shots(frame)
    local c = frame.world_center
    local function at(dx, dy, dz) return { x = c.x + dx, y = c.y + dy, z = c.z + dz } end
    return {
        { name = "authored" },
        { name = "ivory's shoulder", eye = at(0.22, 0.36, 0.58), look = at(0, -0.02, -0.06), swing = 0.1 },
        { name = "overhead", eye = at(0.05, 0.95, 0.2), look = at(0, 0, 0), swing = 0.12 },
        { name = "behind the clock", eye = at(-0.82, 0.16, 0.3), look = at(0.05, 0.0, -0.02), swing = 0.06 },
        { name = "ebony's side", eye = at(-0.32, 0.4, -0.62), look = at(0, 0, 0.04), swing = 0.1 },
        { name = "across the ranks", eye = at(0.62, 0.09, -0.05), look = at(-0.1, 0.03, 0), swing = 0.08 },
        { name = "the room", eye = at(1.55, 0.78, 1.75), look = at(-0.5, -0.1, -0.6), swing = 0.05 },
    }
end

local function swing_eye(eye, look, angle)
    -- Turn the eye around the vertical line through what it looks at.
    local dx, dz = eye.x - look.x, eye.z - look.z
    local ca, sa = math.cos(angle), math.sin(angle)
    return { x = look.x + dx * ca - dz * sa, y = eye.y, z = look.z + dx * sa + dz * ca }
end

local function shot_pose(shot, t)
    -- t runs 0..1 over the hold: a slow swing from one side to the other.
    if shot.name == "authored" then
        local d = GAME.director
        return swing_eye(d.home_eye, d.home_look, math.sin(t * math.pi * 2) * 0.18), d.home_look
    end
    return swing_eye(shot.eye, shot.look, (t - 0.5) * 2 * shot.swing), shot.look
end

local function lerp3(a, b, t)
    return { x = a.x + (b.x - a.x) * t, y = a.y + (b.y - a.y) * t, z = a.z + (b.z - a.z) * t }
end

local function update_camera(dt)
    local d = GAME.director
    local cam = GAME.frame.camera
    if not cam then return end
    d.time = d.time + dt
    local shot = d.shots[d.index]
    local eye, look = shot_pose(shot, math.min(1, d.time / (HOLD + GLIDE)))
    if d.from then
        -- Glide from where the last shot ended.
        local t = math.min(1, d.time / GLIDE)
        local e = t * t * t * (t * (t * 6 - 15) + 10)
        eye = lerp3(d.from.eye, eye, e)
        look = lerp3(d.from.look, look, e)
        if t >= 1 then d.from = nil end
    end
    if not d.enabled then
        eye, look = d.home_eye, d.home_look
    end
    set_translation(cam, eye.x, eye.y, eye.z)
    look_at(cam, look.x, look.y, look.z)
    -- Keep the lens focused on what the shot looks at.
    local dist = math.sqrt((eye.x - look.x) ^ 2 + (eye.y - look.y) ^ 2 + (eye.z - look.z) ^ 2)
    if math.abs(dist - (d.focus or 0)) > 0.01 then
        d.focus = dist
        set_postprocess({ depth_of_field = { focus_distance = dist } })
    end
    if d.enabled and d.time >= HOLD + GLIDE then
        d.from = { eye = eye, look = look }
        d.index = d.index % #d.shots + 1
        d.time = 0
    end
end

-- ---------------------------------------------------------------------------
-- Hooks
-- ---------------------------------------------------------------------------

function on_created(self)
    local frame, err = map_scene()
    GAME = {
        frame = frame, error = err, number = 0, score = { 0, 0, 0 }, speed = 1, millis = THINK_MILLIS,
        sound = true, paused = err ~= nil, promoted = {}, status = "Starting the engine",
        uci = uci.new(),
    }
    if not frame then
        log_error("Chess: " .. err)
        return
    end
    -- A hidden copy of the pawn model, to turn promoted pawns back.
    GAME.pawn_template = spawn_entity("Chess.PawnTemplate")
    set_mesh(GAME.pawn_template, frame.templates.pawn)
    set_component(GAME.pawn_template, "Visible", false)
    show_markers(false)
    local home_eye, home_look
    if frame.camera then
        local p = get_world_translation(frame.camera)
        local fwd = get_forward(frame.camera)
        local c = frame.world_center
        local reach = math.sqrt((p.x - c.x) ^ 2 + (p.y - c.y) ^ 2 + (p.z - c.z) ^ 2)
        home_eye = { x = p.x, y = p.y, z = p.z }
        home_look = { x = p.x + fwd.x * reach, y = p.y + fwd.y * reach, z = p.z + fwd.z * reach }
    end
    GAME.director = { enabled = true, index = 1, time = 0, shots = shots(frame),
                      home_eye = home_eye, home_look = home_look }
    if not GAME.uci:start() then
        GAME.error = GAME.uci.error
        GAME.paused = true
        log_error("Chess: " .. GAME.error)
    end
    new_game()
end

function on_destroyed(self, reason)
    if GAME and GAME.uci then GAME.uci:stop() end
    if GAME and GAME.pawn_template then despawn_entity(GAME.pawn_template) end
end

function update(self, dt)
    if not GAME or not GAME.frame then return end
    if is_key_just_pressed("Space") then GAME.paused = not GAME.paused end
    if is_key_just_pressed("R") then restart() end
    if is_key_just_pressed("C") then GAME.director.enabled = not GAME.director.enabled end
    if is_key_just_pressed("M") then GAME.sound = not GAME.sound end
    if GAME.director.home_eye then update_camera(dt) end

    local reply = GAME.uci:poll(dt)
    if reply and reply.restarted then
        GAME.status = GAME.uci.error or "Restarting the engine"
        GAME.thinking = false
        GAME.reconnect = true
    end
    if GAME.reconnect and GAME.uci:ready() then
        GAME.reconnect = false
        GAME.uci:new_game()
    end
    if GAME.paused then return end
    dt = dt * GAME.speed
    if reply and reply.best and GAME.thinking then
        GAME.thinking = false
        local info = reply.info or {}
        if info.cp then
            GAME.eval = (GAME.position.white_to_move and 1 or -1) * info.cp / 100
        elseif info.mate then
            GAME.eval = (GAME.position.white_to_move and 1 or -1) * (info.mate > 0 and 99 or -99)
        end
        GAME.pending = reply.best
    end
    if GAME.clock_press then
        GAME.clock_press = GAME.clock_press - dt
        if GAME.clock_press <= 0 then
            GAME.clock_press = nil
            sound("chess_clock", SOUND_VOLUME * 0.7)
        end
    end
    if GAME.animation then
        tick_animation(dt)
        return
    end
    GAME.delay = math.max(0, (GAME.delay or 0) - dt)
    if GAME.delay > 0 then return end
    if GAME.result then
        restart()
        return
    end
    if GAME.pending then
        local best = GAME.pending
        GAME.pending = nil
        on_best(best)
        return
    end
    if not GAME.thinking and GAME.uci:ready() then
        if GAME.uci:search(GAME.position.history, GAME.millis) then
            GAME.thinking = true
            GAME.status = "Considering the next move"
        end
    end
end

-- ---------------------------------------------------------------------------
-- HUD
-- ---------------------------------------------------------------------------

local GOLD = { 0.80, 0.69, 0.45, 1 }
local PAPER = { 0.93, 0.90, 0.83, 1 }
local DIM = { 0.6, 0.6, 0.58, 1 }

function on_ui(self, ui)
    if not GAME then return end
    ui:anchored_panel("chess_title", "top_left", 24, 20, nil, nil, function(p)
        p:label("XAVCHESS  /  EXHIBITION", { color = GOLD, size = 11 })
        p:label("The Long Game", { color = PAPER, size = 26 })
    end)
    ui:anchored_panel("chess_session", "top_right", 24, 20, 250, nil, function(p)
        p:label(("GAME %03d"):format(math.max(1, GAME.number)), { color = GOLD, size = 11 })
        if GAME.error then
            p:label(GAME.error, { color = { 1, 0.5, 0.4, 1 }, size = 13 })
            return
        end
        local pos = GAME.position
        if pos then
            local ivory = pos.white_to_move and not GAME.result
            p:label((ivory and "> " or "  ") .. "Ivory", { color = ivory and PAPER or DIM, size = 16 })
            p:label((not ivory and not GAME.result and "> " or "  ") .. "Ebony",
                { color = (not ivory and not GAME.result) and PAPER or DIM, size = 16 })
            p:separator()
            local info = GAME.uci.info or {}
            local status = GAME.paused and "Paused" or GAME.status
            p:label(status, { color = PAPER, size = 13 })
            if GAME.thinking and info.depth then
                p:label(("Depth %d | %dk nodes"):format(info.depth, (info.nodes or 0) // 1000),
                    { color = DIM, size = 11 })
            end
            p:label(("Evaluation %+.2f (Ivory)"):format(GAME.eval or 0), { color = GOLD, size = 11 })
            p:separator()
            local n = #pos.notation
            local first = math.max(1, n - 11)
            if first % 2 == 0 then first = first - 1 end
            for i = first, n, 2 do
                local white = pos.notation[i] or ""
                local black = pos.notation[i + 1] or ""
                p:label(("%3d.  %-8s %s"):format((i + 1) // 2, white, black), { color = DIM, size = 12 })
            end
            p:separator()
            p:label(("Ivory %d | Ebony %d | Draws %d"):format(GAME.score[1], GAME.score[2], GAME.score[3]),
                { color = DIM, size = 11 })
        end
    end)
    ui:screen_label("bottom_left", 20, 14, "SPACE pause   R new game   C camera   M sound",
        { color = { 0.85, 0.83, 0.78, 0.55 }, size = 11 })
end

-- ---------------------------------------------------------------------------
-- Test hooks
-- ---------------------------------------------------------------------------

RT = {
    status = function()
        local pos = GAME.position or {}
        return {
            game = GAME.number, ply = pos.history and #pos.history or 0, result = GAME.result,
            error = GAME.error, engine = GAME.uci.state, thinking = GAME.thinking,
            captures = GAME.captures and { GAME.captures[true], GAME.captures[false] },
            shot = GAME.director and GAME.director.shots[GAME.director.index].name,
            last = pos.notation and pos.notation[#pos.notation],
        }
    end,
    speed = function(x) GAME.speed = x end,
    search = function(ms) GAME.millis = ms end,
}
