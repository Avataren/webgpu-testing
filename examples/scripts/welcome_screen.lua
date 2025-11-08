-- @editor
-- Welcome Screen Plugin
-- VSCode-style welcome screen with quick actions and recent projects

-- Path to recent projects file
local RECENT_PROJECTS_FILE = "examples/scripts/.recent_projects.txt"
local MAX_RECENT_PROJECTS = 10

-- Helper function to load recent projects from file
local function load_recent_projects()
    if not file_exists(RECENT_PROJECTS_FILE) then
        return {}
    end

    local content = read_file(RECENT_PROJECTS_FILE)
    if content:match("^ERROR:") then
        return {}
    end

    local projects = {}
    for line in content:gmatch("[^\r\n]+") do
        local trimmed = line:match("^%s*(.-)%s*$")
        if trimmed ~= "" then
            table.insert(projects, trimmed)
        end
    end

    return projects
end

-- Helper function to save recent projects to file
local function save_recent_projects(projects)
    local content = table.concat(projects, "\n")
    write_file(RECENT_PROJECTS_FILE, content)
end

-- Helper function to add a project to recent projects (ensuring no duplicates)
local function add_recent_project(project_path)
    local projects = load_recent_projects()

    -- Remove duplicates (case-sensitive path comparison)
    local filtered = {}
    for _, p in ipairs(projects) do
        if p ~= project_path then
            table.insert(filtered, p)
        end
    end

    -- Add new project at the beginning
    table.insert(filtered, 1, project_path)

    -- Keep only MAX_RECENT_PROJECTS
    while #filtered > MAX_RECENT_PROJECTS do
        table.remove(filtered)
    end

    save_recent_projects(filtered)
end

function on_created(self_entity)
    log_info("Welcome Screen plugin created")

    -- Initialize state
    set_string("new_project_name", "MyProject")
    set_string("new_project_path", "")
    set_string("status_message", "")
    set_bool("show_create_form", false)
    set_bool("welcome_visible", true)
end

function on_ui(self_entity, ui)
    -- Don't show if not visible
    if not get_bool("welcome_visible", true) then
        return
    end

    local show_create = get_bool("show_create_form", false)
    local status = get_string("status_message", "")
    local new_project_path = get_string("new_project_path", "")

    -- Add top padding
    ui:label("")
    ui:label("")

    -- Header section with larger text
    ui:heading("Welcome to WebGPU Editor")
    ui:label("")
    ui:label("Get started by opening an existing project or creating a new one")
    ui:label("")
    ui:label("")
    ui:separator()
    ui:label("")
    ui:label("")

    -- Quick Actions Section
    ui:heading("Start")
    ui:label("")

    if ui:button("📂  Open Project Folder...") then
        local path = open_folder_dialog()
        if path then
            -- Add to recent projects
            add_recent_project(path)

            -- Send command to editor to load the project
            load_project(path)
            set_string("status_message", "Loading project: " .. path)
            log_info("Requested project load: " .. path)

            -- Hide welcome screen
            set_bool("welcome_visible", false)
        end
    end

    ui:label("")

    if ui:button("✨  Create New Project") then
        local new_show_create = not show_create
        set_bool("show_create_form", new_show_create)
        if new_show_create then
            set_string("status_message", "")
        end
    end

    ui:label("")
    ui:label("")
    ui:separator()
    ui:label("")

    -- Create Project Form (shown when "Create New Project" is clicked)
    if show_create then
        ui:label("")
        ui:heading("Create New Project")
        ui:label("")
        ui:label("")

        ui:label("Project Name:")
        ui:label("")
        local project_name = get_string("new_project_name", "MyProject")
        local new_name = ui:text_edit("project_name_input", project_name)
        if new_name ~= project_name then
            set_string("new_project_name", new_name)
        end

        ui:label("")
        ui:label("")
        ui:label("Location:")
        ui:label("")
        if new_project_path == "" then
            ui:label("(no folder selected)")
        else
            ui:label(new_project_path)
        end

        ui:label("")

        if ui:button("Choose Folder...") then
            local path = open_folder_dialog()
            if path then
                set_string("new_project_path", path)
                set_string("status_message", "Location set to: " .. path)
            end
        end

        ui:label("")
        ui:label("")

        -- Only allow creating if both name and path are set
        local can_create = project_name ~= "" and new_project_path ~= ""
        local button_label = "Create Project"
        if not can_create then
            button_label = "Create Project (choose name and location first)"
        end

        if ui:button(button_label) then
            if can_create then
                local full_path = new_project_path .. "/" .. project_name

                -- Add to recent projects
                add_recent_project(full_path)

                -- Send command to editor to create the project
                create_project(project_name, new_project_path)
                set_string("status_message", "Creating project: " .. full_path)
                log_info("Requested project creation: " .. project_name .. " at " .. new_project_path)

                -- Reset form after creation attempt
                set_bool("show_create_form", false)
                set_string("new_project_path", "")

                -- Hide welcome screen
                set_bool("welcome_visible", false)
            end
        end

        ui:label("")
        ui:label("")
        ui:separator()
        ui:label("")
    end

    -- Recent Projects Section
    ui:heading("Recent Projects")
    ui:label("")

    local recent_projects = load_recent_projects()
    if #recent_projects == 0 then
        ui:label("")
        ui:label("No recent projects")
        ui:label("")
        ui:label("Recent projects will appear here once you")
        ui:label("open or create your first project.")
        ui:label("")
    else
        for i, project_path in ipairs(recent_projects) do
            -- Extract just the folder name for display
            local folder_name = project_path:match("([^/\\]+)$") or project_path

            if ui:button("📁  " .. folder_name) then
                -- Add to recent (moves it to top)
                add_recent_project(project_path)

                -- Load the project
                load_project(project_path)
                set_string("status_message", "Loading project: " .. project_path)
                log_info("Requested project load from recent: " .. project_path)

                -- Hide welcome screen
                set_bool("welcome_visible", false)
            end

            -- Show full path as smaller text
            ui:label("  " .. project_path)

            if i < #recent_projects then
                ui:label("")
            end
        end
    end

    ui:label("")
    ui:label("")
    ui:separator()
    ui:label("")
    ui:label("")

    -- Getting Started Tips
    ui:heading("Getting Started")
    ui:label("")
    ui:label("💡 Projects contain all your scenes, assets, and scripts")
    ui:label("")
    ui:label("📁 Projects are stored as folders on your disk")
    ui:label("")
    ui:label("🎮 Each project can have multiple scenes")

    -- Status message
    if status ~= "" then
        ui:label("")
        ui:label("")
        ui:separator()
        ui:label("")
        ui:label(status)
    end
end
