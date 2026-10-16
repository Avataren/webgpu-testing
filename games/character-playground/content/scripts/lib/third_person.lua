-- Third-person character controller: the keyboard and mouse steer a CharacterController
-- with a SpringCamera behind it, and the character's animation follows what the
-- controller reports (idle, walk, run, strafe, jump, fall, land).
--
-- Two control schemes:
--
-- * "action" (default): WASD moves relative to the camera and the character turns to
--   face where it runs. A click captures the mouse to look around (Escape releases it).
-- * "mmo": World of Warcraft style. W/S move forward and back along the character's
--   facing, A/D turn it, Q/E strafe. Holding the left button looks around the character
--   (the view stays where you leave it, and swings back behind once it moves); holding
--   the right button steers: the character turns with the camera and A/D strafe. Both
--   buttons together run forward. Up and down with either button tilts the camera.
--
-- Use it from a script on the character's root entity (the one with the
-- CharacterController component):
--
--   local ThirdPerson = require("lib.third_person")
--   local player
--
--   function on_created(self)
--       player = ThirdPerson.new(self, { camera = "PlayerCamera", scheme = "mmo" })
--   end
--   function update(self, dt) player:update(dt) end
--   function on_jumped(self) player:jumped() end
--   function on_landed(self, speed) player:landed(speed) end
--
-- Every step is a method, so a game changes one part by replacing it on its instance
-- (or on ThirdPerson itself for every character):
--
--   function player:choose_animation(state) ... end               -- more clips
--   local landed = player.landed
--   function player:landed(speed) landed(self, speed); play_sound(...) end
--
-- How the body moves (speeds, jump height, slopes, steps) is the CharacterController
-- component's; change it with set_component(self, "CharacterController", { ... }).

local ThirdPerson = {}
ThirdPerson.__index = ThirdPerson

ThirdPerson.defaults = {
    scheme = "action",
    -- The spring camera framing the character (an entity or a name). Without one the
    -- "action" scheme steers in world axes (forward is -Z).
    camera = "PlayerCamera",
    keys = {
        forward = { "W", "ArrowUp" },
        back = { "S", "ArrowDown" },
        -- "action": move sideways. "mmo": turn (strafe while the right button is held).
        left = { "A", "ArrowLeft" },
        right = { "D", "ArrowRight" },
        -- "mmo" only.
        strafe_left = { "Q" },
        strafe_right = { "E" },
        jump = { "Space" },
        -- Held: walk instead of run (or run instead of walk when always_run is false).
        walk = { "Shift" },
    },
    always_run = true,
    -- "action": a click into the game captures the mouse for looking around.
    capture_mouse = true,
    -- "mmo": keyboard turning (degrees per second), mouse look (degrees per pixel),
    -- how fast the camera swings back behind a moving character (per second; 0 never),
    -- and how far the body turns into a diagonal run (degrees).
    turn_rate = 180,
    mouse_sensitivity = 0.2,
    -- Hide (and hold) the cursor while a button drags the view; it comes back where the
    -- drag started.
    hide_cursor_while_dragging = true,
    invert_y = false,
    camera_follow = 2.5,
    diagonal_turn = 45,
    -- The character's clip names; leave a strafe clip out (nil) if the character has
    -- none, and it runs sideways facing the way it goes.
    clips = {
        idle = "Idle", walk = "Walk", run = "Run", jump = "Jump", fall = "Fall", land = "Land",
        strafe_left = "StrafeLeft", strafe_right = "StrafeRight",
    },
    -- Ground speeds (m/s) at which the clips' feet keep pace; the clips play faster or
    -- slower to match the actual speed.
    walk_clip_speed = 1.4,
    run_clip_speed = 4.6,
    strafe_clip_speed = 3.0,
    -- Above this speed (m/s) the run clip plays instead of the walk.
    run_threshold = 3.2,
    -- Landing at least this fast (m/s downward) plays the land clip.
    hard_landing = 8.0,
    -- Seconds in the air before the fall clip (short drops keep walking).
    fall_delay = 0.15,
    -- How long the jump and land clips hold (s).
    jump_time = 0.3,
    land_time = 0.35,
}

