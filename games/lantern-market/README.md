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
| F | follow someone walking (their state, what they are doing and their needs show); the camera rides a spring arm that pulls in before walls; right-drag to look around, the wheel zooms |
| Return | take over the person followed, and let them go again: their brain sleeps and you steer them with World of Warcraft style controls (W/S walk, A/D turn, Q/E sidestep, Shift walks slowly, left-drag looks, right-drag steers, both buttons run) over the navmesh, where the crowd makes way; the camera stays on them until you let go, and their brain starts afresh from there |
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
now), `RT.tag()` and `RT.possess()` (take over the person followed, or let go) are the
other hooks, and `MARKET` holds the director's state.

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
- The scene keeps the level's meshes embedded, so after rebuilding market.glb run
  `tools/sync_level_meshes.py` (with the editor closed): it converts the GLB as the
  engine's importer does and writes only the meshes that changed into the scene and the
  `content/gltf/market` package (`--check` only reports them). Then re-bake the light
  probes and the navmesh in the editor.
- Physics is only there for the camera: mesh `Collider`s on the buildings, roofs, awnings,
  bridges, ground, trees, the gate and the carts (not on thin or small things: iron posts
  and railings, fruit, lanterns, foliage), which `GameCamera`'s `SpringCamera` sweeps
  against in the follow shots.
- Lighting: the engine's procedural sky with its day and night (play starts at noon on
  2026-09-26, a full moon, at 48°N; a day takes three minutes): the Sun follows it as the
  sun and then the moon, nights are dark, so the glowing lanterns, the twenty lantern point
  lights (shadowed through the shadow atlas) and the windows carry them. Auto exposure
  goes down to EV -4. The light probe volumes are baked to `content/lighting/` at a dozen
  hours of the day for the web and GPUs without ray tracing (`light_probes_bake
  {times_of_day: "auto"}`). The dusk HDR sky (`qwantani_dusk_2`, Poly Haven, CC0) is still
  in the environment, used when the procedural sky is turned off.
- Fog: the environment's height fog lies low (base height -0.5 m, thinning to 37% every
  3 m, density 0.025): a light haze over the plaza by day, mist filling the canal, and at
  night halos round the lanterns, which light it through their shadow maps (the graphics
  setting `volumetric_fog`; off keeps the haze without the halos).
- The windows look into rooms (the engine's interior mapping, `interior.wgsl`): each of
  the 141 windows' panes (1.2 x 1.6 m, the rooms' proportions) is a uv cell of its own
  in `Market_WindowGlow`, and the material traces a room behind it from
  `content/textures/interiors.png`, eight townhouse rooms (a kitchen, a bedroom, a
  workshop, a hall, a study, a storeroom, a tavern room, a parlour) that each window picks
  from, mirrors, lights or leaves dark, and curtains, behind leaded glass
  (`window_lead_*.png`). `tools/build_interiors.py` models and renders the rooms and draws
  the glazing in Blender (headless works, about a minute). `tools/build_level.py` leaves
  out windows that the terrace, a stair or a neighbouring house would cover.
- `content/scripts/director.lua`: loading the cast, defining the brains, spawning, the
  keys and the test hooks.
- `content/scripts/market/brains/`: one module per role (`townsfolk.lua`, `vendor.lua`,
  `guard.lua`, `thief.lua`, `child.lua`, `performer.lua`), each defining its brain with
  `ai_machine` and the Lua functions it calls.
- `content/scripts/market/`: `core.lua` (the cast, clips, news, helpers), `places.lua` (the
  spots people go and what they do there, vendor posts, hideouts, from the level's
  numbers), `people.lua` (spawning each role with its brain, senses and blackboard; the
  cast size), `props.lua` (the carts and the gate as state machines), `camera.lua` (the
  shots; the follow shots hand the spring arm its target), `possess.lua` (taking someone
  over: their brain paused, `engine.third_person` driving their NavAgent with `nav_steer`)
  and `hud.lua`.

People are `NavAgent`s with `drive: transform` (the crowd moves them along the navmesh; no
physics) and `AiAgent`s, spawned with `clone_entity` from templates, given their clips with
`animator_add_clips` and a locomotion tree with `animation_tree_generate`. Vendors have no
NavAgent: they never walk.
