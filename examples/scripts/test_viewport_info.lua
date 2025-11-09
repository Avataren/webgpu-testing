-- @tool
-- Test script to validate viewport information is correctly passed to Lua UI

function on_ui(self_entity, ui)
    ui:heading("Viewport Info Test")
    ui:separator()

    -- Get viewport size
    local vp = ui:get_viewport_size()
    ui:label("Viewport width: " .. tostring(vp.width))
    ui:label("Viewport height: " .. tostring(vp.height))

    -- Get DPI scaling
    local dpi = ui:get_pixels_per_point()
    ui:label("DPI scale (pixels per point): " .. tostring(dpi))

    ui:separator()

    -- Show calculated physical pixels
    if vp.width > 0 and vp.height > 0 then
        local phys_width = vp.width * dpi
        local phys_height = vp.height * dpi
        ui:label("Physical pixels: " .. tostring(math.floor(phys_width)) .. " x " .. tostring(math.floor(phys_height)))
    else
        ui:label("Viewport not available (no game viewport visible?)")
    end

    ui:separator()
    ui:label("This @tool script runs in both editor and play modes")
end
