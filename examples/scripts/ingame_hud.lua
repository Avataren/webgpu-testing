-- @tool
-- Example in-game HUD using anchored panels.

local function format_fps(dt)
    if dt <= 0 then
        return "FPS: --"
    end
    return string.format("FPS: %.1f", 1.0 / dt)
end

function on_ui(self_entity, ui)
    ui:anchored_panel("hud_health", "top_left", 16, 16, 200, 50, function(panel)
        panel:heading("Health")
        panel:label("HP: " .. tostring(get_i32("player_health", 100)))
    end)

    ui:anchored_panel("pause_menu", "center", 0, 0, 260, 160, function(panel)
        panel:heading("Pause Menu")
        panel:label(format_fps(get_f64("last_dt", 0.0)))
        panel:separator()
        panel:button("Resume")
        panel:button("Quit")
    end)
end

function update(self_entity, dt)
    set_f64("last_dt", dt)
end
