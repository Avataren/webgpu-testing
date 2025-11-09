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

local function remember_directory(path)
    if not path or path == "" then
        return
    end

    local dir = get_file_directory(path)
    if dir ~= "" then
        set_string("last_directory", dir)
    end
end

local function load_file_from_path(path)
    if not path or path == "" then
        return false
    end

    local content = read_file(path)
    if string.sub(content, 1, 5) == "ERROR" then
        set_string("status_message", content)
        return false
    end

    set_string("text_content", content)
    set_string("file_path", path)
    set_bool("has_unsaved_changes", false)

    local version = ensure_file_version()
    set_f64("file_version", version + 1)

    remember_directory(path)

    set_string("status_message", "Loaded: " .. path)
    return true
end

local SUPPORTED_EXTENSIONS = { "lua", "wgsl", "txt", "rn", "rs", "toml", "json" }
local DEFAULT_NEW_NAME = "untitled.lua"
local PANEL_LIMITS = {
    min = 420,
    max = 980,
    margin = 32,
    min_editor_height = 180,
    max_editor_height = 860,
    editor_ratio = 0.75,
}
local TYPOGRAPHY_BASE = {
    line_px = 22,
    char_px = 8,
    separator_px = 6,
}
local ACTION_LAYOUT = {
    buttons = 4,
    button_width = 128,
    button_gap = 16,
    hint_lines = 1,
}

local function truncate_middle(text, max_chars)
    if not text then
        return ""
    end

    if max_chars <= 3 or #text <= max_chars then
        return text
    end

    local head = math.floor((max_chars - 3) / 2)
    local tail = max_chars - 3 - head

    return string.sub(text, 1, head) .. "..." .. string.sub(text, -tail)
end

local function format_display_name(path)
    if not path or path == "" then
        return "Untitled document"
    end

    if #path > 56 then
        return ".../" .. get_filename(path)
    end

    return path
end

local function format_directory_label(path)
    if not path or path == "" then
        return "Workspace root"
    end

    local dir = get_file_directory(path)
    if dir == "" then
        return "Workspace root"
    end

    return truncate_middle(dir, 48)
end

local function derive_typography(pixels_per_point)
    local ppp = math.max(0.5, pixels_per_point or 1)
    return {
        line = TYPOGRAPHY_BASE.line_px / ppp,
        char = math.max(4, TYPOGRAPHY_BASE.char_px / ppp),
        separator = TYPOGRAPHY_BASE.separator_px / ppp,
    }
end

