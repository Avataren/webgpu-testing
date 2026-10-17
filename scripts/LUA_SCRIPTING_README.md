# Lua Scripting Guide

This document describes the Lua scripting API and provides examples of common patterns.

## Overview

Scripts are Lua 5.4 files (or inline sources) attached to entities through a
`LuaScriptComponent`. Each component gets its own script instance with a private
environment, so two scripts can both define `update` without clashing. Scripts read the
world directly (transforms, queries, components) and change it through commands that are
applied right after each hook returns.

## Important Conventions

### Rotation Order (Euler Angles)

The `set_rotation(entity, yaw, pitch, roll)` function uses **YXZ** Euler rotation order:

1. **Yaw** - Rotation around Y axis (turning left/right)
2. **Pitch** - Rotation around X axis (looking up/down)
3. **Roll** - Rotation around Z axis (tilting/rolling)

**All angles must be in radians:**
- 90 degrees = π/2 ≈ 1.5708 radians
- 180 degrees = π ≈ 3.1416 radians
- 360 degrees = 2π ≈ 6.2832 radians

```lua
-- Rotate 90 degrees around Y axis (yaw)
set_rotation(entity, 1.5708, 0, 0)

-- Rotate 45 degrees pitch
set_rotation(entity, 0, 0.7854, 0)

-- Helper function to convert degrees to radians
function deg_to_rad(degrees)
    return degrees * (3.14159265359 / 180)
end

set_rotation(entity, deg_to_rad(90), 0, 0)
```

### Coordinate System

The engine uses a **right-handed 3D coordinate system**:

- **X axis**: Right (positive = right, negative = left)
- **Y axis**: Up (positive = up, negative = down)
- **Z axis**: Back (positive = toward camera, negative = away from camera)

```lua
-- Move entity to the right
translate(entity, 5, 0, 0)

-- Move entity up
translate(entity, 0, 3, 0)

-- Move entity forward (away from camera)
translate(entity, 0, 0, -10)
```

### Array Indexing

**Lua uses 1-based indexing** - the first element of a table is at index 1, not 0.

All API functions that return arrays use this convention:

```lua
-- Get children of an entity
local children = get_children(parent_entity)
if children then
    log_info("Child count: " .. #children)

    -- Iterate using 1-based indices
    for i = 1, #children do
        local child = children[i]
        log_info("Child " .. i .. ": " .. child)
    end
end

-- Query entities
local enemies = query_entities_with_component("Enemy")
for i = 1, #enemies do
    local enemy = enemies[i]
    -- Do something with enemy
end
```

### Entity Handles

Commands you queue (`set_translation`, `set_component`, `set_material`, `look_at`,
`set_parent`, `despawn_entity`, ...) apply when the hook returns. Most of them accept a
handle spawned earlier in the same hook (`spawn_primitive`, `spawn_light`, `spawn_camera`,
`spawn_entity`). The exceptions need an entity that already exists: `import_gltf`,
`subscribe_event`, `unsubscribe_event`, and `set_parent` when the *child* already exists but
the new parent was spawned in the same hook. Reading (`get_component`, `has_component`,
`get_world_translation`) only sees entities from earlier frames.

Entities are represented as **64-bit integers (i64)** in Lua. You can:
- Store them in variables
- Pass them to functions
- Store them in tables or state
- Compare them with `==`

```lua
-- Store entity reference
local player = spawn_entity("Player")
set_state("player_ref", player)

-- Retrieve and use it later
local player = get_state("player_ref", 0)
if player ~= 0 then
    set_translation(player, 0, 10, 0)
end
```

### World vs Local Transforms

- **Getter functions** (`get_world_translation`, `get_world_rotation`) return transforms in **world space** (accounting for parent hierarchy)
- **Setter functions** (`set_translation`, `set_rotation`, `set_scale`) set **local transforms** relative to the parent

```lua
-- Set local position (relative to parent)
set_translation(child, 5, 0, 0)

-- Get world position (absolute in scene)
local world_pos = get_world_translation(child)
if world_pos then
    log_info(string.format("World position: %.2f, %.2f, %.2f",
        world_pos.x, world_pos.y, world_pos.z))
end
```

## Available API Functions

### Logging
- `log_debug(message)` - Log debug message
- `log_info(message)` - Log info message
- `log_warn(message)` - Log warning message
- `log_error(message)` - Log error message

### State Management
- `set_state(key, value)` - Store any value
- `get_state(key, default)` - Retrieve value with default
- `try_get_state(key)` - Retrieve value or nil
- `set_f64(key, number)` - Store number
- `get_f64(key, default)` - Retrieve number (errors if missing and no default given)
- `set_i32(key, integer)` - Store integer
- `get_i32(key, default)` - Retrieve integer with default
- `set_bool(key, boolean)` - Store boolean
- `get_bool(key, default)` - Retrieve boolean with default
- `set_string(key, string)` - Store string
- `get_string(key, default)` - Retrieve string with default
- For-entity variants: `set_state_for`, `get_state_for`, etc.

### Runtime
- `is_playing()` - `true` while scripts run for play mode, `false` in editor mode. Lets a
  `@tool` script keep game logic out of the editor.
- `get_time()` - Seconds of script time: the sum of the frame steps scripts have been given.
  It starts at 0 when play starts or stops, and follows `time_scale`. Use it for timers
  (`if get_time() > spawn_at then ...`) instead of summing `dt` yourself.

### Entity Management
- `spawn_entity(name)` - Create new entity
- `despawn_entity(entity)` - Remove an entity and everything parented below it. Applied when
  the hook returns; despawning twice, or an entity that is already gone, is harmless.
  An entity spawned in the same hook is simply never created. Scripts attached to removed
  entities get `on_destroyed`.
- `entity_exists(entity)` - `true` if the entity is still in the world (handles of despawned
  entities never get reused, so a stale handle stays `false`)
- `entity_id(entity)` - the entity's id (the number callbacks and AI brains' `ctx.agent`
  use), also for a handle `spawn_entity` / `clone_entity` returned, once the entity exists;
  `nil` otherwise
- `set_name(entity, name)` - Set entity name
- `find_entity_by_name(name)` - Find entity by name
- `attach_inline_script(entity, name, source)` - Attach inline Lua script
- `attach_script(entity, path)` - Attach script from file
- `import_gltf(entity, path, scale)` - Import GLTF model below `entity` (which must exist
  already, i.e. not be spawned in the same hook). The model is there from the next frame:
  `get_children(entity)` lists it then. Works in the editor and the player.
