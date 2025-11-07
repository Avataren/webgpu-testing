-- @editor
-- Script Editor Plugin
-- A basic script editor for editing Lua script files

function on_created(self_entity)
    log_info("Script Editor plugin created")

    -- Initialize state
    set_f64("selected_index", -1.0)
    set_string("current_content", "")
    set_string("original_content", "")
    set_string("current_path", "")
    set_string("status_message", "Ready")
    set_bool("is_dirty", false)
    set_bool("needs_refresh", true)
end

function on_ui(self_entity, ui)
    -- Load scripts on first render or when requested
    local needs_refresh = get_bool("needs_refresh", true)
    if needs_refresh then
        refresh_script_list()
        return
    end

    ui:heading("Script Editor")

    -- Toolbar
    if ui:button("Refresh Scripts") then
        refresh_script_list()
        return
    end

    if ui:button("Save") then
        local current_path = get_string("current_path", "")
        if current_path ~= "" then
            local current_content = get_string("current_content", "")
            local error = write_file(current_path, current_content)
            if error == "" then
                local path_for_msg = get_string("current_path", "")
                set_string("status_message", "Saved " .. path_for_msg)
                set_bool("is_dirty", false)
            else
                set_string("status_message", error)
            end
        else
            set_string("status_message", "No script selected")
        end
        return
    end

    local is_dirty = get_bool("is_dirty", false)
    local dirty_indicator = ""
    if is_dirty then
        dirty_indicator = " *"
    end

    ui:separator()

    -- Status bar
    local status_message = get_string("status_message", "Ready")
    ui:label(status_message)

    ui:separator()

    -- Script list
    ui:label("Available Scripts:")
    show_script_list(ui)

    ui:separator()

    -- Editor
    local current_path = get_string("current_path", "")
    if current_path ~= "" then
        ui:label("Editing: " .. current_path .. dirty_indicator)

        local content = get_string("current_content", "")
        local new_content = ui:text_edit_multiline("editor", content, nil, 500.0)

        if new_content ~= content then
            set_string("current_content", new_content)
            local original = get_string("original_content", "")
            set_bool("is_dirty", new_content ~= original)
        end
    else
        ui:label("Select a script to edit")
    end
end

function refresh_script_list()
    -- This would list available scripts in the scripts directory
    set_bool("needs_refresh", false)
    set_string("status_message", "Script list refreshed")
end

function show_script_list(ui)
    -- Simple placeholder - in a real implementation this would scan the scripts directory
    ui:label("(Script listing not yet implemented)")
end
