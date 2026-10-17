-- The camera: a round of shots that show the navigation off (the square from above,
-- under the arcade with people on the terrace overhead, the towpath passing under the
-- bridges, the stage the children jump off, the gate), and a follow camera behind
-- whoever is walking.
--
-- The follow shots ride GameCamera's spring arm (its SpringCamera component): it trails
-- the person, swings round behind them as they walk, and pulls in before the buildings
-- (their colliders) instead of passing through walls. Right-drag looks around, the
-- wheel zooms. The other shots hand the arm no target and place the camera themselves.

local C = require("market/core")

local Camera = { shot = 1, time = 0, auto = true, follow = nil }

local SHOT_TIME = 15

local function orbit(cx, cy, cz, radius, height, speed, start)
    return function(t)
        local a = start + t * speed
        return { cx + math.cos(a) * radius, height, cz + math.sin(a) * radius }, { cx, cy, cz }
    end
end

local function dolly(from, to, look_from, look_to)
    return function(t)
        local u = C.smooth(t / SHOT_TIME)
        local eye = { C.lerp(from[1], to[1], u), C.lerp(from[2], to[2], u), C.lerp(from[3], to[3], u) }
        local target = { C.lerp(look_from[1], look_to[1], u), C.lerp(look_from[2], look_to[2], u), C.lerp(look_from[3], look_to[3], u) }
        return eye, target
    end
end

Camera.SHOTS = {
    { name = "The square", fov = 55, at = orbit(0, 0.5, 0, 25, 12, 0.045, 0.6) },
    { name = "Under the arcade", fov = 62, at = dolly({ -19, 1.7, -14.3 }, { 15, 1.8, -14.0 }, { -11, 1.3, -10.5 }, { 23, 1.6, -10.0 }) },
    { name = "The towpath", fov = 60, at = dolly({ -17, -1.0, 13.4 }, { -6, -0.9, 13.6 }, { 0, -1.2, 15.0 }, { 8, -1.0, 15.5 }) },
    { name = "From the terrace", fov = 55, at = dolly({ -10, 6.4, -13.3 }, { 8, 6.6, -13.3 }, { -2, 0.4, 3 }, { 4, 0.4, 3 }) },
    { name = "The stage", fov = 52, at = dolly({ -2.0, 2.7, 9.0 }, { -5.5, 2.4, -3.5 }, { -15, 1.6, 4.5 }, { -15, 1.6, 1.5 }) },
    { name = "The gate", fov = 55, at = dolly({ 2.2, 1.9, 5.2 }, { -1.6, 2.3, 6.6 }, { 0, 1.5, 12.8 }, { 0, 1.7, 13.0 }) },
    { name = "Following", fov = 60, follow = true, distance = 3.8, pitch = -14 },
    { name = "Overhead", fov = 50, at = orbit(0, 0, 2, 9, 42, 0.02, 1.2) },
    { name = "The chase", fov = 62, follow = true, chase = true, distance = 4.6, pitch = -12 },
}

-- The shot a theft cuts to (while the camera runs by itself).
Camera.CHASE = 9

function Camera.setup()
    Camera.e = find_entity_by_name("GameCamera")
end

function Camera.next()
    Camera.shot = Camera.shot % #Camera.SHOTS + 1
    Camera.time = 0
    Camera.follow = nil
end

function Camera.select(index)
    if Camera.SHOTS[index] then
        Camera.shot = index
        Camera.time = 0
        Camera.follow = nil
    end
end

function Camera.name()
    local shot = Camera.SHOTS[Camera.shot]
    if shot.chase and Camera.follow then
        return "The chase: " .. Camera.follow.name
    elseif shot.follow and Camera.follow then
        return "Following " .. Camera.follow.name
    end
    return shot.name
end

-- Put the spring arm on `person` for `shot` (nil lets go of it): cut to behind them,
-- looking the way they face. Models face +Z, so that is against their forward (-Z) axis;
-- yaw 0 looks along -Z.
local function ride(person, shot)
    if person == Camera.riding and shot == Camera.riding_shot then
        return
    end
    Camera.riding, Camera.riding_shot = person, shot
    if not person then
        spring_camera_set(Camera.e, { target = false })
        return
    end
    local f = get_forward(person.e)
    spring_camera_set(Camera.e, {
        target = person.e,
        yaw = f and math.deg(math.atan(f.x, f.z)) or nil,
        pitch = shot.pitch,
        distance = shot.distance,
        snap = true,
    })
end

-- Who the chase shot follows: the thief (or, without one, the guard on a chase).
local function chase_target(people)
    if people.thief then
        return people.thief
    end
    local chaser = ai_global().chase_by
    return chaser and C.person(chaser)
end

function Camera.update(dt, people)
    if not Camera.e then
        return
    end
    Camera.time = Camera.time + dt
    local shot = Camera.SHOTS[Camera.shot]
    if Camera.auto and Camera.time > (shot.follow and SHOT_TIME * 1.6 or SHOT_TIME) then
        Camera.next()
        shot = Camera.SHOTS[Camera.shot]
    end
    -- A theft: cut to the chase (and stay with it while the hue and cry lasts).
    local wanted = ai_global().thief_wanted
    if Camera.auto and wanted and not Camera.was_wanted and people.thief then
        Camera.select(Camera.CHASE)
        shot = Camera.SHOTS[Camera.shot]
    end
    Camera.was_wanted = wanted
    if Camera.auto and shot.chase and wanted then
        Camera.time = math.min(Camera.time, SHOT_TIME)
    end
    if shot.chase then
        Camera.follow = chase_target(people)
    elseif shot.follow then
        if not Camera.follow or Camera.time > 12 * (Camera.follows or 1) then
            Camera.follow = people.walker(Camera.follow)
            Camera.follows = (Camera.follows or 0) + 1
        end
    end
    if shot.follow then
        -- The spring arm places the camera after the scripts run (without anyone to
        -- follow, it stays where it is).
        local person = Camera.follow
        ride(person and get_world_translation(person.e) and person, shot)
    else
        Camera.follows = nil
        ride(nil)
        local eye, target = shot.at(Camera.time)
        set_translation(Camera.e, eye[1], eye[2], eye[3])
        look_at(Camera.e, target[1], target[2], target[3])
    end
    if shot.fov ~= Camera.fov then
        Camera.fov = shot.fov
        set_camera(Camera.e, { fov_deg = shot.fov })
    end
end

return Camera
