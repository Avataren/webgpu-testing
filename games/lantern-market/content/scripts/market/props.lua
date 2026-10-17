-- The things that move about the market and get in people's way, as AI agents with state
-- machines whose Lua `update` moves them (they have no NavAgent):
--
-- * two handcarts on their rounds: `rolling` (moving to the next stop, perceived as
--   `cart`, so people step aside) and `parked` (a NavObstacle at rest carves the
--   navmesh, so paths go round it; perceived as `parked_cart`), each hauled by a puller
--   walking in front of it;
-- * the portcullis of the stone bridge: `open`, `closing` (a rattle people turn to),
--   `closed` (it carves the bridge shut: people heading south reroute over the
--   footbridge) and `opening`, on timers, or at once on a `toggle` event (the G key).

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

-- The puller plays on layer 1, over the idle its template's animation tree keeps on
-- layer 0 (pullers are not agents: they go where the cart goes).
local function puller_clip(cart, clip, speed)
    if cart.puller and C.clips[clip] and cart.puller_clip ~= clip then
        cart.puller_clip = clip
        animator_play(cart.puller, clip, { layer = 1, fade = 0.35, speed = speed or 1 })
    end
end

-- `rolling`: on toward the stop; "parked" when it gets to one it waits at.
local function roll(ctx)
    local cart = Props.carts[ctx.agent]
    if not cart then
        return nil
    end
    local target = cart.round[cart.stop]
    local dx, dz = target[1] - cart.x, target[2] - cart.z
    local d = math.sqrt(dx * dx + dz * dz)
    local walk = C.clips["walking"] and C.clips["walking"].speed or 1.7
    puller_clip(cart, "walking", CART_SPEED / math.max(walk, 0.1) * 1.15)
    if d < 0.05 then
        if target[3] > 0 then
            ctx.bb.park_time = target[3]
            return "parked"
        end
        cart.stop = cart.stop % #cart.round + 1
        return nil
    end
    -- Turn first when the next leg goes another way, then roll.
    local want = yaw_of_travel(dx, dz)
    cart.yaw = turn_toward(cart.yaw, want, 1.4 * ctx.dt)
    local facing = math.abs((want - cart.yaw + math.pi) % (2 * math.pi) - math.pi)
    local speed = CART_SPEED * math.max(0, 1 - facing / 0.6)
    local step = math.min(d, speed * ctx.dt)
    cart.x = cart.x + dx / d * step
    cart.z = cart.z + dz / d * step
    set_translation(cart.e, cart.x, 0, cart.z)
    set_rotation(cart.e, cart.yaw, 0, 0)
    return nil
end

local function park(ctx)
    local cart = Props.carts[ctx.agent]
    if cart then
        cart.parked = true
        puller_clip(cart, "breathing-idle")
        set_component(ctx.agent, "AiAgent", { tags = { "parked_cart" } })
    end
end

local function unpark(ctx)
    local cart = Props.carts[ctx.agent]
    if cart then
        cart.parked = false
        cart.stop = cart.stop % #cart.round + 1
        set_component(ctx.agent, "AiAgent", { tags = { "cart" } })
    end
end

local function gate_at(ctx, y)
    Props.gate.y = y
    set_translation(ctx.agent, 0, y, GATE.z)
end

local function gate_state(name, duration)
    return function(ctx)
        Props.gate.state = name
        Props.gate.since = get_time()
        Props.gate.duration = duration
        if name == "open" then
            gate_at(ctx, GATE.up)
        elseif name == "closed" then
            gate_at(ctx, 0)
        end
    end
end

local function lower(ctx)
    gate_at(ctx, GATE.up * C.smooth(1 - ctx.state_time / GATE.move_time))
end

local function raise(ctx)
    gate_at(ctx, GATE.up * C.smooth(ctx.state_time / (GATE.move_time * 1.3)))
end

function Props.define()
    ai_machine("cart", {
        initial = "rolling",
        states = {
            rolling = { update = roll },
            parked = { enter = park, exit = unpark, transitions = { { "rolling", after = "park_time" } } },
        },
    })
    ai_machine("gate", {
        initial = "open",
        states = {
            open = {
                enter = gate_state("open", GATE.open_time),
                transitions = {
                    { "closing", after = GATE.open_time },
                    { "closing", event = "toggle" },
                },
            },
            closing = {
                enter = gate_state("closing", GATE.move_time),
                update = lower,
                -- A rattle once, then nothing more to do.
                tree = { "sequence", { "noise", "portcullis", radius = 14 }, { "running" } },
                transitions = { { "closed", after = GATE.move_time } },
            },
            closed = {
                enter = gate_state("closed", GATE.closed_time),
                transitions = {
                    { "opening", after = GATE.closed_time },
                    { "opening", event = "toggle" },
                },
            },
            opening = {
                enter = gate_state("opening", GATE.move_time * 1.3),
                update = raise,
                transitions = { { "open", after = GATE.move_time * 1.3 } },
            },
        },
    })
end

-- Find the carts and the gate, give them brains, and pullers to the carts.
function Props.setup(spawn_puller)
    for name, round in pairs(ROUNDS) do
        local e = find_entity_by_name(name)
        local id = e and entity_id(e)
        if id then
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
            }
            cart.yaw = yaw_of_travel(first[1] - cart.x, first[2] - cart.z)
            cart.puller = spawn_puller and spawn_puller(e, PULLER_OFFSET)
            Props.carts[id] = cart
            ai_set_brain(e, "cart", { tags = { "cart" }, tick_rate = 0 })
        end
    end
    local gate = find_entity_by_name("Gate")
    if gate then
        Props.gate = { e = gate, y = GATE.up, state = "open", since = get_time(), duration = GATE.open_time }
        set_translation(gate, 0, GATE.up, GATE.z)
        ai_set_brain(gate, "gate", { tags = { "gate" }, tick_rate = 0 })
    end
end

-- Open or close the gate now.
function Props.toggle_gate()
    if Props.gate then
        ai_send(Props.gate.e, "toggle")
    end
end

function Props.gate_text()
    local gate = Props.gate
    if not gate then
        return "no gate"
    end
    local left = math.max(0, math.ceil(gate.duration - (get_time() - gate.since)))
    if gate.state == "open" then
        return string.format("open, closes in %d s", left)
    elseif gate.state == "closed" then
        return string.format("closed, opens in %d s", left)
    end
    return gate.state
end

function Props.cart_text()
    local parts = {}
    for _, cart in pairs(Props.carts) do
        local label = cart.name:gsub("Cart", "cart ")
        table.insert(parts, label .. (cart.parked and " parked" or " rolling"))
    end
    table.sort(parts)
    return table.concat(parts, ", ")
end

function Props.clear()
    for _, cart in pairs(Props.carts) do
        if cart.puller then
            despawn_entity(cart.puller)
        end
    end
    Props.carts = {}
    Props.gate = nil
end

return Props
