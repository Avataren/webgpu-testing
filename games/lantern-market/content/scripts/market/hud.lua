-- The HUD: the market's name and what is going on (who is doing what, by role and
-- state; the gate, the carts), the news (thefts, arrests, games of tag, the show), the
-- person the camera follows (their state, action and needs), the camera shot, and the
-- keys. B puts each person's state over their head.

local C = require("market/core")

local Hud = { hidden = false, labels = false, fps = 0 }

local GOLD = { 1.0, 0.82, 0.5, 1.0 }
local SOFT = { 0.95, 0.92, 0.86, 0.9 }
local NEWS = { 1.0, 0.9, 0.7, 1.0 }

-- The words each role's states read as.
local DESCRIBE = {
    townsfolk = { life = "going about", chat = "chatting", tempted = "tempted by a vendor", gather = "called over",
        startled = "startled", robbed = "robbed", dodge = "dodging a cart" },
    vendor = { idle = "waiting", hawk = "calling out", serve = "serving" },
    guard = { patrol = "on patrol", salute = "saluting", investigate = "investigating", chase = "CHASING",
        search = "searching", arrest = "arresting" },
    thief = { prowl = "prowling", stalk = "stalking a mark", steal = "picking a pocket", escape = "ESCAPING",
        evade = "dodging a guard", hide = "lying low", caught = "caught" },
    child = { play = "playing", it = "it!", run_away = "running from it", tagged = "tagged" },
    performer = { perform = "performing", rest = "on a break" },
}

-- A colour for a state name (the same name, the same colour).
local function state_color(name)
    local h = 0
    for i = 1, #name do
        h = (h * 31 + name:byte(i)) % 360
    end
    local x = h / 60
    local c = 1 - math.abs(x % 2 - 1)
    local r, g, b
    if x < 1 then r, g, b = 1, c, 0
    elseif x < 2 then r, g, b = c, 1, 0
    elseif x < 3 then r, g, b = 0, 1, c
    elseif x < 4 then r, g, b = 0, c, 1
    elseif x < 5 then r, g, b = c, 0, 1
    else r, g, b = 1, 0, c end
    return { 0.4 + 0.6 * r, 0.4 + 0.6 * g, 0.4 + 0.6 * b, 1.0 }
end

local function bar(value)
    local n = math.floor(C.clamp01(value or 0) * 10 + 0.5)
    return string.rep("#", n) .. string.rep(".", 10 - n)
end

function Hud.update(dt)
    if dt > 0 then
        Hud.fps = Hud.fps + (1 / dt - Hud.fps) * math.min(1, dt * 2)
    end
end

local function role_line(role, label, counts)
    local states = counts.roles[role]
    if not states then
        return nil
    end
    local parts = {}
    for state, n in pairs(states) do
        local words = DESCRIBE[role] and DESCRIBE[role][state] or state
        table.insert(parts, { n = n, text = n > 1 and string.format("%d %s", n, words) or words })
    end
    table.sort(parts, function(a, b)
        return a.n > b.n
    end)
    local texts = {}
    for i = 1, math.min(4, #parts) do
        texts[i] = parts[i].text
    end
    return label .. ": " .. table.concat(texts, ", ")
end

local function follow_panel(ui, person)
    local status = ai_status(person.e)
    if not status then
        return
    end
    ui:anchored_panel("market_follow", "bottom_right", 14, 40, 300, nil, function(panel)
        panel:heading(person.name)
        local words = DESCRIBE[person.role] and DESCRIBE[person.role][status.state] or status.state
        panel:label(string.format("%s: %s", person.role, words ~= "" and words or "-"),
            { color = state_color(status.state ~= "" and status.state or person.role) })
        if status.action then
            panel:label("Doing: " .. status.action)
        end
        if person.role == "townsfolk" then
            local board = ai_blackboard(person.e)
            if board then
                for _, need in ipairs({ "hunger", "thirst", "rest", "shopping", "curiosity", "social" }) do
                    panel:label(string.format("%-9s %s", need, bar(board[need])), { size = 13 })
                end
            end
        elseif person.role == "thief" then
            local board = ai_blackboard(person.e)
            panel:label(string.format("Purses: %d, caught %d times", board and board.loot or 0, board and board.caught or 0))
        end
        if status.error then
            panel:label(status.error, { color = { 1, 0.4, 0.4, 1 } })
        end
    end)
end

-- Each person's state over their head (near the camera).
local function draw_labels(ui, people)
    local camera = Hud.camera and get_world_translation(Hud.camera)
    for _, person in ipairs(people.list) do
        local p = get_world_translation(person.e)
        if p and (not camera or C.dist2(p.x, p.z, camera.x, camera.z) < 35 * 35) then
            local state = ai_state(person.e)
            if state and state ~= "" then
                local words = DESCRIBE[person.role] and DESCRIBE[person.role][state] or state
                ui:world_label(p.x, p.y + 2.1, p.z, words, { color = state_color(state), size = 13, backdrop = true })
            end
        end
    end
end

function Hud.draw(ui, info, people, follow)
    if Hud.hidden then
        ui:screen_label("bottom_left", 16, 12, "H: show the HUD", { color = SOFT, size = 13 })
        return
    end
    if info.loading then
        ui:screen_label("center", 0, -20, "Lantern Market", { color = GOLD, size = 46 })
        ui:screen_label("center", 0, 34, info.loading, { color = SOFT, size = 18 })
        return
    end
    if info.nocast then
        ui:screen_label("center", 0, -30, "Lantern Market", { color = GOLD, size = 46 })
        ui:screen_label("center", 0, 30, "No townsfolk yet: fill the asset library and run", { color = SOFT, size = 18 })
        ui:screen_label("center", 0, 58, "games/lantern-market/tools/fetch_cast.py", { color = SOFT, size = 16 })
        return
    end
    local counts = info.counts
    ui:anchored_panel("market_status", "top_left", 14, 14, 380, nil, function(panel)
        panel:heading("Lantern Market")
        panel:label(string.format("%d people, %d on the move", counts.total, counts.walking))
        for _, line in ipairs({
            role_line("townsfolk", "Townsfolk", counts),
            role_line("vendor", "Vendors", counts),
            role_line("guard", "Watch", counts),
            role_line("thief", "Thief", counts),
            role_line("child", "Children", counts),
            role_line("performer", "Performer", counts),
        }) do
            panel:label(line)
        end
        panel:label("Gate: " .. info.gate)
        panel:label("Carts: " .. info.carts)
        panel:label(string.format("Navmesh: %d polygons, %d obstacles carving", info.polys, info.carving))
        panel:label(string.format("%.0f fps", Hud.fps))
    end)
    local y = 44
    local now = get_time()
    for _, item in ipairs(C.news) do
        local age = now - item.time
        if age < 25 then
            local alpha = math.max(0.25, 1 - age / 25)
            ui:screen_label("top_right", 18, y, item.text, { color = { NEWS[1], NEWS[2], NEWS[3], alpha }, size = 15 })
            y = y + 22
        end
    end
    ui:screen_label("top_right", 18, 14, info.shot, { color = GOLD, size = 18 })
    if follow then
        follow_panel(ui, follow)
    end
    if Hud.labels then
        draw_labels(ui, people)
    end
    ui:screen_label(
        "bottom_left",
        16,
        12,
        "Tab next shot   1-9 shots   F follow   B states   V senses   N navmesh   P paths   X x-ray   G gate   T tag   Click: call people over   H hide",
        { color = SOFT, size = 13 }
    )
end

return Hud
