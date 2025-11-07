-- @editor
-- Practical UI example: Transform Controller
-- Attach this to any entity to control its position, rotation, and scale via UI

function on_created(self_entity)
    log_info("=== Transform Controller UI Initialized ===")

    -- Get initial transform values
    local pos = get_world_translation(self_entity)
    if pos then
        set_state("pos_x", pos.x)
        set_state("pos_y", pos.y)
        set_state("pos_z", pos.z)
    else
        set_state("pos_x", 0.0)
        set_state("pos_y", 0.0)
        set_state("pos_z", 0.0)
    end

    set_state("rot_x", 0.0)
    set_state("rot_y", 0.0)
    set_state("rot_z", 0.0)

    set_state("scale", 1.0)

    set_state("auto_rotate", false)
    set_state("rotation_speed", 1.0)
end

function update(self_entity, dt)
    -- Handle auto-rotation if enabled
    local auto_rotate = get_state("auto_rotate", false)
    if auto_rotate then
        local rot_y = get_state("rot_y", 0.0)
        local speed = get_state("rotation_speed", 1.0)
        rot_y = rot_y + dt * speed
        set_state("rot_y", rot_y)

        local rot_x = get_state("rot_x", 0.0)
        local rot_z = get_state("rot_z", 0.0)
        set_rotation(self_entity, rot_x, rot_y, rot_z)
    end
end

function on_ui(self_entity, ui)
    ui:heading("Transform Controller")
    ui:label("Control entity transform with sliders")

    ui:separator()

    -- POSITION
    ui:heading("Position")

    ui:label("X:")
    local pos_x = get_state("pos_x", 0.0)
    local new_pos_x = ui:drag_value("pos_x", pos_x)
    if math.abs(new_pos_x - pos_x) > 0.001 then
        set_state("pos_x", new_pos_x)
        local pos_y = get_state("pos_y", 0.0)
        local pos_z = get_state("pos_z", 0.0)
        set_translation(self_entity, new_pos_x, pos_y, pos_z)
    end

    ui:label("Y:")
    local pos_y = get_state("pos_y", 0.0)
    local new_pos_y = ui:drag_value("pos_y", pos_y)
    if math.abs(new_pos_y - pos_y) > 0.001 then
        set_state("pos_y", new_pos_y)
        local pos_x = get_state("pos_x", 0.0)
        local pos_z = get_state("pos_z", 0.0)
        set_translation(self_entity, pos_x, new_pos_y, pos_z)
    end

    ui:label("Z:")
    local pos_z = get_state("pos_z", 0.0)
    local new_pos_z = ui:drag_value("pos_z", pos_z)
    if math.abs(new_pos_z - pos_z) > 0.001 then
        set_state("pos_z", new_pos_z)
        local pos_x = get_state("pos_x", 0.0)
        local pos_y = get_state("pos_y", 0.0)
        set_translation(self_entity, pos_x, pos_y, new_pos_z)
    end

    if ui:button("Reset Position") then
        set_translation(self_entity, 0.0, 0.0, 0.0)
        set_state("pos_x", 0.0)
        set_state("pos_y", 0.0)
        set_state("pos_z", 0.0)
    end

    ui:separator()

    -- ROTATION
    ui:heading("Rotation")

    ui:label("X (pitch):")
    local rot_x = get_state("rot_x", 0.0)
    local new_rot_x = ui:slider("rot_x", rot_x, -3.14159, 3.14159)
    if math.abs(new_rot_x - rot_x) > 0.001 then
        set_state("rot_x", new_rot_x)
        local rot_y = get_state("rot_y", 0.0)
        local rot_z = get_state("rot_z", 0.0)
        set_rotation(self_entity, new_rot_x, rot_y, rot_z)
    end

    ui:label("Y (yaw):")
    local rot_y = get_state("rot_y", 0.0)
    local new_rot_y = ui:slider("rot_y", rot_y, -3.14159, 3.14159)
    if math.abs(new_rot_y - rot_y) > 0.001 then
        set_state("rot_y", new_rot_y)
        local rot_x = get_state("rot_x", 0.0)
        local rot_z = get_state("rot_z", 0.0)
        set_rotation(self_entity, rot_x, new_rot_y, rot_z)
    end

    ui:label("Z (roll):")
    local rot_z = get_state("rot_z", 0.0)
    local new_rot_z = ui:slider("rot_z", rot_z, -3.14159, 3.14159)
    if math.abs(new_rot_z - rot_z) > 0.001 then
        set_state("rot_z", new_rot_z)
        local rot_x = get_state("rot_x", 0.0)
        local rot_y = get_state("rot_y", 0.0)
        set_rotation(self_entity, rot_x, rot_y, new_rot_z)
    end

    if ui:button("Reset Rotation") then
        set_rotation(self_entity, 0.0, 0.0, 0.0)
        set_state("rot_x", 0.0)
        set_state("rot_y", 0.0)
        set_state("rot_z", 0.0)
    end

    ui:separator()

    -- AUTO ROTATE
    ui:heading("Auto Rotate")
    local auto_rotate = get_state("auto_rotate", false)
    local new_auto_rotate = ui:checkbox("auto_rotate", auto_rotate, "Enable auto-rotation")
    if new_auto_rotate ~= auto_rotate then
        set_state("auto_rotate", new_auto_rotate)
    end

    if auto_rotate then
        ui:label("Rotation Speed:")
        local speed = get_state("rotation_speed", 1.0)
        local new_speed = ui:slider("rot_speed", speed, 0.0, 5.0)
        if math.abs(new_speed - speed) > 0.001 then
            set_state("rotation_speed", new_speed)
        end
    end

    ui:separator()

    -- SCALE
    ui:heading("Scale")
    ui:label("Uniform Scale:")
    local scale = get_state("scale", 1.0)
    local new_scale = ui:slider("scale", scale, 0.1, 5.0)
    if math.abs(new_scale - scale) > 0.001 then
        set_state("scale", new_scale)
        set_scale(self_entity, new_scale, new_scale, new_scale)
    end

    if ui:button("Reset Scale") then
        set_scale(self_entity, 1.0, 1.0, 1.0)
        set_state("scale", 1.0)
    end

    ui:separator()

    -- Quick preset buttons
    ui:heading("Quick Presets")
    if ui:button("Identity Transform") then
        set_translation(self_entity, 0.0, 0.0, 0.0)
        set_rotation(self_entity, 0.0, 0.0, 0.0)
        set_scale(self_entity, 1.0, 1.0, 1.0)
        set_state("pos_x", 0.0)
        set_state("pos_y", 0.0)
        set_state("pos_z", 0.0)
        set_state("rot_x", 0.0)
        set_state("rot_y", 0.0)
        set_state("rot_z", 0.0)
        set_state("scale", 1.0)
        log_info("Reset to identity transform")
    end
end
