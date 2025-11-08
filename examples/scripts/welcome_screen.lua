-- @editor
-- Welcome Screen Plugin
-- VSCode-style welcome screen with quick actions and recent projects

function on_created(self_entity)
    log_info("Welcome Screen plugin created")

    -- Initialize state
    set_string("new_project_name", "MyProject")
    set_string("new_project_path", "")
    set_string("status_message", "")
    set_bool("show_create_form", false)
end

function on_ui(self_entity, ui)
    local show_create = get_bool("show_create_form", false)
    local status = get_string("status_message", "")
    local new_project_path = get_string("new_project_path", "")

    -- Header section
    ui:heading("Welcome to WebGPU Editor")
    ui:label("Get started by opening an existing project or creating a new one")
    ui:separator()

    -- Quick Actions Section
    ui:heading("Start")

    if ui:button("📂  Open Project Folder...") then
        local path = open_folder_dialog()
        if path then
            -- TODO: Send command to editor to load project
            set_string("status_message", "Selected: " .. path)
            log_info("Open project: " .. path)
        end
    end

    if ui:button("✨  Create New Project") then
        local new_show_create = not show_create
        set_bool("show_create_form", new_show_create)
        if new_show_create then
            set_string("status_message", "")
        end
    end

    ui:separator()

    -- Create Project Form (shown when "Create New Project" is clicked)
    if show_create then
        ui:heading("Create New Project")
        ui:label("")

        ui:label("Project Name:")
        local project_name = get_string("new_project_name", "MyProject")
        local changed, new_name = ui:text_edit("project_name_input", project_name)
        if changed then
            set_string("new_project_name", new_name)
        end

        ui:label("")
        ui:label("Location:")
        if new_project_path == "" then
            ui:label("(no folder selected)")
        else
            ui:label(new_project_path)
        end

        if ui:button("Choose Folder...") then
            local path = open_folder_dialog()
            if path then
                set_string("new_project_path", path)
                set_string("status_message", "Location set to: " .. path)
            end
        end

        ui:label("")

        -- Only allow creating if both name and path are set
        local can_create = project_name ~= "" and new_project_path ~= ""
        local button_label = "Create Project"
        if not can_create then
            button_label = "Create Project (choose name and location first)"
        end

        if ui:button(button_label) then
            if can_create then
                -- TODO: Send command to editor to create project
                local full_path = new_project_path .. "/" .. project_name
                set_string("status_message", "Creating project: " .. full_path)
                log_info("Create project: " .. project_name .. " at " .. new_project_path)

                -- Reset form after creation attempt
                set_bool("show_create_form", false)
                set_string("new_project_path", "")
            end
        end

        ui:separator()
    end

    -- Recent Projects Section
    ui:heading("Recent Projects")
    ui:label("No recent projects")
    ui:label("")
    ui:label("Recent projects will appear here once you")
    ui:label("open or create your first project.")
    ui:separator()

    -- Getting Started Tips
    ui:heading("Getting Started")
    ui:label("💡 Projects contain all your scenes, assets, and scripts")
    ui:label("📁 Projects are stored as folders on your disk")
    ui:label("🎮 Each project can have multiple scenes")

    -- Status message
    if status ~= "" then
        ui:separator()
        ui:label(status)
    end
end
