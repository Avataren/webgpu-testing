-- Editor cube spin example (Lua version)
-- Demonstrates state management and rotation

function on_created(self_entity)
    -- In Lua, we use tables instead of structs
    local cube_state = {
        angle = 0.0,
        frameCount = 0.0
    }
    set_state("cube_state", cube_state)
    log_info("Lua cube spin script initialized")
end

function update(self_entity, dt)
    -- Get state with default fallback
    local state = get_state("cube_state", { angle = 0.0, frameCount = 0.0 })

    -- Update angle and frame count
    local angle = state.angle + dt * 1.5
    local frameCount = state.frameCount + 1.0

    -- Apply rotation
    set_rotation(self_entity, angle, 0.0, 0.0)

    -- Save updated state
    set_state("cube_state", {
        angle = angle,
        frameCount = frameCount
    })
end
