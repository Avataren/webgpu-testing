# Characters: your own model, its animations and its controls

This guide takes a character from Mixamo (or any rigged model) to something you can run
around with: import, animations, the animation tree, controls and camera. It follows
Godot's way of putting a character together, so the pieces map one to one:

| Godot | This engine | What it does |
| --- | --- | --- |
| `CharacterBody3D` + `CollisionShape3D` | `CharacterController` | A capsule that walks, climbs steps, slides off steep slopes, jumps |
| `AnimationPlayer` | `Animator` | Plays and cross-fades clips from glTF/GLB files on the skeleton |
| `AnimationTree` (state machine, `BlendSpace1D/2D`) | `AnimationTree` | Picks the animation from parameters: speed, grounded, your own |
| `SpringArm3D` + `Camera3D` | `SpringCamera` on a camera | Third-person camera that follows and pulls in at walls |
| A script on the body | A Lua script (`require("engine.third_person")`) | Turns keys and mouse into movement |

The character is an entity with the `CharacterController` (and `AnimationTree` and
script); the imported model is its child, with the `Animator` on the model's skeleton.
The script only moves the character. The tree sees how it moves (speed, falling, landing)
and plays the clips, so scripts never name a clip to walk.

## The quickest start: the asset library

The engine's asset library (`docs/asset_library.md`) holds characters and animations ready
to use: the editor's **Library** tab (next to Assets) shows them with thumbnails. Pick a
character and **Add to scene**, select it and **Make Character**, then select the new
`Player` and add animations from the library (**Add to Player**, or double-click):
idle, walk, run, jump... Its Animation Tree section's **Generate from clips** builds the
locomotion. The clips can come from another rig than the character's (Mixamo, Unreal
mannequin or Rigify names): they bind by humanoid role.

## Quick start: a Mixamo character

You need Blender installed (from blender.org, Steam, a package manager or flatpak). The
editor converts FBX files with it in the background; set `BLENDER_PATH` to its executable
if it is somewhere unusual.

### 1. Download from Mixamo

1. Pick a character and download it: **FBX Binary**, **With Skin**, no animation
   (T-pose) or with one of its animations.
2. For each animation you want, keep the same character selected, pick the animation and
   download **FBX Binary, Without Skin**. Name the files after what they are: the clip is
   called after the file (Mixamo calls every clip `mixamo.com`), and the tree generator
   finds clips by name.

   | Download | Save as | Becomes |
   | --- | --- | --- |
   | Idle (any idle) | `Idle.fbx` | standing still |
   | Walking | `Walking.fbx` | walk blend point |
   | Running | `Running.fbx` | run blend point |
   | Walking Backwards | `Walking Backwards.fbx` | backpedal (else the walk plays backwards) |
   | Left Strafe / Right Strafe | `Left Strafe.fbx`, `Right Strafe.fbx` | sideways (one side is enough: it is mirrored) |
   | Jump | `Jump.fbx` | jump state |
   | Falling Idle | `Falling Idle.fbx` | fall state |
   | Landing (Falling To Landing) | `Landing.fbx` | hard-landing state |

   Leave **In Place** unticked for walks, runs and strafes: the engine measures how fast
   each clip travels and blends them at those speeds, so the feet do not slide, and plays
   them in place itself.

### 2. Import

1. **File > Import Model...** and pick the character's FBX. A few seconds later it is in
   the scene as `<name> (Instance)`, with an `Animator` on its skeleton.
2. Select the `(Instance)` entity. In the inspector's **Animator** section, click **Add
   clips...** and pick all the animation FBX files at once. They are converted, copied to
   `content/animations/` and added to the animator; the **Clips** table lists them with
   their length, how fast they travel and how many bones they drive (a warning colour
   means the skeleton lacks some of the clip's bones).

   (**File > Import Animations...** imports them without adding them to a character;
   **Add clips...** can then pick them from `content/animations/`.)

### 3. Make it a character

1. Give the floor a collider, or the character falls through it: select the floor, **Add
   Component > Physics > Collider** (new projects made over MCP have one already).
2. Select the model's `(Instance)` entity again and click **Make Character** in its
   **Character** box (leave **Third-person controls** ticked).
3. Press Play. W/S move, A/D turn, Q/E strafe, hold the left mouse button to look around,
   the right one to steer, both to run forward, Space jumps, Shift walks, the wheel zooms.

Make Character:

- puts the model under a new entity (called `Player`, or `<model> Character`) standing at
  its feet, with a `CharacterController` sized from the model (height, a radius of about a
  sixth of it, speeds and steps scaled for the size);
- turns the model to face +Z if its toes point the other way (characters walk towards
  +Z);
- adds an `AnimationTree` generated from the clips (see below);
- has the animator play travelling clips in place (the controller moves the character);
- gives the active camera a `SpringCamera` following the character (or makes a camera);
- writes `content/scripts/player.lua` with the engine's third-person controls and
  attaches it.

Everything it made is ordinary components: change them in the inspector.

## The AnimationTree

Select the character to see its **Animation Tree** section. **Generate from clips**
builds this from the animator's clips by name (`Locomotion`, then whichever of the air
states the clips allow):

