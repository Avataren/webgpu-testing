-- The player: the model, input (or the autopilot), movement, firing and death.

local C = require("robotron.config")
local LX, LZ, P_SPEED, FIRE_INT, B_SPEED = C.LX, C.LZ, C.P_SPEED, C.FIRE_INT, C.B_SPEED
local N_PBULLETS, MAT_T = C.N_PBULLETS, C.MAT_T
local U = require("robotron.util")
local clamp, frand = U.clamp, U.frand
local S = require("robotron.state")
local g, pbullets, reg, uname, sfx, set_vis = S.g, S.pbullets, S.reg, S.uname, S.sfx, S.set_vis
local Models = require("robotron.models")
local mat_of, model, build_model = Models.mat_of, Models.model, Models.build_model
local Fx = require("robotron.fx")
local materialize, hit_flash = Fx.materialize, Fx.hit_flash
local Autopilot = require("robotron.autopilot")
local bot_think = Autopilot.bot_think

local atan, sqrt, floor, min = math.atan, math.sqrt, math.floor, math.min

local player = nil

local function build_player()
    local root = reg(spawn_entity("RT_player"))
    local m = model("player")
    local fnodes, parts = build_model(root, m, true)
    player = { e = root, m = m, fnodes = fnodes, parts = parts, frame = 1, vis = true, flash = false }
    S.player = player
end

local function show_player(v)
    if player.vis == v then return end
    player.vis = v
    set_vis(player.e, v)
    if not v then set_translation(player.e, 0, -50, 0) end
end

local function flash_player(col)
    for _, p in ipairs(player.parts) do share_material(p[1], mat_of(col or p[2])) end
    player.flash = col ~= nil
end

local function materialize_player(delay)
    g.p_spawn = delay + MAT_T * 0.8
    g.p_emitted = false
    show_player(false)
end

local function clear_bullets()
    for i = 1, #pbullets do
        local b = pbullets[i]
        b.alive = false
        if b.vis then
            b.vis = false
            set_vis(b.e, false)
        end
    end
end

