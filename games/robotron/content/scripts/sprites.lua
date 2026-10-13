-- Pixel art for every actor, after the 1982 sprites. robotron.lua turns each sprite into a voxel
-- model (one cube per same-coloured rectangle, extruded `depth` voxels) and uses the same pixels for
-- the line "materialize" spawn-in and the stretched-apart explosions.
--
-- Rows run top to bottom; '.' is empty. `legend` maps a character to a colour in COLORS. Frames of
-- one sprite must have the same size; pixels shared by every frame are built once.

local M = {}

-- name -> { r, g, b, emissive }
M.COLORS = {
    red = { 1.0, 0.06, 0.04, 0.35 },
    dred = { 0.55, 0.02, 0.03, 0.2 },
    orange = { 1.0, 0.42, 0.02, 0.4 },
    yellow = { 1.0, 0.85, 0.05, 0.45 },
    green = { 0.08, 0.85, 0.12, 0.3 },
    dgreen = { 0.02, 0.38, 0.06, 0.2 },
    lgreen = { 0.45, 1.0, 0.3, 0.35 },
    blue = { 0.1, 0.22, 1.0, 0.4 },
    dblue = { 0.04, 0.08, 0.5, 0.25 },
    cyan = { 0.1, 0.9, 1.0, 0.5 },
    magenta = { 1.0, 0.08, 0.85, 0.45 },
    pink = { 1.0, 0.38, 0.62, 0.35 },
    purple = { 0.5, 0.12, 1.0, 0.4 },
    white = { 0.92, 0.92, 0.95, 0.3 },
    grey = { 0.45, 0.47, 0.52, 0.15 },
    skin = { 1.0, 0.62, 0.42, 0.25 },
    brown = { 0.5, 0.22, 0.06, 0.15 },
    black = { 0.04, 0.04, 0.05, 0.0 },
    eye_red = { 1.0, 0.1, 0.05, 3.5 },
    eye_cyan = { 0.3, 1.0, 1.0, 3.0 },
    eye_green = { 0.3, 1.0, 0.2, 3.0 },
    glow_white = { 1.0, 1.0, 1.0, 3.0 },
    glow_yellow = { 1.0, 0.85, 0.2, 3.0 },
}

