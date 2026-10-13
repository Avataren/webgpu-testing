-- Breakout. One controller script drives the paddle, ball, bricks, camera and HUD.
-- Attached to the GameController entity; runs in play mode only.
--
-- Keys: Left/Right or A/D move, Space launches / restarts after game over, R restarts,
-- P toggles the autopilot (it plays the game by itself, handy for testing).
--
-- The scene is built in the editor (see entity names below). Lua can't create meshes or
-- materials, so bricks, ball trail and life icons are pre-placed entities that this
-- script moves, scales and hides.

local FIELD_L, FIELD_R, FIELD_T = -8.0, 8.0, 7.0
local KILL_Y = -7.6
local PADDLE_Y, PADDLE_HW, PADDLE_HH = -6.2, 1.2, 0.2
local PADDLE_SCALE = { 2.4, 0.4, 0.7 }
local PADDLE_SPEED = 16.0
local BALL_R = 0.28
local BRICK_HW, BRICK_HH = 0.6, 0.25
local BRICK_SCALE = { 1.2, 0.5, 0.6 }
local ROW_POINTS = { 70, 60, 50, 30, 20, 10 } -- row 0 is the top (red) row
local CAM = { x = 0.0, y = -0.2, z = 21.0 }
local START_LIVES = 3
local TRAIL_COUNT = 6
local INTRO_TIME = 1.2
local DIE_TIME = 0.28

local g = {
    score = 0, best = 0, lives = START_LIVES, level = 1,
    phase = "intro", t = 0,
    px = 0.0, pv = 0.0, squash = 0,
    bx = 0.0, by = PADDLE_Y + PADDLE_HH + BALL_R, vx = 0.0, vy = 0.0,
    speed = 9.0, base_speed = 9.0, pulse = 0, shake = 0, combo = 0,
    bricks = {}, bricks_left = 0, hist = {}, aim = 0.0,
    trail_visible = nil, lives_shown = nil, ball_visible = nil,
}

GAME = g -- global so lua_eval can inspect and poke the game state while testing

local ent = {}

local function clamp(v, lo, hi)
    if v < lo then return lo elseif v > hi then return hi end
    return v
end

local function sign(v)
    if v < 0 then return -1 end
    return 1
end

local function ease_out_back(k)
    local c1 = 1.70158
    local x = k - 1
    return 1 + (c1 + 1) * x * x * x + c1 * x * x
end

local function find(name)
    local e = find_entity_by_name(name)
    if not e then log_error("breakout: entity '" .. name .. "' not found") end
    return e
end

local function set_visible(e, v)
    set_component(e, "Visible", v)
end

-- Rebuild the brick list from the Bricks entity's children (named brick_r<row>_c<col>).
local function collect_bricks()
    g.bricks = {}
    local root = find("Bricks")
    for _, e in ipairs((root and get_children(root)) or {}) do
        local r, c = string.match(get_component(e, "Name") or "", "^brick_r(%d+)_c(%d+)$")
        if r then
            local row, col = tonumber(r), tonumber(c)
            g.bricks[#g.bricks + 1] = {
                e = e, row = row, col = col,
                x = -8.0 + 0.6667 + col * 1.3333,
                y = 6.0 - row * 0.62,
                alive = true, dying = nil, spin = 1,
                delay = row * 0.07 + col * 0.012,
            }
        end
    end
end

local function level_base_speed()
    return math.min(8.5 + 0.7 * (g.level - 1), 13.0)
end

local function start_level()
    for _, b in ipairs(g.bricks) do
        b.alive, b.dying = true, nil
        set_visible(b.e, true)
        set_rotation(b.e, 0, 0, 0)
        set_translation(b.e, b.x, b.y, 0)
        set_scale(b.e, 0.001, 0.001, 0.001)
    end
    g.bricks_left = #g.bricks
    g.base_speed = level_base_speed()
    g.speed = g.base_speed
    g.combo = 0
    g.vx, g.vy = 0.0, 0.0
    g.phase, g.t = "intro", 0
end

local function new_game()
    g.score, g.lives, g.level = 0, START_LIVES, 1
    start_level()
end

local function serve()
    g.phase, g.t = "serve", 0
    g.speed = g.base_speed
    g.combo = 0
    g.hist = {}
end

