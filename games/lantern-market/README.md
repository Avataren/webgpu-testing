# Lantern Market

A market square at dusk with ninety people going about their business: the showcase of the
engine's navigation (`docs/navigation.md`) and AI (`docs/ai.md`). Everyone, and every cart
and the portcullis, has a brain: a state machine or behaviour tree that decides what they
do, what they notice and how they react to each other.

## The place

- **The square**: six stalls, a fountain with benches, a stage with steps at the back.
  Children run up the steps and jump off the front (off-mesh `NavLink`s).
- **The arcade and the terrace**: shops under a colonnade, with a terrace on top of it
  reached by a grand stair on the west and a ramp on the east. People walk both floors,
  one above the other.
- **The canal**: towpaths down by the water pass under a stone bridge and a wooden
  footbridge. Stairs lead down from the square and from the south bank.
- **What moves**: two handcarts on their rounds. People steer round a rolling cart (and
  step out of its way), and a parked one cuts a hole into the navmesh so paths go round it.
  The stone bridge's portcullis closes now and then with a rattle people turn to; it
  carves the bridge shut, and people heading south reroute over the footbridge.

## Who is there and what they do

- **72 townsfolk** live by their needs: hunger, thirst, rest, shopping, curiosity and
  company grow over time, and a utility node picks what answers the most pressing one: a
  meal at a stall, a drink at the fountain, a sit on a bench, the shops, the sights (the
  terrace, the bridges, the towpaths, the south bank), the show when it is on, a chat, a
  stroll. They invite someone nearby to talk and walk to meet them, turn when someone
  cries "thief!", answer a vendor's call when they fancy a purchase, come over when you
  click, and step aside for a rolling cart.
- **6 vendors** stand behind their counters: they idle, call out to passers-by (a wave and
  a call that tempts nearby townsfolk) and serve whoever comes to their stall.
- **A pickpocket** cases the busy places, stalks someone standing at a stall or a shop
  when no guard is near, lifts their purse and runs: away from the victim, then to a
  hideout under a bridge or at the end of the arcade, dodging guards. The victim cries
  out; he lies low until the hue and cry dies down.
- **3 guards** patrol both floors and both bridges and salute each other in passing. A cry
  of "thief!" sends them to look; one who sees the wanted thief gives chase and calls the
  rest of the watch; losing sight of him they search where he was last seen; catching him
  they arrest him, and he is walked off over the canal.
- **6 children** run about, climb the stage and jump off, watch the show, and every minute
  or so play tag: whoever is it counts, then chases the nearest child; the others run when
  it comes close.
- **A performer** dances on the stage (the show draws the curious), bows, takes a drink at
  the fountain and comes back.

Everyone walks and runs with an animation tree generated from their clips and fed by
their agent; what they do where they stop plays over it on another animator layer.

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
the crowd are copies of those templates. Ninety skinned characters are heavy on the GPU:
for a lighter run, create `content/scripts/market/cast_size_local.lua` (git-ignored), for
example `return { townsfolk = 12, vendors = 2, guards = 2, children = 3 }`.

| Key | |
|---|---|
| Tab, 1-9 | next camera shot, or a shot: the square, under the arcade, the towpath, from the terrace, the stage, the gate, following someone, overhead, the chase (the camera cuts to it when a purse is lifted) |
| F | follow someone walking (their state, what they are doing and their needs show) |
| B | what everyone is doing, over their heads |
| V | the AI's view: view cones, where people are going, a ring in each state's colour |
| T | start a game of tag |
| Click | call a few people over to that spot (any floor) |
| G | open or close the gate now |
| N, P, X | draw the navmesh, the agents' paths, x-ray |
| H | hide the HUD |

Over MCP: `ai_status` (everyone's brain, state and action), `ai_inspect {entity}` (one
person's blackboard, tree or machine and senses), `ai_debug {all: true}`, and
`lua_eval {entity: "Director", code: "RT.status()"}`; `RT.gate()`, `RT.summon(x, y, z, n)`,
`RT.shot(n)`, `RT.debug(true)`, `RT.ai(true)`, `RT.theft()` (the thief looks for a purse
now) and `RT.tag()` are the other hooks, and `MARKET` holds the director's state.

## Without a GPU

The whole market can be simulated headless, without rendering anything: the real navmesh
bake, `people.lua` and every brain, with stand-in characters (no meshes, no skinning).
It reports how often each state was entered, thefts and arrests, games of tag, shows,
errors and anyone stuck walking:

```sh
cargo test --release --lib lantern_market_simulation -- --ignored --nocapture
MARKET_SIM_SIZE="townsfolk=10,vendors=2,guards=3,children=0,thief=1,performer=0" \
    cargo test --release --lib lantern_market_simulation -- --ignored --nocapture
```

Obstacles do not carve there (a carved tile is rebuilt from the level's meshes, which
the headless scene does not load). A run of 180 simulated seconds takes about 4 s; the AI
costs about 0.3 ms a frame for the ninety.

## How it is built

- `tools/build_level.py` builds the level in Blender (headless works, or through the
  Blender MCP) and writes `content/models/market.glb`: static geometry joined per material,
  plus the carts, the gate and the lanterns as their own objects. The scene imports it,
  gives the materials generated textures (`content/textures/`, from `texture_generate`)
  and adds the navigation: a `NavMeshSurface` (baked to `content/navigation/navmesh.navmesh`),
  `NavModifier`s on what nobody walks on (water, roofs, awnings, foliage, lantern heads),
  `NavObstacle`s on the carts and the gate, and three `NavLink`s off the stage.
- Lighting: a dusk sky (`qwantani_dusk_2`, Poly Haven, CC0), a low sun, glowing lanterns
  and windows, twenty lantern point lights (shadowed through the shadow atlas), and light
  probe volumes baked to `content/lighting/` for the web and GPUs without ray tracing.
- `content/scripts/director.lua`: loading the cast, defining the brains, spawning, the
  keys and the test hooks.
- `content/scripts/market/brains/`: one module per role (`townsfolk.lua`, `vendor.lua`,
  `guard.lua`, `thief.lua`, `child.lua`, `performer.lua`), each defining its brain with
  `ai_machine` and the Lua functions it calls.
- `content/scripts/market/`: `core.lua` (the cast, clips, news, helpers), `places.lua` (the
  spots people go and what they do there, vendor posts, hideouts, from the level's
  numbers), `people.lua` (spawning each role with its brain, senses and blackboard; the
  cast size), `props.lua` (the carts and the gate as state machines), `camera.lua` (the
  shots) and `hud.lua`.

People are `NavAgent`s with `drive: transform` (the crowd moves them along the navmesh; no
physics) and `AiAgent`s, spawned with `clone_entity` from templates, given their clips with
`animator_add_clips` and a locomotion tree with `animation_tree_generate`. Vendors have no
NavAgent: they never walk.
