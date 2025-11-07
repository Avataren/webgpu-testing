-- Fractal Cube Rotation Script (Lua version)
-- Handles rotation for individual cube nodes in the fractal

function update(self_entity, dt)
    -- Get rotation axis and speed from state
    local axis = get_state("rotation_axis", {0.0, 1.0, 0.0})
    local speed = get_f64("rotation_speed", 1.0)
    local angle = get_f64("angle", 0.0)

    -- Update angle
    local new_angle = angle + dt * speed

    -- Apply rotation around the specified axis
    -- The axis is the direction vector of this cube's offset from its parent
    -- We rotate around this axis to create the spinning effect
    local axis_x = axis[1]  -- Lua arrays are 1-indexed
    local axis_y = axis[2]
    local axis_z = axis[3]

    -- Set rotation using the current angle
    -- For rotation around an arbitrary axis, we use the axis components as rotation angles
    -- This creates the characteristic fractal spinning effect
    set_rotation(self_entity, axis_y * new_angle, axis_x * new_angle, axis_z * new_angle)

    -- Store updated angle
    set_f64("angle", new_angle)
end
