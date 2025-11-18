
-- Layout Playground Plugin
-- Demonstrates the centered_area helper and viewport-aware sizing utilities.

local PANEL_MIN = 320
local PANEL_MAX = 760

local FEATURE_CARDS = {
    {
        title = "Centered Panels",
        body = [[The new layout helper keeps your content readable on ultra-wide monitors.
Use viewport width to derive a friendly column width, then call ui:centered_area()
to clamp and center the entire section.]],
    },
    {
        title = "Viewport Metrics",
        body = [[You can now query ui:get_viewport_size() and ui:get_pixels_per_point().
These values make it easy to adapt padding, spacing, and typography to in-game or editor panels.]],
    },
    {
        title = "Stateful Sections",
        body = [[Combine the layout helpers with persistent script state (set_bool, set_f64, etc.)
to build configurable dashboards. Toggling compact mode in this demo updates spacing live.]],
    },
}

local function draw_metric(ui, label, value)
    ui:label(string.format("%-14s %s", label .. ":", value))
end

local function draw_card(ui, card, compact)
    ui:heading(card.title)
    ui:label(card.body)
    if not compact then
        ui:separator()
    else
        ui:label("")
    end
end

function on_created(self_entity)
    log_info("Layout Playground plugin ready")
    set_bool("layout_compact_cards", false)
end

function on_ui(self_entity, ui)
    local viewport = ui:get_viewport_size()
    local width = viewport.width

    -- Fallback width when the viewport has not been initialized yet.
    if width <= 0 then
        width = 480
    end

    local panel_width = math.max(PANEL_MIN, math.min(PANEL_MAX, width * 0.6))
    local pixels_per_point = ui:get_pixels_per_point()

    -- Top-level layout: keep everything centered and clamped to a readable width.
    ui:centered_area(panel_width, function(center)
        center:heading("Layout Playground")
        center:label("Example of the viewport-aware layout utilities in the Lua UI API.")
        center:separator()

        center:heading("Viewport Metrics")
        draw_metric(center, "Width", string.format("%.0f pt", viewport.width))
        draw_metric(center, "Height", string.format("%.0f pt", viewport.height))
        draw_metric(center, "Pixels/Point", string.format("%.2f", pixels_per_point))
        center:separator()

        local compact = get_bool("layout_compact_cards", false)
        local new_compact = center:checkbox(
            "layout_compact_cards",
            compact,
            "Compact card spacing"
        )
        if new_compact ~= compact then
            set_bool("layout_compact_cards", new_compact)
            compact = new_compact
        end

        center:separator()
        center:heading("Feature Cards")

        for _, card in ipairs(FEATURE_CARDS) do
            draw_card(center, card, compact)
        end
    end)
end

