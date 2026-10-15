-- Camera tour: slow sweeps through the room, one shot per ray-traced effect, gliding
-- from each shot into the next (overlapping, so the camera never stops) and looping.
--
-- Keys: Space pauses, ArrowRight / ArrowLeft skip to the next / previous shot,
-- H hides the captions.

local GEM = { 2.8, 1.5, 0.0 }
local GLASS = { -2.8, 1.62, 0.0 }
-- The caustics table: a glass ball, a glass rod and a steel ring under two spotlights.
local CAUSTICS = { 5.15, 0.95, 7.55 }

-- Seconds the shots overlap while the camera glides from one into the next.
local GLIDE = 6.0
-- Gliding cameras rise to at least this height halfway, to pass over the pedestals.
local GLIDE_HEIGHT = 3.4

local function lerp(a, b, t)
    return a + (b - a) * t
end

local function lerp3(a, b, t)
    return { lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t) }
end

local function smoother(t)
    t = math.max(0.0, math.min(1.0, t))
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
end

-- Mostly constant speed, eased only near the ends.
local function cruise(t)
    return 0.7 * t + 0.3 * smoother(t)
end

-- Heading, pitch and distance from `eye` to `target`.
local function view_of(eye, target)
    local dx, dy, dz = target[1] - eye[1], target[2] - eye[2], target[3] - eye[3]
    local flat = math.sqrt(dx * dx + dz * dz)
    return math.atan(dx, dz), math.atan(dy, flat), math.sqrt(dx * dx + dy * dy + dz * dz)
end

-- The point `distance` from `eye` along heading `yaw` and `pitch`.
local function view_target(eye, yaw, pitch, distance)
    local flat = math.cos(pitch) * distance
    return { eye[1] + math.sin(yaw) * flat, eye[2] + math.sin(pitch) * distance, eye[3] + math.cos(yaw) * flat }
end

local function orbit(center, radius, height, degrees)
    local a = math.rad(degrees)
    return { center[1] + math.sin(a) * radius, height, center[3] + math.cos(a) * radius }
end

-- Each shot maps u (0 to 1 over its length; a little outside while gliding) to an eye
-- position, a point to look at and a field of view.
local SHOTS = {
    {
        title = "Ray-traced reflections",
        caption = "Mirror wall, mirror sphere and polished marble, all traced",
        seconds = 18,
        pose = function(u)
            return lerp3({ 0.0, 3.4, 12.5 }, { 0.6, 2.1, 6.5 }, u),
                lerp3({ 0.0, 1.5, -2.0 }, { 0.0, 1.3, -4.0 }, u), lerp(55, 48, u)
        end,
    },
    {
        title = "Dispersion",
        caption = "A brilliant-cut gem (ior 2.42): each wavelength bends differently",
        seconds = 22,
        pose = function(u)
            local eye = orbit(GEM, lerp(1.7, 1.25, u), lerp(2.05, 1.75, u), lerp(35, 215, u))
            return eye, { GEM[1], GEM[2] - 0.05, GEM[3] }, 42
        end,
    },
    {
        title = "Refraction",
        caption = "A glass sphere inverts and magnifies the room behind it",
        seconds = 20,
        pose = function(u)
            local eye = orbit(GLASS, lerp(1.6, 1.35, u), lerp(1.95, 1.7, u), lerp(200, 345, u))
            return eye, GLASS, 45
        end,
    },
    {
        title = "Caustics",
        caption = "Glass and a steel ring focus lamplight onto the table, traced from the lamps (path tracing)",
        seconds = 20,
        pose = function(u)
            local eye = orbit(CAUSTICS, lerp(2.0, 1.45, u), lerp(2.35, 1.8, u), lerp(-25, 25, u))
            return eye, CAUSTICS, 45
        end,
    },
    {
        title = "Floor level",
        caption = "Reflections of everything, rough to sharp",
        seconds = 18,
        pose = function(u)
            return lerp3({ -6.2, 0.35, 1.3 }, { 6.2, 0.45, 1.3 }, u),
                lerp3({ -1.5, 0.7, -5.0 }, { 1.5, 0.7, -5.0 }, u), 60
        end,
    },
    {
        title = "Through the looking glass",
        caption = "The mirror shows the room behind the camera, off screen",
        seconds = 18,
        pose = function(u)
            return lerp3({ -3.6, 2.5, -3.0 }, { 3.6, 2.2, -3.4 }, u),
                lerp3({ 1.2, 2.1, -7.9 }, { -1.2, 2.0, -7.9 }, u), 50
        end,
    },
    {
        title = "Ray-traced shadows",
        caption = "Sun and lamps trace a shadow ray per pixel; ambient occlusion is traced too",
        seconds = 18,
        pose = function(u)
            return lerp3({ 6.4, 1.3, 7.0 }, { -5.5, 4.8, 8.5 }, u), { 0.0, 1.0, 0.0 }, 52
        end,
    },
    {
        title = "Neon",
        caption = "Emissive neon in the polished floor; reflections follow the walking character",
        seconds = 16,
        pose = function(u)
            return lerp3({ 3.4, 1.1, 2.2 }, { -3.4, 1.5, 2.2 }, u),
                lerp3({ 1.5, 2.0, 14.0 }, { -1.5, 2.4, 14.0 }, u), 55
        end,
    },
}

