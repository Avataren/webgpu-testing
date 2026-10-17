-- Possession: Return takes over the person the camera follows, and Return lets go.
--
-- While possessed their brain sleeps and the engine's third-person controls
-- (engine.third_person, World of Warcraft style, driving their NavAgent) steer them: they
-- keep to the navmesh like everyone else (stairs, bridges, both floors; walls and the
-- canal's edge stop them), the crowd makes way, and their locomotion tree follows their
-- speed as it always does. The camera stays on them (no other shots, no one else) until
-- they are let go; then their brain starts afresh from where they stand.

local ThirdPerson = require("engine.third_person")

local Possess = { person = nil, player = nil }

-- Quicker off the mark than the crowd (m/s²), so the controls feel direct.
local ACCELERATION = 14

function Possess.active()
    return Possess.person ~= nil
end

-- Take `person` over, with `camera` (GameCamera, a SpringCamera) behind them. Vendors
-- have no NavAgent (they never walk), so they cannot be taken.
function Possess.start(person, camera)
    if Possess.person or not person or person.speed <= 0 or not camera then
        return false
    end
    local agent = get_component(person.e, "NavAgent")
    local arm = get_component(camera, "SpringCamera")
    if not agent or not arm then
        return false
    end
    -- Halt what the brain was doing (its walk, the clip over it), then let it sleep.
    ai_restart(person.e)
    ai_pause(person.e, true)
    nav_stop(person.e)
    Possess.saved = {
        acceleration = agent.acceleration,
        mouse_look = arm.mouse_look,
        auto_follow = arm.auto_follow,
    }
    set_component(person.e, "NavAgent", { acceleration = ACCELERATION })
    -- The controls swing the camera round behind them themselves.
    set_component(camera, "SpringCamera", { auto_follow = 0 })
    Possess.person = person
    Possess.camera = camera
    Possess.player = ThirdPerson.new(person.e, {
        camera = camera,
        scheme = "mmo",
        drive = "navigation",
        walk_speed = math.max(person.speed, 1.3),
        run_speed = person.run_speed,
    })
    return true
end

function Possess.update(dt)
    if Possess.player then
        Possess.player:update(dt)
    end
end

-- Let go: they stop where they are and their brain wakes up there.
function Possess.stop()
    local person = Possess.person
    if not person then
        return nil
    end
    Possess.player:release()
    local saved = Possess.saved
    set_component(person.e, "NavAgent", { acceleration = saved.acceleration })
    set_component(Possess.camera, "SpringCamera", { mouse_look = saved.mouse_look, auto_follow = saved.auto_follow })
    ai_pause(person.e, false)
    ai_restart(person.e)
    Possess.person, Possess.player, Possess.camera, Possess.saved = nil, nil, nil, nil
    return person
end

return Possess
