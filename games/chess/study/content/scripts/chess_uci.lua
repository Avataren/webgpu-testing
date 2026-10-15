-- A UCI engine session over the process API, driven a little every frame so the
-- game never waits on the engine. The spectator asks for a search with search();
-- poll() returns { best = "e2e4" } once the engine answers, and keeps `info` (depth,
-- nodes, score from the side to move) up to date meanwhile.

local U = {}
U.__index = U

-- The engine: the classical-eval XavChess build that ./runchess.sh makes. Relative
-- paths are tried against the working directory (the repository when the editor or
-- player runs from it) and then the project.
U.CANDIDATES = {
    "target/chess-uci/release/xavchess-hce",
    "../../../target/chess-uci/release/xavchess-hce",
}

-- One full-strength setting for both colours, as in the native spectator. Root noise
-- stays off: it can pick moves that drop material.
U.OPTIONS = {
    "setoption name Threads value 16",
    "setoption name Hash value 128",
    "setoption name Ponder value false",
    "setoption name NeuralEval value false",
    "setoption name DirichletAlpha value 0.0",
}

function U.new()
    return setmetatable({ id = nil, state = "off", error = nil, info = {}, waited = 0 }, U)
end

function U:start()
    self:stop()
    local last
    for _, path in ipairs(U.CANDIDATES) do
        local id, err = process_spawn(path)
        if id then
            self.id = id
            self.path = path
            break
        end
        last = err
    end
    if not self.id then
        self.state = "failed"
        self.error = "No UCI engine (" .. tostring(last) .. "). Build it with ./runchess.sh."
        return false
    end
    process_write(self.id, "uci")
    self.state = "handshake"
    self.waited = 0
    return true
end

function U:stop()
    if self.id then
        process_write(self.id, "quit")
        process_kill(self.id)
    end
    self.id = nil
    self.state = "off"
end

function U:ready()
    return self.state == "ready"
end

-- Start a fresh game (clears the engine's tables). The engine answers readyok.
function U:new_game()
    if self.state ~= "ready" and self.state ~= "searching" then return end
    if self.state == "searching" then
        process_write(self.id, "stop")
        self.discard = true
    end
    process_write(self.id, "ucinewgame")
    process_write(self.id, "isready")
    self.state = "resetting"
    self.waited = 0
end

function U:search(moves, millis)
    if self.state ~= "ready" then return false end
    local position = "position startpos"
    if #moves > 0 then position = position .. " moves " .. table.concat(moves, " ") end
    process_write(self.id, position)
    process_write(self.id, "go movetime " .. math.floor(millis))
    self.state = "searching"
    self.info = {}
    self.waited = 0
    self.budget = millis / 1000
    return true
end

local function parse_info(line, info)
    local depth = line:match(" depth (%d+)")
    if depth then info.depth = tonumber(depth) end
    local nodes = line:match(" nodes (%d+)")
    if nodes then info.nodes = tonumber(nodes) end
    local cp = line:match(" score cp (%-?%d+)")
    if cp then info.cp, info.mate = tonumber(cp), nil end
    local mate = line:match(" score mate (%-?%d+)")
    if mate then info.mate, info.cp = tonumber(mate), nil end
end

function U:poll(dt)
    if not self.id then return nil end
    -- Time spent waiting on the engine; idle time between searches does not count.
    self.waited = self.state == "ready" and 0 or self.waited + dt
    local result
    for _, line in ipairs(process_read_lines(self.id)) do
        if self.state == "handshake" and line == "uciok" then
            for _, option in ipairs(U.OPTIONS) do process_write(self.id, option) end
            process_write(self.id, "isready")
            self.state = "configuring"
        elseif line == "readyok" and (self.state == "configuring" or self.state == "resetting") then
            self.state = "ready"
        elseif line:sub(1, 5) == "info " then
            parse_info(line, self.info)
        elseif line:sub(1, 9) == "bestmove " then
            local best = line:match("^bestmove (%S+)")
            if self.discard then
                self.discard = false
            elseif self.state == "searching" then
                self.state = "ready"
                result = { best = best, info = self.info }
            end
        end
    end
    -- A silent or dead engine is restarted; the caller asks again.
    local limit = self.state == "searching" and (self.budget or 1) + 15 or 20
    if not process_running(self.id) or self.waited > limit then
        self.error = "The engine stopped answering; restarting it."
        self:start()
        return { restarted = true }
    end
    return result
end

return U
