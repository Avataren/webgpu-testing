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

    -- Header section
    ui:heading("Welcome to WebGPU Editor")
    ui:label("Get started by opening an existing project or creating a new one")
    ui:separator()

    -- Quick Actions Section
    ui:heading("Start")

    if ui:button("Open Project Folder...") then
        -- TODO: This needs open_folder_dialog() which doesn't exist yet
        -- For now, just show a message
        set_string("status_message", "Open folder dialog not yet implemented in Lua API")
        log_info("Open project requested")
    end

    if ui:button("Create New Project") then
        set_bool("show_create_form", not show_create)
        set_string("status_message", "")
    end

    ui:separator()

    -- Create Project Form (shown when "Create New Project" is clicked)
    if show_create then
        ui:heading("Create New Project")

        local project_name = get_string("new_project_name", "MyProject")
        local changed, new_name = ui:text_edit("project_name_input", project_name)
        if changed then
            set_string("new_project_name", new_name)
        end

        ui:label("Project Name: " .. project_name)
        ui:label("")
        ui:label("Location: (folder dialog not yet available)")

        if ui:button("Choose Folder...") then
            set_string("status_message", "Folder dialog not yet implemented in Lua API")
        end

        ui:label("")

        if ui:button("Create Project") then
            -- TODO: Implement project creation
            set_string("status_message", "Project creation: " .. project_name)
            log_info("Create project: " .. project_name)
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
    ui:label("Projects contain all your scenes, assets, and scripts")
    ui:label("Projects are stored as folders on your disk")
    ui:label("Each project can have multiple scenes")

    -- Status message
    if status ~= "" then
        ui:separator()
        ui:label("Status: " .. status)
    end
end
