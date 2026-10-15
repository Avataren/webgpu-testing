-- Voxel models: each frame of a sprite in sprites.lua becomes an engine voxel model (one
-- optimized mesh per colour), built once per model and shared by every actor of that kind.

local C = require("robotron.config")
local SPR, COLORS, VOX, BULGE = C.SPR, C.COLORS, C.VOX, C.BULGE
local S = require("robotron.state")
local reg = S.reg

-- Surface finishes: painted (the default) and polished metal for the robots. Glowing colours
-- (emissive 1 or more, or unlit) look the same in both.
local FINISH = {
    paint = { roughness = 0.55, metallic = 0.05, glow = 1.0 },
    -- clear-coated rather than bare: bare metal in this dark arena mirrors only black
    metal = { roughness = 0.12, metallic = 0.45, glow = 0.8 },
}

-- one hidden template entity per colour and finish; every part of that colour draws with its
-- material
local mats = {}
local function mat_of(name, finish)
    local c = COLORS[name]
    local f = FINISH[finish or "paint"]
    if c[5] or c[4] >= 1.0 then f = FINISH.paint end
    local key = f == FINISH.paint and name or name .. ":" .. finish
    local m = mats[key]
    if m then return m end
    local e = reg(spawn_primitive("cube", "RT_mat_" .. key))
    set_material(e, { color = { c[1], c[2], c[3] }, emissive = c[4] * f.glow, unlit = c[5] or false,
        roughness = f.roughness, metallic = f.metallic })
    set_component(e, "Visible", false)
    set_component(e, "CanCastShadow", false)
    set_translation(e, 0, -80, 0)
    mats[key] = e
    return e
end

-- A sprite frame as a solid: every pixel is extruded `depth` voxels, and more where the sprite is
-- wide, as if each run of pixels were the cross-section of a cylinder (the thinner of its row and
-- column runs), up to `bulge` extra a side. Bodies and heads swell into rounded shapes while
-- limbs and outlines stay slim. Returns the flat cell list voxel_model takes.
local function inflate(rows, legend, depth, bulge)
    local H, W = #rows, #rows[1]
    local col = {}
    for r = 1, H do
        col[r] = {}
        for c = 1, W do col[r][c] = legend[rows[r]:sub(c, c)] end
    end
    -- half the chord of the circle across a run of n pixels, at pixel i (1..n) of it
    local function chord(i, n)
        local h = n * 0.5
        local x = (i - 0.5 - h) / h
        return h * math.sqrt(math.max(0, 1 - x * x))
    end
    local extra, most = {}, 0
    for r = 1, H do extra[r] = {} end
    for r = 1, H do
        local c = 1
        while c <= W do
            if col[r][c] then
                local e = c
                while e < W and col[r][e + 1] do e = e + 1 end
                for i = c, e do extra[r][i] = chord(i - c + 1, e - c + 1) end
                c = e + 1
            else
                c = c + 1
            end
        end
    end
    for c = 1, W do
        local r = 1
        while r <= H do
            if col[r][c] then
                local e = r
                while e < H and col[e + 1][c] do e = e + 1 end
                for i = r, e do
                    local n = math.min(bulge, math.floor(math.min(extra[i][c], chord(i - r + 1, e - r + 1)) - 0.25))
                    if n < 0 then n = 0 end
                    extra[i][c] = n
                    if n > most then most = n end
                end
                r = e + 1
            else
                r = r + 1
            end
        end
    end
    local D = depth + 2 * most
    local cells = {}
    for r = 1, H do
        local y = H - r
        for c = 1, W do
            local k = col[r][c]
            if k then
                local n = extra[r][c]
                for z = most - n, most + depth + n - 1 do
                    cells[1 + (c - 1) + W * (y + H * z)] = k
                end
            end
        end
    end
    return cells, { W, H, D }
end

local function prep_model(key, def)
    local frames = def.frames
    local H, W = #frames[1], #frames[1][1]
    for f, rows in ipairs(frames) do
        if #rows ~= H then error(string.format("sprite %s frame %d has %d rows, expected %d", key, f, #rows, H)) end
        for r, line in ipairs(rows) do
            if #line ~= W then error(string.format("sprite %s frame %d row %d is %d wide, expected %d", key, f, r, #line, W)) end
            for c = 1, W do
                local ch = line:sub(c, c)
                if ch ~= "." then
                    local col = def.legend[ch]
                    if not col or not COLORS[col] then error(string.format("sprite %s: no colour for '%s'", key, ch)) end
                end
            end
        end
    end
    -- each frame becomes a voxel model the engine meshes: one mesh per colour, hidden faces
    -- dropped and flat runs merged, shared by every actor of this kind
    local m = { key = key, W = W, H = H, depth = def.depth or 3, finish = def.finish, nf = #frames, vox = {}, px = {} }
    for f, rows in ipairs(frames) do
        local cells, size = inflate(rows, def.legend, m.depth, def.bulge or BULGE)
        m.vox[f] = voxel_model { voxels = cells, size = size, voxel = VOX, pivot = { 0.5, 0, 0.5 } }
    end
    -- frame-1 pixels in model space (x, height, colour) for the particle effects
    for r, line in ipairs(frames[1]) do
        for c = 1, W do
            local col = def.legend[line:sub(c, c)]
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

-- spawns model m under root: one node per frame (only the first visible) holding a mesh per
-- colour; returns the frame nodes and every part with its colour
local function build_model(root, m, cast)
    local parts, fnodes = {}, {}
    local mats = {}
    for f = 1, m.nf do
        for _, p in ipairs(m.vox[f].parts) do mats[p.key] = mat_of(p.key, m.finish) end
    end
    for f = 1, m.nf do
        local node, ents = spawn_voxels(m.vox[f], "RT_frame", { parent = root, materials = mats, cast_shadow = cast })
        if f ~= 1 then set_component(node, "Visible", false) end
        for col, e in pairs(ents) do parts[#parts + 1] = { e, col } end
        fnodes[f] = node
    end
    -- a single-frame model needs no frame switching
    if m.nf == 1 then fnodes = {} end
    return fnodes, parts
end

return {
    mat_of = mat_of, model = model, build_model = build_model,
}
