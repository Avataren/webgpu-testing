# Character playground

A test course for the engine's physics (Rapier), `CharacterController` and
`SpringCamera`, with a rigged, animated robot made in Blender.

Open it in the editor (`File > Open Project`, or `engine_launch {project}` over MCP) and
press Play, or run it with `cargo run --release --bin player games/character-playground`.

The controls are World of Warcraft style by default; F3 switches to action-game controls.

| Input | MMO (default) | Action (F3) |
|---|---|---|
| W / S | forward / walk backwards | move, relative to the camera |
| A / D | turn (strafe while holding the right button) | move sideways |
| Q / E | strafe | |
| left drag | look around the robot; the view stays, and swings back behind once he moves | |
| right drag | steer: he turns with the camera | look around |
| both buttons | run forward | |
| click | | capture the mouse (Escape releases it) |
| up / down while dragging | tilt the camera | tilt the camera |
| Shift | walk | walk |
| Space | jump | jump |
| wheel | zoom | zoom |

F1 shows every collider, F2 toggles a double jump.

## The course

- **Ramps** at 15, 30, 45 and 60 degrees up to a 2 m platform: the controller climbs
  slopes up to its `max_slope` (50) and slides off steeper ones.
- **Stairs** with 0.25 m steps (walked up; `step_height` is 0.35) and **tall steps** of
  0.5 m that need jumps.
- **Pillars** of rising height up to the **summit**, a 4 m drop for hard landings (dust,
  thud and the land clip).
- **Mover** and **Lift**: kinematic platforms moved by `mover.lua` that the robot rides.
- **Crates and balls**: dynamic bodies it pushes over.
- **Jump pad**: a sensor whose `jump_pad.lua` launches whatever enters it.
- **Tunnel and columns**: the spring camera pulls in instead of clipping through them.

## Scripts

- `content/scripts/lib/third_person.lua` is the reusable controller, with the two
  control schemes (`scheme = "mmo"` or `"action"`), mouse capture and the
  idle/walk/run/strafe/jump/fall/land animation state machine (backpedalling plays the
  walk backwards). Every step is a method, so a game changes one by replacing it (see the
  header of the file). Copy it into another project's `content/scripts/lib/` to use it.
- `content/scripts/player.lua` uses it and adds this game's extras: sounds, landing dust,
  a respawn, debug keys and the HUD.
- `content/scripts/mover.lua` and `content/scripts/jump_pad.lua` drive the platforms and
  the pad.

The body's tuning (speeds, jump height, slopes, steps, gravity) is on the Player's
`CharacterController` and the camera's on `PlayerCamera`'s `SpringCamera`, both editable in
the inspector.

## The robot

`tools/robot.py` builds `content/gltf/robot/robot.glb` in Blender (5.2):

```
blender -b --python games/character-playground/tools/robot.py -- --preview
```

It is a skin-modifier mannequin weighted to a 21-bone rig by bone heat, with rigid armour
pieces, and eight procedurally keyed, in-place clips: Idle, Walk, Run, StrafeLeft,
StrafeRight, Jump, Fall and Land.
Bones use Mixamo-style names (`Hips`, `Spine`, `LeftUpLeg`, ...). `--preview` also renders
a few poses into `tools/build/`. Re-running it replaces the GLB the scene uses (clips are
read from it when play starts; a change to the mesh needs a re-import).
