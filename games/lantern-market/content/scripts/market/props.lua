-- The things that move about the market and get in people's way: two handcarts
-- hauled round their rounds (NavObstacles: people steer round them while they roll,
-- and once they stand still they cut a hole in the navmesh, so paths go round them),
-- and the portcullis of the stone bridge, which closes now and then (it carves the
-- bridge shut: people heading south reroute over the footbridge).

local C = require("market/core")

local Props = { carts = {}, gate = nil }

-- Cart rounds: stops with how long the cart parks there (seconds, 0 = drive through).
-- Cart B parks across the stone bridge's north end on its round.
local ROUNDS = {
    CartA = {
        { -12.0, -9.6, 7 },
        { 0.0, -9.6, 9 },
        { 12.0, -9.6, 7 },
        { 0.0, -9.6, 0 },
    },
    CartB = {
        { 12.5, -8.0, 6 },
        { 12.5, 8.0, 0 },
        { 6.0, 11.2, 0 },
        { 0.0, 11.2, 12 },
        { 6.0, 11.2, 0 },
        { 12.5, 8.0, 0 },
    },
}
local CART_SPEED = 0.95
local PULLER_OFFSET = -2.25

-- The portcullis: up (inside the gatehouse) and down, and how long it stays each way.
local GATE = { z = 12.5, up = 2.75, open_time = 26, closed_time = 16, move_time = 2.5 }

local function yaw_of_travel(dx, dz)
    -- The cart's local -Z (its handles) points along the way it goes.
    return math.atan(-dx, -dz)
end

local function turn_toward(current, target, step)
    local d = (target - current + math.pi) % (2 * math.pi) - math.pi
    if d > step then
        d = step
    elseif d < -step then
        d = -step
    end
    return current + d
end

function Props.setup(spawn_puller)
    for name, round in pairs(ROUNDS) do
        local e = find_entity_by_name(name)
        if e then
            local w = get_world_translation(e)
            local first = round[1]
            local cart = {
                name = name,
                e = e,
                round = round,
                stop = 1,
                x = w and w.x or first[1],
                z = w and w.z or first[2],
                yaw = 0,
                state = "moving",
                timer = 0,
            }
            local dx, dz = first[1] - cart.x, first[2] - cart.z
            cart.yaw = yaw_of_travel(dx, dz)
            cart.puller = spawn_puller and spawn_puller(e, PULLER_OFFSET)
            table.insert(Props.carts, cart)
        end
    end
    local gate = find_entity_by_name("Gate")
    if gate then
        Props.gate = { e = gate, y = GATE.up, state = "open", timer = GATE.open_time * 0.5 }
        set_translation(gate, 0, GATE.up, GATE.z)
    end
end

-- The puller plays on layer 1, over the idle its template's animation tree keeps on
-- layer 0 (pullers are not agents: they go where the cart goes).
local function puller_clip(cart, clip, speed)
    if cart.puller and C.clips[clip] and cart.puller_clip ~= clip then
        cart.puller_clip = clip
        animator_play(cart.puller, clip, { layer = 1, fade = 0.35, speed = speed or 1 })
    end
end

local function update_cart(cart, dt)
    if cart.state == "parked" then
        cart.timer = cart.timer - dt
        puller_clip(cart, "breathing-idle")
        if cart.timer <= 0 then
            cart.stop = cart.stop % #cart.round + 1
            cart.state = "moving"
        end
        return
    end
    local target = cart.round[cart.stop]
    local dx, dz = target[1] - cart.x, target[2] - cart.z
    local d = math.sqrt(dx * dx + dz * dz)
    local walk = C.clips["walking"] and C.clips["walking"].speed or 1.7
    puller_clip(cart, "walking", CART_SPEED / math.max(walk, 0.1) * 1.15)
    if d < 0.05 then
        if target[3] > 0 then
            cart.state = "parked"
            cart.timer = target[3]
        else
            cart.stop = cart.stop % #cart.round + 1
        end
        return
    end
    -- Turn first when the next leg goes another way, then roll.
    local want = yaw_of_travel(dx, dz)
    cart.yaw = turn_toward(cart.yaw, want, 1.4 * dt)
    local facing = math.abs((want - cart.yaw + math.pi) % (2 * math.pi) - math.pi)
    local speed = CART_SPEED * math.max(0, 1 - facing / 0.6)
    local step = math.min(d, speed * dt)
    cart.x = cart.x + dx / d * step
    cart.z = cart.z + dz / d * step
    set_translation(cart.e, cart.x, 0, cart.z)
    set_rotation(cart.e, cart.yaw, 0, 0)
end

local function update_gate(gate, dt)
    gate.timer = gate.timer - dt
    if gate.state == "open" and gate.timer <= 0 then
        gate.state = "closing"
        gate.timer = GATE.move_time
    elseif gate.state == "closing" then
        gate.y = GATE.up * C.smooth(gate.timer / GATE.move_time)
        if gate.timer <= 0 then
            gate.y = 0
            gate.state = "closed"
            gate.timer = GATE.closed_time
        end
    elseif gate.state == "closed" and gate.timer <= 0 then
        gate.state = "opening"
        gate.timer = GATE.move_time * 1.3
    elseif gate.state == "opening" then
        gate.y = GATE.up * (1 - C.smooth(gate.timer / (GATE.move_time * 1.3)))
        if gate.timer <= 0 then
            gate.y = GATE.up
            gate.state = "open"
            gate.timer = GATE.open_time
        end
    end
    set_translation(gate.e, 0, gate.y, GATE.z)
end

function Props.update(dt)
    for _, cart in ipairs(Props.carts) do
        update_cart(cart, dt)
    end
    if Props.gate then
        update_gate(Props.gate, dt)
    end
end

-- Open or close the gate now.
function Props.toggle_gate()
    local gate = Props.gate
    if not gate then
        return
    end
    if gate.state == "open" or gate.state == "opening" then
        gate.state = "open"
        gate.timer = 0
    else
        gate.state = "closed"
        gate.timer = 0
    end
end

function Props.gate_text()
    local gate = Props.gate
    if not gate then
        return "no gate"
    end
    if gate.state == "open" then
        return string.format("open, closes in %d s", math.ceil(gate.timer))
    elseif gate.state == "closed" then
        return string.format("closed, opens in %d s", math.ceil(gate.timer))
    end
    return gate.state
end

function Props.cart_text()
    local parts = {}
    for _, cart in ipairs(Props.carts) do
        local label = cart.name:gsub("Cart", "cart ")
        if cart.state == "parked" then
            table.insert(parts, string.format("%s parked (%d s)", label, math.ceil(cart.timer)))
        else
            table.insert(parts, label .. " rolling")
        end
    end
    return table.concat(parts, ", ")
end

return Props
