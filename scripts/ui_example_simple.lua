-- @editor
-- Simple Lua UI example demonstrating basic widgets
-- Attach this to any entity in the editor to see a basic UI panel

function on_created(self_entity)
    log_info("=== Simple UI Example Initialized ===")

    -- Initialize some state for the widgets
    set_state("button_clicks", 0)
    set_state("checkbox_value", false)
    set_state("text_value", "Hello from Lua!")
end

function on_ui(self_entity, ui)
    -- Add a heading
    ui:heading("Simple Lua UI Example")

    -- Add a label
    ui:label("This UI is created with Lua!")

    -- Add a separator
    ui:separator()

    -- Add a button that counts clicks
    local click_count = get_state("button_clicks", 0)
    if ui:button("Click Me!") then
        click_count = click_count + 1
        set_state("button_clicks", click_count)
        log_info(string.format("Button clicked %d times", click_count))
    end

    ui:label(string.format("Button clicked: %d times", click_count))

    ui:separator()

    -- Add a checkbox
    local checkbox = get_state("checkbox_value", false)
    local new_checkbox = ui:checkbox("checkbox_id", checkbox, "Enable feature")
    if new_checkbox ~= checkbox then
        set_state("checkbox_value", new_checkbox)
        log_info(string.format("Checkbox changed to: %s", tostring(new_checkbox)))
    end

    ui:separator()

    -- Add a text edit field
    local text = get_state("text_value", "")
    local new_text = ui:text_edit("text_id", text)
    if new_text ~= text then
        set_state("text_value", new_text)
        log_info(string.format("Text changed to: %s", new_text))
    end
end
