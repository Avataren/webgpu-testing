# Navigation

Navmeshes, paths and crowds of agents that walk them. The navmesh is generated from the
scene's own geometry, so floors over floors (bridges, balconies, tunnels, an arcade under
a terrace) stay separate and agents walk both. Agents plan paths over it, steer round each
other and round moving obstacles, wait or reroute when the way closes, cross off-mesh links
(jumps, drops, ladders) and drive their animation trees. Everything is set up with
components, so it works the same from the editor's inspector, from Lua and over MCP.

The code is in `src/navigation/` behind the `navigation` cargo feature (on by default;
`--no-default-features` builds the engine without it). Internals are in
[ARCHITECTURE.md](ARCHITECTURE.md#navigation); the Lua functions are in
[LUA_SCRIPTING_README.md](../scripts/LUA_SCRIPTING_README.md#navigation).
`games/lantern-market` is the showcase: 90 townsfolk on a two-level market square with
bridges, carts and a portcullis.

## Quick start

**Editor.** Select any entity (an empty works) and add *Add component > Navigation >
NavMesh Surface*, then press **Bake** in its section. Turn on *Navmesh* in the section's
debug row to see it, and use *Test path* to click two points in the Scene view: the path
between them is drawn with its length. Add *Nav Agent* to characters, and drive them from a
script (`nav_set_destination`) or give them a *Destination* to head for when play starts.

**MCP.**

```text
entity_spawn {name: "NavMesh", nav_mesh_surface: {}}
navmesh_bake {}                        # waits for the bake, returns tiles and polygons
navigation_debug {mesh: true}          # then screenshot
navigation_path {from: [0, 0, 0], to: [12, 3.8, -14]}
entity_update {entity: "Guard", nav_agent: {speed: 1.4}}
```

**Lua.**

```lua
function on_created(self)
    nav_set_destination(self, 12, 0, -4)
end

function on_nav_arrived(self)
    local p = nav_random_point(0, 0, 0, 20)
    if p then nav_set_destination(self, p.x, p.y, p.z) end
end
```

## Components

All five are saved with the scene, edited in the inspector's *Navigation* menu, read and
patched with Lua `get_component` / `set_component` (`"NavAgent"` or `"nav_agent"`) and over
MCP (`entity_update {nav_agent: {...}}`; `null` removes one). Only the fields given change.

### NavMeshSurface

One navmesh, built from the static geometry around it.

| Field | Default | |
|---|---|---|
| `agent_radius` | 0.4 | Walls and drops keep this far from agents' centres (the mesh is shrunk by it) |
| `agent_height` | 1.8 | Lower ceilings are not walkable |
| `agent_max_climb` | 0.45 | Tallest step walked up or down |
| `agent_max_slope` | 45 | Steepest walkable slope (degrees) |
| `cell_size`, `cell_height` | 0.2, 0.1 | Voxel size: smaller follows walls more closely and builds slower |
| `tile_size` | 48 | Tile edge in voxels; obstacles rebuild the tiles they touch, so smaller tiles update faster |
| `min_region_area`, `merge_region_area` | 1, 8 | Islands smaller than the first (m²) are dropped; regions smaller than the second merge |
| `max_edge_length`, `max_edge_error` | 12, 1.3 | Wall simplification (m, voxels) |
| `detail_sample_distance`, `detail_sample_max_error` | 1.2, 0.1 | Height detail, so the mesh follows ramps and stairs (0 turns it off) |
| `geometry` | `render_meshes` | `render_meshes`, `colliders` (fixed physics colliders) or `both` |
| `collect` | `all` | `all` (the whole scene) or `children` (the surface entity's descendants) |
| `bounds` | `[0, 0, 0]` | A box around the entity to build in (zero: everything collected) |
| `baked` | none | Project path of the bake; set by baking |
| `build_on_start` | true | Without a bake, build when play starts |
| `agent_type` | `""` | Agents with the same type walk this navmesh |

Static visible meshes count. Skinned meshes, dynamic and kinematic bodies, agents,
obstacles and invisible entities never do.

### NavModifier

On a piece of scenery (with `include_children` for a whole imported model): `ignore` leaves
it out entirely (foliage, lantern heads), `walkable: false` keeps it solid but never walked
on (roofs, awnings, water: anything an agent must not stand on, even when it is low enough
to step onto), and `area` gives its walkable tops an area id (1..63, 63 is plain ground) that
agents can price.

### NavAgent

| Field | Default | |
|---|---|---|
| `radius`, `height` | 0.35, 1.8 | Body size for avoidance |
| `speed`, `acceleration` | 1.5, 8 | m/s and m/s² |
| `turn_speed` | 540 | Degrees per second when turning to face its movement |
| `stopping_distance` | 0.15 | Arrive this close |
| `avoidance` | true | Steer round other agents and moving obstacles |
| `priority` | 0.5 | Higher-priority agents expect others to make way (0..1) |
| `separation` | 1 | How strongly it keeps its distance from neighbours |
| `face_movement`, `yaw_offset` | true, 0 | Turn toward its movement; offset for models not facing +Z |
| `drive` | `auto` | `transform` (moved along the navmesh: cheap, for crowds), `character` (steers its `CharacterController`: gravity, pushing, colliders) or `auto` (character when it has one) |
| `agent_type` | `""` | Which navmesh it walks |
| `destination` | none | Where it heads when play starts |
| `area_costs` | `[]` | `[{area, cost}]`: multiplies the distance walked there; 0 or less keeps it out |

An agent with an `AnimationTree` (on it or below it) feeds it `speed`, `forward_speed`,
`side_speed` and `grounded`, and fires `jumped` / `landed` on off-mesh links, so the
locomotion tree that `animation_tree_generate` (or *Generate from clips*) builds plays walks
and runs at the speeds agents move.

### NavObstacle

Something that moves and gets in the way (a cart, a door, a portcullis): a `box` (`size`) or
`cylinder` (`radius`, `height`) footprint at `offset`. While it moves, agents steer round it.
With `carve` on, it also cuts a hole into the navmesh, so paths go round it; with
`carve_only_stationary` (the default) only once it has stood still for `stationary_time`
seconds, so a rolling cart does not rebuild tiles every frame. A closed gate that carves a
bridge shut makes agents heading over it replan over another one.

### NavLink

An off-mesh connection from the entity to `end` (local units): a jump down off a stage, a
gap to leap, a ladder. `bidirectional` makes it crossable both ways; `radius` is how far
from either end it reaches for the floor; `area` lets agents price it. Agents cross it on an
arc and scripts get `on_nav_link_start` / `on_nav_link_end`.

## Baking

A bake builds every tile on the thread pool and saves `content/navigation/<surface
name>.navmesh`, setting the surface's `baked` path (save the project to keep it). The
Lantern Market's 90 tiles (about 15,000 triangles, two floors) bake in about 18 ms. Bakes
load once per file and are shared by play sessions. When the surface's settings change
after a bake, its status says *stale*: bake again. A surface without a bake builds when play
starts (`build_on_start`), which suits small scenes and procedurally built levels;
`nav_rebuild()` builds again from the scene as it is now.

## How agents behave

- **Paths**: A* over the polygons, then a funnel for the corners, then a pass that cuts
  corners the floor lets it cut. A goal that cannot be reached (a closed gate, an island)
  gives a *partial* path to the nearest point: the agent walks there and keeps retrying, so
  it moves on when the way opens.
- **Steering**: each agent looks a few corners ahead and steers smoothly round them,
  slowing for the last two body radii.
- **Avoidance**: reciprocal velocity obstacles (ORCA, a port of RVO2) against neighbours,
  moving obstacles and the navmesh's walls. Standing agents leave the avoiding to those
  walking; `priority` shifts the share between walkers.
- **Keeping to a side**: meeting someone, agents lean to the same side, as people do on a
  path, so head-on meetings pass and lanes form where crowds cross on a narrow bridge.
  An agent stuck face to face with oncoming agents for a second steps back and aside for a
  moment, so two pairs walking abreast into each other fall into single file.
- **Stuck**: an agent that wants to move but makes no progress for 2.5 s plans again. One
  stuck within two radii of its destination (someone stands on the spot) has arrived.
- **Obstacles**: carving rebuilds the touched tiles from a cache of their voxels, at most
  six a frame; agents whose corridor crossed a rebuilt tile replan.
- **Steered by a player**: `nav_steer(entity, vx, vz[, yaw])` drives an agent by velocity
  instead of toward a destination (send it every frame). It keeps to the navmesh (walls and
  edges stop it, stairs and ramps carry it), it avoids nobody, and everyone else makes way
  for it, standing or walking. `engine.third_person` drives an agent this way with
  `drive = "navigation"`, which is how a game hands an AI character to the player for a
  while (Lantern Market's possession). `nav_stop` or a destination ends it.

States: `idle`, `planning`, `moving`, `arrived`, `failed` (no path at all), `off_mesh`
(crossing a link), `lost` (off every navmesh); a steered agent is `moving`, with `steering`
set in `nav_agent_state`. Scripts on an agent get `on_nav_arrived(self)`,
`on_nav_failed(self, reason)`, `on_nav_link_start(self, {start, finish, link})` and
`on_nav_link_end(self)`.

## Seeing it

The debug view draws in the Scene view, the game view and screenshots, depth tested:

- *Navmesh*: polygons tinted by area, walls dark, tiles checkered (a selected surface always
  shows its own).
- *Agents*: bodies coloured by state (grey idle, green walking, blue arrived, red failed)
  with velocity arrows.
- *Paths*: each agent's corners to its destination.
- *Obstacles*: red while carving, orange while moving.
- *X-ray*: what scenery hides, faintly.

Toggle them in a surface's inspector section, with Lua `nav_debug_draw{mesh = true, ...}` or
MCP `navigation_debug`. The inspector's *Test path* and MCP `navigation_path` draw a path
between two points (magenta, red when it stops short).

## Remote control

MCP tools: `navmesh_bake` (bakes and waits), `navigation_status`, `navigation_debug`,
`navigation_path`, `navigation_agents`. Engine methods (`engine_call`): `navigation.status`,
`navigation.bake`, `navigation.debug`, `navigation.path`, `navigation.closest`,
`navigation.agents`. The components go through `entity_update` / `entity_get` like any
other.

## Designing for crowds

- Leave room: the navmesh stops `agent_radius` short of walls, so a 1.8 m bridge leaves a
  1 m lane, enough for two people to pass but not for a crowd going both ways at once to
  stream. Keep props out of the main ways (a crate beside a stage corner turned the
  market's road to the footbridge into a single-file neck that jammed).
- Do not put spots where people stand about in a narrow way or a bridge's mouth.
- Mark everything nobody walks on (water, roofs, awnings, counters) with a `NavModifier`
  `walkable: false`, or agents spawned at random points can start on it.
- `nav_random_point(x, y, z, radius)` searches outward from the polygon nearest the point:
  start it on the main floor, not on a fountain or a rooftop island.

## Limits

- Agents larger than the surface's `agent_radius` can brush walls: give them a navmesh of
  their own (`agent_type`).
- Moving obstacles are avoided, not carved; carving waits until they stand still.
- Off-mesh links are crossed on a scripted arc, not by physics.
- Up to 63 area ids per navmesh.
