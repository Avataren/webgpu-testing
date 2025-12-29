-- @tool
-- Example in-game HUD using anchored panels.

local function format_fps(dt)
    if dt <= 0 then
        return "FPS: --"
    end
    return string.format("FPS: %.1f", 1.0 / dt)
end

function on_ui(self_entity, ui)
    local viewport = ui:get_viewport_size()
    local width = viewport.width or 0
    local height = viewport.height or 0

    ui:anchored_panel("hud_top_left", 16, 16, function(panel)
        panel:heading("HUD")
        panel:label(format_fps(get_f64("last_dt", 0.0)))
    end)

    ui:anchored_panel("hud_bottom_right", width - 220, height - 80, function(panel)
        panel:label("Health: " .. tostring(get_i32("player_health", 100)))
        panel:label("Ammo: " .. tostring(get_i32("player_ammo", 30)))
    end)
end

function update(self_entity, dt)
    set_f64("last_dt", dt)
end