local function merged(defaults, options)
    local out = {}
    for key, value in pairs(defaults) do
        if type(value) == "table" then
            local copy = {}
            for k, v in pairs(value) do copy[k] = v end
            out[key] = copy
        else
            out[key] = value
        end
    end
    for key, value in pairs(options or {}) do
        if type(value) == "table" and type(out[key]) == "table" then
            for k, v in pairs(value) do out[key][k] = v end
        else
            out[key] = value
        end
    end
    return out
end

local function pressed(keys)
    for _, key in ipairs(keys) do
        if is_key_pressed(key) then return 1 end
    end
    return 0
end

local function any_just_pressed(keys)
    for _, key in ipairs(keys) do
        if is_key_just_pressed(key) then return true end
    end
    return false
end

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

-- The shortest turn from angle `from` to angle `to` (degrees), -180..180.
local function wrap_degrees(angle)
    return (angle + 180) % 360 - 180
end

-- The entity at or below `root` that an animator drives (imported characters keep their
-- animator on a child of the instance root), breadth first.
local function find_animated(root)
    local queue, head = { root }, 1
    while queue[head] do
        local entity = queue[head]
        head = head + 1
        if animator_state(entity) then return entity end
        for _, child in ipairs(get_children(entity) or {}) do
            queue[#queue + 1] = child
        end
    end
    return nil
end

function ThirdPerson.new(entity, options)
    local self = setmetatable({}, ThirdPerson)
    self.entity = entity
    self.options = merged(ThirdPerson.defaults, options)
    local camera = self.options.camera
    if type(camera) == "string" then camera = find_entity_by_name(camera) end
    self.camera = camera
    self.animated = find_animated(entity)
    self.clip = nil
    self.speed = 1
    self.jump_timer = 0
    self.land_timer = 0
    -- The stick in the character's own frame: x right, y forward.
    self.move = { x = 0, y = 0 }
    -- What the controller reported last frame (nil until the first physics step).
    self.state = nil
    if self.camera then
        -- The "mmo" scheme turns the camera itself; the camera's own mouse look would
        -- fight it. (The wheel still zooms.)
        set_component(self.camera, "SpringCamera", { mouse_look = self.options.scheme ~= "mmo" })
    end
    if self.options.scheme ~= "mmo" then
        character_face(entity, nil)
    end
    return self
end

--- One frame: input, steering, then animation.
function ThirdPerson:update(dt)
    local dx, dz, run, jump
    if self.options.scheme == "mmo" then
        dx, dz, run, jump = self:mmo_steer(dt)
    else
        self:capture_mouse()
        local x, y
        x, y, run, jump = self:read_input()
        dx, dz = self:move_direction(x, y)
        self.move = { x = 0, y = (x ~= 0 or y ~= 0) and 1 or 0 }
    end
    character_move(self.entity, dx, dz, run)
    if jump then character_jump(self.entity) end

    self.jump_timer = math.max(0, self.jump_timer - dt)
    self.land_timer = math.max(0, self.land_timer - dt)
    self.state = character_state(self.entity)
    if self.state and self.animated then
        if self.state.just_jumped then self.jump_timer = self.options.jump_time end
        self:play(self:choose_animation(self.state))
    end
end

-- ---------------------------------------------------------------------------------
-- "action" scheme
-- ---------------------------------------------------------------------------------

function ThirdPerson:capture_mouse()
    if self.options.capture_mouse and is_mouse_button_just_pressed(0) and not is_cursor_locked() then
        set_cursor_locked(true)
    end
end

--- The stick: x (right) and y (forward) in -1..1, whether to run, and whether jump was
--- pressed this frame.
function ThirdPerson:read_input()
    local keys = self.options.keys
    local x = pressed(keys.right) - pressed(keys.left)
    local y = pressed(keys.forward) - pressed(keys.back)
    local run = self.options.always_run ~= (pressed(keys.walk) == 1)
    return x, y, run, any_just_pressed(keys.jump)
end

--- Turn the stick into a world direction on the ground, relative to the camera's view.
function ThirdPerson:move_direction(x, y)
    local fx, fz, rx, rz = 0, -1, 1, 0
    local view = self.camera and spring_camera_state(self.camera)
    if view then
        fx, fz, rx, rz = view.forward.x, view.forward.z, view.right.x, view.right.z
    end
    local dx, dz = fx * y + rx * x, fz * y + rz * x
    local length = math.sqrt(dx * dx + dz * dz)
    if length > 1 then dx, dz = dx / length, dz / length end
    return dx, dz
end

-- ---------------------------------------------------------------------------------
-- "mmo" scheme
-- ---------------------------------------------------------------------------------

--- The camera's angles and the character's facing, read once from the scene.
function ThirdPerson:mmo_init()
    if self.facing then return end
    local config = self.camera and get_component(self.camera, "SpringCamera") or {}
    local view = self.camera and spring_camera_state(self.camera)
    self.cam_yaw = view and view.yaw or config.yaw or 0
    self.cam_pitch = view and view.pitch or config.pitch or -15
    self.min_pitch = config.min_pitch or -75
    self.max_pitch = config.max_pitch or 55
    local rotation = get_world_rotation(self.entity)
    -- Facing: radians about +Y, 0 = the model's +Z.
    self.facing = rotation and rotation.yaw or 0
    self.body_offset = 0
    self.dragging = false
end

--- Mouse and keyboard for the "mmo" scheme; returns the world direction, whether to
--- run and whether to jump.
function ThirdPerson:mmo_steer(dt)
    self:mmo_init()
    local o, keys = self.options, self.options.keys
    local looking = is_mouse_button_pressed(0)
    local steering = is_mouse_button_pressed(1)

    -- The cursor hides while a button drags the view, and comes back where it was.
    local dragging = looking or steering
    if dragging ~= self.dragging then
        if o.hide_cursor_while_dragging then set_cursor_locked(dragging) end
        self.dragging = dragging
    end
    if dragging then
        local delta = get_mouse_delta()
        local invert = o.invert_y and -1 or 1
        self.cam_yaw = self.cam_yaw - delta.x * o.mouse_sensitivity
        self.cam_pitch = clamp(self.cam_pitch - delta.y * o.mouse_sensitivity * invert,
            self.min_pitch, self.max_pitch)
    end

    local y = pressed(keys.forward) - pressed(keys.back)
    local x = pressed(keys.strafe_right) - pressed(keys.strafe_left)
    local turn = pressed(keys.left) - pressed(keys.right) -- + turns left
    if steering then
        -- The right button steers: the character faces the way the camera looks, and
        -- the turn keys strafe.
        x = clamp(x - turn, -1, 1)
        turn = 0
        self.facing = math.rad(self.cam_yaw) + math.pi
    end
    if looking and steering then y = 1 end
    if turn ~= 0 then
        local step = o.turn_rate * turn * dt
        self.facing = self.facing + math.rad(step)
        -- The view turns along, keeping its angle to the character (unless the left
        -- button holds it).
        if not looking then self.cam_yaw = self.cam_yaw + step end
    end

    -- Moving swings the camera back behind the character, unless a button holds it.
    if not dragging and o.camera_follow > 0 and (x ~= 0 or y ~= 0) then
        local behind = math.deg(self.facing) + 180
        local ease = 1 - math.exp(-o.camera_follow * dt)
        self.cam_yaw = self.cam_yaw + wrap_degrees(behind - self.cam_yaw) * ease
    end
    self.cam_yaw = wrap_degrees(self.cam_yaw)
    if self.camera then
        spring_camera_set(self.camera, { yaw = self.cam_yaw, pitch = self.cam_pitch })
    end

    -- The body turns into diagonals (and, without strafe clips, into strafes).
    local clips = o.clips
    local offset = 0
    if x ~= 0 and y ~= 0 then
        offset = math.rad(o.diagonal_turn) * (y > 0 and -x or x)
    elseif x ~= 0 and not (clips.strafe_left and clips.strafe_right) then
        offset = -x * math.pi / 2
    end
    character_face(self.entity, self.facing + offset, steering and offset == self.body_offset)
    self.body_offset = offset

    self.move = { x = x, y = y }
    local f = self.facing
    local dx = math.sin(f) * y - math.cos(f) * x
    local dz = math.cos(f) * y + math.sin(f) * x
    local length = math.sqrt(dx * dx + dz * dz)
    if length > 1 then dx, dz = dx / length, dz / length end
    -- Walking backwards is slower than running.
    local run = y >= 0 and (o.always_run ~= (pressed(keys.walk) == 1))
    return dx, dz, run, any_just_pressed(keys.jump)
end

-- ---------------------------------------------------------------------------------
-- Animation
-- ---------------------------------------------------------------------------------

--- The clip for the character's state, its play options and its playback speed
--- (negative plays it backwards).
function ThirdPerson:choose_animation(s)
    local o, clips = self.options, self.options.clips
    if self.land_timer > 0 and s.grounded then
        if s.horizontal_speed < 1.0 then
            return clips.land, { loop = false, fade = 0.06 }, 1
        end
        -- Moving off straight away: run out of the landing instead.
        self.land_timer = 0
    end
    if not s.grounded then
        if self.jump_timer > 0 and s.vertical_speed > 0 then
            return clips.jump, { loop = false, fade = 0.08 }, 1
        end
        if s.air_time > o.fall_delay or self.clip == clips.jump or self.clip == clips.fall then
            return clips.fall, { fade = 0.25 }, 1
        end
        return self.clip or clips.idle, nil, self.speed
    end
    local speed = s.horizontal_speed
    local move = self.move
    if speed < 0.2 then
        return clips.idle, { fade = 0.25 }, 1
    elseif move.y < 0 then
        -- Backpedalling: the walk, played backwards.
        return clips.walk, { fade = 0.2 }, -clamp(speed / o.walk_clip_speed, 0.5, 2.5)
    elseif move.y == 0 and move.x ~= 0 and clips.strafe_left and clips.strafe_right then
        local clip = move.x > 0 and clips.strafe_right or clips.strafe_left
        return clip, { fade = 0.2 }, clamp(speed / o.strafe_clip_speed, 0.5, 2.2)
    elseif speed < o.run_threshold then
        return clips.walk, { fade = 0.2 }, clamp(speed / o.walk_clip_speed, 0.5, 2.5)
    end
    return clips.run, { fade = 0.2 }, clamp(speed / o.run_clip_speed, 0.6, 2.0)
end

--- Cross-fade to `clip` when it changes, and set the playback speed.
function ThirdPerson:play(clip, options, speed)
    if clip and clip ~= self.clip then
        animator_play(self.animated, clip, options)
        self.clip = clip
    end
    if speed and math.abs(speed - self.speed) > 0.01 then
        animator_set_speed(self.animated, speed)
        self.speed = speed
    end
end

--- Engine callback (forward on_jumped here).
function ThirdPerson:jumped()
    self.jump_timer = self.options.jump_time
end

--- Engine callback (forward on_landed here); `speed` is the downward speed at impact.
function ThirdPerson:landed(speed)
    if speed >= self.options.hard_landing then
        self.land_timer = self.options.land_time
        self.clip = nil -- replay the land clip even if the last landing just ended
    end
end

--- Give the cursor back (when switching schemes, or before the script goes away).
function ThirdPerson:release()
    if self.dragging and self.options.hide_cursor_while_dragging then set_cursor_locked(false) end
    self.dragging = false
end

return ThirdPerson
