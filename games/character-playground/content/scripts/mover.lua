-- Moves a kinematic platform back and forth (the entity needs a kinematic RigidBody so
-- characters ride it and crates get pushed). The path is picked by the entity's name.

local PATHS = {
    Mover = { offset = { 0, 0, -8 }, period = 7, pause = 1.0 },
    Lift = { offset = { 0, 3.65, 0 }, period = 6, pause = 1.5 },
}

local start, path

function on_created(self)
    start = get_world_translation(self)
    path = PATHS[get_component(self, "Name")] or { offset = { 0, 2, 0 }, period = 6, pause = 1 }
end

-- 0 -> 1 -> 0 over a period, easing in and out and resting at each end.
local function shuttle(t, period, pause)
    local travel = period / 2 - pause
    local phase = t % period
    local half = period / 2
    local k
    if phase < half then
        k = math.max(0, math.min(1, (phase - pause) / travel))
    else
        k = 1 - math.max(0, math.min(1, (phase - half - pause) / travel))
    end
    return k * k * (3 - 2 * k)
end

function update(self, dt)
    local k = shuttle(get_time(), path.period, path.pause)
    set_translation(self, start.x + path.offset[1] * k, start.y + path.offset[2] * k, start.z + path.offset[3] * k)
end