local function launch()
    local a = (math.random() - 0.5) * 0.5 + g.pv * 0.01
    g.vx = g.speed * math.sin(a)
    g.vy = g.speed * math.cos(a)
    g.phase = "play"
end

local function normalize_velocity()
    -- Keep the scalar speed and make sure the ball never travels almost horizontally.
    local min_vy = 0.3 * g.speed
    if math.abs(g.vy) < min_vy then
        g.vy = sign(g.vy) * min_vy
    end
    local len = math.sqrt(g.vx * g.vx + g.vy * g.vy)
    g.vx, g.vy = g.vx / len * g.speed, g.vy / len * g.speed
end

local function lose_ball()
    g.lives = g.lives - 1
    g.shake = 1.0
    g.combo = 0
    if g.lives <= 0 then
        g.best = math.max(g.best, g.score)
        g.phase, g.t = "over", 0
    else
        serve()
    end
end

local function hit_brick(b)
    b.alive = false
    b.dying = 0
    b.spin = (math.random() < 0.5) and -1 or 1
    g.combo = g.combo + 1
    g.score = g.score + ROW_POINTS[b.row + 1] * math.min(g.combo, 4)
    g.bricks_left = g.bricks_left - 1
    g.speed = math.min(g.speed + 0.09, g.base_speed + 4.0)
    g.pulse = 1.0
    g.shake = math.max(g.shake, 0.25)
    if g.bricks_left == 0 then
        g.best = math.max(g.best, g.score)
        g.phase, g.t = "cleared", 0
    end
end

local function collide_bricks()
    for _, b in ipairs(g.bricks) do
        if b.alive and math.abs(g.bx - b.x) < BRICK_HW + BALL_R and math.abs(g.by - b.y) < BRICK_HH + BALL_R then
            local cx = clamp(g.bx, b.x - BRICK_HW, b.x + BRICK_HW)
            local cy = clamp(g.by, b.y - BRICK_HH, b.y + BRICK_HH)
            local dx, dy = g.bx - cx, g.by - cy
            if dx * dx + dy * dy < BALL_R * BALL_R then
                if dx == 0 and dy == 0 then
                    g.vy = -g.vy
                elseif math.abs(dx) / BRICK_HW > math.abs(dy) / BRICK_HH then
                    g.vx = sign(dx) * math.abs(g.vx)
                    g.bx = cx + sign(dx) * BALL_R
                else
                    g.vy = sign(dy) * math.abs(g.vy)
                    g.by = cy + sign(dy) * BALL_R
                end
                hit_brick(b)
                normalize_velocity()
                return
            end
        end
    end
end

local function collide_paddle()
    local cx = clamp(g.bx, g.px - PADDLE_HW, g.px + PADDLE_HW)
    local cy = clamp(g.by, PADDLE_Y - PADDLE_HH, PADDLE_Y + PADDLE_HH)
    local dx, dy = g.bx - cx, g.by - cy
    if g.vy >= 0 or dx * dx + dy * dy >= BALL_R * BALL_R then return end
    if g.by > PADDLE_Y then
        -- Top face: aim the rebound by where the ball landed, and add some of the paddle's motion.
        local t = clamp((g.bx - g.px) / PADDLE_HW, -1, 1)
        local angle = t * 1.05
        g.by = PADDLE_Y + PADDLE_HH + BALL_R
        g.vx = g.speed * math.sin(angle) + g.pv * 0.08
        g.vy = g.speed * math.cos(angle)
        normalize_velocity()
        g.squash = 1.0
        g.combo = 0
        g.aim = (math.random() - 0.5) * 1.6
    else
        -- Caught the side of the paddle on the way down: bounce away and keep falling.
        g.vx = sign(g.bx - g.px) * math.abs(g.vx)
        g.bx = g.px + sign(g.bx - g.px) * (PADDLE_HW + BALL_R)
    end
end

local function step_ball(dt)
    local steps = math.max(1, math.ceil(g.speed * dt / 0.12))
    local h = dt / steps
    for _ = 1, steps do
        g.bx = g.bx + g.vx * h
        g.by = g.by + g.vy * h
        if g.bx < FIELD_L + BALL_R then
            g.bx = FIELD_L + BALL_R
            g.vx = math.abs(g.vx)
        elseif g.bx > FIELD_R - BALL_R then
            g.bx = FIELD_R - BALL_R
            g.vx = -math.abs(g.vx)
        end
        if g.by > FIELD_T - BALL_R then
            g.by = FIELD_T - BALL_R
            g.vy = -math.abs(g.vy)
            normalize_velocity()
        end
        collide_paddle()
        collide_bricks()
        if g.phase ~= "play" then return end
        if g.by < KILL_Y then
            lose_ball()
            return
        end
    end
