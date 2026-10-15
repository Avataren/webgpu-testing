-- Effects: the pixel particles of the original (materialize, explode, afterimages), bursts,
-- shockwaves, score popups and screen shake, the rectangle zoom between waves, and the walls'
-- colour cycle.

local C = require("robotron.config")
local COLORS, HW, HD, VOX, SPREAD, TRANS_T = C.COLORS, C.HW, C.HD, C.VOX, C.SPREAD, C.TRANS_T
local FLASH, LOOK, LEAN = C.FLASH, C.LOOK, C.LEAN
local U = require("robotron.util")
local frand = U.frand
local S = require("robotron.state")
local g, reg, uname, set_vis = S.g, S.reg, S.uname, S.set_vis
local Models = require("robotron.models")
local mat_of = Models.mat_of

local abs, floor, min, max, sqrt = math.abs, math.floor, math.min, math.max, math.sqrt

-- model-space point (x, height) of an upright, leaning, camera-facing actor at (wx, wy, wz) -> world
local function lean_pt(wx, wy, wz, x, h)
    return wx + x, wy + h * g.lean_c, wz - h * g.lean_s
end

-- The sprite's vertical for the effects: the way pixels spread or fly "up the sprite", as a
-- unit (0, ay, az), with `base` the height (from the feet) they spread from and `scale` how
-- far. From the game's steep view they move along the floor (-z), centred on the sprite, which
-- reads as up and down on screen like the arcade's; from a low camera that runs into the view,
-- so they rise from the actor's feet instead (centred, half would go under the floor). Between,
-- the two blend with the actors' lean, which follows the camera (camera.lua).
local function sprite_vertical(m)
    local k = g.lean / LEAN
    local ay, az = 1 - k, -k
    local l = sqrt(ay * ay + az * az)
    return ay / l, az / l, m.height * 0.5 * k, 0.5 + 0.5 * k
end

-- one pixel as a GPU particle: no drag, so it travels in a straight line for `life` seconds
local function px_particle(x, y, z, col, vx, vy, vz, life, inten, fade)
    local c = COLORS[col]
    local cs = { c[1], c[2], c[3], 1 }
    emit_particles(x, y, z, {
        count = 1, color = cs, color_end = fade and { c[1] * 0.4, c[2] * 0.4, c[3] * 0.4, 0 } or cs,
        intensity = inten or 1.6, speed = 0, life = life, size = VOX * 1.1, size_end = fade and VOX * 0.7 or VOX * 1.1,
        velocity = { vx, vy, vz }, drag = 0,
    })
end

-- MATERIALIZE: the sprite's pixel rows (mode "v") or columns ("h") start far apart and close up;
-- "r" spreads both ways a little (an enforcer growing out of a spheroid)
local function materialize(m, wx, wy, wz, mode, t)
    local ay, az, base, scale = sprite_vertical(m)
    for i = 1, #m.px do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        local ox, o = 0, 0
        if mode == "h" then
            ox = p[1] * SPREAD * 1.6
        elseif mode == "v" then
            o = (p[2] - base) * SPREAD * scale
        else
            ox, o = p[1] * 3.5, (p[2] - base) * 3.5 * scale
        end
        local oy, oz = o * ay, o * az
        px_particle(fx + ox, fy + oy, fz + oz, p[3], -ox / t, -oy / t, -oz / t, t, 1.7, false)
    end
end

-- EXPLODE: the sprite stretches apart along the line of fire (dx, dz), or every way (no direction)
local function explode(m, wx, wy, wz, dx, dz, life, k)
    life = life or 0.55
    k = k or 9.0
    local ay, az, base, scale = sprite_vertical(m)
    local horiz = dx and abs(dx) >= abs(dz)
    local vert = dx and not horiz
    for i = 1, #m.px do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        local vx, v, push = 0, 0, 0
        if horiz then
            vx = p[1] * k * 1.6 + dx * 3.0
        elseif vert then
            v, push = (p[2] - base) * k * scale, dz * 3.0
        else
            vx, v = p[1] * k, (p[2] - base) * k * scale
        end
        px_particle(fx, fy, fz, p[3], vx, v * ay, v * az + push, life * frand(0.8, 1.1), 2.0, true)
    end
end

-- a prog's afterimage: every pixel in one colour, left behind for a moment
local function afterimage(m, wx, wy, wz, col)
    for i = 1, #m.px, 2 do
        local p = m.px[i]
        local fx, fy, fz = lean_pt(wx, wy, wz, p[1], p[2])
        px_particle(fx, fy, fz - 0.05, col, 0, 0, 0, 0.22, 1.1, true)
    end
end

local function burst(col, x, y, z, count, speed, size)
    local c = COLORS[col] or COLORS.white
    emit_particles(x, y, z, { count = count, color = { c[1], c[2], c[3], 1 }, intensity = 2.2, speed = { speed * 0.25, speed },
        life = { 0.25, 0.6 }, size = (size or 0.12), size_end = 0.0, gravity = -12, drag = 1.5 })
