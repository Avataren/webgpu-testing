-- The HUD: the market's name and what is going on (people walking and lingering, the
-- gate, the carts), the camera shot, and the keys.

local Hud = { hidden = false, fps = 0 }

local GOLD = { 1.0, 0.82, 0.5, 1.0 }
local SOFT = { 0.95, 0.92, 0.86, 0.9 }

function Hud.update(dt)
    if dt > 0 then
        Hud.fps = Hud.fps + (1 / dt - Hud.fps) * math.min(1, dt * 2)
    end
end

function Hud.draw(ui, info)
    if Hud.hidden then
        ui:screen_label("bottom_left", 16, 12, "H: show the HUD", { color = SOFT, size = 13 })
        return
    end
    if info.loading then
        ui:screen_label("center", 0, -20, "Lantern Market", { color = GOLD, size = 46 })
        ui:screen_label("center", 0, 34, info.loading, { color = SOFT, size = 18 })
        return
    end
    if info.nocast then
        ui:screen_label("center", 0, -30, "Lantern Market", { color = GOLD, size = 46 })
        ui:screen_label("center", 0, 30, "No townsfolk yet: fill the asset library and run", { color = SOFT, size = 18 })
        ui:screen_label("center", 0, 58, "games/lantern-market/tools/fetch_cast.py", { color = SOFT, size = 16 })
        return
    end
    ui:anchored_panel("market_status", "top_left", 14, 14, 330, nil, function(panel)
        panel:heading("Lantern Market")
        panel:label(string.format("%d people: %d walking, %d lingering", info.people, info.walking, info.lingering))
        panel:label("Gate: " .. info.gate)
        panel:label("Carts: " .. info.carts)
        panel:label(string.format("Navmesh: %d polygons, %d obstacles carving", info.polys, info.carving))
        panel:label(string.format("%.0f fps", Hud.fps))
    end)
    ui:screen_label("top_right", 18, 14, info.shot, { color = GOLD, size = 18 })
    ui:screen_label(
        "bottom_left",
        16,
        12,
        "Tab next shot   1-8 shots   F follow   N navmesh   P paths   X x-ray   G gate   Click: call people over   H hide",
        { color = SOFT, size = 13 }
    )
end

return Hud
