-- @editor
-- Comprehensive Lua UI example demonstrating ALL available widgets
-- Attach this to any entity in the editor to see a full widget showcase

function on_created(self_entity)
    log_info("=== Comprehensive UI Example Initialized ===")

    -- Initialize state for all widgets
    set_state("button_clicks", 0)
    set_state("checkbox_enabled", true)
    set_state("slider_value", 0.5)
    set_state("drag_value", 100.0)
    set_state("text_input", "Edit me!")
    set_state("multiline_text", "Line 1\nLine 2\nLine 3")
    set_state("color_r", 1.0)
    set_state("color_g", 0.5)
    set_state("color_b", 0.0)
end

function on_ui(self_entity, ui)
    ui:heading("Lua UI Widget Showcase")
    ui:label("All available egui widgets via Lua")

    ui:separator()

    -- 1. LABEL
    ui:heading("1. Labels")
    ui:label("This is a simple text label")
    ui:label("Labels can display any text")

    ui:separator()

    -- 2. BUTTON
    ui:heading("2. Button")
    local clicks = get_state("button_clicks", 0)
    if ui:button("Click Counter") then
        clicks = clicks + 1
        set_state("button_clicks", clicks)
    end
    ui:label(string.format("Clicked: %d times", clicks))

    ui:separator()

    -- 3. CHECKBOX
    ui:heading("3. Checkbox")
    local enabled = get_state("checkbox_enabled", true)
    local new_enabled = ui:checkbox("check_id", enabled, "Feature enabled")
    if new_enabled ~= enabled then
        set_state("checkbox_enabled", new_enabled)
    end
    ui:label(string.format("Status: %s", new_enabled and "Enabled" or "Disabled"))

    ui:separator()

    -- 4. SLIDER
    ui:heading("4. Slider")
    ui:label("Drag the slider to change value (0.0 - 1.0):")
    local slider_val = get_state("slider_value", 0.5)
    local new_slider = ui:slider("slider_id", slider_val, 0.0, 1.0)
    if math.abs(new_slider - slider_val) > 0.001 then
        set_state("slider_value", new_slider)
    end
    ui:label(string.format("Value: %.3f", new_slider))

    ui:separator()

    -- 5. DRAG VALUE
    ui:heading("5. Drag Value")
    ui:label("Click and drag to change the number:")
    local drag_val = get_state("drag_value", 100.0)
    local new_drag = ui:drag_value("drag_id", drag_val)
    if math.abs(new_drag - drag_val) > 0.001 then
        set_state("drag_value", new_drag)
    end
    ui:label(string.format("Value: %.2f", new_drag))

    ui:separator()

    -- 6. TEXT EDIT (single line)
    ui:heading("6. Text Edit (Single Line)")
    local text = get_state("text_input", "")
    local new_text = ui:text_edit("text_id", text)
    if new_text ~= text then
        set_state("text_input", new_text)
    end
    ui:label(string.format("Length: %d characters", string.len(new_text)))

    ui:separator()

    -- 7. TEXT EDIT MULTILINE
    ui:heading("7. Text Edit (Multiline)")
    ui:label("Multiline text editor:")
    local multiline = get_state("multiline_text", "")
    -- Use a fixed size for the multiline editor
    local new_multiline = ui:text_edit_multiline("multiline_id", multiline, 300, 100)
    if new_multiline ~= multiline then
        set_state("multiline_text", new_multiline)
    end

    ui:separator()

    -- 8. COLOR EDIT
    ui:heading("8. Color Picker")
    ui:label("Click the color button to change color:")
    local r = get_state("color_r", 1.0)
    local g = get_state("color_g", 0.5)
    local b = get_state("color_b", 0.0)

    local color = ui:color_edit("color_id", r, g, b)
    local new_r = color.r
    local new_g = color.g
    local new_b = color.b

    if math.abs(new_r - r) > 0.001 or math.abs(new_g - g) > 0.001 or math.abs(new_b - b) > 0.001 then
        set_state("color_r", new_r)
        set_state("color_g", new_g)
        set_state("color_b", new_b)
    end

    ui:label(string.format("RGB: (%.2f, %.2f, %.2f)", new_r, new_g, new_b))

    ui:separator()

    -- 9. SEPARATOR (already used throughout)
    ui:heading("9. Separator")
    ui:label("Separators divide sections (used above)")

    ui:separator()
    ui:label("That's all 10 widget types! 🎉")
end