end

-- an expanding ring of glowing particles on the floor (a shockwave)
local function ring(x, z, maxr, dur, col)
    local c = COLORS[col] or COLORS.white
    emit_particles(x, 0.15, z, { count = floor(24 + maxr * 10), shape = "ring", color = { c[1], c[2], c[3], 1 }, intensity = 2.5,
        speed = maxr / dur, life = dur, size = 0.14, size_end = 0.04, drag = 0 })
end

local function add_shake(a) g.shake = min(1.0, max(g.shake, a)) end

-- LIGHT FLASHES: a few pooled point lights that light the floor and the actors around an
-- explosion and fade out. The renderer takes four point lights.
local N_FLASH = 3
local flashes = {}
local function build_lights()
    for i = 1, N_FLASH do
        local e = reg(spawn_light("point", { color = { 1, 1, 1 }, intensity = 0, range = 1 }, uname("RT_flash")))
        set_translation(e, 0, -40, 0)
        flashes[i] = { e = e, t = 0, dur = 1, peak = 0, on = false }
    end
end

local function flash_light(x, y, z, col, peak, range, dur)
    -- reuse the dimmest light (an idle one first)
    local best, bv = nil, 1e9
    for i = 1, N_FLASH do
        local f = flashes[i]
        local v = f.on and f.peak * (1 - f.t / f.dur) or -1
        if v < bv then best, bv = f, v end
    end
    if not best or bv > peak then return end
    local c = COLORS[col] or COLORS.white
    best.t, best.dur, best.peak, best.on = 0, dur or 0.3, peak, true
    set_translation(best.e, x, y, z)
    set_light(best.e, { color = { c[1], c[2], c[3] }, intensity = peak, range = range or 5 })
end

local function update_lights(dt)
    for i = 1, N_FLASH do
        local f = flashes[i]
        if f.on then
            f.t = f.t + dt
            if f.t >= f.dur then
                f.on = false
                set_light(f.e, { intensity = 0 })
                set_translation(f.e, 0, -40, 0)
            else
                local k = 1 - f.t / f.dur
                set_light(f.e, { intensity = f.peak * k * k })
            end
        end
    end
end

-- HIT FLASH: the HitVolume post-process volume (a sphere with a long blend distance) adds
-- chromatic aberration, a vignette and a punch of exposure. Moving it toward the camera fades
-- it in: at the camera it applies fully, one blend distance away not at all.
local HIT_BLEND = 10.0
local function hit_flash(a) g.hit = min(1.0, max(g.hit or 0, a)) end

local function update_hit(dt)
    local v = g.hit_vol
    if not v or not g.cam then return end
    local a = g.hit or 0
    if a <= 0 and g.hit_shown == 0 then return end
    a = max(0, a - dt * 1.6)
    g.hit = a
    g.hit_shown = a
    local p = get_world_translation(g.cam)
    set_translation(v, p.x, p.y + HIT_BLEND * (1 - a) + (a <= 0 and 100 or 0), p.z)
end

