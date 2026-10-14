# Robotron 2084 (in voxels)

A twin-stick arena shooter after the 1982 original, built entirely through the engine MCP.
Every actor is a voxel model of its arcade sprite standing on a lit floor, casting a real
shadow, and the original's effects are rebuilt from the same pixels:

- **Materialize**: at the start of a wave (and after a death) every enemy and the player beam in
  as spread-apart pixel lines that close up into the sprite; rows on odd waves, columns on even.
- **Explosions** stretch a shot enemy apart along the line of fire.
- **Rectangle zoom**: concentric colour-cycling rectangles zoom out from the player between waves.
- **Death**: everything freezes while the player flashes through the colours and bursts; then the
  surviving enemies materialize again in new places (the family stays put).
- Progs leave rainbow afterimages, cruise missiles zig-zag, a brain's victim flickers while it is
  reprogrammed, a hulk leaves a skull where it crushed someone, and the border cycles colours. As in the arcade there is no music during play.

The game boots into an attract mode where an autopilot plays a demo game under the title;
press Enter to take over.

## Run

```
# editor: open games/robotron (project_open), then press Play
# player:
cargo run --bin player -- games/robotron      # or engine_launch {target: "player", project: ...}
```

## Controls

| Input | Action |
| --- | --- |
| `W A S D` | Move (8 directions) |
| Arrow keys | Fire (8 directions; hold two for diagonals) |
| Left mouse button | Fire toward the cursor |
| `Enter` / `Space` / click | Start a game from the attract screen |
| `B` | Toggle the autopilot during a game |
| `P` | Pause |
| `Esc` | Back to the demo |

## Rules (as in the original)

Enemy behaviour, speeds, fire rates and the wave table come from the 1982 source code
(Williams' 6809 assembly, published as `historicalsource/robotron`), converted from its 60 Hz
frames and pixels. The player covers one arcade pixel a frame, as in the arcade.

- One hit kills you. Three lives; an extra man every 25,000 points.
- A wave ends when every grunt, brain, spheroid, enforcer, quark and tank is gone. Hulks,
  electrodes, progs and projectiles do not count and vanish with it.
- Rescue the family (Mommy, Daddy, Mikey): 1000, 2000 ... up to 5000 points each, rising within a
  wave and reset when you die. They wander in eight directions and steer around electrodes.
- **Grunts** (100) step four pixels at a time toward you, each after a random 1..N ticks. Every grunt
  you kill makes the rest an eighth faster, and every 3.75 s their top speed rises (twice as fast
  when you are not scoring). A grunt that walks into an electrode dies.
- **Electrodes** are static and deadly; shooting them is optional. Hulks crush them.
- **Hulks** cannot be killed; a shot knocks one back a little. They walk one axis at a time toward a
  family member (a quarter of them toward you) and crush the family, leaving a skull.
- **Brains** (500) start near the walls and walk to the nearest family member, stop and reprogram it
  into a **Prog** (100), which runs at you faster than you can move along one axis at a time.
  Brains fire zig-zagging **cruise missiles** (25, shootable, up to eight in the air) that bounce
  off the walls until shot.
- **Spheroids** (1000) start at the side walls and drift about, drop one to five **Enforcers**
  (150) and then leave through a wall. Enforcers glide toward you and fire **sparks** (25,
  shootable) that curve and slide along the walls.
- **Quarks** (1000) start at the top or bottom wall and drop one to five **Tanks** (200), then
  leave. Tanks fire shells (25, shootable), half of them banked off the wall nearest you.
- After a death the wave restarts with what is left in new places: progs and projectiles are gone,
  and every four enforcers left become a spheroid again.
- The 40-wave table is the original's (brain waves 5, 10, 15 ..., tank waves 7, 12, 17 ...,
  grunt swarms 9, 19, 29, 39); after wave 40 it repeats waves 21-40. Electrodes change shape every
  wave.

## Files

- `content/scripts/robotron.lua`: the controller script on `GameController`: the engine hooks
  (`on_created`, `update`, `on_destroyed`, `on_ui`) and the test globals (`GAME`, `RT`). Every
  actor and bullet is spawned and pooled by the scripts.
- `content/scripts/robotron/`: the game, one module per part, each loaded with `require`:

  | Module | What it holds |
  |---|---|
  | `config` | tuning, the arena's size, the arcade's units (`FR`, `PXM`, `BYM`) |
  | `util` | maths helpers |
  | `state` | `GAME` (`g`), the actor and bullet lists, sound and entity helpers |
  | `models` | sprites to voxel models |
  | `fx` | pixel particles, popups, shake, the rectangle zoom, the walls' colours |
  | `actors` | the kinds (`K`), pooled units, spawning, kills and scoring, shared queries |
  | `autopilot` | the attract-mode / `B` pilot |
  | `player` | the player, input, shots and death |
  | `ai` | enemy and family AI and the per-frame actor update |
  | `combat` | collisions |
  | `waves` | the 40-wave tables, where actors start, starting and ending a wave, the restart after a death |
  | `flow` | the game's state machine, new games, the keys outside play |
  | `camera` | fitting the camera to the window |
  | `hud` | on-screen text |

  A module only requires modules above it in this list, so there are no load cycles. Shared
  tables in `state` are changed in place, never replaced, so every module can keep a reference.
- `content/scripts/sprites.lua`: the pixel art, after the arcade sprites, with walk frames. Each
  sprite becomes a voxel model (one cube per same-coloured rectangle of pixels, extruded `depth`
  voxels); pixels shared by all frames are built once, the rest go under a node per frame. The
  materialize, explosion and afterimage effects emit one GPU particle per pixel.
- `content/audio/*.wav`: generated by `python3 tools/gen_audio.py content/audio` (needs numpy).
- The arena (floor and border), the sun and the camera are ordinary scene entities. The script
  fits the camera to the window shape so the whole arena shows with room for the HUD.

## The autopilot

`bot_think` in the script plays the game: every 70 ms it simulates the player's eight moves 0.7 s
ahead against predicted enemy and projectile positions and picks the safest one, leaning toward
the nearest family member that no hulk or brain is right next to (scored by how close the path
comes to them, so it does not overshoot one nearby); every 50 ms it picks the firing direction
that lines up the most valuable targets (leading moving ones, skipping those a hulk or electrode
blocks). With family left and the wave nearly over it stops shooting at distant enemies, and
spares the last two (keeping them out of its line of fire) unless one gets within 1.8 m or is a
brain near a human, so the wave stays open while it collects the family; after 25 s it gives up
and finishes the wave.

## Testing hooks

`GAME` holds the state and cumulative `GAME.stats`. A global `RT` table (use
`lua_eval {entity: "GameController"}`) has `RT.jump(wave)`, `RT.spawn(kind, x, z)`, `RT.god(true)`,
`RT.bot(false)`, `RT.botinfo()` (the autopilot's human and enemy counts, hold time, position), `RT.kill()`, `RT.lives(n)`, `RT.units(kind)`, `RT.count()`, `RT.new(demo)`,
`RT.pooled()` (pool sizes and cube count) and `RT.lineup()` (one of every actor in a row, for
screenshots with `runtime_set {time_scale: 0}`).
A soak test: `playtest {seconds: 45, time_scale: 16, lean: true, entity: "GameController",
eval: "return GAME.stats"}`.
