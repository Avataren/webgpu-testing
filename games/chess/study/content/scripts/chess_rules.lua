-- Chess position bookkeeping for the spectator. The UCI engine chooses every move;
-- this only applies them (castling, en passant, promotion), notices check, mate,
-- stalemate and draws, and writes short algebraic notation for the move list.
--
-- Squares are 0..63 with a1 = 0, b1 = 1, ..., h8 = 63. Pieces are tables
-- { white = bool, kind = "pawn" | "knight" | "bishop" | "rook" | "queen" | "king", ... }.

local R = {}

local LETTER = { knight = "N", bishop = "B", rook = "R", queen = "Q", king = "K", pawn = "" }
local PROMOTE = { q = "queen", r = "rook", b = "bishop", n = "knight" }
local BACK = { "rook", "knight", "bishop", "queen", "king", "bishop", "knight", "rook" }

function R.square_name(sq)
    return string.char(97 + sq % 8) .. tostring(sq // 8 + 1)
end

function R.parse_square(text)
    local f = text:byte(1) - 97
    local r = text:byte(2) - 49
    if f < 0 or f > 7 or r < 0 or r > 7 then return nil end
    return r * 8 + f
end

-- The starting position: board[sq] = { white, kind }.
function R.start_board()
    local board = {}
    for f = 0, 7 do
        board[f] = { white = true, kind = BACK[f + 1] }
        board[8 + f] = { white = true, kind = "pawn" }
        board[48 + f] = { white = false, kind = "pawn" }
        board[56 + f] = { white = false, kind = BACK[f + 1] }
    end
    return board
end

function R.new_game()
    local g = {
        board = R.start_board(),
        white_to_move = true,
        castling = { K = true, Q = true, k = true, q = true },
        ep = nil,
        halfmove = 0,
        history = {},   -- UCI moves
        notation = {},  -- SAN-like text per ply
        seen = {},      -- position key -> count
    }
    g.seen[R.key(g)] = 1
    return g
end

local CHAR = { pawn = "p", knight = "n", bishop = "b", rook = "r", queen = "q", king = "k" }

function R.key(g)
    local parts = {}
    for sq = 0, 63 do
        local p = g.board[sq]
        parts[#parts + 1] = p and (p.white and CHAR[p.kind]:upper() or CHAR[p.kind]) or "."
    end
    local c = g.castling
    return table.concat(parts) .. (g.white_to_move and "w" or "b") ..
        (c.K and "K" or "") .. (c.Q and "Q" or "") .. (c.k and "k" or "") .. (c.q and "q" or "") ..
        (g.ep and tostring(g.ep) or "-")
end

local KNIGHT = { { 1, 2 }, { 2, 1 }, { 2, -1 }, { 1, -2 }, { -1, -2 }, { -2, -1 }, { -2, 1 }, { -1, 2 } }
local KING = { { 1, 0 }, { 1, 1 }, { 0, 1 }, { -1, 1 }, { -1, 0 }, { -1, -1 }, { 0, -1 }, { 1, -1 } }
local DIAG = { { 1, 1 }, { 1, -1 }, { -1, 1 }, { -1, -1 } }
local LINE = { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }

local function at(board, f, r)
    if f < 0 or f > 7 or r < 0 or r > 7 then return nil, false end
    return board[r * 8 + f], true
end

-- Does the piece on `from` attack `to` (ignoring pins)?
function R.attacks(board, from, to)
    local p = board[from]
    if not p then return false end
    local ff, fr = from % 8, from // 8
    local tf, tr = to % 8, to // 8
    local df, dr = tf - ff, tr - fr
    if p.kind == "pawn" then
        return math.abs(df) == 1 and dr == (p.white and 1 or -1)
    elseif p.kind == "knight" then
        return (math.abs(df) == 1 and math.abs(dr) == 2) or (math.abs(df) == 2 and math.abs(dr) == 1)
    elseif p.kind == "king" then
        return math.max(math.abs(df), math.abs(dr)) == 1
    end
    local diagonal = math.abs(df) == math.abs(dr) and df ~= 0
    local straight = (df == 0) ~= (dr == 0)
    if p.kind == "bishop" and not diagonal then return false end
    if p.kind == "rook" and not straight then return false end
    if p.kind == "queen" and not (diagonal or straight) then return false end
    local sf = df == 0 and 0 or (df > 0 and 1 or -1)
    local sr = dr == 0 and 0 or (dr > 0 and 1 or -1)
    local f, r = ff + sf, fr + sr
    while f ~= tf or r ~= tr do
        if board[r * 8 + f] then return false end
        f, r = f + sf, r + sr
    end
    return true
end

function R.attacked(board, sq, by_white)
    for from = 0, 63 do
        local p = board[from]
        if p and p.white == by_white and R.attacks(board, from, sq) then
            return true
        end
    end
    return false
end

function R.in_check(g, white)
    for sq = 0, 63 do
        local p = g.board[sq]
        if p and p.white == white and p.kind == "king" then
            return R.attacked(g.board, sq, not white)
        end
    end
    return false
end

local function insufficient(board)
    local minors, others = 0, 0
    for sq = 0, 63 do
        local p = board[sq]
        if p and p.kind ~= "king" then
            if p.kind == "knight" or p.kind == "bishop" then minors = minors + 1 else others = others + 1 end
        end
    end
    return others == 0 and minors <= 1
end

-- Apply a UCI move. Returns a delta describing what moved for the animation, or
-- nil and an error when the move does not fit the position.
function R.apply(g, uci)
    local from = R.parse_square(uci:sub(1, 2))
    local to = R.parse_square(uci:sub(3, 4))
    if not from or not to then return nil, "bad move " .. uci end
    local board = g.board
    local p = board[from]
    if not p or p.white ~= g.white_to_move then
        return nil, ("%s: no %s piece on %s"):format(uci, g.white_to_move and "white" or "black", uci:sub(1, 2))
    end
    local target = board[to]
    if target and target.white == p.white then return nil, uci .. ": own piece on the target" end
    local delta = { from = from, to = to, piece = p, captured = nil, captured_square = nil }
    local df = to % 8 - from % 8

    -- Notation, worked out before the board changes.
    local san
    if p.kind == "king" and math.abs(df) == 2 then
        san = df > 0 and "O-O" or "O-O-O"
    else
        local capture = target ~= nil or (p.kind == "pawn" and df ~= 0)
        local prefix = LETTER[p.kind]
        if p.kind == "pawn" then
            prefix = capture and string.char(97 + from % 8) or ""
        else
            local same_file, same_rank, other = false, false, false
            for sq = 0, 63 do
                local q = board[sq]
                if sq ~= from and q and q.white == p.white and q.kind == p.kind and R.attacks(board, sq, to) then
                    other = true
                    same_file = same_file or sq % 8 == from % 8
                    same_rank = same_rank or sq // 8 == from // 8
                end
            end
            if other then
                if not same_file then prefix = prefix .. string.char(97 + from % 8)
                elseif not same_rank then prefix = prefix .. tostring(from // 8 + 1)
                else prefix = prefix .. R.square_name(from) end
            end
        end
        san = prefix .. (capture and "x" or "") .. R.square_name(to)
    end

    -- En passant: a pawn moving diagonally onto an empty square.
    if p.kind == "pawn" and df ~= 0 and not target then
        local victim = to + (p.white and -8 or 8)
        delta.captured = board[victim]
        delta.captured_square = victim
        board[victim] = nil
    elseif target then
        delta.captured = target
        delta.captured_square = to
    end
    board[to] = p
    board[from] = nil
    -- Castling moves the rook too.
    if p.kind == "king" and math.abs(df) == 2 then
        local rank = from // 8
        local rook_from = rank * 8 + (df > 0 and 7 or 0)
        local rook_to = rank * 8 + (df > 0 and 5 or 3)
        delta.rook = { from = rook_from, to = rook_to, piece = board[rook_from] }
        board[rook_to] = board[rook_from]
        board[rook_from] = nil
    end
    -- Promotion.
    local promo = uci:sub(5, 5)
    if p.kind == "pawn" and (to // 8 == 7 or to // 8 == 0) then
        local kind = PROMOTE[promo] or "queen"
        board[to] = { white = p.white, kind = kind, entity = p.entity, promoted_from = "pawn" }
        delta.promotion = kind
        delta.piece = board[to]
        san = san .. "=" .. LETTER[kind]
    end
    -- Rights, en passant square, clocks.
    local c = g.castling
    for _, sq in ipairs({ from, to }) do
        if sq == 4 then c.K, c.Q = false, false end
        if sq == 60 then c.k, c.q = false, false end
        if sq == 0 then c.Q = false end
        if sq == 7 then c.K = false end
        if sq == 56 then c.q = false end
        if sq == 63 then c.k = false end
    end
    g.ep = (p.kind == "pawn" and math.abs(to - from) == 16) and (from + to) // 2 or nil
    g.halfmove = (p.kind == "pawn" or delta.captured) and 0 or g.halfmove + 1
    g.white_to_move = not g.white_to_move
    g.history[#g.history + 1] = uci
    if R.in_check(g, g.white_to_move) then san = san .. "+" end
    g.notation[#g.notation + 1] = san
    local key = R.key(g)
    g.seen[key] = (g.seen[key] or 0) + 1
    delta.repetitions = g.seen[key]
    return delta
end

-- A result the position decides on its own (draws); mate and stalemate are noticed
-- when the engine has no move.
function R.draw_result(g, ply_limit)
    if g.seen[R.key(g)] and g.seen[R.key(g)] >= 3 then return "Draw by repetition" end
    if g.halfmove >= 100 then return "Draw by the fifty-move rule" end
    if insufficient(g.board) then return "Draw by insufficient material" end
    if #g.history >= ply_limit then return "Draw at the exhibition limit" end
    return nil
end

function R.no_move_result(g)
    if R.in_check(g, g.white_to_move) then
        local last = g.notation[#g.notation]
        if last then g.notation[#g.notation] = last:gsub("%+$", "") .. "#" end
        return g.white_to_move and "Ebony wins by checkmate" or "Ivory wins by checkmate"
    end
    return "Draw by stalemate"
end

return R
