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

local PANEL_WIDTH = 640.0

local function spacer(ui, lines)
    lines = lines or 1
    for _ = 1, lines do
        ui:label("")
    end
end

local function hero_section(ui)
    spacer(ui, 2)
    ui:heading("✨ Welcome to WebGPU Editor")
    ui:label("Design, iterate, and ship GPU-driven experiences from one place.")
    ui:label("Pick a starting point below to begin creating.")
    spacer(ui, 1)
    ui:separator()
    spacer(ui, 1)
end

local function render_open_button(ui)
    if ui:button("📂  Open Project Folder...") then
        local path = open_folder_dialog()
        if path then
            add_recent_project(path)
            load_project(path)
            set_string("status_message", "Loading project: " .. path)
            log_info("Requested project load: " .. path)
            set_bool("welcome_visible", false)
        end
    end

    ui:label("Open an existing workspace folder from disk.")
    spacer(ui, 1)
end

local function render_create_toggle(ui, show_create)
    local label = show_create and "➖  Hide Create Project" or "✨  Create New Project"
    if ui:button(label) then
        local new_state = not show_create
        set_bool("show_create_form", new_state)
        if new_state then
            set_string("status_message", "")
        end
        show_create = new_state
    end

    ui:label("Generate a fresh starter project with curated defaults.")
    spacer(ui, 1)

    return show_create
end

local function render_quick_actions(ui, show_create)
    ui:heading("🚀 Quick Start")
    ui:label("Choose how you'd like to jump in.")
    spacer(ui, 1)

    render_open_button(ui)
    return render_create_toggle(ui, show_create)
end

local function render_create_form(ui, current_path)
    spacer(ui, 1)
    ui:separator()
    spacer(ui, 1)

    ui:heading("🆕 Create a New Project")
    ui:label("Give it a name and choose a destination folder.")
    spacer(ui, 1)

    ui:label("Project Name")
    spacer(ui, 0)
    local project_name = get_string("new_project_name", "MyProject")
    local new_name = ui:text_edit("project_name_input", project_name)
    if new_name ~= project_name then
        set_string("new_project_name", new_name)
        project_name = new_name
    end

    spacer(ui, 1)
    ui:label("Install Location")
    if current_path == "" then
        ui:label("(No folder selected yet)")
    else
        ui:label("📁  " .. current_path)
    end

    spacer(ui, 1)
    if ui:button("Choose Folder...") then
        local path = open_folder_dialog()
        if path then
            current_path = path
            set_string("new_project_path", path)
            set_string("status_message", "Location set to: " .. path)
        end
    end

    spacer(ui, 1)
    local can_create = project_name ~= "" and current_path ~= ""
    local button_label = can_create and "🚀  Create Project" or "Select a name & folder to continue"
    if ui:button(button_label) and can_create then
        local full_path = current_path .. "/" .. project_name
        add_recent_project(full_path)
        create_project(project_name, current_path)
        set_string("status_message", "Creating project: " .. full_path)
        log_info("Requested project creation: " .. project_name .. " at " .. current_path)
        set_bool("show_create_form", false)
        set_string("new_project_path", "")
        set_bool("welcome_visible", false)
        current_path = ""
    end

    spacer(ui, 1)
    return current_path
end

local function render_recent_projects_section(ui)
    spacer(ui, 1)
    ui:separator()
    spacer(ui, 1)

    ui:heading("📁 Recent Projects")
    ui:label("Jump back into something you opened recently.")
    spacer(ui, 1)

    local recent_projects = load_recent_projects()
    if #recent_projects == 0 then
        ui:label("No recent projects yet.")
        ui:label("Open or create a project and it will appear here.")
        spacer(ui, 1)
        return
    end

    for _, project_path in ipairs(recent_projects) do
        local folder_name = project_path:match("([^/\\]+)$") or project_path

        if ui:button("▶  " .. folder_name) then
            add_recent_project(project_path)
            load_project(project_path)
            set_string("status_message", "Loading project: " .. project_path)
            log_info("Requested project load from recent: " .. project_path)
            set_bool("welcome_visible", false)
            return
        end

        ui:label("   " .. project_path)
        spacer(ui, 1)
    end
end

local function render_tips_section(ui)
    spacer(ui, 1)
    ui:separator()
    spacer(ui, 1)

    ui:heading("💡 Helpful Hints")
    ui:label("• Press Play to preview your scene instantly.")
    ui:label("")
    ui:label("• Drag a folder into the editor to load it as a project.")
    ui:label("")
    ui:label("• UI plugins live in examples/scripts — tweak them anytime.")
end

local function render_status_line(ui, status)
    if status == "" then
        return
    end

    spacer(ui, 1)
    ui:separator()
    spacer(ui, 1)
    ui:label("Status: " .. status)
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

    ui:centered_area(PANEL_WIDTH, function(center)
        hero_section(center)
        show_create = render_quick_actions(center, show_create)

        if show_create then
            new_project_path = render_create_form(center, new_project_path)
        end

        render_recent_projects_section(center)
        render_tips_section(center)
        render_status_line(center, status)
    end)
end
