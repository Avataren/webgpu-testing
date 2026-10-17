# Lantern Market

A market square at dusk with ninety people going about their business on the navmesh:
the engine's navigation showcase (`docs/navigation.md`). The level has floors over floors,
handcarts and a portcullis that get in the way, and lanterns coming on.

- **The square**: six stalls, a fountain with benches, a stage with steps at the back.
  Children run up the steps and jump off the front (off-mesh `NavLink`s).
- **The arcade and the terrace**: shops under a colonnade, with a terrace on top of it
  reached by a grand stair on the west and a ramp on the east. People walk both floors,
  one above the other.
- **The canal**: towpaths down by the water pass under a stone bridge and a wooden
  footbridge. Stairs lead down from the square and from the south bank.
- **What moves**: two handcarts on their rounds. People steer round a rolling cart, and a
  parked one cuts a hole into the navmesh so paths go round it. The stone bridge's
  portcullis closes now and then; it carves the bridge shut, and people heading south
  reroute over the footbridge.
- **Who is there**: 80 townsfolk wandering between stalls, the fountain, the benches, the
  terrace, the shops, the bridges, the towpaths and the south bank, doing something at each
  (talking, pointing, drinking, clapping); 6 children; 3 guards patrolling both floors and
  both bridges; a performer dancing on the stage with an audience; two cart pullers.
  Everyone walks and runs with an animation tree generated from their clips and fed by
  their agent.

## Getting the cast

The characters and clips come from the asset library (`docs/asset_library.md`), not from
this repository: Mixamo's licence forbids redistributing its files, so they are copied in
locally and git-ignored.

```sh
tools/library/library.py fetch-pack all        # Quaternius CC0 clips and mannequins
tools/library/library.py fetch-mixamo          # Mixamo characters and clips (your token)
games/lantern-market/tools/fetch_cast.py       # copy the cast into content/
```

`cast.json` lists the characters with their roles and the clips by library id;
`fetch_cast.py --check` reports what the library lacks. Without a cast, the market says how
to get one.

## Running it

```sh
cargo run --release --all-features --bin player -- games/lantern-market
```

or open the project in the editor and press Play. Loading imports each character once;
the crowd are copies of those templates.

| Key | |
|---|---|
| Tab, 1-8 | next camera shot, or a shot: the square, under the arcade, the towpath, from the terrace, the stage, the gate, following someone, overhead |
| F | follow someone walking |
| Click | call a few people over to that spot (any floor) |
| G | open or close the gate now |
| N, P, X | draw the navmesh, the agents' paths, x-ray |
| H | hide the HUD |

Over MCP: `lua_eval {entity: "Director", code: "RT.status()"}`; `RT.gate()`,
`RT.summon(x, y, z, n)`, `RT.shot(n)` and `RT.debug(true)` are the other hooks, and
`MARKET` holds the state.

## How it is built

- `tools/build_level.py` builds the level in Blender (headless works, or through the
  Blender MCP) and writes `content/models/market.glb`: static geometry joined per material,
  plus the carts, the gate and the lanterns as their own objects. The scene imports it,
  gives the materials generated textures (`content/textures/`, from `texture_generate`)
  and adds the navigation: a `NavMeshSurface` (baked to `content/navigation/navmesh.navmesh`),
  `NavModifier`s on what nobody walks on (water, roofs, awnings, foliage, lantern heads),
  `NavObstacle`s on the carts and the gate, and three `NavLink`s off the stage.
- Lighting: a dusk sky (`qwantani_dusk_2`, Poly Haven, CC0), a low sun, glowing lanterns
  and windows, four lantern point lights (two shadowed under the arcade, two by the gate),
  and a light probe volume in the arcade so it takes the bounced light of the square
  instead of sky through the terrace (baked to `content/lighting/arcade_probes.probes` for
  the web and GPUs without ray tracing).
- `content/scripts/director.lua`: loading the cast, spawning, the run loop and input.
- `content/scripts/market/`: `core.lua` (the cast, clips, helpers), `places.lua` (the spots
  people go and what they do there, from the level's numbers), `people.lua` (roles and
  their choices), `props.lua` (the carts' rounds and the gate), `camera.lua` (the shots) and
  `hud.lua`.

People are `NavAgent`s with `drive: transform` (the crowd moves them along the navmesh;
no physics), spawned with `clone_entity` from templates, given their clips with
`animator_add_clips` and a locomotion tree with `animation_tree_generate`. Gestures play
on animator layer 1 over the tree. With the whole cast walking, navigation costs about
0.3 ms a frame.