end

local function update_paddle(dt)
    local desired = 0.0
    if get_bool("autopilot", false) then
        desired = clamp((g.bx + g.aim - g.px) * 10.0, -PADDLE_SPEED, PADDLE_SPEED)
    else
        if is_key_pressed("ArrowLeft") or is_key_pressed("A") then desired = desired - PADDLE_SPEED end
        if is_key_pressed("ArrowRight") or is_key_pressed("D") then desired = desired + PADDLE_SPEED end
    end
    g.pv = g.pv + (desired - g.pv) * math.min(1.0, 14.0 * dt)
    local limit = FIELD_R - PADDLE_HW
    local nx = g.px + g.pv * dt
    if nx < -limit or nx > limit then
        nx = clamp(nx, -limit, limit)
        g.pv = 0.0
    end
    g.px = nx
end

local function animate_bricks(dt)
    if g.phase == "intro" then
        for _, b in ipairs(g.bricks) do
            local k = clamp((g.t - b.delay) / 0.55, 0, 1)
            local s = math.max(0.001, ease_out_back(k))
            set_scale(b.e, BRICK_SCALE[1] * s, BRICK_SCALE[2] * s, BRICK_SCALE[3] * s)
            set_translation(b.e, b.x, b.y, (1 - clamp(s, 0, 1)) * 5.0)
        end
    end
    for _, b in ipairs(g.bricks) do
        if b.dying then
            b.dying = b.dying + dt
            local k = b.dying / DIE_TIME
            if k >= 1 then
                set_visible(b.e, false)
                b.dying = nil
            else
                local s = (1 - k) * (1 + 0.5 * k)
                set_scale(b.e, BRICK_SCALE[1] * s, BRICK_SCALE[2] * s, BRICK_SCALE[3] * s)
                set_rotation(b.e, 0, 0, k * 1.2 * b.spin)
                set_translation(b.e, b.x, b.y, k * 1.5)
            end
        end
    end
end

