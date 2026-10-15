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
local BULGE = 2                          -- how many voxels a sprite swells out on each side
local LEAN = math.rad(16)                -- actors lean back toward the camera so the sprite reads
local CAM_PITCH = math.rad(60)           -- camera angle below the horizon
local FOV = 26.0
local MAT_T = 0.75                       -- materialize time
local SPREAD = 10.0                      -- how far apart the pixel lines start
local TRANS_T = 0.95                     -- rectangle zoom
local TITLE_T = 8.0                      -- seconds the attract mode shows its title

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

-- Two looks for the arena. The raster renderer has to fake the neon's light: translucent glow
-- strips on the floor along the border (Spill_*), a cool and a warm directional fill light (the
-- warm one takes the border's colour) and a strong sun so the actors read. With path tracing
-- the border tube lights the arena by itself (soft, coloured, shadowed by the actors and
-- reflected in the floor), so the fakes go: a dimmer sun, no fills, no strips, a floor
-- light enough to catch the neon and glossy enough to mirror it, and a brighter tube.
-- `floor` is the Arena's material; the raster values are the scene's own. `ambient` is the
-- environment's ambient light, which path-traced rays also see as a grey sky: too much of it
-- washes out the neon. `hdr` is the environment's sky: with path tracing a neon skyline at night
-- (Poly Haven's Shanghai Bund, CC0) lights the scene softly from every side and glints in the
-- glossy floor and robots. It is not drawn behind the arena (`background = false`): seen
-- directly, its lower half is cut off by the black Floor plane and looks out of place, so the
-- void around the arena stays black.
local LOOK = {
    raster = {
        sun = 2.4, fill_cool = 0.55, fill_warm = 0.35, spills = true, ambient = 0.18,
        wall_glow = 2.6, body_emissive = 0.15, grid = 0.4,
        floor = { color = { 0.3216, 0.3216, 0.3412 }, roughness = 0.2196, metallic = 0.149,
            base_color_texture = "content/textures/floor_base.png" },
        hdr = { enabled = false },
    },
    pt = {
        sun = 1.4, fill_cool = 0.0, fill_warm = 0.0, spills = false, ambient = 0.03,
        wall_glow = 10.0, body_emissive = 0.0, grid = 0.6,
        floor = { color = { 1.0, 1.0, 1.0 }, roughness = 0.12, metallic = 0.0,
            base_color_texture = "content/textures/floor_base_pt.png" },
        hdr = { enabled = true, intensity = 1.0, background = false,
            path = "content/textures/shanghai_bund_2k.hdr" },
    },
}

return {
    LOOK = LOOK,
    SPR = SPR, COLORS = COLORS, HW = HW, HD = HD, P_R = P_R,
    LX = LX, LZ = LZ, P_SPEED = P_SPEED, FIRE_INT = FIRE_INT, B_SPEED = B_SPEED,
    B_R = B_R, N_PBULLETS = N_PBULLETS, START_LIVES = START_LIVES, EXTRA_LIFE = EXTRA_LIFE, SFX_VOLUME = SFX_VOLUME,
    PLAY_MUSIC = PLAY_MUSIC, MUSIC_VOLUME = MUSIC_VOLUME, VOX = VOX, BULGE = BULGE, LEAN = LEAN, CAM_PITCH = CAM_PITCH,
    FOV = FOV, MAT_T = MAT_T, SPREAD = SPREAD, TRANS_T = TRANS_T, TITLE_T = TITLE_T, FR = FR,
    PXM = PXM, BYM = BYM, GRUNT_STEP = GRUNT_STEP, PROG_SPEED = PROG_SPEED, CL = CL,
    SL = SL, FLASH = FLASH,
}
