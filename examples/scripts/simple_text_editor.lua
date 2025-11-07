-- @editor
-- Simple Text Editor Plugin
-- Demonstrates multiline text editing and file I/O

function on_created(self_entity)
    log_info("Simple Text Editor plugin created")

    -- Initialize state
    set_string("text_content", "")
    set_string("file_path", "examples/scripts/notes.txt")
    set_string("status_message", "Ready")
end

function on_ui(self_entity, ui)
    ui:heading("Simple Text Editor")

    -- Get current state
    local file_path = get_string("file_path", "examples/scripts/notes.txt")
    local status = get_string("status_message", "Ready")

    -- File path input
    ui:label("File path:")
    local new_path = ui:text_edit("file_path", file_path)
    set_string("file_path", new_path)

    -- Buttons
    ui:separator()

    if ui:button("Load File") then
        local path = get_string("file_path", "")
        local content = read_file(path)
        if string.sub(content, 1, 5) == "ERROR" then
            set_string("status_message", content)
        else
            set_string("text_content", content)
            local path_for_msg = get_string("file_path", "")
            set_string("status_message", "Loaded from " .. path_for_msg)
        end
    end

    if ui:button("Save File") then
        local path = get_string("file_path", "")
        local content = get_string("text_content", "")
        local error = write_file(path, content)
        if error == "" then
            local path_for_msg = get_string("file_path", "")
            set_string("status_message", "Saved to " .. path_for_msg)
        else
            set_string("status_message", error)
        end
    end

    if ui:button("New File") then
        set_string("text_content", "")
        set_string("status_message", "New file")
    end

    -- Status message
    ui:separator()
    ui:label(status)
    ui:separator()

    -- Multiline text editor
    ui:label("Content:")
    local content = get_string("text_content", "")
    local new_content = ui:text_edit_multiline("content", content, nil, 400.0)
    set_string("text_content", new_content)
end
