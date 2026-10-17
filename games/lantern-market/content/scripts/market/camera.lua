-- The camera: a round of shots that show the navigation off (the square from above,
-- under the arcade with people on the terrace overhead, the towpath passing under the
-- bridges, the stage the children jump off, the gate), and a follow camera behind
-- whoever is walking.

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
    { name = "Following", fov = 60, follow = true },
    { name = "Overhead", fov = 50, at = orbit(0, 0, 2, 9, 42, 0.02, 1.2) },
}

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
    if shot.follow and Camera.follow then
        return "Following " .. Camera.follow.char.name
    end
    return shot.name
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
    local eye, target
    if shot.follow then
        if not Camera.follow or Camera.time > 12 * (Camera.follows or 1) then
            Camera.follow = people.walker(Camera.follow)
            Camera.follows = (Camera.follows or 0) + 1
        end
        local person = Camera.follow
        local p = person and get_world_translation(person.e)
        local f = person and get_forward(person.e)
        if not p or not f then
            return
        end
        -- Models face +Z, so behind them is along their forward (-Z) axis.
        local back = { f.x, 0, f.z }
        local want_eye = { p.x + back[1] * 3.6, p.y + 2.1, p.z + back[3] * 3.6 }
        local want_target = { p.x - back[1] * 2.0, p.y + 1.3, p.z - back[3] * 2.0 }
        local k = 1 - math.exp(-dt * 3)
        Camera.eye = Camera.eye or want_eye
        Camera.target = Camera.target or want_target
        for i = 1, 3 do
            Camera.eye[i] = C.lerp(Camera.eye[i], want_eye[i], k)
            Camera.target[i] = C.lerp(Camera.target[i], want_target[i], k)
        end
        eye, target = Camera.eye, Camera.target
    else
        Camera.follows = nil
        eye, target = shot.at(Camera.time)
        Camera.eye = { eye[1], eye[2], eye[3] }
        Camera.target = { target[1], target[2], target[3] }
    end
    set_translation(Camera.e, eye[1], eye[2], eye[3])
    look_at(Camera.e, target[1], target[2], target[3])
    if shot.fov ~= Camera.fov then
        Camera.fov = shot.fov
        set_camera(Camera.e, { fov_deg = shot.fov })
    end
end

return Camera