local function sync_scene(dt)
    -- Paddle: roll with its motion, squash when the ball lands.
    g.squash = math.max(0, g.squash - dt * 7.0)
    local sy = PADDLE_SCALE[2] * (1 - 0.35 * g.squash)
    local sx = PADDLE_SCALE[1] * (1 + 0.08 * g.squash)
    set_translation(ent.paddle, g.px, PADDLE_Y + PADDLE_HH - sy * 0.5, 0)
    set_scale(ent.paddle, sx, sy, PADDLE_SCALE[3])
    set_rotation(ent.paddle, 0, 0, -g.pv / PADDLE_SPEED * 0.12)

    -- Ball (rides the paddle until launched), its glow and its trail.
    if g.phase == "serve" or g.phase == "intro" then
        g.bx = g.px
        g.by = PADDLE_Y + PADDLE_HH + BALL_R
    end
    g.pulse = math.max(0, g.pulse - dt * 6.0)
    local r = BALL_R * (1 + 0.25 * g.pulse)
    set_translation(ent.ball, g.bx, g.by, 0)
    set_scale(ent.ball, r, r, r)
    set_translation(ent.ball_light, g.bx, g.by, 1.2)

    local ball_visible = g.phase ~= "over"
    if ball_visible ~= g.ball_visible then
        g.ball_visible = ball_visible
        set_visible(ent.ball, ball_visible)
        set_visible(ent.ball_light, ball_visible)
    end

    if g.phase == "play" then
        table.insert(g.hist, 1, { g.bx, g.by })
        if #g.hist > TRAIL_COUNT * 2 + 1 then table.remove(g.hist) end
    end
    local trail_visible = g.phase == "play"
    if trail_visible ~= g.trail_visible then
        g.trail_visible = trail_visible
        for i = 1, TRAIL_COUNT do set_visible(ent.trail[i], trail_visible) end
    end
    if trail_visible then
        for i = 1, TRAIL_COUNT do
            local p = g.hist[i * 2] or g.hist[#g.hist]
            if p then set_translation(ent.trail[i], p[1], p[2], -0.05) end
        end
    end

    -- Lives icons.
    if g.lives ~= g.lives_shown then
        g.lives_shown = g.lives
        for i = 1, START_LIVES do set_visible(ent.life[i], i <= g.lives) end
    end

    -- Camera: slight parallax with the paddle plus shake.
    g.shake = math.max(0, g.shake - dt * 1.8)
    local amp = g.shake * g.shake * 0.35
    set_translation(ent.camera,
        CAM.x + g.px * 0.05 + (math.random() - 0.5) * 2 * amp,
        CAM.y + (math.random() - 0.5) * 2 * amp,
        CAM.z)
end

local function publish()
    set_f64("score", g.score)
    set_i32("lives", g.lives)
    set_i32("level", g.level)
    set_i32("bricks_left", g.bricks_left)
    set_string("phase", g.phase)
end

function on_created(self_entity)
    ent.paddle = find("Paddle")
    ent.ball = find("Ball")
    ent.ball_light = find("BallLight")
    ent.camera = find("GameCamera")
    ent.trail, ent.life = {}, {}
    for i = 1, TRAIL_COUNT do ent.trail[i] = find("Trail" .. i) end
    for i = 1, START_LIVES do ent.life[i] = find("Life" .. i) end
    collect_bricks()
    g.px, g.pv, g.squash, g.shake = 0.0, 0.0, 0, 0
    g.hist = {}
    g.trail_visible, g.lives_shown, g.ball_visible = nil, nil, nil
    g.boot = 0
    log_info(string.format("breakout: ready, %d bricks", #g.bricks))
end

function update(self_entity, dt)
    -- The editor takes its pre-play snapshot only after the first play frame's scripts have
    -- run, so anything changed on that frame would survive "stop" and get saved with the
    -- scene. Leave the scene alone until the second frame.
    if g.boot == 0 then
        g.boot = 1
        return
    elseif g.boot == 1 then
        g.boot = 2
        new_game()
    end

    dt = math.min(dt, 1 / 30)
    g.t = g.t + dt

    if is_key_just_pressed("R") then new_game() end
    if is_key_just_pressed("P") then set_bool("autopilot", not get_bool("autopilot", false)) end
    local auto = get_bool("autopilot", false)

    update_paddle(dt)

    if g.phase == "intro" then
        if g.t >= INTRO_TIME then
            for _, b in ipairs(g.bricks) do
                set_scale(b.e, BRICK_SCALE[1], BRICK_SCALE[2], BRICK_SCALE[3])
                set_translation(b.e, b.x, b.y, 0)
            end
            serve()
        end
    elseif g.phase == "serve" then
        if is_key_just_pressed("Space") or (auto and g.t > 0.7) then launch() end
    elseif g.phase == "play" then
        step_ball(dt)
    elseif g.phase == "cleared" then
        if g.t > 1.8 then
            g.level = g.level + 1
            start_level()
        end
    elseif g.phase == "over" then
        if is_key_just_pressed("Space") or (auto and g.t > 2.0) then new_game() end
    end

    animate_bricks(dt)
    sync_scene(dt)
    publish()
end

local MESSAGES = {
    serve = { "READY", "Press SPACE to launch   |   Arrows / A D to move" },
    cleared = { "LEVEL CLEAR", "Get ready..." },
    over = { "GAME OVER", "Press SPACE to play again" },
}

function on_ui(self_entity, ui)
    ui:anchored_panel("breakout_hud", "top_left", 18, 12, 300, 80, function(p)
        p:heading(string.format("SCORE  %06d", g.score))
        p:label(string.format("LEVEL %d     BALLS %d     BEST %d", g.level, g.lives, g.best))
        if g.combo >= 2 then p:label(string.format("COMBO x%d", math.min(g.combo, 4))) end
        if get_bool("autopilot", false) then p:label("AUTOPILOT (P to take over)") end
    end)
    local msg = MESSAGES[g.phase]
    if msg then
        ui:anchored_panel("breakout_msg", "center", 0, 40, 300, 70, function(p)
            p:heading(msg[1])
            p:label(msg[2])
        end)
    end
end
