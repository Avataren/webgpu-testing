-- The attract-mode title: ROBOTRON over 2084, in voxels. Each letter's face stands one voxel
-- proud of a dark backing a pixel wider all round, shaded top to bottom with vertex colours, and
-- the letters cycle through the colours one after another like the arcade's title. The logo
-- hangs in front of the camera (a child of it), pops in when the demo starts and drops away
-- before play shows through.

local C = require("robotron.config")
local FOV, TITLE_T = C.FOV, C.TITLE_T
local S = require("robotron.state")
local g, reg, set_vis = S.g, S.reg, S.set_vis

local sin, floor, min, max, tan, rad = math.sin, math.floor, math.min, math.max, math.tan, math.rad

-- 7 x 9 glyphs with two-pixel strokes
local FONT = {
    R = { "######.", "#######", "##...##", "##...##", "######.", "#####..", "##.###.", "##..###", "##...##" },
    O = { ".#####.", "#######", "##...##", "##...##", "##...##", "##...##", "##...##", "#######", ".#####." },
    B = { "######.", "#######", "##...##", "##..##.", "######.", "##..###", "##...##", "#######", "######." },
    T = { "#######", "#######", "..###..", "..###..", "..###..", "..###..", "..###..", "..###..", "..###.." },
    N = { "##...##", "###..##", "####.##", "#######", "##.####", "##..###", "##...##", "##...##", "##...##" },
    ["2"] = { ".#####.", "#######", "##...##", ".....##", "..####.", ".####..", "##.....", "#######", "#######" },
    ["0"] = { ".#####.", "#######", "##..###", "##.####", "#######", "####.##", "###..##", "#######", ".#####." },
    ["8"] = { ".#####.", "#######", "##...##", "##...##", ".#####.", "#######", "##...##", "#######", ".#####." },
    ["4"] = { "....###", "...####", "..##.##", ".##..##", "##...##", "#######", "#######", ".....##", ".....##" },
}
local GW, GH, GAP = 7, 9, 2
local FACE, BACK = 2, 3            -- voxels of letter face and of backing
local VOXEL = 0.1
local DIST = 12.0                  -- metres in front of the camera
local SHOW_T = TITLE_T             -- seconds the title shows when the demo starts (as the HUD's subtitle)

local logo = nil

-- hue 0..1 -> rgb, full value
local function hue_rgb(h, s)
    h = (h % 1) * 6
    local i = floor(h)
    local f = h - i
    local p, q, t = 1 - s, 1 - s * f, 1 - s * (1 - f)
    if i == 0 then return 1, t, p elseif i == 1 then return q, 1, p elseif i == 2 then return p, 1, t
    elseif i == 3 then return p, q, 1 elseif i == 4 then return t, p, 1 else return 1, p, q end
end

-- the cells of a word: letter i's face is keyed "<i>:<row>" (its own part "L<i>", a vertex colour
-- per row), the backing is "back"
local function word_model(text, voxel)
    local n = #text
    local W, H, D = n * GW + (n - 1) * GAP + 2, GH + 2, FACE + BACK
    local face = {}
    for y = 0, H - 1 do face[y] = {} end
    for i = 1, n do
        local glyph = FONT[text:sub(i, i)]
        local ox = 1 + (i - 1) * (GW + GAP)
        for r = 1, GH do
            for c = 1, GW do
                if glyph[r]:sub(c, c) == "#" then face[H - 1 - r][ox + c - 1] = { i, r } end
            end
        end
    end
    local cells, colors, parts = {}, {}, {}
    local function set(x, y, z, k) cells[1 + x + W * (y + H * z)] = k end
    for y = 0, H - 1 do
        for x = 0, W - 1 do
            local f = face[y][x]
            local backed = f ~= nil
            for dy = -1, 1 do
                for dx = -1, 1 do
                    local row = face[y + dy]
                    if row and row[x + dx] then backed = true end
                end
            end
            if backed then
                for z = 0, BACK - 1 do set(x, y, z, "back") end
            end
            if f then
                local key = f[1] .. ":" .. f[2]
                -- bright along the top, darker toward the foot of each letter
                local v = 1.0 - 0.65 * (f[2] - 1) / (GH - 1)
                colors[key] = { v, v, v }
                parts[key] = "L" .. f[1]
                for z = BACK, D - 1 do set(x, y, z, key) end
            end
        end
    end
    return voxel_model { voxels = cells, size = { W, H, D }, voxel = voxel, pivot = { 0.5, 0.5, 0.5 },
        colors = colors, parts = parts }
