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
- `set_name(entity, name)` - Set entity name
- `find_entity_by_name(name)` - Find entity by name
- `attach_inline_script(entity, name, source)` - Attach inline Lua script
- `attach_script(entity, path)` - Attach script from file
- `import_gltf(entity, path, scale)` - Import GLTF model

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
  mesh (`model.parts[i].mesh` of a `voxel_model`), or `nil` to remove it
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
  scripts spawn). `kind` adds a light or switches its kind.
- `spawn_camera([fields][, name])` → `entity` - Spawn a camera
- `set_camera(entity, fields)` - Change a camera, adding one if needed. Fields:
  `projection` (`"perspective"`/`"orthographic"`), `fov_deg`, `near`, `far`, `left`,
  `right`, `bottom`, `top`, and `active = true` to view the game through it
- `set_active_camera(entity)` - View the game through the entity's camera

Lights and cameras sit at their entity's position and point along its -Z, so
`set_translation` and `look_at` place and aim them (also right after spawning).
Unknown field names are an error. Like other changes, they apply before the frame is
drawn and are undone when play mode stops.

```lua
local flash = spawn_light("point", { color = {1, 0.6, 0.2}, intensity = 40, range = 6 })
set_translation(flash, x, y, z)
local cam = spawn_camera({ fov_deg = 50, active = true }, "Cutscene")
set_translation(cam, 0, 4, 8)
look_at(cam, 0, 0, 0)
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
