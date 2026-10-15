-- The HUD: score, men, wave, the attract-mode subtitle and popups.

local C = require("robotron.config")
local COLORS, TITLE_T = C.COLORS, C.TITLE_T
local S = require("robotron.state")
local g = S.g

local floor, min, max = math.floor, math.min, math.max

local function colv(name, a)
    local c = COLORS[name] or COLORS.white
    return { min(1, c[1] * 1.1), min(1, c[2] * 1.1), min(1, c[3] * 1.1), a or 1.0 }
end
local HUD_RED = { 1.0, 0.25, 0.2, 1 }

-- neon text: translucent copies around the label make a soft halo in its colour
local HALO = { { -2, 0 }, { 2, 0 }, { 0, -2 }, { 0, 2 }, { -3, -3 }, { 3, 3 }, { -3, 3 }, { 3, -3 } }
local function neon_label(ui, anchor, x, y, text, size, c, halo)
    local h = { c[1], c[2], c[3], (c[4] or 1) * (halo or 0.16) }
    local o = { size = size, color = h }
    local s = size / 40
    for i = 1, #HALO do
        ui:screen_label(anchor, x + HALO[i][1] * s, y + HALO[i][2] * s, text, o)
    end
    ui:screen_label(anchor, x, y, text, { size = size, color = c })
end

local function draw_hud(ui)
    neon_label(ui, "top_left", 24, 12, string.format("%d", g.score), 34, HUD_RED)
    local men = ""
    for _ = 1, min(max(g.lives - (g.p_alive and 1 or 0), 0), 10) do men = men .. "I" end
    ui:screen_label("top_left", 24, 52, men ~= "" and ("MEN " .. men) or "", { size = 18, color = { 1, 0.85, 0.3, 1 } })
    ui:screen_label("top_right", 24, 12, "HIGH " .. g.hi, { size = 22, color = { 0.6, 0.8, 1, 0.9 } })
    if g.wave > 0 then
        neon_label(ui, "bottom_center", 0, 8, g.wave .. " WAVE", 26, HUD_RED)
    end
    if g.bot and not g.demo then
        ui:screen_label("top_right", 24, 42, "AUTOPILOT", { size = 18, color = { 1, 0.6, 0.2, 1 } })
    end

    if g.demo then
        -- the title itself is the voxel logo (logo.lua)
        if g.demo_t < TITLE_T then
            local a = min(1, (TITLE_T - g.demo_t) * 0.5, g.demo_t * 1.5)
            ui:screen_label("center", 0, 10, "SAVE THE LAST HUMAN FAMILY", { size = 24, color = { 1, 1, 1, a * 0.9 } })
        end
        if floor(g.time * 2) % 2 == 0 then
            ui:screen_label("bottom_center", 0, 70, "PRESS ENTER TO PLAY", { size = 34, color = { 1, 1, 1, 1 } })
        end
        ui:screen_label("bottom_center", 0, 44, "WASD move   ARROWS or mouse fire", { size = 16, color = { 0.7, 0.8, 1, 0.9 } })
    end
    if g.paused then
        ui:screen_label("center", 0, -40, "PAUSED", { size = 56, color = { 1, 1, 1, 1 } })
    end
    if g.banner_t > 0 and g.banner ~= "" and not g.paused then
        local a = min(1, g.banner_t * 2)
        neon_label(ui, "center", 0, -40, g.banner, 60, { 1, 0.2, 0.15, a }, 0.22)
    end
    for i = 1, #g.popups do
        local p = g.popups[i]
        local c = colv(p.col, 1 - p.t / 1.3)
        ui:world_label(p.x, 1.4 + p.t * 1.2, p.z, p.text, { size = p.size, color = c })
    end
end

return {
    draw_hud = draw_hud,
}