- `clone_entity(entity[, name])` → `entity` - Copy an entity and everything below it: a
  character with its skeleton, meshes and animator (the copy's meshes deform with the
  copy's bones and its animator starts playback afresh), a light rig, a prop. Meshes and
  materials are shared with the original until `set_material` changes one. The copy is a
  sibling of the original until `set_parent` moves it (`set_parent(copy, nil)` makes it a
  root); its name, transform, components and animations (`animator_play`,
  `animator_add_clips`, ...) can be set in the same hook, like a spawned entity's. Import a
  character once, hide it (`set_component(e, "Visible", false)`), and clone it for crowds:
  cloning is cheap, importing reads the file again.

```lua
-- A crowd from one imported character (the template is hidden; its copies are shown).
local copy = clone_entity(template, "Dancer " .. i)
set_translation(copy, x, 0, z)
set_component(copy, "Visible", true)
animator_add_clips(copy, "content/animations/samba.glb")
animator_play(copy, "samba", { fade = 0, start = i * 0.37 })
```

Handles returned by `spawn_entity`, `clone_entity` and the other spawning functions name
their entity in every later hook too: `get_children`, `get_world_translation`,
`get_component`, `animator_state` and the other reading functions take them like ids.

### Meshes and Materials
- `spawn_primitive(kind[, name[, {material_of = entity}]])` → `entity` - Spawn a visible
  entity with a primitive mesh (`cube`, `sphere`, `plane`, `cylinder`, `cone`, `torus`) and
  its own material, or the material of `material_of` (see `share_material`). Sizes at scale 1:

  | Primitive | Size | Notes |
  |-----------|------|-------|
  | `cube` | 1 x 1 x 1, centred | |
  | `sphere` | radius 1 (2 m across) | |
  | `plane` | 1 x 1 in XY, facing +Z | rotate -90° about X for a floor |
  | `cylinder` | radius 1, length 1 along Z (z -0.5..0.5) | |
  | `cone` | base radius 1 at z = -1, tip at the origin | tip points +Z: give ships and arrows a 180° yaw |
  | `torus` | ring radius 1 to the tube centre, tube radius 0.35 (1.35 outer), axis along Z | the tube scales with the ring, so a thin ring cannot be made by scaling |

  The cone's tip points toward +Z, the opposite of the engine's -Z forward, so a ship or
  arrow made from a cone needs a 180° yaw to point where `look_at` / `get_forward` say.
- `share_material(entity, source)` - Draw `entity` with `source`'s material. Many entities
  of one color (sparks, bullets, bricks) then use one material instead of one each.
  `set_material` on one of them gives it its own copy again.
- `set_mesh(entity, mesh)` - Replace the geometry with a primitive (by name), a generated
  mesh (`model.parts[i].mesh` of a `voxel_model`), another entity (draw that entity's mesh,
  e.g. a promoted pawn taking the queen's model), or `nil` to remove it
- `set_material_color(entity, r, g, b[, a])` - Base color in 0..1
- `set_material(entity, fields)` - Change material fields: `color` (`{r, g, b[, a]}` or
  `"#rrggbb"`), `emissive` (strength; the surface glows in its own color, and values above 1
  are HDR, so `emissive = 3` blooms strongly while staying lit), `metallic`, `roughness` (0..1), `unlit`, `transparent`,
  `double_sided`, `nearest_filtering` (booleans) and texture paths such as
  `base_color_texture` (`false` removes a texture)

Changes apply before the frame is drawn. An entity that shares its material with others
gets its own copy on the first change, so recoloring one brick leaves the rest alone.
Materials no entity uses any more (after `despawn_entity`) are freed, so spawning and
despawning entities over a long game does not grow memory.

```lua
local spark = spawn_primitive("sphere", "Spark")
set_translation(spark, 0, 2, 0)
set_material(spark, { color = {1, 0.6, 0.1}, emissive = 1, unlit = true })
set_material_color(hit_brick, 1, 1, 1) -- flash
```

### Procedural geometry
Voxel models are meshed by the engine: faces between solid voxels are dropped and
same-coloured faces in one plane merge into large rectangles, so a model draws a few
meshes with a handful of quads instead of one cube per voxel. Each key (colour) becomes its
own mesh so it can have its own material. Identical models share their meshes (and draw
instanced), and meshes no entity uses any more are freed. Ambient occlusion is baked into
the vertex colours, so creases and inner corners darken; faces only merge where their
corner shading matches, so it stays exact.

- `voxel_model(desc)` → `model` - Build and mesh a model. `desc` has exactly one source:
  - `rows = {"..RR..", ".RGGR.", ...}` - text art, top row first; each character is a voxel
    of that key (`.` and space are empty), extruded `depth` voxels (default 1);
  - `layers = { rows, rows, ... }` - 3D text art, slices from the front (+Z) to the back;
  - `size = {W, H, D}` with `voxels = {...}` - a flat array, index `1 + x + W*(y + H*z)`;
  - `size = {W, H, D}` with `fill = function(x, y, z) return key end` - called for every
    cell (0-based, y up, z toward the viewer).

  Keys are strings or integers; `nil`, `false`, `0` and `""` are empty and `true` is key 1.
  `legend = {R = "red", G = 2, x = false}` maps text-art characters to keys (without it
  each character is its own key). `voxel` is the size of one voxel in meters (number or
  `{x, y, z}`, default 0.1) and `pivot` the fraction of the model's box at the entity's
  origin (default `{0.5, 0, 0.5}`: standing centred on it). `palette = {[key] = material}`
  is kept for `spawn_voxels`. Limits: 1024 voxels per side, 16M cells, 255 keys.

  The look: `ao` is the strength of the baked ambient occlusion (0..1, default 1; 0 turns
  it off). `colors = {[key] = "#rrggbb" or {r, g, b[, a]}}` gives keys a vertex colour,
  which multiplies the material colour (and so the glow of emissive materials).
  `parts = {[key] = part_key}` puts several keys into one mesh, which draws with one
  material while each key keeps its vertex colour; `part = part_key` does that for every
  key `parts` does not list. A gradient is many keys with their own `colors` in one part.

  The result: `size`, `voxel`, `min`, `max` (local bounds), `voxels` (solid count),
  `quads`, `triangles`, `cube_quads` (what one cube per voxel would draw), `palette`, and
  `parts` = `{ {key = ..., keys = {...}, mesh = GeneratedMesh, quads = n, voxels = n}, ... }`
  in order of first use (`keys` are the keys a grouped part draws). `set_mesh(entity, part.mesh)` draws a part on any entity.
- `spawn_voxels(model_or_desc[, name[, opts]])` → `root, parts` - Spawn a model (or build
  one from a description first): an entity `name` (default `"Voxels"`) with a child
  `name.key` per part; `parts[key]` is that child. `opts.materials[key]` is an entity to
  share the material of, a colour (`"#rrggbb"` or `{r, g, b}`), or material fields; without
  one the `palette` entry is used, and a key that is a `"#rrggbb"` string colours itself.
  `opts.parent` parents the root, `opts.cast_shadow = false` keeps the parts out of shadow
  maps.
- `noise(x[, y[, z]])` → `number` - Smooth gradient (Perlin) noise in about -1..1 with
  features about one unit apart; 0 at integer coordinates, so scale them (`x * 0.1`).
- `fbm(x, y, z[, octaves])` → `number` - Several octaves of `noise` (default 4) for
  rougher, natural-looking shapes, in about -1..1.

```lua
-- A sprite with walk frames: build each frame once and share it between all actors.
local frames = {}
for i, rows in ipairs(SPRITE_FRAMES) do
    frames[i] = voxel_model{ rows = rows, depth = 3, voxel = 0.08, legend = { H = "hair", S = "skin" } }
end
local hair = spawn_primitive("cube", "HairMat") -- hidden template whose material parts share
set_component(hair, "Visible", false)
local root, parts = spawn_voxels(frames[1], "Hero", { materials = { hair = hair, skin = "#ffb080" } })

-- Terrain from noise: 64 x 24 x 64 voxels of 0.5 m.
local terrain = voxel_model{
    size = {64, 24, 64}, voxel = 0.5, pivot = {0.5, 0, 0.5},
    fill = function(x, y, z)
        local h = 8 + fbm(x * 0.05, 0, z * 0.05, 5) * 8
        if y > h then return nil end
        return y > h - 1 and "grass" or (y > h - 4 and "dirt" or "rock")
    end,
    palette = { grass = "#4a9a35", dirt = "#7a5230", rock = { color = "#777780", roughness = 0.9 } },
}
log_info(("terrain: %d voxels, %d quads (%d as cubes)"):format(terrain.voxels, terrain.quads, terrain.cube_quads))
spawn_voxels(terrain, "Terrain")

-- A glowing bar shaded from yellow to red: one key per row, all in one part.
local rows, colors = {}, {}
for r = 1, 8 do
    local k = string.char(64 + r) -- "A".."H"
    rows[r] = string.rep(k, 20)
    colors[k] = { 1, 1 - r / 8, 0.1 }
end
spawn_voxels(voxel_model{ rows = rows, depth = 2, colors = colors, part = "bar" }, "Bar",
    { materials = { bar = { color = { 1, 1, 1 }, emissive = 2 } } })
```

To change a model (digging, building), build a new one and `set_mesh` each part's entity
to the new part's mesh, or despawn the old root and spawn again. For big worlds, split
them into chunks (e.g. 32³) so a change re-meshes only one chunk.

### Lights and Cameras
- `spawn_light(kind[, fields][, name])` → `entity` - Spawn a `"point"`, `"directional"`
  or `"spot"` light
- `set_light(entity, fields)` - Change a light. Fields: `color` ({r, g, b} in 0..1 or
  `"#rrggbb"`), `intensity`, `range` (point, spot), `shadow_size` (directional),
  `inner_angle_deg`/`outer_angle_deg` (spot), `cast_shadow` (default false for lights
  scripts spawn), `shadow_priority` (point, spot: weight in the shadow budget, default
  1). `kind` adds a light or switches its kind.
- `spawn_camera([fields][, name])` → `entity` - Spawn a camera
- `set_camera(entity, fields)` - Change a camera, adding one if needed. Fields:
  `projection` (`"perspective"`/`"orthographic"`), `fov_deg`, `near`, `far`, `left`,
  `right`, `bottom`, `top`, and `active = true` to view the game through it
- `set_active_camera(entity)` - View the game through the entity's camera

Lights and cameras sit at their entity's position and point along its -Z, so
`set_translation` and `look_at` place and aim them (also right after spawning).
Directional lights: the first 4 count. Point and spot lights have no fixed limit (up to
4096; a surface only pays for the lights that reach it), so a market full of lanterns or
a few hundred sparks of light are fine. Their shadows (`cast_shadow = true`) share a
budget: lights that look bigger on screen, are brighter or have a higher
`shadow_priority` get larger shadow maps first, and lights past the budget are drawn
unshadowed (the graphics settings' `shadow_atlas_size` sets the room).
Unknown field names are an error. Like other changes, they apply before the frame is
drawn and are undone when play mode stops.

```lua
local flash = spawn_light("point", { color = {1, 0.6, 0.2}, intensity = 40, range = 6 })
set_translation(flash, x, y, z)
local cam = spawn_camera({ fov_deg = 50, active = true }, "Cutscene")
set_translation(cam, 0, 4, 8)
look_at(cam, 0, 0, 0)
```

### Post-processing
- `set_postprocess(settings)` - Change the scene's infinite post-process volume (created if
  the scene has none). Only the settings you name change.
- `set_postprocess(volume_entity, settings)` - Change a specific post-process volume entity
  (a dark cave, underwater), e.g. one found with `find_entity_by_name`.

`settings` takes the groups of the `postprocess_set` tool: `exposure`, `local_exposure`,
`tonemapping`, `color`, `bloom`, `ssao`, `ssr`, `depth_of_field`, `motion_blur`, `vignette`,
`chromatic_aberration`, `film_grain`, `sharpen` and `anti_aliasing`; `postprocess_get`
lists every field. A `volume = { priority = .., weight = .., enabled = .. }` field edits
the volume itself. An unknown setting name is an error. Like other changes, it applies
before the frame is drawn and is undone when play mode stops, along with a volume a script
created. Scripts cannot read the blended result; keep the values you set in a local.

```lua
-- A damage flash that fades out over half a second.
local hurt = 0
function on_hit() hurt = 1 end
function update(self, dt)
    if hurt > 0 then
        hurt = math.max(0, hurt - dt * 2)
        set_postprocess({
            vignette = { enabled = hurt > 0, intensity = 0.6 * hurt },
            color = { saturation = 1 - 0.5 * hurt },
        })
    end
end
```

- `set_environment(settings)` - Change the scene's environment: `ambient_intensity` (the
  flat ambient light, and the sky that path-traced and global illumination rays see where
  nothing is in the way),
  `clear_color` (the background, `{r, g, b[, a]}` or `"#rrggbb"`) and `hdr = { enabled,
  intensity, path, background }` (an HDR sky; `background = false` keeps it to lighting and
  reflections, with `clear_color` drawn behind the scene). Only the settings you name change; an unknown name is an
  error. Undone when play mode stops.

```lua
-- Path tracing lights the scene from its emissive surfaces: dim the sky for it.
set_environment({ ambient_intensity = path_traced() and 0.03 or 0.18 })
```

### Particle effects
- `emit_particles(x, y, z[, options])` - A one-off burst of GPU particles at a world point:
  sparks, explosions, warp-in rings, debris. Thousands of particles cost almost nothing on
  the CPU, unlike spark entities moved from Lua, and they render in the editor and the
  player. Options (all optional):
  - `count` (default 24, up to 4096)
  - `color`, `color_end` - `{r, g, b[, a]}` or `"#rrggbb"`; particles fade from `color` to
    `color_end` (default: fade out)
  - `intensity` - multiplies both colors; above 1 is HDR and blooms (try 2-4)
  - `speed` - outward m/s, a number or `{min, max}` (default `{2, 6}`)
  - `life` - seconds, a number or `{min, max}` (default `{0.4, 0.8}`)
  - `size`, `size_end` - quad size in meters at spawn and at death (default 0.15 -> 0)
  - `velocity` - `{x, y, z}` added to every particle (inherit a ship's motion)
  - `gravity` - vertical acceleration, negative falls (default 0); `drag` - fraction of
    speed lost per second (default 1)
  - `radius` - start within this distance of the point (default 0)
  - `shape` - `"sphere"` (default, all directions) or `"ring"` (flat in the XZ plane: a
    shockwave on a top-down playfield)
  - `blend` - `"add"` (default, glows) or `"alpha"`

  Unknown option names are an error. Particles are camera-facing quads; they are not
  entities, so `scene_diff` and queries do not see them.

```lua
emit_particles(x, 0.5, z, { count = 60, color = "#ffb030", intensity = 3, speed = {3, 9} })
emit_particles(x, 0.2, z, { count = 48, shape = "ring", speed = 8, life = 0.35, color = {0.3, 0.8, 1} })
emit_particles(x, 1, z, { count = 30, gravity = -9.8, drag = 0.2, color = "#999999", blend = "alpha" })
```

### Skeletal animation
Skinned glTF characters (imported with `asset_import`, or `import_gltf`) get an
`Animator` on their root entity. It plays clips from glTF/GLB files, binding them to bones
by name, so a clip file made for one character drives any character with the same bone
names (Mixamo-style packs). Bone names match without case or a namespace prefix:
`"Hips"` finds `mixamorig:Hips`. Translations are rescaled to the skeleton's proportions.
Every function takes the character root or any entity below it (its mesh, a bone): the
nearest animator up the hierarchy is used, else the nearest below (a character made with
the editor's "Make Character" has its model below the root). Bones the clip names that the
skeleton lacks bind by humanoid role (Mixamo `LeftArm`, Unreal `upperarm_l` and Rigify
`DEF-upper_arm.L` are the same bone), and clips from another rig or export are retargeted:
each bone turns away from its rest pose as it does in the clip, so both rigs need the same
rest posture (T-pose). The asset library (`docs/asset_library.md`) has clips for all of
them. Animators run in play mode only; while editing, characters
keep their rest pose.

- `animator_play(entity, clip[, options])` - Cross-fade to `clip`. Options: `fade` (seconds,
  default 0.2), `loop` (default true), `speed` (default 1; negative plays backwards),
  `layer` (default 0), `start` (time in seconds). Calling it every frame with the same
  clip keeps it playing; a non-looping clip that ran to its end plays again.
- `animator_stop(entity[, { layer, fade }])` - Fade out a layer (default all) to the rest pose.
- `animator_set_speed(entity, speed)` - Scales every clip; 0 pauses.
- `animator_set_layer(entity, layer, { weight, fade, mask })` - Layers above 0 override
  the ones below: `weight` 0..1 (faded over `fade` seconds); `mask` a bone name so only it
  and the bones below it move (`"Spine"` = upper body), `false` clears it.
- `animator_add_clips(entity, path)` - Make the clips of a glTF/GLB file playable
  (project-relative path). A file holding one clip plays under its file name
  (`walk.glb` -> `"walk"`).
- `animator_set_root_motion(entity, mode)` - `"keep"` (default), `"in_place"` (remove the
  hips' horizontal travel) or `"apply"` (move the entity by it: walks without foot sliding).
- `animator_state(entity[, layer])` → `{clip, time, duration, playing, looping, finished,
  fading, speed}` or `nil`
- `animator_clips(entity)` → `{ {name, duration, tracks, bound, missing, root_speed}, ... }`
  (after the animator's first frame); `root_speed` is how fast the clip travels over the
  ground (m/s, about 0 for clips made in place)
- `find_bone(entity, name)` → the bone entity below `entity`, or `nil`. Parent a sword to
  the hand bone to hold it; it follows the animation.

```lua
function update(self, dt)
    local moving = is_key_pressed("W")
    animator_play(self, moving and "Run" or "Idle", { fade = 0.25 })
    if is_key_just_pressed("Space") then
        animator_play(self, "Wave", { layer = 1, loop = false })
    end
end

function on_created(self)
    animator_set_layer(self, 1, { weight = 1, mask = "Spine" })
    animator_set_root_motion(self, "apply")
    set_parent(find_entity_by_name("Sword"), find_bone(self, "RightHand"))
end
```

### Animation trees
Like Godot's AnimationTree, an `AnimationTree` component picks a character's animation from
parameters, so scripts move the character and never name clips. It has **states**, each
playing a motion: one clip, a 1D blend space (clips placed along one parameter, e.g. idle
at 0, walk at 1.4, run at 4.6 by `speed`) or a 2D blend space (clips placed by two
parameters, e.g. `side_speed` and `forward_speed`, so strafes and a backpedal mix in).
**Transitions** go from a state (or `*`, any state) to another when their condition holds
(`"grounded && speed > 1"`, `"!grounded"`, `"landed and landing_speed > 8"`; `&& || !`,
`and or not`, comparisons, numbers, `true`/`false`), optionally only `after` the state's
motion has played that far (0..1), cross-fading over `fade` seconds. They are checked in
order; the first that holds is taken.

With `from_character` (the default) the character's `CharacterController` feeds `speed`,
`forward_speed`, `side_speed` (along its facing, m/s), `vertical_speed`, `grounded` (0/1),
`air_time`, `landing_speed` and the triggers `jumped` and `landed`. In the editor, the
inspector's "Generate from clips" (or "Make Character") builds a locomotion tree from the
animator's clips by name: idle, walk, run, strafes, a backpedal (or the walk played
backwards), jump, fall and land, with blend points at the speeds the clips travel. Add
your own states and transitions in the inspector, and drive them from scripts. These take
the character's root, its model or a bone:

- `animation_set(entity, name, value)` - Set a parameter (a number, or true/false).
- `animation_trigger(entity, name)` - Fire a trigger: true for the next evaluation only.
- `animation_get(entity, name)` → the parameter's value (a number)
- `animation_state(entity)` → `{state, previous, time, progress}` or `nil`
- `animation_travel(entity, state[, fade])` - Go to a state now.
- `animation_tree_generate(entity)` - Give the entity a locomotion tree built from its
  animator's clips by name (as "Generate from clips"), fed by the character controller or
  nav agent it belongs to; travelling clips then play in place. It is built after the hook
  returns, from the clips added so far: spawn a crowd member, add its clips, then call it.

```lua
-- A "Dance" state entered when dancing, left when the character moves:
--   Locomotion -> Dance   when "dancing"
--   Dance -> Locomotion   when "!dancing || speed > 0.5"
function update(self, dt)
    if is_key_just_pressed("G") then
        animation_set(self, "dancing", animation_get(self, "dancing") == 0)
    end
end
```

The tree plays on layer 0: `animator_play(e, "Wave", { layer = 1 })` with
`animator_set_layer(e, 1, { mask = "Spine" })` still waves over whatever it picks.

### Physics
Rigid bodies, walking characters and third-person cameras, simulated with Rapier in play
mode after every script's `update`. Entities get them as components (inspector, MCP
`entity_update`, or `set_component` below):

- `Collider` - what an entity collides as. On its own it is fixed scenery placed by the
  transform. `shape.kind` is `auto` (fits the mesh: primitives exactly, other meshes as
  triangles, or as their convex hull on dynamic bodies), `box` (`size`), `sphere`
  (`radius`), `capsule` / `cylinder` (`radius`, `height`), `mesh` or `convex_hull`; sizes
  are local units stretched by the entity's scale. `friction`, `restitution`, `density`,
  `offset`, `sensor` (a trigger volume) and `include_children` (take the meshes below it,
  for imported models and levels).
- `RigidBody` - `kind` `dynamic` (moved by the simulation, written back to the transform),
  `fixed` or `kinematic` (follows its transform and pushes things: platforms, doors);
  `mass`, `gravity_scale`, `linear_damping`, `angular_damping`, `lock_rotation`, `ccd`.
- `CharacterController` - a walking capsule whose origin is its feet. Scripts steer it;
  it handles slopes up to `max_slope`, steps up to `step_height`, gravity, jumps
  (`jump_height`, `air_jumps`, `coyote_time`, `jump_buffer`), `walk_speed` / `run_speed`,
  `acceleration`, `air_control`, turning to face its movement, pushing dynamic bodies and
  riding kinematic platforms. Models should face +Z (glTF's convention), else set
  `yaw_offset`.
- `SpringCamera` - put it on a camera: it follows `target` (an entity name; empty means
  the parent) at `distance` around `pivot_offset`, with lag, mouse look (while the cursor
  is locked or the right button is held), wheel zoom, and pulls in before scenery.

Functions:

- `physics_raycast(ox, oy, oz, dx, dy, dz[, max_distance[, options]])` → `{entity,
  distance, point = {x, y, z}, normal = {x, y, z}}` or `nil`. Options: `exclude` (an
  entity or a list), `sensors = true` (hit trigger volumes too), `ignore_dynamic = true`.
- `physics_spherecast(ox, oy, oz, dx, dy, dz, radius[, max_distance[, options]])` - the
  same for a swept ball.
- `physics_overlap_sphere(x, y, z, radius[, options])` → list of entities.
- `get_velocity(entity)` / `get_angular_velocity(entity)` → `{x, y, z}` or `nil`.
- `set_velocity(entity, x, y, z)`, `apply_impulse(entity, x, y, z)` (kg·m/s; on a
  character it adds m/s to its velocity: launch pads, knockback),
  `set_angular_velocity(entity, x, y, z)`, `apply_torque_impulse(entity, x, y, z)`.
- `set_gravity(x, y, z)`, `get_gravity()`; `physics_debug_draw(true)` draws every
  collider.
- `character_move(entity, x, z[, run])` - steer: a world-space direction on the ground
  whose length (0..1) is the share of walk (or run) speed. It holds until changed; send
  `0, 0` to stop.
- `character_jump(entity)` - jump now, or as soon as it lands within `jump_buffer`.
- `character_face(entity, yaw[, instant])` - face `yaw` (radians about +Y; 0 faces +Z, so
  `math.atan(x, z)` faces a direction), turning at the controller's `turn_speed` or at
  once. While set it no longer turns toward its movement, so it can strafe and walk
  backwards; `character_face(entity, nil)` hands the facing back.
- `character_state(entity)` → `{grounded, velocity, speed, horizontal_speed,
  vertical_speed, ground_normal, ground_entity, air_time, just_landed, landing_speed,
  just_jumped, sliding, running, move_input, yaw}`.
- `spring_camera_state(camera)` → `{yaw, pitch, distance, pivot, forward, right, target}`;
  `forward` and `right` are the view's horizontal axes, for camera-relative movement.
- `spring_camera_rotate(camera, yaw_degrees, pitch_degrees)` and
  `spring_camera_set(camera, {yaw, pitch, distance, target, snap})` (`target` an entity
  or `false` for the component's; `snap` skips the lag once).
- `set_cursor_locked(locked)` / `is_cursor_locked()` - capture the mouse for mouse look
  (hidden, unbounded `get_mouse_delta()`). Releasing puts the cursor back where it was
  captured, so a drag-to-look can capture on press and release on button up. Escape,
  losing focus or leaving play releases it too.

Physics callbacks arrive at the start of the next frame (see Lifecycle Hooks):
`on_collision_enter(self, other)` / `on_collision_exit`, `on_trigger_enter(self, other)` /
`on_trigger_exit` (sensors and what enters them), and on characters `on_jumped(self)` and
`on_landed(self, speed)`.

Change a component's settings with `set_component`: only the fields given change.

```lua
-- A third-person character: steer relative to the camera, jump on Space.
function update(self, dt)
    local view = spring_camera_state(find_entity_by_name("PlayerCamera"))
    local x = (is_key_pressed("D") and 1 or 0) - (is_key_pressed("A") and 1 or 0)
    local y = (is_key_pressed("W") and 1 or 0) - (is_key_pressed("S") and 1 or 0)
    if view then
        character_move(self, view.forward.x * y + view.right.x * x,
                             view.forward.z * y + view.right.z * x, true)
    end
    if is_key_just_pressed("Space") then character_jump(self) end
end

function on_landed(self, speed)
    if speed > 8 then play_sound("content/audio/thud.wav") end
end

-- Double jump power-up
set_component(player, "CharacterController", { air_jumps = 1 })
```

### Third-person controller
The engine's module `engine.third_person` is a complete keyboard and mouse controller for a
character with a `CharacterController` and a camera with a `SpringCamera` (the editor's
"Make Character" writes a script using it). It only steers: the character's
AnimationTree animates it. Every step is a method a game can replace.

```lua
local ThirdPerson = require("engine.third_person")
local controls

function on_created(self)
    controls = ThirdPerson.new(self, { camera = "PlayerCamera", scheme = "mmo" })
end
function update(self, dt) controls:update(dt) end
function on_jumped(self) controls:jumped() end
function on_landed(self, speed) controls:landed(speed) end
function on_destroyed(self, reason) if controls then controls:release() end end
```

- Schemes: `"action"` (default; WASD relative to the camera, the character turns to run
  that way, a click captures the mouse to look around, Escape releases it) and `"mmo"`
  (World of Warcraft style: W/S along the facing, A/D turn, Q/E strafe, left drag looks
  around and the view swings back behind once the character moves, right drag steers,
  both buttons run, up and down with either button tilts the camera).
- Options (`ThirdPerson.defaults`): `camera` (entity or name), `keys` (lists per action:
  `forward`, `back`, `left`, `right`, `strafe_left`, `strafe_right`, `jump`, `walk`),
  `always_run` (Shift walks), `capture_mouse`, `turn_rate` (degrees/s), `mouse_sensitivity`
  (degrees/pixel), `hide_cursor_while_dragging`, `invert_y`, `camera_follow` (how fast the
  view swings back behind), `diagonal_turn` (degrees the body turns into a diagonal run),
  `strafes` (false: the character turns to run sideways instead of strafing).
- Replace a method on the instance (or on `ThirdPerson` for every character): `read_input`,
  `move_direction`, `mmo_steer`, and the hooks `jumped`, `landed(speed)` for sounds and
  effects:
  `local landed = controls.landed; function controls:landed(speed) landed(self, speed); play_sound(...) end`.
- `controls.state` is the last `character_state`; `controls:release()` gives the cursor
  back (when switching schemes, or before the script goes away).

To change it beyond that, copy `engine/third_person.lua` from the engine
(`src/scripting/lua/modules/`) into your scripts folder as `engine/third_person.lua`: a
project's own file takes its place.

### Navigation
Navmeshes, paths and agents that walk them (`docs/navigation.md` is the full guide).
Entities get them as components, set up in the inspector's Navigation menu, over MCP or
with `set_component`:

- `NavMeshSurface` - a navmesh built from the scene's static geometry (bake it in the
  inspector or with MCP `navmesh_bake`; without a bake it is built when play starts).
  `agent_radius`, `agent_height`, `agent_max_climb`, `agent_max_slope`, voxel sizes,
  `bounds`, `geometry` (`render_meshes`, `colliders`, `both`), `agent_type`.
- `NavAgent` - walks a navmesh: `radius`, `speed`, `acceleration`, `turn_speed`,
  `stopping_distance`, `avoidance`, `priority`, `separation`, `drive` (`transform` moves
  it, `character` steers its `CharacterController`, `auto`), `destination` (headed for at
  play start), `area_costs` (`{{area = 2, cost = 3}}`). An `AnimationTree` on or below it
  is fed `speed`, `forward_speed`, `side_speed` and `grounded`.
- `NavObstacle` - a `box` or `cylinder` footprint agents steer round; with `carve` it cuts a
  hole into the navmesh once it stands still, so paths go round it (a parked cart, a closed
  gate).
- `NavLink` - an off-mesh connection from the entity to `end` (local): a jump, a drop, a
  ladder (`bidirectional`).
- `NavModifier` - on scenery: `ignore` it, make it `walkable = false` (solid, never walked
  on), or give its tops an `area`.

Agents walk in play mode after every script's `update` (before the physics), so a
destination set in `update` is planned and walked the same frame.

- `nav_set_destination(entity, x, y, z)` - walk there (the nearest point on the navmesh).
  Returns false when the entity is not an agent.
- `nav_stop(entity)`, `nav_warp(entity, x, y, z)` - stop where it is; put it somewhere else
  at once.
- `nav_face(entity, yaw)` - face `yaw` (radians about +Y; 0 faces +Z) while it stands;
  walking hands the facing back to its movement.
- `nav_agent_state(entity)` → `{state, position, velocity, speed, desired_velocity, target,
  remaining_distance, partial, corners}` or `nil`. `state` is `idle`, `planning`, `moving`,
  `arrived`, `failed`, `off_mesh` or `lost`; `partial` means the way is blocked for now and
  it walks as near as it can (it keeps retrying).
- `nav_find_path(x0, y0, z0, x1, y1, z1[, {agent_type}])` → `{points = {{x, y, z}, ...},
  complete, length}` or `nil`.
- `nav_closest_point(x, y, z[, radius])` → `{x, y, z}` or `nil`.
- `nav_random_point([x, y, z, radius])` - a random point on the navmesh, reachable from the
  polygon nearest `x, y, z` within `radius` when given (start it on the main floor).
- `nav_raycast(x0, y0, z0, x1, y1, z1)` → `{hit, point, normal}`: walking a straight line
  over the floor, where a wall stops it.
- `nav_pick(ox, oy, oz, dx, dy, dz)` → `{x, y, z}` where a ray (from
  `screen_to_world_ray`) first meets a navmesh, on any floor, or `nil`: click-to-move.
- `nav_debug_draw(enabled | {mesh, agents, paths, obstacles, x_ray})`.
- `nav_rebuild([surface])` - build the navmeshes again from the scene as it is now.
- `nav_status()` → `{surfaces = {{entity, name, tiles, polys, source, agents}}, agents,
  obstacles, carving}`.

Callbacks on an agent's script: `on_nav_arrived(self)`, `on_nav_failed(self, reason)`,
`on_nav_link_start(self, {start, finish, link})` and `on_nav_link_end(self)`.

```lua
-- Click to send this agent somewhere; wander after arriving.
function update(self, dt)
    if is_mouse_button_just_pressed(0) then
        local m = get_mouse_position()
        local ray = m and screen_to_world_ray(m.x, m.y)
        local p = ray and nav_pick(ray.ox, ray.oy, ray.oz, ray.dx, ray.dy, ray.dz)
        if p then nav_set_destination(self, p.x, p.y, p.z) end
    end
end

function on_nav_arrived(self)
    local p = nav_random_point(0, 0, 6, 20)
    if p then nav_set_destination(self, p.x, p.y, p.z) end
end
```


### AI
Behaviour trees and state machines for `AiAgent`s (`docs/ai.md` is the full guide). Brains
are tables; their functions get `ctx` (`ctx.agent`, `ctx.bb` its blackboard, `ctx.group`,
`ctx.global`, `ctx.dt`, `ctx.state`, `ctx.memory`, `ctx.event`) and run in the context of the
script that defined them.

- `ai_tree(name, root)` / `ai_machine(name, {initial, states, transitions})` /
  `ai_define(name, def)` - define a brain (redefining restarts its agents).
- `ai_register(name, f)` - a function JSON brains can name; `ai_brains()` lists brains.
- `ai_set_brain(entity, name[, {tags, group, tick_rate, run_speed, blackboard,
  perception}])` - give an entity an `AiAgent` (or `set_component(e, "AiAgent", {...})`).
- `ai_get(e, key)`, `ai_set(e, key, value)` (`group.key`, `global.key` reach shared
  boards); `ai_blackboard(e)`, `ai_group_board(name)`, `ai_global()` return boards to
  index and assign.
- `ai_send(e, event[, data[, from]])` (`from`: the sender, e.g. `ctx.agent`), `ai_broadcast(event[, data[, {x, y, z, radius, tag,
  group}]])`, `ai_noise(x, y, z, radius, tag[, data[, from]])`.
- `ai_state(e)` (`"work/serve"`), `ai_action(e)`, `ai_status(e)`, `ai_senses(e)`,
  `ai_sees(e[, tag])`, `ai_agents([{brain, tag, state, group}])`.
- `ai_restart(e)`, `ai_pause(e, paused)`, `ai_debug_draw(true | {senses, goals, percepts,
  states})`.

Scripts on an agent get `on_ai_state(self, {from, to})`.

```lua
function on_created(self)
    ai_tree("idler", { "selector",
        { "sequence", when = "sees('cart')", abort = "lower",
            { "flee", "nearest('cart')", 4 } },
        { "sequence", { "wander", 6 }, { "play", "look-around", time = { 2, 5 } } },
    })
    ai_set_brain(self, "idler", { tags = { "townsfolk" }, perception = { sight_range = 8 } })
end
```
### Files a game ships

A game's export (the editor's Build menu, web builds) carries only the content it uses,
found by following the scenes, materials and scripts. In scripts that means string
literals naming project files (`play_sound("content/audio/jump.wav")`) and the modules
they `require`. A path built at run time (`"content/audio/step" .. n .. ".wav"`) is not
seen: name its folder as a literal (`local STEPS = "content/audio/steps/"`) and the whole
folder ships.

### Saving data
- `save_data(key, text)` - Keep a small text value between runs (a high score, settings);
  `nil` deletes it. Store tables as JSON or a Lua literal.
- `load_data(key)` → `string` or `nil`

Values belong to the open project (its folder name) and the user, not to the project's
files, so they never end up in its repository: native builds write
`<user data dir>/wgpu-cube/saves/<project>/<key>.txt` (`~/.local/share` on Linux), web
builds use the browser's `localStorage`. Keys are 1-64 letters, digits, `_`, `-` or `.`.

```lua
local best = tonumber(load_data("highscore") or "0")
if score > best then save_data("highscore", tostring(score)) end
```

### Settings page

Every game gets the engine's graphics settings page (display mode and window size, vsync,
anti-aliasing, render resolution and FSR upscaling, ray-traced effects, FPS readout). The
player's choices are saved per game and win over the scene's post-process volumes.
Escape opens and closes it.

- `open_settings()` / `close_settings()` - show or hide it, e.g. from a pause menu
- `settings_open()` → `bool` - true while it shows (pause the game then if it should)
- `set_settings_hotkey(enabled)` - turn Escape's toggle off for a game that uses Escape
  itself (the page stays available through `open_settings()`)
- `settings_hotkey()` → `bool` - whether Escape toggles it; save it in `on_created` and
  restore it in `on_destroyed`, so the editor gets its Escape back when play stops
- `path_traced()` → `bool` - true while the player has path tracing on and the GPU can
  run it. Path tracing lights the scene from emissive surfaces and bounces light by
  itself, so a game can drop lighting it fakes for the raster renderer (glow decals, fill
  lights) and make more things glow; check it every frame, since the player can switch
  it on the settings page at any time
- `ray_traced_gi()` → `bool` - true while ray-traced global illumination lights the
  scene: the player has it on, the GPU can run it and path tracing (which wins) is off.
  Bounce light, glowing surfaces and the sky through windows and doors then reach the
  scene traced and occluded, while the raster renderer keeps the direct light; fill
  lights faked for the raster lighting can go, as for `path_traced()`

- `graphics_settings()` → `table` - the settings as the page shows them: `anti_aliasing`,
  `render_scale`, `upscaler`, `sharpness`, `ray_tracing` (the master switch: off, every
  effect takes its raster version and light probes show their bakes), `ray_traced_shadows`,
  `reflections` and
  `ambient_occlusion` (`"off"`, `"screen_space"`, `"ray_traced"`), `ray_traced_refraction`,
  `global_illumination`, `path_tracing` (and its `path_tracing_*` options), `vsync`,
  `display_mode`, `show_fps`, plus `capabilities` (`ray_tracing`, `path_tracing`,
  `upscaling`): what the GPU can run
- `set_graphics_settings(fields[, { save = false }])` - change the named settings as the
  page would (an unknown name or value is an error). They are saved as the player's choice
  unless `save = false`, which keeps the change to this run: a benchmark trying
  configurations, or a show that wants raster lighting, sets the player's own settings back
  when it is done

```lua
function update(self, dt)
    if settings_open() then
        return -- the game waits while the player is in the settings
    end
    -- ...
end

-- A benchmark pass at half resolution, then the player's own settings again.
local before = graphics_settings()
before.capabilities = nil
set_graphics_settings({ render_scale = 0.5, upscaler = "fsr", vsync = false }, { save = false })
-- ... measure ...
set_graphics_settings(before, { save = false })
```

### Frame statistics
- `frame_stats()` → `table` - what the last complete frame cost, for a performance overlay
  or a benchmark: `frame_ms` and `fps` (wall time, including waiting for the display),
  `cpu` (milliseconds per stage of the frame loop: `scripts`, `animations`, `transforms`,
  `render_collect`, `render_gpu`, `gpu_wait`, `present`, ...), `gpu_ms` and `gpu` (GPU
  milliseconds per render stage from timestamp queries, a frame or two old; `nil` without
  them, as in browsers), `draw_calls`, `instances`, `culled_instances`,
  `skinned_instances`, `skinned_vertices` and `entities`

```lua
function on_ui(self, ui)
    local s = frame_stats()
    ui:screen_label("top_left", 16, 12, string.format("%.0f fps  GPU %.1f ms",
        s.fps, s.gpu_ms or 0), { backdrop = true })
end
```

### Modules
- `require(name)` - Load a module from another file and return what it returns (usually a
  table). It runs once per script; later calls return the same value. The module sees the
  scripting API, and its own globals stay in the module. Modules can `require` other
  modules: they share the script's cache, so a module required from several places is one
  instance (a good home for state they all use).
- `include(name)` - Run another file inside this script, as if its text were pasted here:
  the functions and globals it defines become this script's. Runs on every call.

`name` is a path without `.lua` (`"enemies"`, `"ai.autopilot"` or `"ai/autopilot"`), looked
up next to the script first, then under the scripts root, also when a module requires
another. Editing a required or included file hot-reloads every script that uses it.
Modules under `engine.` come with the engine when the project has no file of that name:
`engine.third_person` (see "Third-person controller").

```lua
-- content/scripts/game.lua
local waves = require("waves")       -- content/scripts/waves.lua: return { {grunts = 15}, ... }
include("autopilot")                 -- defines autopilot_update() in this script
```

A large game splits well into a module per subsystem: a state module (`game/state.lua`
returns the shared tables) that the others require, and the script itself keeping the hooks
(`on_created`, `update`, `on_ui`) and the globals you inspect with `lua_eval` (`GAME`, test
hooks), since a module's globals are not the script's. Two modules cannot require each
other at load time (the error says `require cycle`); move what both need into a third, or
call `require` inside the function that needs it.

Modules should only define things when loaded. Spawn entities and play sounds from functions
the script calls in its hooks.

### Audio
- `play_sound(path[, {volume = 1, pitch = 1, loop = false}])` → `id` - Play a wav, ogg,
  mp3 or flac file. Paths are relative to the project (`"content/audio/hit.wav"`) or
  absolute. `pitch` is the playback speed.
- `stop_sound(id)` - Stop one sound
- `stop_all_sounds()` - Stop everything
- `set_master_volume(volume)` - Gain applied to every sound

Sounds stop when play mode ends. Without an audio device sounds are silent but still
recorded, so automated tests can check what played (engine tool `audio_status`).

In web builds, files are fetched relative to the page and decoded by the browser. The
first play of a file starts once it has loaded, and a missing file is logged instead of
raising an error. Browsers hold audio back until the player first clicks or presses a
key; sounds played before that start then.

### Processes
Talk to another program over its stdin and stdout a line at a time, without blocking the
frame: a UCI chess engine, a solver, a simulation. Native builds only (on the web
`process_spawn` returns `nil` and a message). The program must be inside the working
directory or the open project; it runs directly, with no shell. Every process is killed
when play mode stops; kill your own in `on_destroyed` so hot reloads do not leave one
behind.

- `process_spawn(path[, args])` → `id`, or `nil, message` when it cannot start. Relative
  paths are tried against the working directory, then the project and its `content/`.
- `process_write(id, line)` → `boolean` - send a line (the newline is added)
- `process_read_lines(id)` → `{line, ...}` - the lines it printed since the last call
- `process_running(id)` → `boolean`
- `process_kill(id)` → `boolean`

```lua
local engine
function on_created(self)
    engine = process_spawn("target/chess-uci/release/xavchess-hce")
    process_write(engine, "uci")
end
function update(self, dt)
    for _, line in ipairs(process_read_lines(engine)) do
        if line == "uciok" then process_write(engine, "isready") end
    end
end
function on_destroyed(self)
    process_kill(engine)
end
```

See `games/chess/study/content/scripts/chess_uci.lua` for a complete UCI session.

### Script Access
- `get_entity_scripts(entity)` - List script descriptors for an entity
- `read_script_source(entity)` - Read script contents (inline or file-backed)
- `reload_script(entity)` - Re-read script contents for change detection

Script descriptors include:
- `kind` (`"inline"` or `"file"`)
- `name` (inline only)
- `path` (file only)
- `source` (inline only)

### Transform API
- `translate(entity, x, y, z)` - Move entity by delta
- `set_translation(entity, x, y, z)` - Set entity position
- `get_world_translation(entity)` - Get entity position (returns table with `x`, `y`, `z` fields)
- `rotate(entity, axis_x, axis_y, axis_z, angle)` - Rotate around axis
- `set_rotation(entity, yaw, pitch, roll)` - Set rotation (Euler angles in radians)
- `get_world_rotation(entity)` - Get rotation (returns table with `yaw`, `pitch`, `roll` fields, radians)
- `set_scale(entity, x, y, z)` - Set entity scale
- `look_at(entity, target_x, target_y, target_z)` - Point -Z towards the target while keeping the horizon level in a Y-up world. A target at the entity's position leaves its rotation unchanged.
  Cameras retain the supplied focus point for view and directional-shadow placement.
  If later movement or rotation points elsewhere, the old focus hint is ignored.
- `get_forward(entity)`, `get_right(entity)`, `get_up(entity)` - World-space unit vectors
  (tables with `x`, `y`, `z`) for the way the entity faces (-Z), its +X and its +Y. Use them
  instead of deriving directions from yaw: `local f = get_forward(ship)`, then
  `translate(ship, f.x * speed * dt, 0, f.z * speed * dt)`.

### Hierarchy API
Visibility is inherited: an entity is drawn only if it and all its ancestors are visible,
so `set_component(root, "Visible", false)` hides a whole actor.

- `set_parent(entity, parent)` - Set parent entity (or nil to unparent)
- `get_parent(entity)` - Get parent entity (returns number or nil)
- `get_children(entity)` - Get child entities (returns table array)

### Input API
- `is_key_pressed(key)` - Check if key is held down
- `is_key_just_pressed(key)` - Check if key was just pressed this frame
- `is_key_just_released(key)` - Check if key was just released this frame
- `is_mouse_button_pressed(button)` - Check if mouse button is held (0=left, 1=right, 2=middle)
- `is_mouse_button_just_pressed(button)` - Check if mouse button was just pressed
- `is_mouse_button_just_released(button)` - Check if mouse button was just released
- `get_mouse_position()` - Get mouse position (returns table with x, y)
- `get_mouse_delta()` - Get mouse movement delta (returns table with x, y)
- `get_mouse_scroll_delta()` - Get scroll wheel delta (returns table with x, y)

### View API
Conversions between the game view and the world, in the same window-pixel coordinates
`get_mouse_position()` reports (physical pixels; in the editor the game view is a part of
the window). They use the active camera and the area the renderer last drew into, and
return `nil` before the first frame is drawn.

- `get_viewport_size()` - `{x, y, width, height}` of the game view in window pixels
- `world_to_screen(x, y, z)` - `{x, y, on_screen}` in window pixels, or `nil` when the
  point is behind the camera
- `screen_to_world_ray(x, y)` - `{ox, oy, oz, dx, dy, dz}`: the ray's start on the near
  plane and its unit direction
- `screen_to_ground(x, y[, height])` - `{x, y, z}` where that ray meets the plane
  `y = height` (default 0), or `nil` when it never does

```lua
-- Aim a ship at the mouse on a flat arena
local mouse = get_mouse_position()
local target = screen_to_ground(mouse.x, mouse.y, 0)
if target then look_at(ship, target.x, 0, target.z) end
```

### Component API
- `has_component(entity, component_name)` - Check if entity has component
- `get_component(entity, component_name)` - Get component data (returns table or nil).
  Vectors in component tables are **arrays**, not `{x, y, z}` tables: read
  `t.translation[1]`, not `t.translation.x`
- `set_component(entity, component_name, value)` - Set component data
- `add_component(entity, component_name, value)` - Add component to entity
- `remove_component(entity, component_name)` - Remove component from entity

#### Supported component names

**Add/set-supported components**
- `Name` (string)
- `Visible` (boolean)
- `CanCastShadow` (boolean)
- `RotateAnimation` (`{ axis = {x, y, z}, speed = number }`)
- `OrbitAnimation` (`{ center = {x, y, z}, radius = number, speed = number, offset = number }`)
- `RigidBody`, `Collider`, `CharacterController`, `SpringCamera` (see Physics; a table
  sets only the fields it holds, so `set_component(e, "Collider", { friction = 0.1 })`
  keeps the shape)

**Readable/removable components**
- `Transform` or `TransformComponent` (`{ translation = {x, y, z}, rotation = {x, y, z, w}, scale = {x, y, z} }`,
  each an array: `translation[1..3]`, `rotation[1..4]` (quaternion), `scale[1..3]`)
- `Camera` or `CameraComponent` (`{ projection = { kind = "perspective" | "orthographic", ... } }`)
- `PointLight` (`{ color = {x, y, z}, intensity = number, range = number }`)
- `DirectionalLight` (`{ color = {x, y, z}, intensity = number, shadow_size = number }`)
- `SpotLight` (`{ color = {x, y, z}, intensity = number, inner_angle = number, outer_angle = number, range = number }`)
- `Mesh` or `MeshComponent` (`{ index = number }`)
- `Material` or `MaterialComponent` (`{ index = number }`)
- `PrimitiveMeshComponent` (`{ descriptor = ... }`)
- `ParticleEmitterComponent` (`{ spawn_rate = number, ... }`)

### Query API
- `query_entities_with_component(component_name)` - Find all entities with component
- `get_entities_in_radius(x, y, z, radius)` - Find entities within radius
- `get_nearest_entity(x, y, z)` - Find nearest entity to point
- `get_nearest_entity_with_component(x, y, z, component_name)` - Find nearest entity with component
- `get_entities_in_box(min, max)` - Find entities in axis-aligned box

**Supported component names for queries**
- `Name`
- `Visible`
- `CanCastShadow`
- `RotateAnimation`
- `OrbitAnimation`
- `Transform` or `TransformComponent`
- `Camera` or `CameraComponent`
- `PointLight`
- `DirectionalLight`
- `SpotLight`
- `Mesh` or `MeshComponent`
- `Material` or `MaterialComponent`
- `PrimitiveMeshComponent`
- `ParticleEmitterComponent`

### Events API
- `emit_event(event_name, data)` - Emit custom event with data
- `subscribe_event(event_name, callback_name)` - Subscribe to event
- `unsubscribe_event(event_name)` - Unsubscribe from event

### File I/O API (Sandboxed)
Paths are relative to the engine's working directory, not the project; use `save_data` /
`load_data` for values a game keeps between runs.

- `read_file(path)` - Read text file (only from `scripts/` or `examples/scripts/`)
- `write_file(path, contents)` - Write text file (only to allowed directories)
- `file_exists(path)` - Check if file exists
- `list_files(dir_path)` - List files in directory

### Clipboard API
- `get_clipboard()` - Get clipboard text (always returns empty string for security)
- `set_clipboard(text)` - Copy text to clipboard

### UI API

The UI API provides egui widgets for editor panels and in-game HUDs. UI is only available
in the `on_ui(self_entity, ui)` hook. It works in the editor and in the standalone player;
top-level `anchored_panel`s are drawn as overlays on the game view, everything else goes
into a window titled after the entity.

Text uses egui's bundled default fonts. Glyph coverage is limited; unsupported
characters render as missing-glyph boxes even when the text API accepts the string.
Use ASCII for portable health bars and map markers. The HUD API currently has no
project-font loading option.

**Note**: All UI widgets are methods called on the `ui` parameter passed to `on_ui()`.

#### Basic Widgets

- `ui:label(text[, {color = {r, g, b, a}, size = 16}])` - Display static text label; colors
  are 0..1
- `ui:heading(text[, {color, size}])` - Display heading text (larger/bold unless `size` is given)
- `ui:separator()` - Display horizontal separator line

#### Interactive Widgets

- `ui:button(text, [id])` → `boolean` - Display button, returns true if clicked.
  Buttons are identified by their label; pass `id` when the label is not unique or changes
- `ui:checkbox(id, current_value, label)` → `boolean` - Display checkbox, returns new value
- `ui:text_edit(id, current_value)` → `string` - Single-line text input, returns new value
- `ui:text_edit_multiline(id, current_value, width, height)` → `string` - Multi-line text editor, returns new value
- `ui:slider(id, current_value, min, max)` → `number` - Horizontal slider, returns new value
- `ui:drag_value(id, current_value)` → `number` - Draggable number widget, returns new value
- `ui:color_edit(id, r, g, b)` → `table{r, g, b}` - Color picker, returns table with RGB values (0.0-1.0)

#### Layout and HUD

- `ui:anchored_panel(id, anchor, offset_x, offset_y, width, height, callback)` - HUD overlay
  pinned to the game view. `anchor` is one of `top_left`, `top_center`, `top_right`,
  `center_left`, `center`, `center_right`, `bottom_left`, `bottom_center`, `bottom_right`;
  offsets are logical points from that anchor; `width`/`height` may be `nil` to fit the
  content. `callback(panel)` builds the contents with the same widget methods.
- `ui:screen_label(anchor, offset_x, offset_y, text[, {color, size, backdrop = false}])` -
  Text pinned to the game view (same anchors and inset offsets as `anchored_panel`) with no
  panel around it: big titles, wave banners, "GAME OVER".

Offsets are **insets**: positive values always move the text or panel inward from its
anchor, whichever corner it is. Never pass negative offsets to reach a right or bottom
edge; that pushes the text off screen.

```lua
ui:screen_label("top_left", 16, 12, "SCORE 1200")      -- 16 right, 12 down from the top left
ui:screen_label("top_right", 16, 12, "LIVES 3")        -- 16 left, 12 down from the top right
ui:screen_label("bottom_left", 16, 12, "WAVE 4")       -- 16 right, 12 up from the bottom left
ui:screen_label("bottom_right", 16, 12, "x2")          -- 16 left, 12 up from the bottom right
ui:screen_label("center", 0, 0, "GAME OVER", { size = 64 })
```
- `ui:world_label(x, y, z, text[, {color = {r, g, b, a}, size = 16, backdrop = false}])` →
  `boolean` - Text drawn over the game view at a world position (floating score, names).
  Returns `false` when the point is behind the camera or off screen. Labels are not hidden
  by geometry.
- `ui:horizontal(callback)` / `ui:horizontal_wrapped(callback)` - Lay out widgets in a row
- `ui:centered_area(width, callback)` - Center a block horizontally (`width` 0 to fit)
- `ui:menu_bar(callback)`, `ui:menu(text, callback)`, `ui:menu_item(id, text)` → `boolean` - Menus
- `ui:get_viewport_size()` → `table{width, height}` - Size of the area the UI is drawn into
- `ui:get_pixels_per_point()` → `number` - UI scale factor

```lua
function on_ui(self_entity, ui)
    ui:anchored_panel("score", "top_left", 16, 16, nil, nil, function(panel)
        panel:heading("Score: " .. get_i32("score", 0))
    end)
end
```

See `examples/scripts/ingame_hud.lua` for a complete HUD.

#### Widget IDs and Values

Interactive widgets are identified by their `id` (or label, for buttons). If the same id
appears more than once in a frame, later occurrences are numbered automatically, so ids
stay unique as long as the UI is built in the same order each frame.

Clicks and edits are reported on the **next** frame. Value widgets return the user's
edit on the frame after it happens and otherwise return the value you passed in, so your
script owns the value and can change it at any time:

```lua
local name = get_string("name", "")
name = ui:text_edit("player_name", name)
if ui:button("Clear") then
    name = ""
end
set_string("name", name)
```

#### UI Example

```lua
-- @editor
function on_created(self_entity)
    set_state("counter", 0)
    set_state("text", "Hello!")
    set_state("enabled", true)
    set_state("slider_val", 0.5)
end

function on_ui(self_entity, ui)
    ui:heading("My UI Panel")
    ui:label("Control panel for entity")
    ui:separator()

    -- Button
    local count = get_state("counter", 0)
    if ui:button("Increment") then
        count = count + 1
        set_state("counter", count)
    end
    ui:label(string.format("Count: %d", count))

    -- Checkbox
    local enabled = get_state("enabled", true)
    enabled = ui:checkbox("enable_id", enabled, "Enable feature")
    set_state("enabled", enabled)

    -- Text edit
    local text = get_state("text", "")
    text = ui:text_edit("text_id", text)
    set_state("text", text)

    -- Slider
    local value = get_state("slider_val", 0.5)
    value = ui:slider("slider_id", value, 0.0, 1.0)
    set_state("slider_val", value)
    ui:label(string.format("Value: %.2f", value))
end
```

See `ui_example_simple.lua`, `ui_example_comprehensive.lua`, and `ui_transform_controller.lua` for more examples.

## Example Scripts

### Basic Cube Rotation
See `editor_cube_spin.lua` for a simple rotation example with state management.

### Fractal Cube Animation
See `fractal_cube_rotate.lua` for complex rotation with axis control.

### Event System
- `event_emitter.lua` - Demonstrates event emission
- `event_listener.lua` - Demonstrates event subscription and handling

### Comprehensive API Test
See `lua_api_test.lua` for a test of all API categories.

### Widget Showcase
See `ui_example_comprehensive.lua` for every available UI widget.

## Lifecycle Hooks

Scripts can implement these lifecycle functions:

```lua
function on_created(self_entity, previous)
    -- Called once when the script starts (and again after a hot reload). `previous` is
    -- what on_destroyed returned before the reload, or nil.
end

function update(self_entity, dt)
    -- Called every frame; dt is delta time in seconds
end

function on_ui(self_entity, ui)
    -- Called every frame to build UI
end

function on_destroyed(self_entity, reason)
    -- Called when the script component or its entity is removed, and before a hot
    -- reload. `reason` is "removed" or "reload". Despawn what on_created spawned here.
    -- Before a reload, whatever it returns is handed to the new code's on_created.
end

-- Physics callbacks (play mode), delivered at the start of the next frame, before
-- update. Define only the ones you need.
function on_collision_enter(self_entity, other) end -- also on_collision_exit
function on_trigger_enter(self_entity, other) end   -- sensors; also on_trigger_exit
function on_jumped(self_entity) end                 -- CharacterController only
function on_landed(self_entity, speed) end          -- downward speed at impact (m/s)
```

A script that creates its own entities (bullet pools, effects) should remove them in
`on_destroyed`; otherwise every hot reload runs `on_created` again and leaves a second set
behind. Either despawn them, or have `on_created` look the entities up by name
(`find_entity_by_name`) before spawning.

```lua
local pool = {}

function on_created(self_entity)
    for i = 1, 20 do
        pool[i] = spawn_primitive("sphere", "Bullet" .. i)
    end
end

function on_destroyed(self_entity, reason)
    for _, bullet in ipairs(pool) do
        despawn_entity(bullet)
    end
    pool = {}
end
```

Top-level code runs once when the script is loaded, with the full API available.

When a script file changes on disk it is hot-reloaded: `on_destroyed(self, "reload")` runs
on the old code at the start of the next frame, then the new code is compiled,
`on_created` runs again, and values stored with the state API (`set_f64`, `set_state`, ...)
are restored afterwards.

Script locals reset on a reload. To keep a game going while you edit it (instead of
restarting the demo at wave 1), return the state from `on_destroyed` and take it back in
`on_created`. Any Lua value works, including tables with functions; entering or leaving
play mode always starts fresh.

Spawned handles kept in that state remain valid across a hot reload: the replacement
script retains its handle registry, including the allocation counter. These handles
are scoped to that script; use entity IDs or unique names to share references with
other scripts. A queued reload needs a simulation update. In a frozen manual session,
advance with `runtime_step`; waiting for rendered frames does not apply the reload.

```lua
function on_created(self_entity, previous)
    GAME = previous or new_game()
end

function on_destroyed(self_entity, reason)
    if reason == "reload" then return GAME end
end
``` Errors are logged once per distinct message and shown in the
editor; one failing script never stops the others.

### Coroutines

```lua
function on_created(self_entity)
    coroutine_create(function()
        wait(2.0)        -- resume after two seconds
        wait_frames(10)  -- resume after ten frames
        log_info("done")
    end)
end
```

Coroutines are started and resumed by the engine. `wait` and `wait_frames` raise an error
when called outside a coroutine.

### Events

Event handlers run after the hook that emitted the event, in the same frame. At most 4096
events are dispatched per frame; anything beyond that is dropped with a warning, which stops
handlers that re-emit their own event from hanging the engine.

## Script Modes

Scripts declare where they run with an annotation in their leading comments:

| Annotation | Runs in |
|------------|---------|
| *(none)*   | Play mode only |
| `-- @editor` | Editor mode only |
| `-- @tool` | Editor and play mode |

`@editor` and `@tool` scripts receive `update` every frame while editing.

## Known Limitations

1. Scripts cannot load glTF meshes other than through `import_gltf`; create those in
   the editor (or through the engine tools). Meshes are limited to the built-in
   primitives.
2. Scripts cannot read material values back; keep the colors you set in script state.
3. `ui:world_label` text is drawn on top of the scene, not occluded by it.

## Best Practices

1. **Always provide default values** when using `get_state()` to handle first-time access
2. **Use typed state functions** (`set_f64`, `set_bool`, etc.) for better performance with simple types
3. **Clean up subscriptions** using `unsubscribe_event()` when no longer needed
4. **Test with `lua_api_test.lua`** to verify API functionality
5. **Check for nil returns** when using functions that may fail (e.g., `find_entity_by_name`)
