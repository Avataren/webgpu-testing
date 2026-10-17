# Marionette

A skeletal animation show and crowd benchmark: the asset library's whole cast of rigged
characters (47 of them: Mixamo's characters and Quaternius's CC0 mannequins) dancing,
running, fighting and filling a stadium, up to about a thousand at once, in a neon arena.
Every clip plays on every rig: Mixamo, Unreal-mannequin and Rigify skeletons bind by
humanoid role.

The show runs about two and a half minutes in six parts:

| Part | What happens | Characters (crowd 1x) |
|---|---|---|
| 1. Roll call | the whole cast in a ring, one clip each, names and triangle counts over their heads; every few seconds the ring changes clip in a ripple | 47 |
| 2. Dance floor | a grid of dancers; new dances spread from the middle outward, each dancer cross-fading as the ripple reaches them | 196 |
| 3. Marathon | six lanes round the track, crawling to sprinting, each runner moved at the speed its clip travels (no foot sliding) | 300 |
| 4. Battle | two armies taunt, charge row by row and fight in pairs: attacks, blocks, dodges, hit reactions, falls and getting up | 300 |
| 5. Stadium | the stands full, cheering and clapping, a Mexican wave going round, dancers on the floor | 1,040 |
| 6. Curtain call | the cast bows to the full stadium | 1,051 |

## Getting the cast

The characters and clips come from the engine's asset library (`docs/asset_library.md`),
not from this repository: Mixamo's licence forbids redistributing its files, so they are
copied in locally and git-ignored.

```sh
tools/library/library.py fetch-pack all        # Quaternius CC0 clips and mannequins
tools/library/library.py fetch-mixamo          # Mixamo characters and clips (your token)
games/marionette/tools/fetch_cast.py           # copy the cast into content/
```

`cast.json` lists the characters (with the groups that decide which parts they appear in)
and the clips by library id. `fetch_cast.py` copies whatever the library has, unchanged,
into `content/characters` and `content/animations` and writes
`content/scripts/cast_data.lua` for the show; `--check` only reports what is missing. A
partial library gives a smaller cast; without any, the show says how to get one.

## Running it

```sh
cargo run --release --all-features --bin player -- games/marionette
```

or open the project in the editor and press Play. Loading imports each character once
(about ten seconds for the full cast); the crowds are copies of those templates
(`clone_entity`).

| Key | |
|---|---|
| Enter or Space | watch the show (it also starts by itself after 12 seconds) |
| B | benchmark: raster, then raster at 50% resolution upscaled with FSR |
| R | benchmark: raster only |
| 1, 2, 3 | crowd size 1x, 2x, 3x (dance floor, marathon, battle and stadium scale with it) |
| F1 | live statistics during the show |
| Escape | the engine's graphics settings |

The show runs with raster lighting: shadow maps, screen-space reflections and ambient
occlusion, and a baked light probe grid (`content/lighting/arena_probes.probes`) for the
bounced light of the neon. It sets that for the run without saving it and puts the
player's own graphics settings back when it stops.

## The benchmark

Each pass plays the whole show with one graphics configuration. In every part the first
1.5 seconds (spawning the crowd, binding clips) are not measured; after that every frame's
time, GPU time (timestamp queries) and CPU time per engine stage are recorded. The results
screen shows, per part and pass, the average frame rate, the 1% low (the 99th percentile
frame) and the GPU milliseconds, and the whole show's average per pass. The full report
(also CPU stages, draw calls, skinned vertices) goes to the log as one line starting
`MARIONETTE_BENCHMARK` (JSON) and is kept in the save data
(`~/.local/share/wgpu-cube/saves/marionette/benchmark_last.txt` on Linux).

Over MCP: `lua_eval {entity: "Director", code: "RT.start('benchmark')"}` (or `'raster'`,
`'show'`), `RT.part(n)` jumps to a part, `RT.scale(n)` sets the crowd size and
`RT.status()` reports where the show is; `SHOW.report` holds the last results.

### Results on the reference machine

Ryzen 9 5950X (16 cores, 32 threads), Radeon RX 7900 XTX (24 GB), 1824x1080, TAA, crowd 1x, release
player, 2026-10-17, with nothing else running. Another program using the GPU or several cores
(a game on the other monitor) lowers these a lot: with one running, the whole show averaged
less than half of this, so compare runs made under the same conditions, and prefer the GPU
milliseconds (timestamp queries) and the CPU stage times in the report to the frame rates.

| Part | Characters | Raster: fps (1% low), GPU ms | Raster + FSR 50%: fps (1% low), GPU ms |
|---|---|---|---|
| Roll call | 47 | 346 (244), 1.3 | 332 (231), 1.4 |
| Dance floor | 196 | 206 (117), 3.0 | 201 (147), 2.5 |
| Marathon | 300 | 137 (82), 3.8 | 149 (117), 3.4 |
| Battle | 300 | 154 (124), 3.3 | 150 (115), 3.4 |
| Stadium | 1,040 | 61 (45), 9.2 | 62 (51), 8.8 |
| Curtain call | 1,051 | 63 (52), 9.2 | 62 (52), 8.9 |
| Whole show | | 168 | 166 |

The crowds are CPU-bound: in the stadium the frame spends about 6.3 ms submitting draws
(one per skinned mesh per pass: 2,933 meshes for 1,040 characters, 6,866 draws), 2.4 ms
propagating transforms, 2.7 ms collecting what to draw and 2.2 ms posing skeletons, while
the GPU needs 9 ms. That is also why FSR at half resolution changes nothing here: the GPU's
work is mostly geometry (skinning 14.5 million vertices, shadow maps, the depth prepass),
not pixels. Earlier the same part ran at 35 fps (GPU 12.2 ms): the engine changes made with
the show (parallel posing, one view for transform propagation, shared palettes, borrowed
meshes) took it from 15 to 29 fps with a game running alongside, and the pooled skinning
that followed (all skinned vertices in a few pool buffers, one compute dispatch per pool
page instead of one per mesh, passes binding a pool once and drawing instance after
instance by base vertex, transform propagation and palettes on the thread pool) took it
from 35 to 61 fps with nothing else running: submission 11.2 to 6.3 ms, transforms 5.2 to
2.4 ms, collecting 4.5 to 2.7 ms, posing 2.7 to 2.2 ms, GPU 12.3 to 9.2 ms.

## How it is built

- `content/scripts/director.lua`: loading, the title screen, the run loop over parts and
  passes, the camera, the sweeping spotlights and the HUD. Test hooks in `RT`.
- `content/scripts/show/core.lua`: the cast, a seeded random generator (the same show every
  run), spawning actors from the templates and playing clips on them.
- `content/scripts/show/part_*.lua`: one file per part, each with `enter`, `update`,
  `camera` (and `labels` for the roll call).
- `content/scripts/show/bench.lua`: the passes, the recorder and the JSON report.
- `content/scripts/show/hud.lua`: title, cards, statistics and the results table.

The arena (floor, track, ten tiers of stands matching `S.STANDS` in `core.lua`, neon
rings, four spotlights, the sun and the probe volume) is in the scene and was built over
MCP with `mesh_lathe` and `mesh_compose`.

Engine features it relies on: Lua `clone_entity` (a character with its skeleton and
animator in one call), `animator_add_clips`/`animator_play` on clones in the hook that
made them, `frame_stats()` for the measurements and `graphics_settings()` /
`set_graphics_settings(..., { save = false })` for the passes.