-- The timeline gives each shot a segment of its length minus GLIDE: the segment opens
-- with the glide in from the previous shot (still moving, through its last GLIDE
-- seconds) and the rest is the shot alone. The last shot glides into the first.
local starts = {}
local loop_seconds = 0.0
for index, shot in ipairs(SHOTS) do
    starts[index] = loop_seconds
    loop_seconds = loop_seconds + shot.seconds - GLIDE
end

local clock = 0.0
-- The first pass opens on the first shot instead of gliding in from the last one.
local opening = true
local paused = false
local captions = true

-- The segment playing at timeline time t, and the seconds into its shot.
local function shot_at(t)
    for index = #SHOTS, 1, -1 do
        if t >= starts[index] then
            return index, t - starts[index]
        end
    end
    return 1, 0.0
end

-- Shot progress (0-1) after `seconds_in` of the shot's `seconds`: constant speed through
-- the glides at either end (so u runs a little past 0 and 1 there), eased in between.
local function shot_u(shot, seconds_in)
    local t = (seconds_in - GLIDE * 0.5) / (shot.seconds - GLIDE)
    if t < 0.0 then
        return t * 0.7
    elseif t > 1.0 then
        return 1.0 + (t - 1.0) * 0.7
    end
    return cruise(t)
end

local function pose_at(t)
    local index, seconds_in = shot_at(t)
    local shot = SHOTS[index]
    local eye, target, fov = shot.pose(shot_u(shot, seconds_in))
    if seconds_in < GLIDE and not (opening and index == 1) then
        local previous = SHOTS[(index - 2) % #SHOTS + 1]
        local from_eye, from_target, from_fov =
            previous.pose(shot_u(previous, previous.seconds - GLIDE + seconds_in))
        local s = smoother(seconds_in / GLIDE)
        local low = math.min(eye[2], from_eye[2])
        -- Turn from one view direction to the other (the shorter way round) rather than
        -- sliding the look-at point between them: shots facing opposite ways would drag
        -- it past the camera and swing the view steeply down.
        local from_yaw, from_pitch, from_distance = view_of(from_eye, from_target)
        local yaw, pitch, distance = view_of(eye, target)
        local turn = (yaw - from_yaw + math.pi) % (2.0 * math.pi) - math.pi
        eye = lerp3(from_eye, eye, s)
        fov = lerp(from_fov, fov, s)
        local lift = math.max(0.0, GLIDE_HEIGHT - low)
        eye[2] = math.max(eye[2], low + lift * math.sin(math.pi * s))
        target = view_target(eye, from_yaw + turn * s, lerp(from_pitch, pitch, s), lerp(from_distance, distance, s))
    end
    return eye, target, fov, index, seconds_in
end

local function skip(direction)
    opening = false
    local index = shot_at(clock)
    local target = (index - 1 + direction) % #SHOTS + 1
    -- Straight to the shot, past the glide into it.
    clock = starts[target] + GLIDE
end

-- Test hooks (lua_eval {entity: "GameCamera"}): TOUR.go(seconds) jumps the timeline.
TOUR = {
    pose_at = pose_at,
    loop_seconds = loop_seconds,
    starts = starts,
    go = function(t)
        opening = false
        clock = t % loop_seconds
    end,
    pause = function(on)
        paused = on
    end,
}

function on_created(self_entity)
    -- Where the first shot's own motion begins (its glide in is skipped).
    clock = GLIDE * 0.5
    opening = true
end

function update(self_entity, dt)
    if is_key_just_pressed("Space") then
        paused = not paused
    end
    if is_key_just_pressed("ArrowRight") then
        skip(1)
    elseif is_key_just_pressed("ArrowLeft") then
        skip(-1)
    end
    if is_key_just_pressed("H") then
        captions = not captions
    end
    if not paused then
        clock = clock + dt
        if clock >= loop_seconds then
            clock = clock % loop_seconds
            opening = false
        end
    end

    local eye, target, fov = pose_at(clock)
    set_translation(self_entity, eye[1], eye[2], eye[3])
    look_at(self_entity, target[1], target[2], target[3])
    set_camera(self_entity, { fov_deg = fov })
end

function on_ui(self_entity, ui)
    if not captions then
        return
    end
    local _, _, _, index, seconds_in = pose_at(clock)
    local shot = SHOTS[index]
    -- Shown from halfway through the glide in until a few seconds later, fading out.
    local shown = seconds_in - GLIDE * 0.5
    if shown < 0.0 or shown > 6.0 then
        return
    end
    local alpha = math.min(1.0, shown / 0.8, (6.0 - shown) / 1.5)
    ui:screen_label("bottom_left", 40, 64, shot.title, { size = 34, color = { 1, 1, 1, alpha } })
    ui:screen_label("bottom_left", 42, 36, shot.caption, { size = 18, color = { 0.85, 0.88, 0.95, alpha * 0.85 } })
end
