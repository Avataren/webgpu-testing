-- The sculpture is authored through engine MCP. This script animates its saved parts.
STATE = {time = 0, paused = false, captions = true, yaw = 0, radius = 20.20, height = 8.4}
local parts = {}
local dust = 0
local base_yaw = math.atan(10.6, 17.2)
local function clamp(x, a, b) return math.max(a, math.min(b, x)) end

function on_created(self, previous)
    if previous then STATE = previous end
    for _, name in ipairs({"Meridian", "Equator", "Precession", "Inner halo", "The pearl", "ObservatoryCamera"}) do
        parts[name] = assert(find_entity_by_name(name), "Missing sculpture part: " .. name)
    end
end

function update(self, dt)
    if is_key_just_pressed("Space") then STATE.paused = not STATE.paused end
    if is_key_just_pressed("H") then STATE.captions = not STATE.captions end
    if is_key_just_pressed("R") then
        STATE = {time = 0, paused = false, captions = true, yaw = 0, radius = 20.20, height = 8.4}
    end
    if is_key_pressed("A") or is_key_pressed("ArrowLeft") then STATE.yaw = STATE.yaw - dt * 0.55 end
    if is_key_pressed("D") or is_key_pressed("ArrowRight") then STATE.yaw = STATE.yaw + dt * 0.55 end
    if is_key_pressed("ArrowUp") then STATE.height = clamp(STATE.height + dt * 3, 4.5, 15) end
    if is_key_pressed("ArrowDown") then STATE.height = clamp(STATE.height - dt * 3, 4.5, 15) end
    if is_mouse_button_pressed(0) then
        local delta = get_mouse_delta()
        STATE.yaw = STATE.yaw - delta.x * 0.004
        STATE.height = clamp(STATE.height + delta.y * 0.025, 4.5, 15)
    end
    local scroll = get_mouse_scroll_delta()
    STATE.radius = clamp(STATE.radius * math.exp(-scroll.y * 0.08), 13.5, 30)
    if not STATE.paused then STATE.time = STATE.time + dt end
    local t = STATE.time
    set_rotation(parts.Meridian, t * 0.085, math.rad(12) + math.sin(t * 0.11) * 0.08, math.rad(-18))
    set_rotation(parts.Equator, math.rad(36) - t * 0.12, math.rad(68) + math.sin(t * 0.09) * 0.10, math.rad(24) + t * 0.027)
    set_rotation(parts.Precession, math.rad(94) + t * 0.16, math.rad(15) + math.sin(t * 0.13) * 0.16, math.rad(-24) - t * 0.045)
    set_rotation(parts["Inner halo"], -t * 0.20, math.rad(75), math.rad(18))
    local pulse = 0.90 + math.sin(t * 0.65) * 0.025
    set_scale(parts["The pearl"], pulse, pulse, pulse)
    local angle = base_yaw + STATE.yaw + math.sin(t * 0.055) * 0.12
    set_translation(parts.ObservatoryCamera, math.sin(angle) * STATE.radius, STATE.height, math.cos(angle) * STATE.radius)
    look_at(parts.ObservatoryCamera, 0, 3.7, 0)
    local view = get_viewport_size()
    if view and view.height > 0 then
        local aspect = view.width / view.height
        local fov = clamp(math.deg(2 * math.atan(math.tan(math.rad(43) / 2) * math.max(1, (16/9) / aspect))), 43, 80)
        set_camera(parts.ObservatoryCamera, {fov_deg = fov})
    end
    if not STATE.paused then
        dust = dust + dt
        if dust >= 0.15 then
            dust = dust % 0.15
            local a = t * 1.7
            emit_particles(math.cos(a) * 2.5, 0.85, math.sin(a) * 2.5, {
                count = 2, color = {0.20, 0.65, 0.65, 0.55}, color_end = {0.09, 0.28, 0.32, 0},
                intensity = 1.2, speed = {0.015, 0.08}, velocity = {0, 0.30, 0},
                life = {5, 8}, size = 0.035, size_end = 0.015, radius = 0.4, drag = 0.1
            })
        end
    end
end

function on_ui(self, ui)
    if not STATE.captions then return end
    local view = ui:get_viewport_size()
    local compact = view.width < 900
    local inset = compact and 22 or 36
    ui:screen_label("top_left", inset, inset, "TIDE OBSERVATORY", {size = compact and 24 or 30, color = {0.86, 0.87, 0.78, 1}})
    ui:screen_label("top_left", inset, inset + 38, "An instrument for an imaginary ocean.", {size = 13, color = {0.46, 0.65, 0.68, 1}})
    if not compact then
        ui:screen_label("top_right", inset, inset + 4, "KINETIC STUDY  /  001", {size = 11, color = {0.47, 0.58, 0.60, 1}})
    end
    ui:screen_label("bottom_left", inset, inset, STATE.paused and "STILL WATER" or "BRASS  /  OBSIDIAN  /  LIGHT", {size = 11, color = {0.62, 0.66, 0.58, 1}})
    local controls = compact and "DRAG orbit   SCROLL zoom   SPACE pause" or "DRAG / A D  orbit     SCROLL  zoom     SPACE  pause"
    ui:screen_label("bottom_right", inset, inset + 20, controls, {size = 11, color = {0.43, 0.58, 0.60, 1}})
    ui:screen_label("bottom_right", inset, inset, "R  reset     H  hide captions", {size = 11, color = {0.43, 0.58, 0.60, 1}})
end

function on_destroyed(self, reason)
    if reason == "reload" then return STATE end
end
