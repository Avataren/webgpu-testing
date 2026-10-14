-- Voxel models: each sprite of sprites.lua becomes one cube per same-coloured rectangle of
-- pixels, built once per model and shared by every actor of that kind.

local C = require("robotron.config")
local SPR, COLORS, VOX = C.SPR, C.COLORS, C.VOX
local S = require("robotron.state")
local reg = S.reg

-- one hidden template entity per colour; every cube of that colour draws with its material
local mats = {}
local function mat_of(name)
    local m = mats[name]
    if m then return m end
    local c = COLORS[name]
    local e = reg(spawn_primitive("cube", "RT_mat_" .. name))
    set_material(e, { color = { c[1], c[2], c[3] }, emissive = c[4], unlit = c[5] or false, roughness = 0.55, metallic = 0.05 })
    set_component(e, "Visible", false)
    set_component(e, "CanCastShadow", false)
    set_translation(e, 0, -80, 0)
    mats[name] = e
    return e
end

-- merge same-coloured pixels into rectangles (grid[r][c] = colour or nil)
local function greedy(grid, W, H)
    local used, out = {}, {}
    for r = 1, H do used[r] = {} end
    for r = 1, H do
        for c = 1, W do
            local col = grid[r][c]
            if col and not used[r][c] then
                local w = 1
                while c + w <= W and grid[r][c + w] == col and not used[r][c + w] do w = w + 1 end
                local h, ok = 1, true
                while ok and r + h <= H do
                    for i = 0, w - 1 do
                        if grid[r + h][c + i] ~= col or used[r + h][c + i] then ok = false break end
                    end
                    if ok then h = h + 1 end
                end
                for rr = r, r + h - 1 do
                    for cc = c, c + w - 1 do used[rr][cc] = true end
                end
                out[#out + 1] = { c = c, r = r, w = w, h = h, col = col }
            end
        end
    end
    return out
end

local function prep_model(key, def)
    local frames = def.frames
    local H, W = #frames[1], #frames[1][1]
    local grids = {}
    for f, rows in ipairs(frames) do
        if #rows ~= H then error(string.format("sprite %s frame %d has %d rows, expected %d", key, f, #rows, H)) end
        local gr = {}
        for r, line in ipairs(rows) do
            if #line ~= W then error(string.format("sprite %s frame %d row %d is %d wide, expected %d", key, f, r, #line, W)) end
            gr[r] = {}
            for c = 1, W do
                local ch = line:sub(c, c)
                if ch ~= "." then
                    local col = def.legend[ch]
                    if not col or not COLORS[col] then error(string.format("sprite %s: no colour for '%s'", key, ch)) end
                    gr[r][c] = col
                end
            end
        end
        grids[f] = gr
    end
    local nf = #frames
    local static, per = {}, {}
    for f = 1, nf do per[f] = {} end
    for r = 1, H do
        static[r] = {}
        for f = 1, nf do per[f][r] = {} end
        for c = 1, W do
            local v = grids[1][r][c]
            local same = v ~= nil
            for f = 2, nf do
                if grids[f][r][c] ~= v then same = false break end
            end
            if same then
                static[r][c] = v
            else
                for f = 1, nf do per[f][r][c] = grids[f][r][c] end
            end
        end
    end
    local m = { key = key, W = W, H = H, depth = def.depth or 3, nf = nf, static = greedy(static, W, H), frames = {}, px = {} }
    if nf > 1 then
        for f = 1, nf do m.frames[f] = greedy(per[f], W, H) end
    end
    -- frame-1 pixels in model space (x, height, colour) for the particle effects
    for r = 1, H do
        for c = 1, W do
            local col = grids[1][r][c]
            if col then m.px[#m.px + 1] = { (c - 0.5 - W / 2) * VOX, (H - r + 0.5) * VOX, col } end
        end
    end
    m.height = H * VOX
    return m
end

local models = {}
local function model(key)
    local m = models[key]
    if m then return m end
    local def
    local v = key:match("^electrode(%d+)$")
    if v then def = SPR.electrode(tonumber(v)) else def = SPR[key] end
    m = prep_model(key, def)
    models[key] = m
    return m
end

-- builds the cubes of model m under root; returns the frame nodes and every cube with its colour
local function build_model(root, m, cast)
    local cubes = {}
    local function add(parent, list)
        for _, rc in ipairs(list) do
            local e = spawn_primitive("cube", "RT_v", { material_of = mat_of(rc.col) })
            set_parent(e, parent)
            set_translation(e, (rc.c - 1 + rc.w / 2 - m.W / 2) * VOX, (m.H - (rc.r - 1) - rc.h / 2) * VOX, 0)
            set_scale(e, rc.w * VOX, rc.h * VOX, m.depth * VOX)
            if not cast then set_component(e, "CanCastShadow", false) end
            cubes[#cubes + 1] = { e, rc.col }
        end
    end
    add(root, m.static)
    local fnodes = {}
    if m.nf > 1 then
        for f = 1, m.nf do
            local n = spawn_entity("RT_frame")
            set_parent(n, root)
            if f ~= 1 then set_component(n, "Visible", false) end
            add(n, m.frames[f])
            fnodes[f] = n
        end
    end
    return fnodes, cubes
end

return {
    mat_of = mat_of, model = model, build_model = build_model,
}
