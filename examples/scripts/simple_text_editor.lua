-- @editor
-- Simple Text Editor Plugin
-- A fully functional text editor with file open/save dialogs

local function ensure_file_version()
    local ok, value = pcall(function()
        return get_f64("file_version")
    end)

    if ok then
        return value
    end

    set_f64("file_version", 0)
    return 0
end

function on_created(self_entity)
    log_info("Simple Text Editor plugin created")

    -- Initialize state
    set_string("text_content", "")
    set_string("file_path", "")
    set_string("last_directory", "content") -- Default to content folder
    set_string("status_message", "Ready - Click 'File > Open' to get started")
    set_bool("has_unsaved_changes", false)
    set_f64("file_version", 0) -- Increment this when loading a new file to reset UI cache
end

function on_ui(self_entity, ui)
    -- Get current state
    local file_path = get_string("file_path", "")
    local status = get_string("status_message", "Ready")
    local has_unsaved = get_bool("has_unsaved_changes", false)
    local last_dir = get_string("last_directory", "content")

    -- Menu bar
    ui:menu_bar(function(ui)
        ui:menu("File", function(ui)
            if ui:menu_item("file_open", "Open...") then
                local path = open_file_dialog({"lua", "wgsl", "txt", "rn", "rs", "toml", "json"}, last_dir)
                if path then
                    local content = read_file(path)
                    if string.sub(content, 1, 5) == "ERROR" then
                        set_string("status_message", content)
                    else
                        set_string("text_content", content)
                        set_string("file_path", path)
                        set_bool("has_unsaved_changes", false)

                        -- Increment version to clear UI cache for the text editor
                        local version = ensure_file_version()
                        set_f64("file_version", version + 1)

                        -- Remember the directory for next time
                        local dir = get_file_directory(path)
                        if dir ~= "" then
                            set_string("last_directory", dir)
                        end

                        set_string("status_message", "Loaded: " .. path)
                    end
                end
            end

            -- Save - only if we have a file path
            if file_path ~= "" then
                if ui:menu_item("file_save", "Save") then
                    local content = get_string("text_content", "")
                    local error = write_file(file_path, content)
                    if error == "" then
                        set_bool("has_unsaved_changes", false)
                        set_string("status_message", "Saved: " .. file_path)
                    else
                        set_string("status_message", error)
                    end
                end
            end

            if ui:menu_item("file_save_as", "Save As...") then
                local content = get_string("text_content", "")

                -- Get the current filename or use default
                local default_name = "untitled.lua"
                if file_path ~= "" then
                    default_name = get_filename(file_path)
                end

                local path = save_file_dialog({"lua", "wgsl", "txt", "rn", "rs", "toml", "json"}, default_name, last_dir)
                if path then
                    local error = write_file(path, content)
                    if error == "" then
                        set_string("file_path", path)
                        set_bool("has_unsaved_changes", false)

                        -- Remember the directory for next time
                        local dir = get_file_directory(path)
                        if dir ~= "" then
                            set_string("last_directory", dir)
                        end

                        set_string("status_message", "Saved: " .. path)
                    else
                        set_string("status_message", error)
                    end
                end
            end

            ui:separator()

            if ui:menu_item("file_new", "New") then
                set_string("text_content", "")
                set_string("file_path", "")
                set_bool("has_unsaved_changes", false)

                -- Increment version to clear UI cache for the text editor
                local version = ensure_file_version()
                set_f64("file_version", version + 1)

                set_string("status_message", "New file created")
            end
        end)
    end)

    -- File path display
    ui:separator()
    if file_path ~= "" then
        local display_name = file_path
        -- Show just the filename if it's a long path
        if #file_path > 50 then
            display_name = ".../" .. get_filename(file_path)
        end
        local label = display_name
        if has_unsaved then
            label = display_name .. " *"
        end
        ui:label("File: " .. label)
    else
        ui:label("No file loaded")
    end

    -- Status message
    ui:separator()
    ui:label(status)
    ui:separator()

    -- Multiline text editor (fills available space)
    -- Use file_version in the widget ID to reset UI cache when loading new files
    local content = get_string("text_content", "")
    local version = ensure_file_version()
    local widget_id = "content_" .. tostring(math.floor(version))
    local new_content = ui:text_edit_multiline(widget_id, content, nil, nil)

    -- Track unsaved changes
    if new_content ~= content then
        set_bool("has_unsaved_changes", true)
    end

    set_string("text_content", new_content)
end
