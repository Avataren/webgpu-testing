# AI: behaviour trees and state machines

Entities with a mind of their own. An `AiAgent` component names the entity's **brain**: a
behaviour tree or a (hierarchical) state machine, written as Lua tables or JSON and shared
by every agent that runs it. Each agent keeps its own **blackboard** of named values, its
**senses** (what it sees, hears and remembers) and its place in the brain. Brains walk the
navmesh, play animation clips, call Lua, talk to each other with events, make noises and
react to what they perceive.

The code is in `src/ai/` (internals in [ARCHITECTURE.md](ARCHITECTURE.md#ai)); the Lua
functions are summed up in [LUA_SCRIPTING_README.md](../scripts/LUA_SCRIPTING_README.md#ai).
`games/lantern-market` is the showcase: townsfolk driven by their needs, vendors calling out,
a pickpocket chased by the guards, children playing tag, a performer drawing a crowd,
carts and a portcullis, all as brains.

## Quick start

**Lua.** A script (usually one "director" for the level) defines brains in `on_created`
and hands them out; functions inside brains run in that script's context.

```lua
function on_created(self)
    ai_tree("wanderer", { "sequence",
        { "wander", 10 },                          -- walk somewhere within 10 m
        { "play", "look-around", time = { 2, 4 } },
        { "wait", { 1, 3 } },
    })
    ai_machine("guard", {
        initial = "patrol",
        states = {
            patrol = {
                tree = { "sequence", { "patrol", "route" }, { "wait", 2 } },
                transitions = {
                    { "chase", event = "see:thief", store_from = "target" },
                    { "investigate", event = "hear:alarm", store_position = "noise_at" },
                },
            },
            chase = {
                tree = { "move_to", "target", run = true, radius = 1.0 },
                transitions = {
                    { "patrol", on = "success" },
                    { "investigate", event = "lose:thief", store_position = "noise_at" },
                },
            },
            investigate = {
                tree = { "sequence",
                    { "move_to", "noise_at", run = true },
                    { "play", "look-around", time = 4 } },
                transitions = { { "patrol", on = "done" } },
            },
        },
    })
    for i = 1, 5 do
        local e = clone_entity(find_entity_by_name("Template"), "Walker " .. i)
        set_component(e, "NavAgent", { speed = 1.4 })
        ai_set_brain(e, "wanderer", { tags = { "townsfolk" } })
    end
end
```

**Editor.** *Add component > AI Agent*, type the brain's name, set its tags and senses. In
play the section shows what it thinks: its state (machines), what its tree is doing, its
blackboard, the tree with each node's last result (running yellow, green, red), the
machine's transitions and recent history, and what it perceives. *Draw* toggles view cones,
goals, percepts and state rings in the Scene view.

**MCP.**

```text
entity_update {entity: "Guard", ai_agent: {brain: "guard", tags: ["guard"], perception: {sight_range: 14}}}
ai_define {name: "guard", brain: {...}}      # a brain from JSON, no Lua needed
ai_status {}                                 # every agent's brain, state and action
ai_inspect {entity: "Guard"}                 # its blackboard, tree / machine, senses
ai_send {entity: "Guard", event: "alarm"}    # or ai_noise {position, radius, tag}
ai_debug {all: true}                         # then screenshot
```

## The AiAgent component

Saved with the scene; `get_component` / `set_component(e, "AiAgent", {...})`,
`entity_update {ai_agent: {...}}` (only the fields given change; `null` removes it).

| Field | Default | |
|---|---|---|
| `brain` | `""` | A brain's name (defined by a script or `ai_define`), or a project JSON file (`content/ai/guard.json`, reloaded when it changes) |
| `enabled` | true | Off: the brain stops (its walk and clips stop, states run `exit`) |
| `tick_rate` | 10 | Thinks per second; agents are spread over frames. 0 thinks every frame |
| `tags` | `[]` | What others perceive it as (`guard`, `child`, `thief`) |
| `group` | `""` | Agents of a group share the `group.` blackboard and `to = "group"` events |
| `run_speed` | 3.2 | Speed (m/s) of moves with `run = true` |
| `blackboard` | `{}` | Starting values; `[x, y, z]` arrays become positions |
| `perception` | | See [Senses](#senses) |

Walking needs a `NavAgent` too (the brain sets its destinations and, for running, its
speed; an animation tree on the agent follows). Without one the brain still thinks, plays
clips, turns, sends events and perceives (vendors behind a counter, a gate, a cart moved by
Lua).

Brains run in play mode, after the scripts' `update` (so values and events scripts set this
frame count) and before the navigation (so walks start this frame). Brains defined by
scripts are gone when play stops; scripts define them again in `on_created`. Redefining a
brain (a script hot-reloaded, `ai_define` again) restarts the agents running it.

## Behaviour trees

A tree is a node; a node is a table whose first item is its type, followed by its children
(composites, decorators) or its main arguments (leaves), and named parameters. JSON uses
objects: `{"type": "sequence", "children": [...], "name": "..."}`. A bare Lua function is a
`lua` leaf. Mistakes (an unknown type or parameter, a decorator with two children, a bad
expression) are reported with their place: `guard > selector[2] > move_to: unknown
parameter(s) targt`.

Each tick walks down from the root to what is running. A node is *entered* the first time
it is ticked and *left* when it finishes or is halted (a guard aborting it, a timeout, its
parent moving on, its state being left). Leaves clean up when left: a walk stops, a clip
fades out, a Lua `exit` runs. A tree whose root finishes starts again next tick.

### Composites

| Type | |
|---|---|
| `sequence` | Children in order until one fails (then it fails); succeeds when all have |
| `selector` (`fallback`) | Children in order until one succeeds; fails when all have |
| `reactive_sequence` | A sequence that starts again from its first child every tick, so earlier conditions are re-checked and a running action behind them is halted when one fails |
| `reactive_selector` | A selector that starts from its first child every tick: a higher-priority child that now succeeds or runs halts the lower one |
| `parallel` | Ticks every child each tick. `success` (default `"all"`) and `failure` (default 1) are how many must succeed or fail (`"all"`, `"one"`, a count); the rest are halted |
| `shuffle_sequence`, `shuffle_selector` | In a random order, shuffled on entry (`shuffle_selector` favours children with a higher `weight`) |
| `random` | Runs one child picked by `weight` |
| `utility` | Scores its children (`score` on each child: a number, expression or function) and runs the best; if it fails, the next best (a selector by score). Negative scores leave a child out. With `interval`, it scores again while running and switches when another child beats the running one by `hysteresis` (0.1) |

### Decorators (one child)

| Type | Parameters | |
|---|---|---|
| `invert` | | Success and failure swap |
| `force_success`, `force_failure` | | Ends with that, whatever the child did |
| `repeat` | `count`, `until` | Runs the child again: `count` times, or for ever. Stops (failing) when the child fails, unless `until = "failure"` (then that is success) or `until = "success"` (repeat until it succeeds) |
| `retry` | `count` | Runs a failed child again, up to `count` more times |
| `timeout` | `time` | Fails (halting the child) when it runs longer |
| `delay` | `time` | Waits, then runs the child |
| `cooldown` | `time` | After the child ends, fails without running it for `time` |
| `once` | | The child runs once; afterwards the node repeats its result |
| `chance` | `p` | Runs the child with probability `p` (rolled on entry), else fails |
| `guard` | `when`, `abort` | Runs the child while `when` holds (see aborts below) |

The common ones are also parameters of any node, wrapped around it in this order: `when`
(with `abort`), `chance`, `cooldown`, `timeout`. So `{ "move_to", "spot", timeout = 20,
when = "not group.alarm", abort = "self" }` reads naturally.

### Guards and aborts

A guard is checked when its node is entered. With `abort` it keeps watching while the tree
runs (like Unreal's observer aborts):

- `abort = "self"`: while its own branch runs, the branch is halted (and fails) as soon as
  the condition no longer holds.
- `abort = "lower"`: while a later sibling of the selector (or `utility`) it is in runs,
  that sibling is halted as soon as the condition holds, and the guarded branch runs
  instead.
- `abort = "both"`: both.

```lua
{ "selector",
    { "sequence", when = "sees('thief')", abort = "both",   -- interrupts the patrol below
        { "move_to", "nearest('thief')", run = true } },
    { "patrol", "route" } }
```

### Other node parameters

`name` (shown in the inspector and `ai_action` instead of the generated label), `score`
and `weight` (read by a `utility`, `random` or `shuffle_selector` parent), `comment`
(ignored).

### Leaves

Numbers can be constants, ranges `{min, max}` (rolled when the node starts), expressions
or functions. *Targets* are expressions naming a position or an entity: a key holding
`{x, y, z}` or an entity, `nearest('thief')`, `self`, or a literal `{x, y, z}`; tables with
`entity` or `position` fields work too.

| Type (positional args) | Parameters | |
|---|---|---|
| `wait` (time) | | Runs for `time` seconds |
| `wait_until` (expr) | | Runs until the condition holds |
| `check` / `condition` (expr) | | Succeeds when the condition holds, else fails |
| `set` (key, value) | `expr` | Sets a blackboard key to a literal `value` or an `expr`'s result (nil clears) |
| `clear` (key) | | Removes a key |
| `add` (key, amount) | `min`, `max` | Adds to a number (missing counts as 0), clamped |
| `log` (message) | | Logs; `{key}` in the text shows a value (`{state}`, `{self}`) |
| `lua` / `action` (run) | `enter`, `exit` | Lua: `enter(ctx)` once, `run(ctx)` every tick, `exit(ctx)` when it ends or is halted (`ctx.result` says how). `run` returns `true` or nothing (success), `false` (failure) or `"running"`; `enter` returning a result ends it at once. `ctx.memory` is a table kept while it runs |
| `move_to` (target) | `radius`, `speed`, `run`, `repath`, `give_up`, `accept_partial` | Walks there. An entity target is followed (re-planned every `repath` s as it moves). With `radius` it succeeds (and stops) that close. Fails when there is no way, after `give_up` s, or after waiting 6 s at the end of a way that is shut for now (a closed gate); `accept_partial` succeeds this close to a blocked way's end instead |
| `wander` (radius) | `center`, `min_distance`, `speed`, `store` | Walks to a random reachable point within `radius` of `center` (else where it is), at least `min_distance` away |
| `flee` (from, distance) | `speed`, `run` (true) | Runs `distance` m away from a position or entity, keeping off the side it came from |
| `follow` (target, distance) | `speed`, `run` | Keeps within `distance` of an entity, for ever (combine with a guard, timeout or parallel) |
| `face` (target) | `yaw` (radians), `wait` | Turns to a target or a heading; `wait` gives it time to turn |
| `stop` | | Stops walking |
| `patrol` (points) | `index` (key, `patrol_index`), `radius`, `speed` | Walks to the next point of a list (positions or entities) and moves the index on (also past a point it cannot reach for now) |
| `pick_point` (store, radius) | `around`, `min_distance` | Puts a random navmesh point on the blackboard |
| `play` (clip) | `layer` (1), `time`, `loop`, `fade` (0.4), `speed`, `stop` (true) | Plays a clip on an animator layer (layer 0 is the locomotion tree's). With `time` it loops that long; without, a one-shot runs to its end; `loop = true` without `time` starts it and succeeds (it keeps playing). It fades out when left (`stop = false` keeps it) |
| `stop_animation` (layer) | `fade` | |
| `anim_param` (name, value), `anim_trigger` (name) | | Animation tree parameters |
| `send` (event, to) | `data` | An event to `self`, an entity (a key holding it), `"group"` or `"all"` |
| `broadcast` (event, radius) | `tag`, `data` | To agents within `radius` (with `tag`); without a radius, to all |
| `noise` (tag, radius) | `data`, `at` | A noise others hear (see [Senses](#senses)) |
| `emit` (event) | `data` | A script event (`subscribe_event` handlers get it) |
| `wait_event` (event) | `store`, `store_from`, `store_position` | Runs until the event arrives, then puts its data, sender and position on the blackboard |
| `find` (tag, store) | `range`, `visible`, `filter` | The nearest thing it perceives with the tag (seen now, or remembered unless `visible`) into `store`; `filter` is a condition where `it` is the candidate |
| `subtree` (tree) | | Runs another tree (by name) here |
| `succeed`, `fail`, `running` | | |

## State machines

```lua
ai_machine("vendor", {
    initial = "idle",
    states = {
        idle = { tree = {...}, transitions = { { "hawk", when = "count('townsfolk', 5) > 0", chance = 0.3 } } },
        hawk = { enter = function(ctx) ... end, tree = {...}, transitions = { { "idle", on = "done" } } },
        serve = { ... },
    },
    transitions = {   -- checked in every state, before the state's own
        { "serve", event = "customer", store_from = "customer", except = { "serve" } },
    },
})
```

A state can have:

- `enter(ctx)`, `exit(ctx)`: functions (or names) run on the way in and out; `ctx.event` is
  the event that caused the transition.
- `update(ctx)`: run every tick; returning a state's name goes there.
- `tree`: a tree (inline or a name) run while in the state, started again when it ends.
- `machine`: a nested machine (inline or a name). The agent's state is then a path like
  `work/serve`; `in_state('serve')` is true for any level.
- `set = { key = value }`: blackboard values set on entry.
- `final = true` (or `"failure"`): entering it ends the machine; the parent state's `on`
  transitions see it, and a root machine stays there.

Transitions (`{ "to", ... }` or `{ to = ..., ... }`), checked in order, the first that holds
is taken; every condition given must hold:

| Field | |
|---|---|
| `event` | This event arrived since the last tick (`see:thief`, `hear:alarm`, a `send`) |
| `when` | A condition (an expression or function; with `event`, `ctx.event` is it) |
| `after` | Seconds in the state (a range is rolled on entry) |
| `on` | The state's tree or machine ended: `"success"`, `"failure"` or `"done"` |
| `chance` | Taken only this often when the rest holds (rolled each time) |
| `store`, `store_from`, `store_position` | Put the event's data, sender, position on the blackboard |
| `except` | Machine-wide transitions: states they are not taken from |

`to` may be `"@previous"` (the state before this one) or `"@restart"` (this one again). A
machine takes at most 8 transitions a tick (more means its rules loop).

## Blackboards

Each agent has its own board; agents of a `group` share `group.` keys, and every agent sees
`global.` keys. In expressions and node parameters: `hunger`, `group.alarm`,
`global.night`. In Lua: `ctx.bb.hunger`, `ctx.group.alarm`, `ctx.global.night`, and from
any script `ai_get(e, "hunger")`, `ai_set(e, "group.alarm", true)`, `ai_blackboard(e)`,
`ai_group_board(name)`, `ai_global()` (index and assign them; `pairs` lists them).

Values are numbers, booleans, text, positions (`{x, y, z}` tables become positions),
entities (their ids, the numbers Lua holds), JSON data, and Lua tables and functions (kept
by reference: reading one back gives the same table).

## Expressions

Conditions, scores and values written as text: `hunger > 0.6 and not sees('guard')`.

- Names read the blackboard; `a.b` reads a field of a table value; `list[2]` an item;
  `self` is the agent and `it` the candidate in a `find` filter.
- `and`/`&&`, `or`/`||`, `not`/`!`, `== ~= != < <= > >=`, `..` (join as text), `+ - * / %`.
  `and`/`or` work as in Lua (`x or 3`).
- `dist(a[, b])` metres from the agent (or between two targets), `sees(tag)`,
  `heard(tag)`, `count(tag[, radius])`, `nearest(tag)`, `in_state(name)`, `time()`,
  `state_time()`, `random()`, `random(a, b)`, `chance(p)`, `abs`, `min`, `max`,
  `clamp(x, a, b)`, `floor`, `ceil`, `sqrt`, `exists(x)`, `len(x)`.

A missing key is nil; arithmetic on nil gives nil and ordering comparisons with it are
false, so `missing > 3` is simply false.

## Lua in brains

Any condition, score, value, `lua` leaf or state hook can be a function. It gets `ctx`:

| Field | |
|---|---|
| `agent` | The agent's entity |
| `bb`, `group`, `global` | Its blackboards |
| `dt`, `time` | Seconds since its last tick; the AI clock |
| `node`, `phase` | Who is calling (`run`, `enter`, `exit`, `condition`, `value`, `state_enter`, `state_update`, `state_exit`) |
| `state`, `state_time` | Its innermost state and how long it has been in it |
| `memory` | A `lua` leaf's table, kept while it runs |
| `event` | `{name, data, from, position}` that caused this state |
| `result` | In `exit`: `"success"`, `"failure"` or `"aborted"` |

Functions run with the full scripting API, in the context of the script that defined the
brain: its entity handles (clones it spawned resolve), its `get_self()`, its state. Brains
from JSON name functions instead: the agent's own script is searched first, then functions
registered with `ai_register(name, f)`, then globals.

Errors are logged once and kept on the agent (`ai_status(e).error`, the inspector,
`ai_inspect`); the node fails and the brain carries on.

## Senses

`perception` on the component:

| Field | Default | |
|---|---|---|
| `sight_range` | 0 | How far it sees (0: blind) |
| `sight_angle` | 120 | Field of view (degrees, the whole cone) |
| `near_range` | 1.5 | Closer than this it notices things whichever way it faces |
| `hearing_range` | 15 | How far it hears; a noise is heard within this and the noise's radius |
| `memory` | 5 | Seconds it remembers what it no longer sees or hears |
| `interests` | `[]` | Tags it pays attention to (empty: anything with a tag) |
| `line_of_sight` | `navmesh` | `navmesh` (a straight walk over the floor; walls block, other floors are out of sight), `physics` (a ray against colliders) or `none` |
| `eye_height` | 1.6 | For the line of sight |
| `rate` | 4 | Looks per second |

It sees other agents with tags. What changes becomes events: `see` and `see:<tag>` when
something comes into view, `lose` / `lose:<tag>` when it goes, `hear` / `hear:<tag>` for a
noise (`noise` leaves, `ai_noise`). Expressions ask `sees('thief')`, `heard('alarm')`,
`count('child', 5)`, `nearest('guard')`; `find` puts the nearest on the blackboard.

## Events

`send`, `broadcast`, `ai_send(e, name, data[, from])`, `ai_broadcast(name, data, {x, y, z, radius,
tag, group})` and the senses put events in agents' inboxes; each tick's events are what
transitions with `event` and `wait_event` leaves see. Events sent during a tick arrive for
the next one. Scripts on an agent get `on_ai_state(self, {from, to})` when its state
changes.

## Testing without a GPU

Brains are game logic: test them without rendering. A `Scene` steps scripts, AI and
navigation headless, so a test can spawn agents as plain entities (a `TransformComponent`,
an `AiAgent`, a `NavAgent`, and an empty `Animator` so clips "play") and run minutes of
game time in seconds:

- `src/ai/tests.rs`, module `in_scene`: brains defined in Lua and run by a scene; the
  market's real brain modules (`lantern_market_brains_define_and_run`).
- `lantern_market_simulation` (ignored by default): the whole of Lantern Market with its
  navmesh bake and every brain, reporting states entered, thefts, arrests, errors and
  agents stuck walking. See `games/lantern-market/README.md`.
- `ai_crowd_benchmark` (ignored): the AI step's cost for 500, 2,000 and 10,000 agents
  (a utility tree, timers, Lua hooks, senses and events). In release, single-threaded,
  about 0.4, 1.6 and 10 ms a frame: roughly a microsecond per agent per frame.

Never load-test the AI with skinned characters in a window: thousands of them exhaust the
GPU's memory long before the AI is busy.

## Debugging

- Inspector (play): state, action, blackboards, the tree with each node's last result,
  utility scores, the machine's transitions and recent history (with why each was taken),
  senses; Restart and Pause.
- Draw (inspector, `ai_debug`, `ai_debug_draw`): view cones, goals, percepts, state rings
  (each state name has its own colour). A selected agent always shows its own.
- `ai_status` / `ai_inspect` over MCP; `ai_state(e)`, `ai_action(e)`, `ai_status(e)`,
  `ai_senses(e)`, `ai_agents({tag, brain, state, group})` in Lua.
- Performance: `engine_stats` frame timings have an `ai` stage; `ai_status` reports agents
  ticked and the step's milliseconds. Lower `tick_rate` and `perception.rate` for crowds.