local function popup(x, z, text, col, size)
    if #g.popups >= 16 then table.remove(g.popups, 1) end
    g.popups[#g.popups + 1] = { x = x, z = z, text = text, col = col or "white", t = 0, size = size or 22 }
end

local function update_effects(dt)
    local keep = {}
    for i = 1, #g.popups do
        local p = g.popups[i]
        p.t = p.t + dt
        if p.t < 1.3 then keep[#keep + 1] = p end
    end
    g.popups = keep
    g.shake = max(0, g.shake - dt * 2.4)
    if g.banner_t > 0 then g.banner_t = g.banner_t - dt end
end

local rects = {} -- transition rectangles, 4 bars each
local N_RECTS = 9
local function build_rects()
    for i = 1, N_RECTS do
        local r = {}
        for s = 1, 4 do
            local e = reg(spawn_primitive("cube", uname("RT_rect"), { material_of = mat_of("fx_red") }))
            set_component(e, "CanCastShadow", false)
            set_component(e, "Visible", false)
            r[s] = e
        end
        r.vis = false
        r.col = 0
        rects[i] = r
    end
end

local function show_rect(r, v)
    if r.vis == v then return end
    r.vis = v
    for s = 1, 4 do set_vis(r[s], v) end
end

-- concentric rectangles in cycling colours zoom out from the player to the border
local function update_transition(dt)
    g.trans_t = g.trans_t + dt
    local t = g.trans_t
    for i = 1, N_RECTS do
        local r = rects[i]
        local ti = t - (i - 1) * 0.055
        local p = ti / 0.42
        if p > 0 and p < 1.15 then
            p = min(1, p)
            local e = p * p * (3 - 2 * p)
            local cx, cz = g.tcx * (1 - e), g.tcz * (1 - e)
            local hx, hz = 0.3 + (HW + 0.15 - 0.3) * e, 0.25 + (HD + 0.15 - 0.25) * e
            local th, y = 0.2, 0.12
            set_translation(r[1], cx, y, cz - hz)
            set_scale(r[1], hx * 2 + th, 0.1, th)
            set_translation(r[2], cx, y, cz + hz)
            set_scale(r[2], hx * 2 + th, 0.1, th)
            set_translation(r[3], cx - hx, y, cz)
            set_scale(r[3], th, 0.1, hz * 2)
            set_translation(r[4], cx + hx, y, cz)
            set_scale(r[4], th, 0.1, hz * 2)
            local col = (i + floor(t * 26)) % #FLASH + 1
            if r.col ~= col then
                r.col = col
                for s = 1, 4 do share_material(r[s], mat_of("fx_" .. FLASH[col])) end
            end
            show_rect(r, true)
        else
            show_rect(r, false)
        end
    end
    if t >= TRANS_T then
        for i = 1, N_RECTS do show_rect(rects[i], false) end
        local f = g.trans_after
        g.trans_after = nil
        if f then f() end
    end
end

-- Whether the frame is path traced (the player's graphics setting, which can change at any
-- time). g.pt_force (true/false) overrides it for testing.
local function pt_on()
    if g.pt_force ~= nil then return g.pt_force end
    return path_traced ~= nil and path_traced()
end

-- Switch the arena between the raster and the path-traced look (config.LOOK)
local function apply_look(pt)
    local L = pt and LOOK.pt or LOOK.raster
    g.pt = pt
    if g.sun then set_light(g.sun, { intensity = L.sun }) end
    if g.fill then set_light(g.fill, { intensity = L.fill_cool }) end
    if g.rim then set_light(g.rim, { intensity = L.fill_warm }) end
    for n = 1, #g.spills do set_vis(g.spills[n], L.spills) end
    if g.arena then set_material(g.arena, L.floor) end
    if set_environment then set_environment({ ambient_intensity = L.ambient, hdr = L.hdr }) end
    g.grid_glow = nil
end

-- every frame, paused or not, so the look follows the settings page while it is open
local function update_look()
    local pt = pt_on()
    if pt ~= g.pt then apply_look(pt) end
end

local function update_walls(dt)
    local pt = g.pt
    local L = pt and LOOK.pt or LOOK.raster
    g.wall_hue = (g.wall_hue + dt * 0.12) % 1
    local h = g.wall_hue * 6
    local i = floor(h)
    local f = h - i
    local r, gg, b
    local q = 1 - f
    if i == 0 then r, gg, b = 1, f, 0
    elseif i == 1 then r, gg, b = q, 1, 0
    elseif i == 2 then r, gg, b = 0, 1, f
    elseif i == 3 then r, gg, b = 0, q, 1
    elseif i == 4 then r, gg, b = f, 0, 1
    else r, gg, b = 1, 0, q end
    local glow = L.wall_glow * (1 + g.shake * 1.15 + (g.hit or 0) * 0.8)
    for n = 1, #g.walls do
        set_material(g.walls[n], { color = { r, gg, b }, emissive = glow })
    end
    -- the dark wall under the tube glows faintly in its colour
    for n = 1, #g.wall_bodies do
        set_material(g.wall_bodies[n], { color = { 0.05 + r * 0.25, 0.05 + gg * 0.25, 0.05 + b * 0.25 }, emissive = L.body_emissive + g.shake * 0.3 })
    end
    if not pt then
        -- the border's light on the floor beside it
        local sa = min(1, 0.35 + g.shake * 0.3 + (g.hit or 0) * 0.3)
        for n = 1, #g.spills do
            set_material(g.spills[n], { color = { r, gg, b, sa }, emissive = 0.2 + g.shake * 0.5 })
        end
        -- the border's colour spills onto the arena through the rim fill light
        if g.rim then set_light(g.rim, { color = { 0.35 + r * 0.65, 0.35 + gg * 0.65, 0.35 + b * 0.65 } }) end
    end
    -- the floor grid pulses with explosions
    if g.arena then
        local pulse = L.grid + g.shake * 0.4 + (g.hit or 0) * 0.6
        if abs(pulse - (g.grid_glow or 0)) > 0.02 then
            g.grid_glow = pulse
            set_material(g.arena, { emissive = pulse })
        end
    end
end

return {
    px_particle = px_particle, materialize = materialize, explode = explode, afterimage = afterimage, burst = burst,
    ring = ring, add_shake = add_shake, popup = popup, update_effects = update_effects, build_rects = build_rects,
    update_transition = update_transition, update_walls = update_walls, update_look = update_look,
    build_lights = build_lights, flash_light = flash_light, update_lights = update_lights,
    hit_flash = hit_flash, update_hit = update_hit,
}