local function fire_bullet(fx, fz)
    local b
    for i = 1, #pbullets do
        if not pbullets[i].alive then b = pbullets[i] break end
    end
    if not b then
        if #pbullets >= N_PBULLETS then return end
        local e = reg(spawn_primitive("cube", uname("RT_bullet"), { material_of = mat_of("glow_white") }))
        set_scale(e, 0.07, 0.07, 1.1)
        set_component(e, "CanCastShadow", false)
        set_component(e, "Visible", false)
        b = { e = e, alive = false, vis = false, x = 0, z = 0, vx = 0, vz = 0, life = 0 }
        pbullets[#pbullets + 1] = b
    end
    b.alive = true
    b.x, b.z = g.px + fx * 0.5, g.pz + fz * 0.5
    b.vx, b.vz = fx * B_SPEED, fz * B_SPEED
    b.life = 1.4
    set_translation(b.e, b.x, 0.8, b.z)
    set_rotation(b.e, atan(-fx, -fz), 0, 0)
    if not b.vis then
        b.vis = true
        set_vis(b.e, true)
    end
    g.stats.shots = g.stats.shots + 1
    sfx("shoot", frand(0.95, 1.1), 0.55, 0.04)
end

local function read_input(dt)
    local mx, mz, fx, fz = 0, 0, 0, 0
    if g.bot then
        bot_think(dt)
        mx, mz, fx, fz = g.bmx, g.bmz, g.bfx, g.bfz
    else
        if is_key_pressed("D") then mx = mx + 1 end
        if is_key_pressed("A") then mx = mx - 1 end
        if is_key_pressed("S") then mz = mz + 1 end
        if is_key_pressed("W") then mz = mz - 1 end
        if is_key_pressed("ArrowRight") then fx = fx + 1 end
        if is_key_pressed("ArrowLeft") then fx = fx - 1 end
        if is_key_pressed("ArrowDown") then fz = fz + 1 end
        if is_key_pressed("ArrowUp") then fz = fz - 1 end
        if fx == 0 and fz == 0 and is_mouse_button_pressed(0) then
            local m = get_mouse_position()
            local gp = m and screen_to_ground(m.x, m.y, 0.8)
            if gp then
                local dx, dz = gp.x - g.px, gp.z - g.pz
                local d = sqrt(dx * dx + dz * dz)
                if d > 0.3 then fx, fz = dx / d, dz / d end
            end
        end
    end
    local ml = sqrt(mx * mx + mz * mz)
    if ml > 0 then mx, mz = mx / ml, mz / ml end
    local fl = sqrt(fx * fx + fz * fz)
    if fl > 0 then fx, fz = fx / fl, fz / fl end
    return mx, mz, fx, fz
end

local function draw_player(mx, mz, dt)
    local moving = mx ~= 0 or mz ~= 0
    if moving then
        g.p_anim = g.p_anim + dt * 9
        local f = 2 + floor(g.p_anim) % 2
        if player.frame ~= f then
            set_vis(player.fnodes[player.frame], false)
            set_vis(player.fnodes[f], true)
            player.frame = f
        end
    elseif player.frame ~= 1 then
        set_vis(player.fnodes[player.frame], false)
        set_vis(player.fnodes[1], true)
        player.frame = 1
    end
    -- turn a little toward the way the player moves (or fires)
    local want = 0
    local hx = (g.fx_last ~= 0 and g.fx_last) or mx
    if hx ~= 0 then want = 0.55 * hx end
    g.p_yaw = g.p_yaw + (want - g.p_yaw) * min(1, dt * 12)
    set_translation(player.e, g.px, 0, g.pz)
    set_rotation(player.e, g.p_yaw, -g.lean, 0)
end

local function update_player(dt)
    if g.invuln > 0 then g.invuln = g.invuln - dt end
    if not g.p_alive then return end
    if g.p_spawn > 0 then
        g.p_spawn = g.p_spawn - dt
        if not g.p_emitted and g.p_spawn <= MAT_T * 0.8 then
            g.p_emitted = true
            materialize(player.m, g.px, 0, g.pz, g.mat_mode, MAT_T * 0.8)
        end
        if g.p_spawn <= 0 then
            show_player(true)
            g.p_yaw = 0
            draw_player(0, 0, dt)
        end
        return
    end
    local mx, mz, fx, fz = read_input(dt)
    g.fx_last = fx
    g.pvx, g.pvz = mx * P_SPEED, mz * P_SPEED
    g.px = clamp(g.px + g.pvx * dt, -LX, LX)
    g.pz = clamp(g.pz + g.pvz * dt, -LZ, LZ)
    if fx ~= 0 or fz ~= 0 then
        g.face = atan(-fx, -fz)
    elseif mx ~= 0 or mz ~= 0 then
        g.face = atan(-mx, -mz)
    end
    g.fire_cd = g.fire_cd - dt
    if (fx ~= 0 or fz ~= 0) and g.fire_cd <= 0 then
        fire_bullet(fx, fz)
        g.fire_cd = FIRE_INT
    end
    show_player(true)
    draw_player(mx, mz, dt)
end

local function kill_player()
    if g.state ~= "running" or not g.p_alive or g.invuln > 0 or g.god or g.p_spawn > 0 then return end
    g.p_alive = false
    g.lives = g.lives - 1
    g.stats.deaths = g.stats.deaths + 1
    g.state = "dying"
    g.dying_t = 0
    g.exploded = false
    g.flash_i = 0
    hit_flash(0.7)
    sfx("player_die", 1.0, 1.0)
end

return {
    build_player = build_player, show_player = show_player, flash_player = flash_player, materialize_player = materialize_player, clear_bullets = clear_bullets,
    update_player = update_player, kill_player = kill_player,
}