-- Legs: three walk frames that replace the bottom rows of a two-legged figure.
local function walker(top, stand, step_a, step_b)
    local f = {}
    for i, legs in ipairs({ stand, step_a, step_b }) do
        local rows = {}
        for _, r in ipairs(top) do rows[#rows + 1] = r end
        for _, r in ipairs(legs) do rows[#rows + 1] = r end
        f[i] = rows
    end
    return f
end

M.player = {
    depth = 3,
    legend = { H = "orange", S = "skin", E = "eye_cyan", W = "white", R = "red", B = "blue", G = "grey" },
    frames = walker({
        "....HHH....",
        "...HHHHH...",
        "...HSSSH...",
        "...SESES...",
        "...SSSSS...",
        "....SSS....",
        "..WWWWWWW..",
        ".WWWRRRWWW.",
        "WW.WWRWW.WW",
        "WW.WWWWW.WW",
        "SS.RRRRR.SS",
        "...BBBBB...",
    }, {
        "...BB.BB...",
        "...BB.BB...",
        "...BB.BB...",
        "..WWW.WWW..",
    }, {
        "...BB.BB...",
        "..WWW.BB...",
        "......BB...",
        "......WWW..",
    }, {
        "...BB.BB...",
        "...BB.WWW..",
        "...BB......",
        "..WWW......",
    }),
}

M.grunt = {
    depth = 4,
    legend = { R = "red", D = "dred", Y = "yellow", E = "eye_green" },
    frames = walker({
        "...RRRRR...",
        "...RYYYR...",
        "...RERER...",
        "...RRRRR...",
        "....DDD....",
        ".RRRRRRRRR.",
        "RRYRRRRRYRR",
        "RR.RRYRR.RR",
        "RR.RYRYR.RR",
        "YY.RRRRR.YY",
        "...DDDDD...",
    }, {
        "...RR.RR...",
        "...RR.RR...",
        "..YYY.YYY..",
    }, {
        "..RR..RR...",
        "..YYY.RR...",
        "......YYY..",
    }, {
        "...RR..RR..",
        "...RR.YYY..",
        "..YYY......",
    }),
}

M.hulk = {
    depth = 6,
    legend = { G = "green", D = "dgreen", L = "lgreen", E = "eye_red", Y = "yellow" },
    frames = walker({
        ".....DDDDD.....",
        ".....DEDED.....",
        ".....DDDDD.....",
        "......DDD......",
        "...GGGGGGGGG...",
        ".GGGLLLLLLLGGG.",
        "GGG.LLLLLLL.GGG",
        "GG..LLYYYLL..GG",
        "GG..LLYYYLL..GG",
        "GG..LLLLLLL..GG",
        "DD..GGGGGGG..DD",
        "DD..DDDDDDD..DD",
    }, {
        "....GGG.GGG....",
        "....GGG.GGG....",
        "....GGG.GGG....",
        "...DDDD.DDDD...",
    }, {
        "....GGG.GGG....",
        "...DDDD.GGG....",
        "........GGG....",
        "........DDDD...",
    }, {
        "....GGG.GGG....",
        "....GGG.DDDD...",
        "....GGG........",
        "...DDDD........",
    }),
}

M.brain = {
    depth = 4,
    legend = { P = "pink", M = "magenta", W = "white", E = "eye_red", B = "blue", C = "cyan" },
    frames = walker({
        "...PPPPPPP...",
        ".PPMPPPPPMPP.",
        "PPMPPMPPMPPMP",
        "PMPPMPPMPPMPP",
        "PPPMPPMPPPMPP",
        ".PPPPPPPPPPP.",
        "...WEW.WEW...",
        "....CCCCC....",
        "...BBBBBBB...",
        "..BB.BBB.BB..",
        "..C..BBB..C..",
    }, {
        ".....B.B.....",
        ".....B.B.....",
        "....BB.BB....",
    }, {
        "....B..B.....",
        "...BB..B.....",
        ".......BB....",
    }, {
        ".....B..B....",
        ".....B..BB...",
        "....BB.......",
    }),
}

M.mommy = {
    depth = 3,
    legend = { Y = "yellow", S = "skin", E = "blue", P = "pink", R = "red" },
    frames = walker({
        "..YYYYY..",
        ".YYYYYYY.",
        ".YSSSSSY.",
        ".YSESESY.",
        ".YSSSSSY.",
        "..YPPPY..",
        "..PPPPP..",
        ".PPPPPPP.",
        "S.PPPPP.S",
        "..PPPPP..",
        ".PPPPPPP.",
        "PPPPPPPPP",
    }, {
        "...S.S...",
        "...S.S...",
        "..RR.RR..",
    }, {
        "..S..S...",
        ".RR..S...",
        ".....RR..",
    }, {
        "...S..S..",
        "...S.RR..",
        "..RR.....",
    }),
}

M.daddy = {
    depth = 3,
    legend = { H = "brown", S = "skin", E = "blue", B = "blue", W = "white", R = "red", K = "grey" },
    frames = walker({
        "..HHHHH..",
        "..HHHHH..",
        "..SSSSS..",
        "..SESES..",
        "..SSSSS..",
        "...SSS...",
        ".BBBWBBB.",
        "BBBBRBBBB",
        "BB.BRB.BB",
        "BB.BBB.BB",
        "SS.BBB.SS",
        "...BBB...",
    }, {
        "..BB.BB..",
        "..BB.BB..",
        "..BB.BB..",
        ".KKK.KKK.",
    }, {
        "..BB.BB..",
        ".KKK.BB..",
        ".....BB..",
        ".....KKK.",
    }, {
        "..BB.BB..",
        "..BB.KKK.",
        "..BB.....",
        ".KKK.....",
    }),
}

M.mikey = {
    depth = 3,
    legend = { H = "orange", S = "skin", E = "blue", R = "red", B = "blue", W = "white" },
    frames = walker({
        ".HHHHH.",
        ".SSSSS.",
        ".SESES.",
        ".SSSSS.",
        "RRRRRRR",
        "R.RRR.R",
        "S.RRR.S",
        "..BBB..",
    }, {
        "..B.B..",
        "..B.B..",
        ".WW.WW.",
    }, {
        ".B..B..",
        "WW..B..",
        "....WW.",
    }, {
        "..B..B.",
        "..B.WW.",
        ".WW....",
    }),
}

M.prog = {
    depth = 3,
    legend = { C = "cyan", E = "eye_red", M = "magenta", Y = "yellow" },
    frames = walker({
        "..CCCCC..",
        "..CECEC..",
        "..CCCCC..",
        "...MMM...",
        ".MMMMMMM.",
        "MYMMMMMYM",
        "M.MMYMM.M",
        "M.MYMYM.M",
        "Y.MMMMM.Y",
        "..MMMMM..",
    }, {
        "..MM.MM..",
        "..MM.MM..",
        "..MM.MM..",
        ".YYY.YYY.",
    }, {
        "..MM.MM..",
        ".YYY.MM..",
        ".....MM..",
        ".....YYY.",
    }, {
        "..MM.MM..",
        "..MM.YYY.",
        "..MM.....",
        ".YYY.....",
    }),
}

M.enforcer = {
    depth = 3,
    legend = { W = "white", E = "eye_red", G = "green", Y = "yellow" },
    frames = {
        {
            "....WWW....",
            "...WEEEW...",
            "..WWWWWWW..",
            ".WWGGGGGWW.",
            "WWGGYGYGGWW",
            ".WWGGGGGWW.",
            "..WWWWWWW..",
            "...W...W...",
        },
        {
            "....WWW....",
            "...WEEEW...",
            "..WWWWWWW..",
            ".WWGGGGGWW.",
            "WWGYGGGYGWW",
            ".WWGGGGGWW.",
            "..WWWWWWW..",
            "..W.....W..",
        },
    },
}

M.spheroid = {
    depth = 2,
    legend = { R = "red", O = "orange", Y = "yellow", W = "glow_white" },
    frames = {
        {
            "...........",
            "...........",
            "...........",
            "....RRR....",
            "...R...R...",
            "...R.W.R...",
            "...R...R...",
            "....RRR....",
            "...........",
            "...........",
            "...........",
        },
        {
            "...........",
            "...OOOOO...",
            "..O.....O..",
            ".O..RRR..O.",
            ".O.R...R.O.",
            ".O.R.W.R.O.",
            ".O.R...R.O.",
            ".O..RRR..O.",
            "..O.....O..",
            "...OOOOO...",
            "...........",
        },
        {
            "...YYYYY...",
            ".YY.....YY.",
            ".Y..OOO..Y.",
            "Y..O...O..Y",
            "Y.O..R..O.Y",
            "Y.O.RWR.O.Y",
            "Y.O..R..O.Y",
            "Y..O...O..Y",
            ".Y..OOO..Y.",
            ".YY.....YY.",
            "...YYYYY...",
        },
    },
}

M.quark = {
    depth = 2,
    legend = { C = "cyan", M = "magenta", W = "glow_white" },
    frames = {
        {
            ".....C.....",
            ".....C.....",
            ".....C.....",
            ".....M.....",
            ".....M.....",
            "CCCMMWMMCCC",
            ".....M.....",
            ".....M.....",
            ".....C.....",
            ".....C.....",
            ".....C.....",
        },
        {
            "C.........C",
            ".C.......C.",
            "..C.....C..",
            "...M...M...",
            "....M.M....",
            ".....W.....",
            "....M.M....",
            "...M...M...",
            "..C.....C..",
            ".C.......C.",
            "C.........C",
        },
    },
}

M.tank = {
    depth = 8,
    legend = { B = "blue", D = "dblue", W = "eye_cyan", Y = "yellow", R = "red", K = "dred" },
    frames = {
        {
            ".....BBB.....",
            "....BWWWB....",
            "....BBBBB....",
            "..BBBBBBBBB..",
            ".BBYBBBBBYBB.",
            "DDDDDDDDDDDDD",
            "RRRRRRRRRRRRR",
            "R.RK.RK.RK.RR",
            "RRRRRRRRRRRRR",
            ".RRRRRRRRRRR.",
        },
        {
            ".....BBB.....",
            "....BWWWB....",
            "....BBBBB....",
            "..BBBBBBBBB..",
            ".BBYBBBBBYBB.",
            "DDDDDDDDDDDDD",
            "RRRRRRRRRRRRR",
            "RK.RK.RK.RK.R",
            "RRRRRRRRRRRRR",
            ".RRRRRRRRRRR.",
        },
    },
}

-- Electrodes change shape every wave; A and B are swapped for the wave's colours.
local ELECTRODE_SHAPES = {
    {
        "AAAAAAAAA",
        "A.......A",
        "A.BBBBB.A",
        "A.B...B.A",
        "A.B.A.B.A",
        "A.B...B.A",
        "A.BBBBB.A",
        "A.......A",
        "AAAAAAAAA",
    },
    {
        "....A....",
        "...ABA...",
        "..AB.BA..",
        ".AB.A.BA.",
        "AB.AAA.BA",
        ".AB.A.BA.",
        "..AB.BA..",
        "...ABA...",
        "....A....",
    },
    {
        "...AAA...",
        "...ABA...",
        "...ABA...",
        "AAAABAAAA",
        "ABBBBBBBA",
        "AAAABAAAA",
        "...ABA...",
        "...ABA...",
        "...AAA...",
    },
    {
        "....A....",
        "....A....",
        "...ABA...",
        "...ABA...",
        "..AB.BA..",
        "..ABBBA..",
        ".AB...BA.",
        ".ABBBBBA.",
        "AAAAAAAAA",
    },
    {
        "A.......A",
        ".A..B..A.",
        "..A.B.A..",
        "...ABA...",
        "BBBBABBBB",
        "...ABA...",
        "..A.B.A..",
        ".A..B..A.",
        "A.......A",
    },
}
local ELECTRODE_COLORS = {
    { "yellow", "red" }, { "cyan", "magenta" }, { "orange", "blue" }, { "lgreen", "purple" },
    { "magenta", "yellow" }, { "white", "red" }, { "red", "cyan" }, { "blue", "orange" },
}
M.ELECTRODE_VARIANTS = #ELECTRODE_SHAPES * #ELECTRODE_COLORS
-- variant v (1-based) -> sprite; the shape changes every wave and the colours every few
function M.electrode(v)
    local s = (v - 1) % #ELECTRODE_SHAPES + 1
    local c = ELECTRODE_COLORS[(v - 1) % #ELECTRODE_COLORS + 1]
    return {
        depth = 2,
        legend = { A = c[1], B = c[2] },
        frames = { ELECTRODE_SHAPES[s] },
    }
end

M.skull = {
    depth = 2,
    legend = { W = "white", K = "black" },
    frames = { {
        "..WWWWW..",
        ".WWWWWWW.",
        "WWWWWWWWW",
        "WWKKWKKWW",
        "WWKKWKKWW",
        "WWWWKWWWW",
        ".WWWWWWW.",
        "..WKWKW..",
        "W..WWW..W",
        ".W.....W.",
        "..W...W..",
    } },
}

M.spark = {
    depth = 1,
    legend = { A = "glow_white", B = "purple" },
    frames = {
        { "B...B", ".B.B.", "..A..", ".B.B.", "B...B" },
        { "..B..", "..B..", "BBABB", "..B..", "..B.." },
    },
}

M.shell = {
    depth = 3,
    legend = { A = "glow_yellow", B = "orange" },
    frames = { { ".B.", "BAB", ".B." } },
}

M.missile = {
    depth = 3,
    legend = { A = "glow_white", B = "magenta" },
    frames = { { "BAB", "AAA", "BAB" } },
}

return M
