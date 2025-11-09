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
    set_state("script_entries", build_sample_entries())
end

function on_ui(self_entity, ui)
    -- Load scripts on first render or when requested
    local needs_refresh = get_bool("needs_refresh", true)
    if needs_refresh then
        refresh_script_list()
        return
    end

    local viewport = ui:get_viewport_size()
    local pixels_per_point = ui:get_pixels_per_point()
    local panel_width = math.max(640, math.min(1080, viewport.width * 0.9))

    local list_max_height = 500.0 / pixels_per_point
    local list_height = math.min(
        list_max_height,
        math.max(200.0 / pixels_per_point, viewport.height * 0.35)
    )
    local detail_height = math.max(
        320.0 / pixels_per_point,
        viewport.height - list_height - 280.0 / pixels_per_point
    )

    ui:centered_area(panel_width, function(panel)
        panel:heading("Script Editor")

        panel:label(string.format("Viewport: %.0f × %.0f (ppp %.2f)", viewport.width, viewport.height, pixels_per_point))

        -- Toolbar
        panel:separator()
        panel:label("Actions:")
        panel:separator()
        if panel:button("Refresh Scripts") then
            refresh_script_list()
            return
        end

        if panel:button("Save") then
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
        local dirty_indicator = is_dirty and " *" or ""

        panel:separator()

        -- Status bar
        local status_message = get_string("status_message", "Ready")
        panel:label(status_message)

        panel:separator()

        -- Script list
        panel:label("Available Scripts:")
        show_script_list(panel, list_height)

        panel:separator()

        -- Editor
        local current_path = get_string("current_path", "")
        if current_path ~= "" then
            panel:label("Editing: " .. current_path .. dirty_indicator)

            local content = get_string("current_content", "")
            local new_content = panel:text_edit_multiline("editor", content, nil, detail_height)

            if new_content ~= content then
                set_string("current_content", new_content)
                local original = get_string("original_content", "")
                set_bool("is_dirty", new_content ~= original)
            end
        else
            panel:label("Select a script to edit from the list above.")
        end
    end)
end

function refresh_script_list()
    local entries = build_sample_entries()
    set_state("script_entries", entries)
    set_bool("needs_refresh", false)
    set_f64("selected_index", -1.0)
    set_f64("script_scroll", 0.0)
    if #entries == 0 then
        set_string("status_message", "No scripts found in sample manifest.")
    else
        set_string("status_message", string.format("Loaded %d sample script(s)", #entries))
    end
end

function show_script_list(ui, max_height)
    local scripts = get_state("script_entries", {})
    if type(scripts) ~= "table" or #scripts == 0 then
        ui:label("No scripts discovered yet. Click Refresh to load sample entries.")
        return
    end

    local row_height = 32.0
    local visible_rows = math.max(1, math.floor(max_height / row_height))
    local max_offset = math.max(0, #scripts - visible_rows)
    local scroll = math.floor(get_f64("script_scroll", 0))
    if scroll > max_offset then
        scroll = max_offset
        set_f64("script_scroll", scroll)
    end

    if max_offset > 0 then
        local new_scroll = ui:slider("script_scroll_slider", scroll, 0, max_offset)
        new_scroll = math.floor(new_scroll + 0.5)
        if new_scroll ~= scroll then
            scroll = new_scroll
            set_f64("script_scroll", scroll)
        end
    else
        set_f64("script_scroll", 0)
    end

    local start_index = scroll + 1
    local end_index = math.min(#scripts, start_index + visible_rows - 1)

    for idx = start_index, end_index do
        local entry = scripts[idx]
        local label = string.format("[%02d] %s", idx, entry.label)
        if ui:button(label) then
            select_script_entry(idx, entry)
        end
    end

    if #scripts > visible_rows then
        ui:label(string.format("Showing %d-%d of %d scripts", start_index, end_index, #scripts))
    end
end

function select_script_entry(index, entry)
    local content = read_file(entry.path)
    if string.sub(content, 1, 5) == "ERROR" then
        set_string("status_message", content)
        return
    end

    set_string("current_path", entry.path)
    set_string("current_content", content)
    set_string("original_content", content)
    set_bool("is_dirty", false)
    set_string("status_message", "Loaded: " .. entry.path)
    set_f64("selected_index", index - 1)
end

function build_sample_entries()
    local samples = {
        { label = "Simple Text Editor", path = "examples/scripts/simple_text_editor.lua" },
        { label = "Welcome Screen", path = "examples/scripts/welcome_screen.lua" },
        { label = "UI Playground", path = "examples/scripts/test_minimal_ui.lua" },
        { label = "Advanced Widgets", path = "examples/scripts/ui_example_comprehensive.lua" },
    }

    local entries = {}
    for _, sample in ipairs(samples) do
        if file_exists(sample.path) then
            table.insert(entries, sample)
        end
    end
    return entries
end
