-- Third-person character controller: the keyboard and mouse steer a CharacterController
-- with a SpringCamera behind it. It only moves the character; its AnimationTree picks
-- the animation from how it moves (like a Godot CharacterBody3D script next to an
-- AnimationTree): give the character one with the inspector's "Generate from clips".
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
-- It can drive a NavAgent instead (`drive = "navigation"`): the character then keeps to
-- the navmesh, walking where agents walk (stairs, bridges; walls and edges stop it), and
-- the crowd makes way for it. That is how a game hands an AI-driven character to the
-- player for a while: stop its brain and its walk, and steer it with this.
--
-- It is built into the engine as `engine.third_person`; the editor's "Make Character"
-- writes a script that uses it. To change it beyond what the options and methods below
-- allow, copy this file to your scripts folder as engine/third_person.lua, which then
-- takes its place.
--
-- Use it from a script on the character's root entity (the one with the
-- CharacterController component):
--
--   local ThirdPerson = require("engine.third_person")
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
--   function player:read_mmo_keys() ... end                        -- other keys
--   local landed = player.landed
--   function player:landed(speed) landed(self, speed); play_sound(...) end
--
-- Animation belongs to the AnimationTree: tell it more with animation_set(self, "aiming",
-- true) or animation_trigger(self, "attack") and add states that use them.
--
-- How the body moves (speeds, jump height, slopes, steps) is the CharacterController
-- component's; change it with set_component(self, "CharacterController", { ... }). A
-- NavAgent driven by this takes walk_speed and run_speed from the options and turns at
-- its own turn_speed; it does not jump.

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
    -- "mmo": the character has strafe animations, so strafing keeps it facing forward;
    -- without them it turns to run the way it strafes.
    strafes = true,
    -- What it moves: "character" (the entity's CharacterController) or "navigation" (its
    -- NavAgent, along the navmesh).
    drive = "character",
    -- "navigation": walking and running speeds (m/s).
    walk_speed = 1.5,
    run_speed = 3.5,
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

function ThirdPerson.new(entity, options)
    local self = setmetatable({}, ThirdPerson)
    self.entity = entity
    self.options = merged(ThirdPerson.defaults, options)
    local camera = self.options.camera
    if type(camera) == "string" then camera = find_entity_by_name(camera) end
    self.camera = camera
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
        self:face(nil)
    end
    return self
end

--- One frame: input, then steering.
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
    self:drive(dx, dz, run, jump)
end

-- ---------------------------------------------------------------------------------
-- The body: a CharacterController, or a NavAgent on the navmesh
-- ---------------------------------------------------------------------------------

--- Turn the body to `yaw` (radians about +Y, 0 = the model's +Z; nil: the way it moves).
function ThirdPerson:face(yaw, instant)
    if self.options.drive == "navigation" then
        self.nav_yaw = yaw
    else
        character_face(self.entity, yaw, instant)
    end
end

--- Move the body along the ground direction (dx, dz) (length up to 1), running or not,
--- and jump when asked (a NavAgent cannot).
function ThirdPerson:drive(dx, dz, run, jump)
    if self.options.drive == "navigation" then
        local speed = run and self.options.run_speed or self.options.walk_speed
        nav_steer(self.entity, dx * speed, dz * speed, self.nav_yaw)
        self.state = nav_agent_state(self.entity)
        return
    end
    character_move(self.entity, dx, dz, run)
    if jump then character_jump(self.entity) end
    self.state = character_state(self.entity)
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

    -- The body turns into diagonals (and, without strafe animations, into strafes).
    local offset = 0
    if x ~= 0 and y ~= 0 then
        offset = math.rad(o.diagonal_turn) * (y > 0 and -x or x)
    elseif x ~= 0 and not o.strafes then
        offset = -x * math.pi / 2
    end
    self:face(self.facing + offset, steering and offset == self.body_offset)
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
-- Engine callbacks (the AnimationTree hears about jumps and landings by itself; these
-- are for a game's sounds and effects)
-- ---------------------------------------------------------------------------------

--- Forward on_jumped here.
function ThirdPerson:jumped() end

--- Forward on_landed here; `speed` is the downward speed at impact (m/s).
function ThirdPerson:landed(speed) end

--- Give the cursor back (when switching schemes, or before the script goes away), and
--- stop a NavAgent's steering where it stands.
function ThirdPerson:release()
    if self.dragging and self.options.hide_cursor_while_dragging then set_cursor_locked(false) end
    self.dragging = false
    if self.options.drive == "navigation" then nav_stop(self.entity) end
end

return ThirdPerson