local function estimate_lines(text, chars_per_line)
    if not text or text == "" then
        return 0
    end
    local capacity = math.max(1, chars_per_line or #text)
    return math.max(1, math.ceil(#text / capacity))
end

local function estimate_action_rows(width)
    local available = math.max(ACTION_LAYOUT.button_width, width - (PANEL_LIMITS.margin * 2))
    local space_per_button = ACTION_LAYOUT.button_width + ACTION_LAYOUT.button_gap
    local per_row = math.max(1, math.floor(available / space_per_button))
    return math.ceil(ACTION_LAYOUT.buttons / per_row)
end

local function estimate_reserved_height(state, metrics)
    local chars_per_line = math.max(12, math.floor((metrics.panel_width - PANEL_LIMITS.margin) / metrics.typography.char))
    local doc_label = "Document: " .. format_display_name(state.file_path)
    local status_badge = state.has_unsaved and "[unsaved]" or "[saved]"
    local header_lines = 1
    if (#doc_label + #status_badge + 4) > chars_per_line then
        header_lines = 2
    end
    local folder_lines = estimate_lines("Folder: " .. format_directory_label(state.file_path), chars_per_line)
    local status_lines = estimate_lines("Status: " .. (state.status or "Ready"), chars_per_line)
    local action_rows = estimate_action_rows(metrics.panel_width)

    local reserved = 0
    reserved = reserved + (header_lines + folder_lines + status_lines) * metrics.typography.line
    reserved = reserved + (ACTION_LAYOUT.hint_lines * metrics.typography.line)
    reserved = reserved + (action_rows * (metrics.typography.line + metrics.spacing.vertical))
    reserved = reserved + (metrics.spacing.vertical * 5)
    reserved = reserved + (metrics.typography.separator * 2)
    return reserved
end

local function compute_layout_metrics(ui, state)
    local viewport = ui:get_viewport_size() or {}
    local width = viewport.width or 0
    local height = viewport.height or 0
    local pixels_per_point = ui:get_pixels_per_point() or 1

    if width <= 0 then
        width = 960
    end

    if height <= 0 then
        height = 720
    end

    local clamped_width = math.max(
        PANEL_LIMITS.min,
        math.min(PANEL_LIMITS.max, width - (PANEL_LIMITS.margin * 2))
    )

    local typography = derive_typography(pixels_per_point)
    local spacing = {
        vertical = typography.line * 0.35,
        horizontal = typography.line * 0.5,
    }

    local reserved_height = estimate_reserved_height(state, {
        panel_width = clamped_width,
        typography = typography,
        spacing = spacing,
    })

    local available = math.max(0, height - reserved_height)
    local editor_height = available
    if PANEL_LIMITS.max_editor_height then
        editor_height = math.min(editor_height, PANEL_LIMITS.max_editor_height)
    end
    if PANEL_LIMITS.editor_ratio and PANEL_LIMITS.editor_ratio > 0 then
        editor_height = math.min(editor_height, height * PANEL_LIMITS.editor_ratio)
    end
    if available >= PANEL_LIMITS.min_editor_height then
        editor_height = math.max(editor_height, PANEL_LIMITS.min_editor_height)
    end
    if available <= 0 then
        editor_height = typography.line * 4
    end

    return {
        viewport = viewport,
        panel_width = clamped_width,
        editor_width = math.max(320, width - (PANEL_LIMITS.margin * 2)),
        editor_height = editor_height,
        typography = typography,
        spacing = spacing,
    }
end

local LayoutContext = {}
LayoutContext.__index = LayoutContext

function LayoutContext.new(ui, metrics, orientation)
    return setmetatable({
        ui = ui,
        metrics = metrics,
        orientation = orientation or "column",
    }, LayoutContext)
end

function LayoutContext:gap(style)
    style = style or "space"

    if self.orientation == "row" then
        if style == "section" then
            self.ui:label("   ")
        elseif type(style) == "number" then
            local blocks = math.max(1, math.floor(style / 8))
            self.ui:label(string.rep(" ", blocks))
        elseif style ~= "none" then
            self.ui:label(" ")
        end
        return
    end

    if style == "space" then
        self.ui:label("")
    elseif style == "section" or style == "line" then
        self.ui:separator()
    elseif type(style) == "number" then
        local lines = math.max(1, math.floor(style / 8))
        for _ = 1, lines do
            self.ui:label("")
        end
    end
end

function LayoutContext:surface(width, node, opts)
    opts = opts or {}

    local target_width = width or self.metrics.panel_width
    local fill_parent = opts.fill_parent or opts.full_bleed or target_width == "fill"

    if fill_parent then
        node(LayoutContext.new(self.ui, self.metrics, "column"))
        return
    end

    self.ui:centered_area(target_width, function(inner)
        node(LayoutContext.new(inner, self.metrics, "column"))
    end)
end

local Layout = {}

function Layout.stack(children, opts)
    opts = opts or {}
    local gap = opts.gap or "space"
    local nodes = children or {}

    return function(ctx)
        for index, child in ipairs(nodes) do
            child(ctx)
            if index < #nodes then
                ctx:gap(gap)
            end
        end
    end
end

function Layout.row(children, opts)
    opts = opts or {}
    local gap = opts.gap or "space"
    local wrap = opts.wrap or false
    local nodes = children or {}

    return function(ctx)
        local function render_row(row_ui)
            local row_ctx = LayoutContext.new(row_ui, ctx.metrics, "row")
            for index, child in ipairs(nodes) do
                child(row_ctx)
                if index < #nodes then
                    row_ctx:gap(gap)
                end
            end
        end

        if wrap then
            ctx.ui:horizontal_wrapped(function(row_ui)
                render_row(row_ui)
            end)
        else
            ctx.ui:horizontal(function(row_ui)
                render_row(row_ui)
            end)
        end
    end
end

function Layout.panel(config)
    local title = config.title
    local description = config.description
    local body = config.body
    local chrome = config.chrome or {}

    return function(ctx)
        if title then
            if chrome.heading == false then
                ctx.ui:label(title)
            else
                ctx.ui:heading(title)
            end
        end

        if description and description ~= "" then
            ctx.ui:label(description)
        end

        if body then
            local inner_gap = chrome.inner_gap
            if inner_gap == nil then
                inner_gap = (title or description) and "space" or "none"
            end

            if inner_gap ~= "none" then
                ctx:gap(inner_gap)
            end

            body(ctx)
        end
    end
end

local function open_document_from_dialog(last_dir)
    local directory = last_dir
    if not directory or directory == "" then
        directory = "content"
    end

    local path = open_file_dialog(SUPPORTED_EXTENSIONS, directory)
    if path then
        load_file_from_path(path)
    end
end

local function save_document(file_path)
    if not file_path or file_path == "" then
        set_string("status_message", "No file selected")
        return
    end

    local content = get_string("text_content", "")
    local error = write_file(file_path, content)
    if error == "" then
        set_bool("has_unsaved_changes", false)
        remember_directory(file_path)
        set_string("status_message", "Saved: " .. file_path)
    else
        set_string("status_message", error)
    end
end

local function save_document_as(current_path, last_dir)
    local content = get_string("text_content", "")
    local default_name = DEFAULT_NEW_NAME
    if current_path and current_path ~= "" then
        default_name = get_filename(current_path)
    end

    local directory = last_dir
    if not directory or directory == "" then
        directory = "content"
    end

    local path = save_file_dialog(SUPPORTED_EXTENSIONS, default_name, directory)
    if not path then
        return
    end

    local error = write_file(path, content)
    if error == "" then
        set_string("file_path", path)
        set_bool("has_unsaved_changes", false)
        remember_directory(path)

        set_string("status_message", "Saved: " .. path)
    else
        set_string("status_message", error)
    end
end

local function new_document()
    set_string("text_content", "")
    set_string("file_path", "")
    set_bool("has_unsaved_changes", false)

    local version = ensure_file_version()
    set_f64("file_version", version + 1)

    set_string("status_message", "New file created")
end

local function build_action_nodes(state)
    local actions = {
        {
            label = "Open...",
            handler = function()
                open_document_from_dialog(state.last_dir)
            end,
            enabled = true,
        },
        {
            label = "Save",
            handler = function()
                save_document(state.file_path)
            end,
            enabled = state.file_path ~= "",
        },
        {
            label = "Save As...",
            handler = function()
                save_document_as(state.file_path, state.last_dir)
            end,
            enabled = true,
        },
        {
            label = "New",
            handler = function()
                new_document()
            end,
            enabled = true,
        },
    }

    local nodes = {}
    for _, action in ipairs(actions) do
        table.insert(nodes, function(ctx)
            if not action.enabled then
                ctx.ui:label(action.label .. " (disabled)")
                return
            end

            if ctx.ui:button(action.label) then
                action.handler()
            end
        end)
    end

    return nodes
end

local function document_header_node(state)
    local display = format_display_name(state.file_path)
    if state.has_unsaved then
        display = display .. " *"
    end

    local folder = format_directory_label(state.file_path)
    local status_badge = state.has_unsaved and "[unsaved]" or "[saved]"

    return Layout.stack({
        Layout.row({
            function(ctx)
                ctx.ui:label("Document: " .. display)
            end,
            function(ctx)
                ctx.ui:label(status_badge)
            end,
        }, { gap = "space", wrap = true }),
        function(ctx)
            ctx.ui:label("Folder: " .. folder)
        end,
    }, { gap = "none" })
end

local function status_message_node(state)
    local status = state.status or "Ready"
    status = truncate_middle(status, 96)

    return function(ctx)
        ctx.ui:label("Status: " .. status)
    end
end

local function editor_panel_node(state)
    return Layout.panel({
        title = "Editor",
        body = function(ctx)
            local new_content = ctx.ui:text_edit_multiline(
                state.widget_id,
                state.content,
                state.editor_width,
                state.editor_height
            )

            if new_content ~= state.content then
                set_bool("has_unsaved_changes", true)
                set_string("text_content", new_content)
                state.content = new_content
                state.has_unsaved = true
            end
        end,
    })
end

local function actions_panel_node(state)
    return Layout.stack({
        Layout.row(build_action_nodes(state), { gap = "space", wrap = true }),
        function(ctx)
            ctx.ui:label("Hint: File menu mirrors these actions.")
        end,
    }, { gap = "space" })
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
    local state = {}

    local function refresh_state()
        state.file_path = get_string("file_path", "")
        state.status = get_string("status_message", "Ready")
        state.has_unsaved = get_bool("has_unsaved_changes", false)
        state.last_dir = get_string("last_directory", "content")
    end

    refresh_state()

    -- Check if the editor has been asked (by the host) to open a file
    if fetch_text_editor_request then
        local requested_path = fetch_text_editor_request()
        if requested_path and requested_path ~= "" and load_file_from_path(requested_path) then
            refresh_state()
        end
    end

    -- Menu bar
    ui:menu_bar(function(ui_bar)
        ui_bar:menu("File", function(menu)
            if menu:menu_item("file_open", "Open...") then
                open_document_from_dialog(state.last_dir)
            end

            if state.file_path ~= "" and menu:menu_item("file_save", "Save") then
                save_document(state.file_path)
            end

            if menu:menu_item("file_save_as", "Save As...") then
                save_document_as(state.file_path, state.last_dir)
            end

            menu:separator()

            if menu:menu_item("file_new", "New") then
                new_document()
            end
        end)
    end)

    -- Sync local state after menu interactions (handlers mutate global state)
    refresh_state()

    -- Multiline text editor uses file_version in the widget ID to reset UI cache when loading new files
    state.content = get_string("text_content", "")
    local version = ensure_file_version()
    state.widget_id = "content_" .. tostring(math.floor(version))

    local metrics = compute_layout_metrics(ui, state)
    state.editor_width = metrics.editor_width
    state.editor_height = metrics.editor_height

    local layout_tree = Layout.stack({
        document_header_node(state),
        status_message_node(state),
        function(ctx)
            ctx.ui:separator()
        end,
        editor_panel_node(state),
        function(ctx)
            ctx.ui:separator()
        end,
        actions_panel_node(state),
    }, { gap = "space" })

    local root_ctx = LayoutContext.new(ui, metrics, "column")
    root_ctx:surface(metrics.panel_width, layout_tree, { fill_parent = true })
end