end

local function spawn_word(parent, text, voxel, name)
    local m = word_model(text, voxel)
    local mats = { back = { color = { 0.06, 0.07, 0.16 }, emissive = 0.15, roughness = 0.3, metallic = 0.6 } }
    for i = 1, #text do mats["L" .. i] = { color = { 1, 1, 1 }, emissive = 0.9, roughness = 0.3, metallic = 0.0 } end
    local node, ents = spawn_voxels(m, name, { parent = parent, materials = mats, cast_shadow = false })
    local letters = {}
    for i = 1, #text do letters[i] = ents["L" .. i] end
    return { e = node, letters = letters, width = m.max[1] - m.min[1], height = m.max[2] - m.min[2] }
end

local function build_logo()
    if logo or not g.cam then return end
    local root = reg(spawn_entity("RT_logo"))
    set_parent(root, g.cam)
    local top = spawn_word(root, "ROBOTRON", VOXEL, "RT_logo_robotron")
    local low = spawn_word(root, "2084", VOXEL * 0.62, "RT_logo_2084")
    set_translation(low.e, 0, -(top.height + low.height) * 0.5 + VOXEL, -0.1)
    logo = { e = root, top = top, low = low, vis = true, width = top.width, height = top.height + low.height }
    set_vis(root, false)
    logo.vis = false
end

local function ease_out_back(t)
    local c = 1.9
    t = t - 1
    return 1 + (c + 1) * t * t * t + c * t * t
end

local function update_logo(dt)
    if not logo then build_logo() end
    if not logo then return end
    local t = g.demo_t
    local show = g.demo and t < SHOW_T
    if show ~= logo.vis then
        set_vis(logo.e, show)
        logo.vis = show
    end
    if not show then return end

    -- fit the logo to the view: at most 80% of its width and a third of its height
    local vp = get_viewport_size()
    local aspect = (vp and vp.height and vp.height > 0) and (vp.width / vp.height) or 1.78
    local hh = DIST * tan(rad(FOV) * 0.5)
    local fit = min(2 * hh * aspect * 0.8 / logo.width, 2 * hh * 0.34 / logo.height)

    -- pop in, sway, then drop away under the floor of the view
    local s, y = 1, hh * 0.5
    if t < 0.8 then s = max(0.001, ease_out_back(t / 0.8)) end
    local out = t - (SHOW_T - 0.7)
    if out > 0 then
        local k = out / 0.7
        y = y + k * 1.5 - k * k * 2.4 * hh
        s = 1 - 0.25 * k
    end
    set_translation(logo.e, 0, y, -DIST)
    -- tipped toward the camera so the tops of the letters show
    set_rotation(logo.e, 0.10 * sin(g.time * 0.9), 0.3 + 0.06 * sin(g.time * 1.3), 0.02 * sin(g.time * 0.7))
    set_scale(logo.e, fit * s, fit * s, fit * s)

    -- colour cycling: a rainbow that runs along the letters
    for _, w in ipairs({ logo.top, logo.low }) do
        local shift = w == logo.low and 0.5 or 0
        for i, e in ipairs(w.letters) do
            local r, gg, b = hue_rgb(g.time * 0.35 - i * 0.07 + shift, 1.0)
            set_material(e, { color = { r, gg, b } })
        end
    end
end

return {
    update_logo = update_logo,
}
