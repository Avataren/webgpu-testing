-- Tuning, the arena's size and the arcade's units. Every other module reads these.

local SPR = require("sprites")
local COLORS = SPR.COLORS

local HW, HD = 19.0, 12.0                -- arena half extents inside the border (x, z)
local P_R = 0.42                         -- player hit radius
local LX, LZ = HW - 0.45, HD - 0.45      -- player position limits
local P_SPEED = 9.0
local FIRE_INT = 0.085
local B_SPEED, B_R = 38.0, 0.16
local N_PBULLETS = 24
local START_LIVES = 3
local EXTRA_LIFE = 25000
local SFX_VOLUME = 0.55
local PLAY_MUSIC = false                 -- the original has no music during play
local MUSIC_VOLUME = 0.12
local VOX = 0.125                        -- one sprite pixel, in meters
local LEAN = math.rad(16)                -- actors lean back toward the camera so the sprite reads
local CAM_PITCH = math.rad(60)           -- camera angle below the horizon
local FOV = 26.0
local MAT_T = 0.75                       -- materialize time
local SPREAD = 10.0                      -- how far apart the pixel lines start
local TRANS_T = 0.95                     -- rectangle zoom

-- Arcade units. The 1982 game runs 60 frames a second and its player covers one pixel a frame, so
-- one pixel is PXM here; x positions there are bytes of two pixels (BYM). Enemy speeds, step sizes
-- and timers below are the original's, converted with these.
local FR = 1 / 60
local PXM = P_SPEED / 60
local BYM = 2 * PXM
local GRUNT_STEP = 4 * PXM
local PROG_SPEED = 4 * PXM / (3 * FR)

local sin, cos = math.sin, math.cos
local CL, SL = cos(LEAN), sin(LEAN) -- the actors' lean

-- glowing unlit colours for the transition rectangles and the death flash
local FLASH = { "red", "orange", "yellow", "green", "cyan", "blue", "magenta", "white" }
for _, n in ipairs(FLASH) do
    local c = COLORS[n == "green" and "lgreen" or n]
    COLORS["fx_" .. n] = { c[1], c[2], c[3], 3.0, true }
end

return {
    SPR = SPR, COLORS = COLORS, HW = HW, HD = HD, P_R = P_R,
    LX = LX, LZ = LZ, P_SPEED = P_SPEED, FIRE_INT = FIRE_INT, B_SPEED = B_SPEED,
    B_R = B_R, N_PBULLETS = N_PBULLETS, START_LIVES = START_LIVES, EXTRA_LIFE = EXTRA_LIFE, SFX_VOLUME = SFX_VOLUME,
    PLAY_MUSIC = PLAY_MUSIC, MUSIC_VOLUME = MUSIC_VOLUME, VOX = VOX, LEAN = LEAN, CAM_PITCH = CAM_PITCH,
    FOV = FOV, MAT_T = MAT_T, SPREAD = SPREAD, TRANS_T = TRANS_T, FR = FR,
    PXM = PXM, BYM = BYM, GRUNT_STEP = GRUNT_STEP, PROG_SPEED = PROG_SPEED, CL = CL,
    SL = SL, FLASH = FLASH,
}