- **Locomotion**: a 2D blend space over `side_speed` (x) and `forward_speed` (y). Idle at
  (0, 0), the walk and run straight ahead at the speeds they travel (1.4 and 4.6 m/s for
  clips made in place), the backpedal behind, the strafes to the sides. Between points the
  clips mix by weight and keep step (the idle keeps its own pace); slower than the walk,
  the walk slows down to 60% of its pace before the idle blends in; past the outermost
  points they play faster ("Match speed"). The speeds are measured at the character's
  size when the tree is generated: a copy scaled bigger or smaller (a crowd of clones)
  scales them with it.
- **Jump** when `jumped` fires, **Fall** while in the air, **Land** on a hard landing
  (`landed && landing_speed > 8`), and back to Locomotion when grounded.

The character's controller feeds the parameters: `speed`, `forward_speed`, `side_speed`
(m/s along its facing), `vertical_speed`, `grounded` (0 or 1), `air_time`,
`landing_speed`, and the triggers `jumped` and `landed`. In play mode the section shows
the current state and every parameter live.

### Adding your own states

Say the character has a `Samba Dancing` clip and should dance on a key press:

1. In **States**, click **+ State**, name it `Dance`, motion `clip`, clip `Samba Dancing`.
2. In **Transitions**, add `Locomotion` to `Dance` when `dancing`, and `Dance` to
   `Locomotion` when `!dancing || speed > 0.5`.
3. In the character's script:

   ```lua
   function update(self, dt)
       controls:update(dt)
       if is_key_just_pressed("G") then
           animation_set(self, "dancing", animation_get(self, "dancing") == 0)
       end
   end
   ```

Conditions combine parameters and numbers with `< <= > >= == !=`, `&& || !` (or `and or
not`); an empty condition always holds. Transitions are checked in order and the first one
that holds is taken; `after` waits until the state's motion has played that far (0..1,
e.g. an attack that returns to Locomotion `after 0.9`). From Lua:

- `animation_set(entity, name, value)`: a number or true/false
- `animation_trigger(entity, name)`: true for one evaluation (`attack`, `hit`)
- `animation_get(entity, name)`, `animation_state(entity)` (`{state, previous, time,
  progress}`)
- `animation_travel(entity, state[, fade])`: go to a state now

The tree plays on layer 0. Layers above it override chosen bones, for an upper-body
action over the legs' walk:

```lua
function on_created(self)
    animator_set_layer(self, 1, { weight = 1, mask = "Spine" })
end
function on_attack(self)
    animator_play(self, "Punch", { layer = 1, loop = false })
end
```

`find_bone(self, "RightHand")` returns the hand bone to parent a sword to.

## Controls

The script Make Character writes is a few lines around the engine's controller:

```lua
local ThirdPerson = require("engine.third_person")
local controls

function on_created(self)
    controls = ThirdPerson.new(self, { camera = "GameCamera", scheme = "mmo" })
end
function update(self, dt) controls:update(dt) end
```

`scheme = "action"` moves relative to the camera instead (WASD, click to capture the
mouse). Options (keys, turn rate, mouse sensitivity, whether the camera swings back behind)
and the methods a game can replace are in the Lua guide, "Third-person controller". How the
body moves (walk and run speed, jump height, slopes, steps, double jumps) belongs to the
`CharacterController` in the inspector.

## Other models

Any rigged glTF/GLB, FBX or .blend works the same way. Clips bind to bones by name
(without case or a `mixamorig:` style prefix) and, when a name is missing, by humanoid
role: Mixamo (`LeftArm`), Unreal mannequin (`upperarm_l`) and Rigify (`DEF-upper_arm.L`)
names all mean the same upper arm. Clips exported from another program or rig than the
model are retargeted: each bone turns away from its rest pose as it does in the clip,
measured in the character's own frame (a model facing -Z in its file takes clips from one
facing +Z). That needs both skeletons to rest in the same posture (both in T-pose); a
T-pose clip on an A-pose model bends the arms.

## Over MCP

```
asset_import {path: ".../Ninja.fbx"}                          # the model (Blender converts it)
asset_import {path: ".../Running.fbx", entity: "Ninja (Instance)"}   # clips onto its animator
character_make {entity: "Ninja (Instance)", script: true}
playtest {seconds: 3, input: [{key: "W", action: "down"}]}
```

`entity_get` on the model lists the clips and their bone coverage; `entity_update {entity:
"Player", animation_tree: {generate: true}}` regenerates the tree, and `animation_tree:
{states, transitions, parameters}` replaces parts of it.

## Troubleshooting

- **It falls through the floor.** The floor has no `Collider` (Make Character says so).
- **It moons the camera / walks backwards.** The model faces -Z and Make Character could not
  tell (no toe bones). Rotate the model entity 180 degrees about Y.
- **The feet slide.** The blend points sit at the clips' speeds; if the clips were made in
  place they use 1.4 and 4.6 m/s. Move the points (or the controller's walk/run speeds) until
  they match.
- **It drifts ahead of its capsule and snaps back.** The clips travel: set the Animator's
  **Root motion** to **In place**.
- **T-pose, or a clip does nothing.** Check the clip's bone count in the Animator's
  **Clips** table: few bound bones mean other bone names. A state whose clip name shows in
  red names a clip the animator does not have.
- **FBX import fails.** The notification says why; Blender's log ends it. "Blender was not
  found": install it or set `BLENDER_PATH`.
