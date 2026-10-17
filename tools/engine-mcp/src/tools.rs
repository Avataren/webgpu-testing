//! MCP tool definitions. Most tools forward to one engine method; a few (launching,
//! building, reading engine output, the Lua reference) run in this server.

use serde_json::{json, Value};

/// How a tool is executed.
pub enum Action {
    /// Forward the arguments to this engine method.
    Engine(&'static str),
    /// Handled by the server itself.
    Local,
}

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
    pub action: Action,
}

impl Tool {
    /// Schemas for stable query reports; flexible editing reports remain open objects.
    pub fn output_schema(&self) -> Value {
        let (properties, required) = match self.name {
            "runtime_step" => (
                json!({"steps":{"type":"integer"},"dt":{"type":"number"},"simulation_steps":{"type":"integer"},"game_time":{"type":"number"},"manual":{"type":"boolean"}}),
                vec!["steps", "dt", "simulation_steps", "game_time", "manual"],
            ),
            "scene_raycast" => (
                json!({"total":{"type":"integer"},"hits":{"type":"array","items":{"type":"object"}},"precision":{"type":"string"}}),
                vec!["total", "hits", "precision"],
            ),
            "scene_query" => (
                json!({"total":{"type":"integer"},"offset":{"type":"integer"},"entities":{"type":"array","items":{"type":"object"}},"precision":{"type":"string"}}),
                vec!["total", "offset", "entities", "precision"],
            ),
            "project_validate" => (
                json!({"ok":{"type":"boolean"},"project":{"type":"string"},"files_checked":{"type":"integer"},"errors":{"type":"array","items":{"type":"object"}},"warnings":{"type":"array","items":{"type":"object"}},"browser":{"type":"object","description":"Browser preflight: ok, errors, warnings"}}),
                vec!["ok", "project", "files_checked", "errors", "warnings"],
            ),
            "profile_capture" => (
                json!({"samples":{"type":"integer"},"frame_time_ms":{"type":"object"},"stages_ms":{"type":"object"},"gpu_total_ms":{"type":["object","null"]},"gpu_stages_ms":{"type":"object"},"renderer_counters":{"type":"object"},"growth":{"type":"object"},"slow_scripts":{"type":"array","items":{"type":"object"}}}),
                vec![
                    "samples",
                    "frame_time_ms",
                    "stages_ms",
                    "growth",
                    "slow_scripts",
                ],
            ),
            _ => (json!({}), Vec::new()),
        };
        object(properties, &required)
    }
    /// MCP hints describe observable behavior, rather than granting permission.
    pub fn annotations(&self) -> Value {
        let read_only = matches!(
            self.name,
            "engine_info"
                | "engine_status"
                | "engine_output"
                | "engine_stats"
                | "lua_api_reference"
                | "scene_list_entities"
                | "scene_tree"
                | "entity_get"
                | "animation_inspect"
                | "assets_list"
                | "script_status"
                | "input_state"
                | "camera_get"
                | "environment_get"
                | "postprocess_get"
                | "texture_fetch"
                | "light_probes_status"
                | "logs"
                | "editor_state"
                | "audio_status"
                | "sound_inspect"
                | "scene_query"
                | "scene_raycast"
                | "prefab_inspect"
                | "project_validate"
                | "profile_capture"
                | "visual_compare"
                | "engine_profile_cpu"
                | "renderdoc_capture"
        );
        json!({"readOnlyHint": read_only, "destructiveHint": !read_only,
            "openWorldHint": matches!(self.name, "web_playtest" | "engine_call"),
            "idempotentHint": read_only && !matches!(self.name, "profile_capture" | "engine_profile_cpu" | "renderdoc_capture")})
    }
}

fn entity_ref() -> Value {
    json!({
        "anyOf": [{ "type": "integer" }, { "type": "string" }],
        "description": "Entity id (the 64-bit id Lua scripts also use) or its unique name"
    })
}

fn vec3(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": 3,
        "maxItems": 3,
        "description": description
    })
}

fn color(description: &str) -> Value {
    json!({
        "anyOf": [
            { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 4 },
            { "type": "string" }
        ],
        "description": format!("{description}: [r, g, b] or [r, g, b, a] in 0..1, or \"#rrggbb\"")
    })
}

fn nullable_object(description: &str, properties: Value) -> Value {
    json!({
        "anyOf": [{ "type": "object", "properties": properties }, { "type": "null" }],
        "description": description
    })
}

/// Fields shared by entity_spawn and entity_update.
fn entity_fields() -> serde_json::Map<String, Value> {
    let material = json!({
        "type": "object",
        "description": "PBR material. Only the given fields change. Shared materials are copied first unless edit_shared is true.",
        "properties": {
            "color": color("Base color"),
            "metallic": { "type": "number", "minimum": 0, "maximum": 1 },
            "roughness": { "type": "number", "minimum": 0, "maximum": 1 },
            "emissive": { "type": "number", "minimum": 0, "maximum": 64, "description": "Emission strength: the surface glows in its own base color. 1 is full color; above 1 is HDR, which is what makes bloom pick a lit surface up (try 2-5)" },
            "transmission": { "type": "number", "minimum": 0, "maximum": 1, "description": "Light passing through the surface: glass, gems, water (1 = clear). Refracted by ior and tinted by the color; ray traced where engine_info surface.ray_tracing is available, otherwise drawn see-through. Use low roughness and metallic 0" },
            "ior": { "type": "number", "minimum": 1, "maximum": 3.55, "description": "Index of refraction for transmissive materials: 1.33 water, 1.5 glass (default), 1.77 sapphire, 2.42 diamond" },
            "dispersion": { "type": "number", "minimum": 0, "maximum": 2.55, "description": "Transmissive materials: how much refraction splits light into colours (gem fire, prism rainbows), as glTF KHR_materials_dispersion (20 / Abbe number): 0 none, 0.34 crown glass, 0.36 diamond; exaggerate up to 1-2 for a showier effect" },
            "unlit": { "type": "boolean" },
            "transparent": { "type": "boolean", "description": "Alpha blending; set the color's alpha below 1" },
            "alpha_cutoff": { "type": "number", "minimum": 0, "maximum": 1, "description": "Cut-out (glTF MASK): the surface is cut away where its alpha (color alpha times the base color texture's) is below this, and is otherwise opaque, with depth and shadows (foliage, fences). 0 turns it off" },
            "double_sided": { "type": "boolean" },
            "nearest_filtering": { "type": "boolean", "description": "Pixel-art style texture sampling" },
            "base_color_texture": { "type": ["string", "null"], "description": "Image path (png/jpg), or null to remove" },
            "normal_texture": { "type": ["string", "null"] },
            "metallic_roughness_texture": { "type": ["string", "null"] },
            "emissive_texture": { "type": ["string", "null"] },
            "occlusion_texture": { "type": ["string", "null"] },
            "edit_shared": { "type": "boolean", "description": "Edit a material shared with other entities in place" }
        }
    });
    let light_common = json!({
        "color": color("Light color"),
        "intensity": { "type": "number" }
    });
    let mut point = light_common.clone();
    point["range"] = json!({ "type": "number", "description": "Where the light fades out, meters (0: no range; it is cut off where it gets too dim to matter)" });
    point["shadow_priority"] = json!({ "type": "number", "description": "Weight in the shadow budget (default 1): lights are ranked by this x their size on screen x brightness for shadow map tiles; higher keeps this light's shadows (and their resolution) ahead of others, 0 gives them up first" });
    let mut spot = point.clone();
    spot["inner_angle_deg"] = json!({ "type": "number" });
    spot["outer_angle_deg"] = json!({ "type": "number" });
    let mut directional = light_common;
    directional["shadow_distance"] = json!({ "type": "number", "description": "How far from the camera (meters of view depth, default 150) the light's shadows reach, in cascaded shadow maps that are sharp near the camera and coarser further out; they fade out toward it. The cascades shrink to what is in view when that is nearer, so most scenes need no tuning; a shorter distance sharpens every cascade" });

    let animator = json!({
        "libraries": { "type": "array", "items": { "type": "string" }, "description": "Replace the clip library files (project-relative glTF/GLB paths)" },
        "add_clips": { "anyOf": [{ "type": "string" }, { "type": "array", "items": { "type": "string" } }], "description": "Add clip library file(s); a file with one clip plays under its file name" },
        "default_clip": { "type": ["string", "null"], "description": "Clip that starts (looping) when play begins" },
        "play": { "anyOf": [{ "type": "string" }, { "type": "object", "properties": {
            "clip": { "type": "string" },
            "fade": { "type": "number", "description": "Cross-fade seconds (default 0.2)" },
            "loop": { "type": "boolean", "description": "Default true" },
            "speed": { "type": "number", "description": "Rate of this clip; negative plays backwards" },
            "layer": { "type": "integer", "description": "0 = base pose; higher layers override (see layer)" },
            "start": { "type": "number", "description": "Start time in seconds" }
        }, "required": ["clip"] }], "description": "Cross-fade to a clip (play mode)" },
        "stop": { "anyOf": [{ "type": "boolean" }, { "type": "object", "properties": { "layer": { "type": "integer" }, "fade": { "type": "number" } } }], "description": "Fade out a layer (default all) back to the rest pose" },
        "speed": { "type": "number", "description": "Multiplies every clip's speed; 0 pauses" },
        "root_motion": { "type": "string", "enum": ["keep", "in_place", "apply"], "description": "Horizontal travel of the root bone: keep it, remove it (walk on the spot), or apply it to the animator entity so the character moves without foot sliding" },
        "layer": { "type": "object", "description": "Set a layer's weight and bone mask", "properties": {
            "index": { "type": "integer" },
            "weight": { "type": "number", "minimum": 0, "maximum": 1 },
            "fade": { "type": "number" },
            "mask": { "type": ["string", "null"], "description": "Only this bone and the bones below it (e.g. \"Spine\" for the upper body); null clears" }
        }, "required": ["index"] }
    });

    let fields = json!({
        "name": { "type": "string" },
        "translation": vec3("Local position [x, y, z] in meters (alias: position)"),
        "rotation": vec3("Local rotation as Euler degrees [x, y, z], applied yaw(Y) then pitch(X) then roll(Z)"),
        "rotation_quat": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "Rotation as quaternion [x, y, z, w]" },
        "scale": { "anyOf": [vec3("Scale"), { "type": "number" }], "description": "[x, y, z] or a uniform number" },
        "look_at": vec3("Rotate so -Z (forward) points at this world point"),
        "parent": { "anyOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }], "description": "New parent entity, or null to unparent. The local transform is kept." },
        "visible": { "type": "boolean" },
        "cast_shadow": { "type": "boolean", "description": "Lights: whether the light casts shadows. Meshes: whether the mesh shows up in shadow maps (default true)" },
        "mesh": { "type": ["string", "null"], "enum": ["cube", "sphere", "plane", "cylinder", "cone", "torus", null], "description": "Replace the geometry with a primitive, or null to remove" },
        "material": material,
        "point_light": nullable_object("Add/update a point light, or null to remove. Any number of point and spot lights light the scene (clustered; see engine_info limits); with cast_shadow they share a shadow budget ranked by shadow_priority, size on screen and brightness (entity_get shows a light's shadow state).", point),
        "spot_light": nullable_object("Add/update a spot light (shines along -Z), or null to remove. Lit and shadowed like point lights (see point_light).", spot),
        "directional_light": nullable_object("Add/update a directional light (shines along -Z), or null to remove. Renderer uses the first 4 directional lights in scene/world order.", directional),
        "camera": nullable_object("Add/update a camera component, or null to remove", json!({
            "projection": { "type": "string", "enum": ["perspective", "orthographic"] },
            "fov_deg": { "type": "number" },
            "near": { "type": "number" },
            "far": { "type": "number" }
        })),
        "active_camera": { "type": "boolean", "description": "true makes this camera the one the game renders from" },
        "script": nullable_object("Attach a Lua script (replaces any existing one; it recompiles next frame), or null to remove. Prefer files in the project's scripts folder.", json!({
            "path": { "type": "string", "description": "Path to a .lua file (absolute, or relative to the repo/project)" },
            "source": { "type": "string", "description": "Inline Lua source instead of a file" },
            "name": { "type": "string", "description": "Name for inline scripts" }
        })),
        "particle_system": { "type": ["object", "null"], "description": "GPU particle system settings (fields as entity_get shows them, plus \"preset\": physics|starfield|boids|optimized_boids), or null to remove" },
        "particle_emitter": { "type": ["object", "null"], "description": "Particle emitter settings as entity_get shows them, or null" },
        "environment": { "type": ["object", "null"], "description": "Environment component (see environment_get for fields), or null" },
        "post_process_volume": { "type": ["object", "null"], "description": "Post-process volume, or null to remove: {enabled, shape: {kind: infinite} | {kind: box, size: [x,y,z]} | {kind: sphere, radius}, priority, weight, blend_distance, overrides: [setting paths] or null for all, settings: {...as postprocess_set}}. A volume spawned into a scene that already has one overrides only the settings given. Infinite volumes apply everywhere; bounded ones inside their shape (following the entity's transform) and fade out over blend_distance. Only given fields change." },
        "light_probe_volume": { "type": ["object", "null"], "description": "Light probe grid for indirect diffuse light (GI), or null to remove: {enabled, size: [x,y,z] box in metres centred on the entity (scales and turns with it), spacing (m, default 0.5), rays_per_probe (32-256, default 128), hysteresis (0-0.995, default 0.97), intensity, relocate (default true: probes inside or against geometry move out of it, within 0.45 spacing), priority (default 0: where volumes overlap, the higher priority, then the smaller volume, wins), use_baked}. Surfaces inside take their diffuse ambient from the probes, which see walls, so sky light enters only through openings. Up to 8 enabled volumes are used: one per room, or a finer one inside a big one. Probes update by ray tracing each frame (needs capabilities.ray_tracing); fit the box to the room or level. Inspect with texture_fetch probes.irradiance / probes.distance. Only given fields change." },
        "rigid_body": { "type": ["object", "null"], "description": "Simulated body (play mode), or null to remove: {kind: dynamic|fixed|kinematic (default dynamic), mass (kg, 0 = from collider density), linear_damping, angular_damping, gravity_scale (default 1), lock_rotation, ccd, initial_velocity: [x,y,z], can_sleep}. Its shape comes from the entity's collider (auto if none). Dynamic bodies write their pose back to the transform each play step; kinematic ones follow the transform (moving platforms). Only given fields change." },
        "collider": { "type": ["object", "null"], "description": "Collision shape, or null to remove. On its own the entity is fixed scenery. {shape: {kind: auto} (fits the mesh: primitives exactly, other meshes as triangles for scenery or convex hull for dynamic bodies) | {kind: box, size: [x,y,z]} | {kind: sphere, radius} | {kind: capsule, radius, height} | {kind: cylinder, radius, height} | {kind: mesh} | {kind: convex_hull}, offset: [x,y,z], friction (0.6), restitution (0), density (1), sensor (trigger volume: scripts get on_trigger_enter/exit), include_children (also the meshes of descendants: imported models and levels)}. Sizes are local units, stretched by the entity's scale. Only given fields change." },
        "character_controller": { "type": ["object", "null"], "description": "Walking character (capsule move-and-slide; the entity origin is the feet), or null to remove. Steered from Lua: character_move(e, x, z, run), character_jump(e), character_state(e). Fields: radius (0.3), height (1.8), max_slope (deg, 50), step_height (0.35), snap_distance (0.3), walk_speed (2.2), run_speed (6), acceleration (40), air_control (0.35), gravity (22), jump_height (1.3), air_jumps (0), coyote_time (0.12), jump_buffer (0.15), max_fall_speed (45), face_movement (true), turn_speed (deg/s, 720), yaw_offset (deg; models should face +Z), push_mass (80 kg), ride_platforms (true). Only given fields change." },
        "spring_camera": { "type": ["object", "null"], "description": "Third-person spring-arm camera (give the entity a camera too), or null to remove: {target: entity name (empty = parent), pivot_offset: [0,1.5,0], distance (4.5), min_distance, max_distance, yaw, pitch (deg, -15 looks down), min_pitch, max_pitch, shoulder_offset, mouse_look (true: turns while the cursor is locked or right mouse held), sensitivity (deg/pixel), invert_y, zoom_step (m per wheel notch), follow_lag (s), vertical_lag (s), collision (pull in before scenery), probe_radius, auto_follow (swing behind a moving target, 0 = off)}. Only given fields change." },
        "nav_mesh_surface": { "type": ["object", "null"], "description": "Navmesh built from the scene's static geometry (bake it with navmesh_bake; without a bake it is built when play starts), or null to remove. Floors over floors (bridges, tunnels, balconies) stay separate. Fields: agent_radius (0.4), agent_height (1.8), agent_max_climb (0.45), agent_max_slope (deg, 45), cell_size (0.2), cell_height (0.1), tile_size (cells, 48; obstacles rebuild the tiles they touch), min_region_area (m2, 1), merge_region_area (8), max_edge_length (12), max_edge_error (cells, 1.3), detail_sample_distance (m, 1.2; 0 = no height detail), detail_sample_max_error (0.1), geometry (render_meshes|colliders|both), collect (all|children), bounds ([x,y,z] box around the entity, zero = everything), baked (project path, set by a bake), build_on_start (true), agent_type (\"\"). Only given fields change." },
        "nav_agent": { "type": ["object", "null"], "description": "Walks the navmesh to destinations (Lua nav_set_destination(e, x, y, z), nav_stop, nav_agent_state; callbacks on_nav_arrived/on_nav_failed), steering around other agents (ORCA) and moving obstacles, replanning when the navmesh changes; or null to remove. The entity origin is the feet; models face +Z (else yaw_offset). Moves the transform, or steers a CharacterController when the entity has one (drive auto|transform|character). An AnimationTree with from_character gets its speeds. Fields: radius (0.35), height (1.8), speed (1.5 m/s), acceleration (8), turn_speed (deg/s, 540), stopping_distance (0.15), avoidance (true), priority (0..1, 0.5), separation (1), face_movement (true), yaw_offset, drive, agent_type, destination ([x,y,z] to head for at play start), area_costs ([{area, cost}])." },
        "ai_agent": { "type": ["object", "null"], "description": "Gives the entity a brain, run in play mode (or null to remove): brain (a name a script defines with ai_tree / ai_machine, or ai_define / the ai_define tool, or a project JSON file like content/ai/guard.json), enabled, tick_rate (thinks per second, 10; 0 every frame), tags (what others perceive it as), group (shares the group. blackboard and group events), run_speed (m/s for run = true moves, 3.2), blackboard (starting values; [x,y,z] arrays are positions; merged key by key, null removes), perception {sight_range (0: blind), sight_angle (deg, 120), near_range (1.5), hearing_range (15), memory (s, 5), interests (tags it notices), line_of_sight (navmesh|physics|none), eye_height, rate (looks/s)}. Walking needs a nav_agent too. See docs/ai.md." },
        "nav_obstacle": { "type": ["object", "null"], "description": "Something that moves about on the navmesh (cart, door, crate): agents steer around it while it moves; standing still it carves a hole into the navmesh (tiles rebuilt, paths reroute). Or null to remove. Fields: shape ({kind: box, size: [x,y,z]} | {kind: cylinder, radius, height}; local units), offset, carve (true), carve_only_stationary (true), move_threshold (m, 0.1), stationary_time (s, 0.5)." },
        "nav_link": { "type": ["object", "null"], "description": "Off-mesh link from the entity to end (local offset): a jump down, a gap, a ladder; agents whose path uses it cross in an arc (Lua callbacks on_nav_link_start(self, {start, finish, link}) / on_nav_link_end). Or null to remove. Fields: end ([x,y,z], default [0,0,2]), bidirectional (false: only entity -> end), radius (m, 0.5: how far each end reaches for the floor), area (63)." },
        "nav_modifier": { "type": ["object", "null"], "description": "How an entity's geometry (and its children's, include_children) goes into navmeshes, or null to remove: ignore (leave it out), walkable (false: solid but never walked on, like rooftops), area (1..63; 63 plain ground; agents' area_costs make areas cheaper or dearer)." },
        "animation_tree": { "type": ["object", "null"], "description": "Animation state machine (like Godot's AnimationTree) driving the character's animator (on the entity, the nearest below, or above), or null to remove. {generate: true} builds a locomotion tree from the character's clips by name (idle/walk/run/backpedal/strafes in a 2D blend space at the speeds the clips travel, then Jump/Fall/Land states); put it on the character root. Fields: from_character (default true: the CharacterController feeds speed, forward_speed, side_speed, vertical_speed, grounded, air_time, landing_speed and the triggers jumped and landed), start, parameters {name: number}, states [{name, motion: {kind: clip, clip, speed} | {kind: blend_1d, parameter, points: [{at, clip, speed}], scale_speed} | {kind: blend_2d, x, y, points: [{at: [x, y], clip, speed}], scale_speed}, looping, speed}], transitions [{from (a state or \"*\"), to, when (e.g. \"!grounded && air_time > 0.15\"), fade, after (0..1 share of the state played)}]. entity_get shows playing {state, previous, time, progress, parameters} in play mode. Lua: animation_set/trigger/get/state/travel." },
        "rotate_animation": nullable_object("Built-in spin animation, or null", json!({
            "axis": vec3("Rotation axis"),
            "speed": { "type": "number", "description": "Radians per second" }
        })),
        "orbit_animation": nullable_object("Built-in orbit animation, or null", json!({
            "center": vec3("Orbit center"),
            "radius": { "type": "number" },
            "speed": { "type": "number" },
            "offset": { "type": "number" }
        })),
        "billboard": nullable_object("Make the entity face the camera, or null", json!({
            "orientation": { "type": "string", "enum": ["face_camera", "face_camera_y_axis"] },
            "lit": { "type": "boolean" }
        })),
        "animator": nullable_object("Skeletal animation on a character (skinned glTF imports get one on their root; entity may be the root or any part of it, the nearest animator up the hierarchy is used), or null to remove. Clips come from glTF/GLB files (libraries) and bind to bones by name, so animation-only files (asset_import with kind animation) drive any rig with matching bone names; translations are rescaled to the skeleton's proportions. Playback runs in play mode and under runtime_step; while editing, play sets default_clip (what starts when play begins). entity_get lists the clips, their bone coverage and what is playing.", animator)
    });
    fields.as_object().cloned().unwrap_or_default()
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn empty() -> Value {
    json!({ "type": "object", "properties": {} })
}

pub fn tools() -> Vec<Tool> {
    let mut spawn_props = entity_fields();
    spawn_props.insert(
        "kind".into(),
        json!({
            "type": "string",
            "enum": ["empty", "cube", "sphere", "plane", "cylinder", "cone", "torus",
                     "point_light", "directional_light", "spot_light", "camera", "particles", "environment", "post_process_volume", "light_probe_volume"],
            "description": "What to create (default empty). Primitive sizes at scale 1: cube 1x1x1 centred (-0.5..0.5); sphere radius 1; cylinder radius 1, length 1 along Z (z -0.5..0.5); cone base radius 1 at z = -1 and tip at the origin, so it points toward +Z, opposite to forward: give ships and arrows a 180 degree yaw; torus ring radius 1 (to the tube centre) with tube radius 0.35 (1.35 outer), axis along Z, and the tube scales with the ring. The plane is 1x1 in XY facing +Z, so use rotation [-90, 0, 0] and a large scale for a floor. entity_get shows local_bounds."
        }),
    );
    spawn_props.insert(
        "preset".into(),
        json!({ "type": "string", "enum": ["physics", "starfield", "boids", "optimized_boids"], "description": "Particle behavior for kind=particles" }),
    );
    let quiet = json!({ "type": "boolean", "description": "Return only {id, name} instead of the entity summary; use for bulk work" });
    spawn_props.insert("quiet".into(), quiet.clone());
    let mut update_props = entity_fields();
    update_props.insert("entity".into(), entity_ref());
    update_props.insert("quiet".into(), quiet);
    let mut duplicate_props = update_props.clone();
    duplicate_props.insert(
        "quiet".into(),
        json!({ "type": "boolean", "description": "One copy: return only {id, name}. Several: return only the count and the first and last copy instead of every id and name" }),
    );
    duplicate_props.insert(
        "count".into(),
        json!({ "type": "integer", "minimum": 1, "maximum": 1000, "description": "Number of copies (default 1). Copies are named \"<name> 1\", \"<name> 2\", ...; put {i} in name to place the number yourself" }),
    );
    duplicate_props.insert(
        "step".into(),
        vec3("Offset between neighbouring copies. The source is cell 0, copy i goes to cell i: translation (default the source's) + step * column + row_step * row"),
    );
    duplicate_props.insert(
        "row_step".into(),
        vec3("Offset between rows when columns is set"),
    );
    duplicate_props.insert(
        "columns".into(),
        json!({ "type": "integer", "minimum": 1, "description": "Cells per row for a grid (counting the source as cell 0); omit for a single row" }),
    );
    update_props.insert(
        "entities".into(),
        json!({ "type": "array", "items": { "anyOf": [{ "type": "integer" }, { "type": "string" }] }, "description": "Apply the same change to all of these entities instead of one" }),
    );
    update_props.insert(
        "name_contains".into(),
        json!({ "type": "string", "description": "Apply the same change to every entity whose name contains this (case-insensitive) instead of one" }),
    );

    let mut tools = vec![
        // ---- Process control (local) ----
        Tool {
            name: "engine_launch",
            description: "Build (incrementally) and start the engine with remote control enabled, then wait until it accepts commands. Use target \"editor\" to build games: it can create/open/save projects. \"player\" runs a saved project; any example name runs that example. If an engine is already listening on the port (e.g. the user started the editor), this attaches to it instead unless restart is true. The first build can take several minutes.",
            schema: object(json!({
                "target": { "type": "string", "description": "editor (default), player, or an example name such as pbr" },
                "project": { "type": "string", "description": "Project directory: opened in the editor, or played by the player" },
                "build": { "type": "boolean", "description": "Run cargo build first (default true)" },
                "release": { "type": "boolean", "description": "Optimized build (default false)" },
                "restart": { "type": "boolean", "description": "Stop an engine this server started before launching" },
                "env": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Child environment overrides, e.g. DISPLAY=:0 or WAYLAND_DISPLAY plus XDG_RUNTIME_DIR. Desktop environment is checked before building on Linux. Omitted values inherit from the MCP server." },
                "monitor": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "Monitor for the window: 1-based number, output name (e.g. HDMI-A-0) or primary. Defaults to settings.json / ENGINE_MONITOR; respect the user's choice and don't override it without reason." },
                "window_size": { "type": "array", "items": { "type": "integer", "minimum": 16 }, "minItems": 2, "maxItems": 2, "description": "[width, height] the window opens at (ENGINE_WINDOW_SIZE), e.g. [800, 450] for a light test window; the default is settings.json's resolution" },
                "binary": { "type": "string", "description": "Run this executable instead of building target: an engine built elsewhere, e.g. a baseline commit's target/baseline/release/editor or examples/sponza, for A/B comparisons. Launched with the same environment and port handling (ENGINE_SHADER_RELOAD=0, so it keeps its built-in shaders); target still names what it is (editor, player or an example) and build is ignored" },
                "renderdoc": { "type": "boolean", "description": "Run under RenderDoc (its Vulkan layer) so renderdoc_capture can capture frames into target/engine-mcp/captures. Needs RenderDoc installed. RenderDoc hides Vulkan features it cannot capture and adds CPU overhead, so leave it off for performance measurements" },
                "timeout_seconds": { "type": "number", "description": "How long to wait for the engine to start (default 120)" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "engine_profile_cpu",
            description: "Sample the running engine's CPU with perf for a few seconds and return the hottest symbols (self time, by shared object), for finding where CPU frame time goes when engine_stats' render_gpu or ui_build look high. Needs Linux perf and a process this server launched (or pid). Returns the top symbols with their share of samples; functions are demangled Rust paths.",
            schema: object(json!({
                "seconds": { "type": "number", "minimum": 1, "maximum": 30, "default": 4, "description": "How long to sample" },
                "top": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25, "description": "Symbols to return" },
                "pid": { "type": "integer", "description": "Process to sample instead of the engine this server launched" },
                "callers": { "type": "boolean", "default": false, "description": "Attribute time to callers too (perf --children), to see which frame stage a hot leaf belongs to" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "engine_stop",
            description: "Stop the engine process this server launched. Unsaved editor changes are lost; call project_save first.",
            schema: empty(),
            action: Action::Local,
        },
        Tool {
            name: "engine_status",
            description: "Whether an engine is listening, the managed process (pid, uptime) and how the last one exited.",
            schema: empty(),
            action: Action::Local,
        },
        Tool {
            name: "engine_output",
            description: "Raw stdout/stderr of the launched engine process: panics, backtraces, wgpu/naga validation errors and startup failures that never reach the in-engine log. Works after the process died.",
            schema: object(json!({
                "lines": { "type": "integer", "description": "How many trailing lines (default 80)" },
                "contains": { "type": "string", "description": "Only lines containing this text" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "engine_build",
            description: "Compile a target (editor, player or an example) without running it and report compiler errors. Use after changing engine Rust code.",
            schema: object(json!({
                "target": { "type": "string", "description": "editor (default), player or an example name" },
                "release": { "type": "boolean" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "lua_api_reference",
            description: "The engine's Lua scripting guide: lifecycle hooks (on_created, update, on_destroyed, on_ui), entity/transform/hierarchy/input/component/query/event/state/UI functions, coroutines and script modes. Read it before writing game scripts. The whole guide is about 6k tokens; pass section (a heading, e.g. \"UI API\" or \"Lights\"), search (text to find, with the heading each hit is under) or outline for just the part you need.",
            schema: object(json!({
                "section": { "type": "string", "description": "Return the sections whose heading contains this (case-insensitive), with their subsections" },
                "search": { "type": "string", "description": "Return the guide's lines containing this text, each with its heading path" },
                "outline": { "type": "boolean", "description": "Return only the list of headings" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "batch",
            description: "Run several tool calls in one round trip, in order, e.g. a sequence of entity_spawn/entity_update calls. Each result is returned in order (screenshot images are left out). Stops at the first error unless stop_on_error is false.",
            schema: object(json!({
                "calls": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string", "description": "Tool name, e.g. entity_spawn" },
                            "arguments": { "type": "object" }
                        },
                        "required": ["tool"]
                    }
                },
                "stop_on_error": { "type": "boolean", "description": "Default true" },
                "quiet": { "type": "boolean", "default": false, "description": "Return identifiers and counts instead of full successful results/geometry recipes; errors and rollback details remain. Applies to sequential, atomic and dry-run batches." },
                "aggregate": { "type": "boolean", "default": false, "description": "Return one summary instead of a row per call: calls per tool, the entities they returned (listed up to 24, else count/first/last), alias ids, and every failed call in full. Overrides quiet; the ok/failure/rolled_back verdict stays. Use for large scene builds." }
            }), &["calls"]),
            action: Action::Local,
        },
        // ---- Overview ----
        Tool {
            name: "engine_info",
            description: "Snapshot of the running engine: frame, fps, runtime mode, active scene, window size, project root, value conventions and, in the editor, project/scene/selection state, plus mcp_server (whether this server is older than its sources and needs a restart, and the repo's commit). Start here.",
            schema: empty(),
            action: Action::Local,
        },
        Tool {
            name: "engine_stats",
            description: "Frame time, fps, CPU time per frame stage, GPU time per render stage (renderer.gpu, from timestamp queries; native only), entity/script counts, draw calls (with shadow views rendered/cached and culled instances) and asset counts. history: N adds one row per frame for the last N frames (draw calls, shadow views, culled instances, GPU stage times), oldest first, to see the frames around a change instead of only the latest; compact drops the explanatory notes.",
            schema: object(json!({
                "history": { "type": "integer", "minimum": 1, "maximum": 240, "description": "Also return the renderer figures of the last N frames, one row each" },
                "compact": { "type": "boolean", "description": "Leave out the notes that explain the fields" }
            }), &[]),
            action: Action::Engine("stats"),
        },
        Tool {
            name: "engine_call",
            description: "Call any engine remote method directly (see engine_info's methods list), for anything without a dedicated tool.",
            schema: object(json!({
                "method": { "type": "string" },
                "params": { "type": "object" }
            }), &["method"]),
            action: Action::Local,
        },
        // ---- Scene inspection ----
        Tool {
            name: "scene_list_entities",
            description: "Flat, paginated list of entities in the active scene with id, name, parent, child count, component names and world position. Filter by name substring, component, or roots only.",
            schema: object(json!({
                "name_contains": { "type": "string" },
                "component": { "type": "string", "description": "e.g. mesh, script, camera, point_light, particle_system" },
                "roots_only": { "type": "boolean" },
                "limit": { "type": "integer", "description": "Default 200" },
                "offset": { "type": "integer" }
            }), &[]),
            action: Action::Engine("scene.entities"),
        },
        Tool {
            name: "scene_tree",
            description: "The entity hierarchy of the active scene as a tree with component names.",
            schema: object(json!({ "max_depth": { "type": "integer" } }), &[]),
            action: Action::Engine("scene.tree"),
        },
        Tool {
            name: "entity_get",
            description: "Everything about one entity: local and world transform (translation, Euler rotation, quaternion, scale), mesh and bounds, material, lights, camera, script (source, mode, last error), particles, environment, animations, parent/children. Skinned meshes show joint and vertex counts and their animator; animators show libraries, every playable clip with duration and bone coverage (bound/missing bones), and what each layer is playing.",
            schema: object(json!({ "entity": entity_ref() }), &["entity"]),
            action: Action::Engine("entity.get"),
        },
        Tool {
            name: "assets_list",
            description: "Materials (with their settings), or counts of meshes and textures, loaded in the scene.",
            schema: object(json!({ "kind": { "type": "string", "enum": ["materials", "meshes", "textures"] } }), &[]),
            action: Action::Engine("assets.list"),
        },
        // ---- Scene editing ----
        Tool {
            name: "entity_spawn",
            description: "Create an entity in the active scene: an empty, a primitive mesh, a light, a camera (made active by default), a GPU particle system, an environment or a post-process volume. Accepts every entity_update field too (material, script, parent, lights, ...). Returns the new entity's id. Units are meters, Y is up, -Z is forward. To make many similar objects, spawn one and use entity_duplicate with count/step/columns.",
            schema: json!({ "type": "object", "properties": spawn_props }),
            action: Action::Engine("entity.spawn"),
        },
        Tool {
            name: "entity_update",
            description: "Change any aspect of an entity in one call: name, transform, parent, visibility, shadows, mesh, material, point/spot/directional light, camera, script, particle system/emitter, environment, post-process volume, built-in animations, billboard. Only the given fields change; component fields set to null remove the component. Pass entities (a list) or name_contains instead of entity to apply the same change to many entities at once. Changes made during play mode are discarded on stop (results then carry discarded_on_stop: true).",
            schema: json!({ "type": "object", "properties": update_props }),
            action: Action::Engine("entity.update"),
        },
        Tool {
            name: "entity_remove_component",
            description: "Remove one component from an entity.",
            schema: object(json!({
                "entity": entity_ref(),
                "component": { "type": "string", "enum": ["transform", "mesh", "material", "visible", "point_light", "directional_light", "spot_light", "cast_shadow", "camera", "script", "particle_system", "particle_emitter", "environment", "post_process_volume", "light_probe_volume", "rigid_body", "collider", "character_controller", "spring_camera", "nav_mesh_surface", "nav_agent", "nav_obstacle", "nav_modifier", "nav_link", "ai_agent", "animation_tree", "billboard", "depth_state", "rotate_animation", "orbit_animation", "gltf", "skinned_mesh", "animator"] }
            }), &["entity", "component"]),
            action: Action::Engine("entity.remove_component"),
        },
        Tool {
            name: "entity_delete",
            description: "Delete an entity and all its children. Pass entities (a list) or name_contains instead of entity to delete many at once.",
            schema: object(json!({
                "entity": entity_ref(),
                "entities": { "type": "array", "items": { "anyOf": [{ "type": "integer" }, { "type": "string" }] }, "description": "Delete all of these entities instead of one" },
                "name_contains": { "type": "string", "description": "Delete every entity whose name contains this (case-insensitive)" }
            }), &[]),
            action: Action::Engine("entity.delete"),
        },
        Tool {
            name: "entity_duplicate",
            description: "Copy an entity with its children (same parent). Inline materials are copied; scripts start fresh. Accepts entity_update fields to apply to the copy, e.g. a new name or translation. With count, step, row_step and columns it lays out a row or grid of copies in one call (e.g. 12x6 bricks: count 71, columns 12, step [1.1,0,0], row_step [0,-0.5,0]) and returns just their ids and names.",
            schema: json!({ "type": "object", "properties": duplicate_props, "required": ["entity"] }),
            action: Action::Engine("entity.duplicate"),
        },
        Tool {
            name: "voxel_spawn",
            description: "Procedural voxel models: build a model from text art, a flat voxel list or a fill expression, and spawn it as one entity with a child per colour. The engine greedy-meshes it (hidden faces dropped, same-coloured faces merged into large quads), so a model costs a few meshes, not one cube per voxel; identical models share their meshes. Corners are shaded with baked ambient occlusion (ao), and colors/parts put several keys into one mesh with per-vertex colours (e.g. a gradient on one material). Returns the root id, each part's key/keys/entity/quads, and quads vs cube_quads (what one cube per voxel would draw). Grid: x right, y up, z toward the viewer. Exactly one source: rows (one layer, extruded depth voxels), layers, size+voxels, or size+fill. Scripts do the same with voxel_model/spawn_voxels (lua_api_reference section \"Procedural geometry\").",
            schema: object(json!({
                "name": { "type": "string", "description": "Root entity name (default Voxels); children are named <name>.<key>" },
                "rows": { "type": "array", "items": { "type": "string" }, "description": "Text art, top row first: each character is a voxel of that key ('.' or space is empty), extruded depth voxels toward -Z" },
                "layers": { "type": "array", "items": { "type": "array", "items": { "type": "string" } }, "description": "3D text art: slices front (+Z) to back, each a list of rows top to bottom" },
                "legend": { "type": "object", "description": "Character -> key (a string such as a colour name or \"#rrggbb\", or an integer; null makes the character empty). Without a legend each character is its own key" },
                "depth": { "type": "integer", "minimum": 1, "description": "Voxels of thickness per text layer (default 1)" },
                "size": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "minItems": 3, "maxItems": 3, "description": "[width, height, depth] in voxels, for voxels and fill" },
                "voxels": { "type": "array", "description": "Flat list of keys, index x + W*(y + H*z); 0, null or \"\" is empty" },
                "fill": { "type": "string", "description": "Lua expression (or statements that return) giving the key for cell x, y, z (0-based ints), or nil/false/0 for empty; true means key 1. In scope: W, H, D, math, noise(x[,y[,z]]) and fbm(x,y,z[,octaves]) (gradient noise, about -1..1, features ~1 unit apart: scale coordinates, e.g. noise(x*0.1, z*0.1)). E.g. terrain: \"y < 6 + fbm(x*0.08, 0, z*0.08)*5 and (y < 4 and 'rock' or 'grass')\"; a ball: \"(x-7.5)^2+(y-7.5)^2+(z-7.5)^2 < 64\". Runs sandboxed, up to 10 s" },
                "voxel": { "anyOf": [{ "type": "number" }, vec3("Per-axis size")], "description": "Size of one voxel in meters (default 0.1)" },
                "pivot": vec3("Fraction of the model's box at the entity origin (default [0.5, 0, 0.5]: standing centred on it; [0.5, 0.5, 0.5] centres it)"),
                "palette": { "type": "object", "description": "Key -> material for that part: a colour (\"#rrggbb\" or [r,g,b]) or material fields as entity_update takes them, e.g. {\"color\": \"#ff4400\", \"emissive\": 3}. Keys that are \"#rrggbb\" colour themselves" },
                "ao": { "type": "number", "minimum": 0, "maximum": 1, "description": "Strength of the ambient occlusion baked into vertex colours: creases and inner corners darken (default 1, 0 = off)" },
                "colors": { "type": "object", "description": "Key -> vertex colour (\"#rrggbb\" or [r,g,b(,a)]), which multiplies the part's material colour. Keys not listed are white" },
                "parts": { "type": "object", "description": "Key -> part key: keys naming the same part share one mesh and one material (palette entry of the part key), each with its own vertex colour" },
                "part": { "type": ["string", "integer"], "description": "Part of every key that parts does not list, e.g. \"body\" puts the whole model in one mesh" },
                "cast_shadow": { "type": "boolean", "description": "Whether the parts cast shadows (default true)" },
                "translation": vec3("Root position in meters"),
                "rotation": vec3("Root rotation, Euler degrees"),
                "scale": { "anyOf": [vec3("Scale"), { "type": "number" }] },
                "parent": entity_ref()
            }), &[]),
            action: Action::Engine("voxel.spawn"),
        },
        // ---- Scripting & runtime ----
        Tool {
            name: "lua_eval",
            description: "Run Lua with the scripting API; return values and printed output. A bare expression returns its value. Without entity, uses a persistent console whose globals survive calls; require/include are unavailable. With entity, uses that script's globals, state and module loaders. World changes apply when the chunk finishes. See lua_api_reference.",
            schema: object(json!({
                "code": { "type": "string" },
                "entity": entity_ref(),
                "read_only": { "type": "boolean", "description": "Skip recording an undo step (use for pure queries)" }
            }), &["code"]),
            action: Action::Engine("lua.eval"),
        },
        Tool {
            name: "script_status",
            description: "Every scripted entity with its source, whether it compiled, and its last runtime error. Check after attaching or editing scripts, and after entering play mode.",
            schema: empty(),
            action: Action::Engine("script.status"),
        },
        Tool {
            name: "script_reload",
            description: "Schedule recompilation for one entity, every user of a file, or all scripts. Reload runs on the next simulation update; frozen manual sessions require runtime_step (wait alone does not advance scripts). The editor also watches project scripts automatically.",
            schema: object(json!({
                "entity": entity_ref(),
                "path": { "type": "string" }
            }), &[]),
            action: Action::Engine("script.reload"),
        },
        Tool {
            name: "runtime_set",
            description: "Enter play mode (game scripts run on_created/update, animations play) or stop (the scene is restored to its state before play, with the same entity ids). Takes effect next frame; then use wait. time_scale speeds up (or slows, or with 0 pauses) play so long playtests finish sooner; speed-ups run several normal-sized update steps per frame, so game logic behaves as at 1x. wait's seconds stay real seconds. Stopping resets time_scale to 1.",
            schema: object(json!({
                "mode": { "type": "string", "enum": ["play", "stop"] },
                "time_scale": { "type": "number", "minimum": 0, "maximum": 16, "description": "Play speed relative to real time (default 1); can be set before or during play" }
            }), &[]),
            action: Action::Engine("runtime.set"),
        },
        Tool {
            name: "playtest",
            description: "One-call playtest: enter play (optionally sped up with time_scale), send scripted input, run for `seconds`, then report failing scripts, new warnings/errors, the sounds played (grouped by file; sounds: list for every play), what changed in the scene (added/removed entities and changed fields), the result of an `eval` Lua expression and a viewport screenshot, and stop again (the scene is restored). Replaces runtime_set + input_send + wait + lua_eval + screenshot + runtime_set.",
            schema: object(json!({
                "seconds": { "type": "number", "description": "Real seconds to play (default 3, up to 240); game time is seconds * time_scale" },
                "time_scale": { "type": "number", "minimum": 0, "maximum": 16, "description": "Play speed (default 1)" },
                "input": { "type": "array", "items": { "type": "object" }, "description": "Input events as for input_send (key/mouse_button/mouse_move/scroll, action, frames, delay in frames). Everything is released at the end." },
                "eval": { "type": "string", "description": "Lua evaluated at the end, e.g. \"return GAME.score\". Without entity it runs in the console; if it fails on a global that exactly one script defines (GAME), it runs again inside that script" },
                "entity": entity_ref(),
                "screenshot": { "type": "boolean", "description": "Include a viewport screenshot (default true)" },
                "max_size": { "type": "integer", "description": "Screenshot longest side (default 1280)" },
                "aspect": { "anyOf": [{ "type": "number" }, { "type": "string" }], "description": "Play with the view laid out at this shape (\"16:9\", \"4:3\"); scripts, cameras and HUDs see it. Restored at the end" },
                "viewport_size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "Play with the view at exactly [width, height] pixels" },
                "diff": { "type": "boolean", "description": "Report what play changed in the scene (default true)" },
                "sounds": { "anyOf": [{ "type": "string", "enum": ["summary", "list"] }, { "type": "boolean" }], "description": "Sounds played: \"summary\" (default) groups them by file with counts and first/last time, \"list\" gives every play, false leaves them out" },
                "lean": { "type": "boolean", "description": "For long soak tests: no screenshot, scene diff or sounds; just failing scripts, warnings and the eval result. Explicit screenshot/diff/sounds settings still win" },
                "diff_limit": { "type": "integer", "description": "Most changed entities listed (default 30)" },
                "stop": { "type": "boolean", "description": "Stop play at the end (default true); false keeps the game running for further inspection" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "scene_mark",
            description: "Remember the current scene state as the baseline for scene_diff (e.g. before play, before a lua_eval bulk edit).",
            schema: empty(),
            action: Action::Engine("scene.mark"),
        },
        Tool {
            name: "scene_diff",
            description: "What changed since scene_mark: entities added and removed, and for each changed entity the fields that differ as path: [old, new] (transform, visibility, material, lights, script errors, ...). Diff before stopping play, because stop restores the scene.",
            schema: object(json!({ "limit": { "type": "integer", "description": "Most changed entities listed (default 100)" } }), &[]),
            action: Action::Engine("scene.diff"),
        },
        Tool {
            name: "wait",
            description: "Let the engine run for a number of frames and/or seconds, then return. Use after play, input or script changes before inspecting or taking a screenshot.",
            schema: object(json!({
                "frames": { "type": "integer" },
                "seconds": { "type": "number", "description": "Up to 240" },
                "until_frame": { "type": "integer", "description": "Return once the engine has completed this absolute frame number (as engine_stats and wait report it), e.g. a frame after a change was applied" }
            }), &[]),
            action: Action::Engine("wait"),
        },
        // ---- Audio ----
        Tool {
            name: "sound_play",
            description: "Play a sound file (wav, ogg, mp3, flac) through the engine, e.g. to try a sound effect before a script uses it. Paths are relative to the project (content/audio/hit.wav) or absolute. Returns its id for sound_stop.",
            schema: object(json!({
                "path": { "type": "string" },
                "volume": { "type": "number", "description": "Linear gain (default 1)" },
                "pitch": { "type": "number", "description": "Playback speed and pitch (default 1)" },
                "loop": { "type": "boolean" },
                "bus": { "type": "string", "description": "Mixer bus to play on (default sfx; the project's buses are listed by audio_status, by default master, music, sfx, ambience, ui, voice)" }
            }), &["path"]),
            action: Action::Engine("audio.play"),
        },
        Tool {
            name: "sound_stop",
            description: "Stop one sound by id, or every sound.",
            schema: object(json!({ "id": { "type": "integer" } }), &[]),
            action: Action::Engine("audio.stop"),
        },
        Tool {
            name: "sound_inspect",
            description: "Measure a sound file, since you cannot listen to it: duration, sample rate, peak and RMS level (dBFS), clipped samples, DC offset, silence before and after the sound, the jump at the loop seam (a click when looped), loudness by tenth of the clip, plain-language notes on problems, and an image (waveform over a log-frequency spectrogram). Use it on every generated or imported sound effect and music loop.",
            schema: object(json!({
                "path": { "type": "string", "description": "wav, ogg, mp3 or flac; relative to the project (content/audio/hit.wav) or absolute" },
                "image": { "type": "boolean", "description": "Include the waveform/spectrogram image (default true)" }
            }), &["path"]),
            action: Action::Local,
        },
        Tool {
            name: "audio_status",
            description: "Sounds playing now and the recently played ones (id, path, bus, volume, pitch, time, audible), the mixer (device, sample rate, voices, audio-thread time per callback and load) and every bus with its fader, mute/solo and live peak/RMS meters in dBFS. You cannot hear the game, so use this (or playtest's sounds) to check that scripts play the right sounds at the right moments and that buses carry signal. A busy game plays the same few files hundreds of times; pass summary for one entry per file.",
            schema: object(json!({
                "since": { "type": "integer", "description": "Only plays with an id at least this (next_id from an earlier call)" },
                "summary": { "type": "boolean", "description": "Group the plays by file: count, first and last time, volume and pitch ranges, and how many were silent, instead of listing each play" }
            }), &[]),
            action: Action::Engine("audio.status"),
        },
        Tool {
            name: "audio_bus",
            description: "Change a mixer bus live: its fader (gain_db, -80 silent to +24), balance (pan -1..1), mute or solo. Changes are heard at once; save writes the project's bus layout (content/audio/mixer.json), as the editor's mixer window does. Returns audio_status's mixer view.",
            schema: object(json!({
                "bus": { "type": "string", "description": "master, music, sfx, ambience, ui, voice or a project bus" },
                "gain_db": { "type": "number" },
                "pan": { "type": "number" },
                "mute": { "type": "boolean" },
                "solo": { "type": "boolean" },
                "save": { "type": "boolean", "description": "Also write content/audio/mixer.json" }
            }), &["bus"]),
            action: Action::Engine("audio.bus"),
        },
        Tool {
            name: "audio_capture",
            description: "Record the next N seconds of the engine's mixed output (what the speakers get) to a 32-bit float WAV, then inspect it with sound_inspect to check a mix without hearing it. Returns at once; wait the seconds, and audio_status lists the finished file under captures. Relative paths are under the project. Native builds only.",
            schema: object(json!({
                "path": { "type": "string", "description": "e.g. captures/mix.wav" },
                "seconds": { "type": "number", "description": "Default 2, up to 600" }
            }), &["path"]),
            action: Action::Engine("audio.capture"),
        },
        // ---- Input ----
        Tool {
            name: "input_send",
            description: "Inject keyboard and mouse input, seen by scripts exactly like real input (is_key_pressed etc.). Keys use the names scripts check: letters upper case (\"W\"), Space, Return, Escape, ArrowUp/Down/Left/Right, Shift, Control, Alt, Tab. Pass one event or an events array; each event has an optional delay in frames. action tap (default) presses for `frames` frames (default 3), down holds, up releases.",
            schema: object(json!({
                "events": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "key": { "type": "string" },
                            "mouse_button": { "anyOf": [{ "type": "integer" }, { "type": "string", "enum": ["left", "right", "middle"] }] },
                            "mouse_move": { "type": "array", "items": { "type": "number" }, "description": "[x, y] window pixels" },
                            "scroll": { "type": "array", "items": { "type": "number" }, "description": "[dx, dy]" },
                            "action": { "type": "string", "enum": ["tap", "down", "up"] },
                            "frames": { "type": "integer" },
                            "delay": { "type": "integer" }
                        }
                    }
                },
                "key": { "type": "string", "description": "Shorthand for a single key event" },
                "action": { "type": "string", "enum": ["tap", "down", "up"] },
                "frames": { "type": "integer" }
            }), &[]),
            action: Action::Engine("input.send"),
        },
        Tool {
            name: "input_state",
            description: "Keys and mouse buttons currently held, the mouse position, and optionally the state of specific keys.",
            schema: object(json!({ "keys": { "type": "array", "items": { "type": "string" } } }), &[]),
            action: Action::Engine("input.state"),
        },
        Tool {
            name: "input_clear",
            description: "Release every key and button and drop queued input events.",
            schema: empty(),
            action: Action::Engine("input.clear"),
        },
        // ---- Viewing ----
        Tool {
            name: "screenshot",
            description: "Capture what the engine window shows (including the editor UI), or only the scene viewport, as an image. Takes one frame (a few with aspect or viewport_size). In editor mode the viewport shows the editor's view camera; pass camera: \"game\" (or a camera entity) to render that frame through the game camera instead, without gizmos or the editor grid and without moving the view. camera can also be a one-off pose {position, target, fov_deg} that looks around without editing anything. A camera other than the current view is a camera cut: temporal effects (TAA, the path tracer's denoiser) restart, so a single frame shows them unconverged; pass settle_frames to hold the view that many frames first (nothing moves, the game keeps running), or use visual_capture to compare settings. aspect previews the framing at another window shape (e.g. \"16:9\", \"4:3\").",
            schema: object(json!({
                "camera": { "anyOf": [
                    { "type": "integer" },
                    { "type": "string" },
                    { "type": "object", "properties": {
                        "position": vec3("Eye position"),
                        "target": vec3("Point to look at (alone, it keeps the current distance and direction)"),
                        "fov_deg": { "type": "number" },
                        "near": { "type": "number" },
                        "far": { "type": "number" }
                    }}
                ], "description": "\"game\" for the scene's active camera, a camera entity id/name, or a one-off view {position, target, fov_deg, near, far} starting from the view camera. Only this capture's frame uses it; nothing is moved." },
                "aspect": { "anyOf": [{ "type": "number" }, { "type": "string" }], "description": "Lay the game out at this width/height, as a number (1.78) or \"16:9\", in the largest region of that shape inside the viewport, and capture it. The layout is real for the capture: scripts see the shape through get_viewport_size() (cameras fitted to the view adapt) and HUDs anchor to it. Use it to check framing at 16:9 and 4:3. Implies target viewport." },
                "viewport_size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "Like aspect, but an exact [width, height] in pixels (shrunk to fit if the viewport is smaller), e.g. [1280, 720] or a phone's [390, 844]" },
                "target": { "type": "string", "enum": ["window", "viewport"], "description": "window (the default) is the whole engine window; viewport is just the scene view. With camera or aspect the default is viewport." },
                "settle_frames": { "type": "integer", "minimum": 0, "maximum": 600, "description": "Render this many frames through camera / aspect before capturing, so TAA and the path tracer's denoiser converge on the held view (32 or more for path tracing). The scene's cameras are not touched; in play mode the game keeps running (runtime_set {time_scale: 0} freezes it). Without camera or aspect it only waits" },
                "points": { "type": "array", "maxItems": 1024, "items": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 2, "maxItems": 2 }, "description": "[[x, y], ...] pixels whose final displayed 8-bit RGBA to return (after tonemapping, anti-aliasing, grain and dither), in the captured image before max_size shrinks it (source_width x source_height: the window, or the viewport). For HDR values before tonemapping use texture_fetch {texture: \"scene\"}" },
                "rect": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 4, "maxItems": 4, "description": "[x, y, width, height] in the same pixels: returns per-channel min, max, mean and standard deviation of the displayed 8-bit values, for brightness and noise checks" },
                "max_size": { "type": "integer", "description": "Longest side in pixels (default 1280)" },
                "save_path": { "type": "string", "description": "Also write the PNG here" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "viewport_preview",
            description: "Lay the game out at another window shape until turned off: the scene renders into the largest region of that aspect (or an exact pixel size) inside the viewport, scripts see it as get_viewport_size(), and HUDs anchor to it. For checking framing and HUD layout at 16:9, 4:3 or a phone size across several screenshots and playtests. Call with neither field to restore the full view. screenshot and playtest also take aspect / viewport_size for a one-off.",
            schema: object(json!({
                "aspect": { "anyOf": [{ "type": "number" }, { "type": "string" }], "description": "Width/height, e.g. \"16:9\" or 1.33" },
                "size": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "[width, height] in pixels" }
            }), &[]),
            action: Action::Engine("view.preview"),
        },
        Tool {
            name: "camera_get",
            description: "The view camera: position, target, forward, projection and the active camera entity.",
            schema: empty(),
            action: Action::Engine("camera.get"),
        },
        Tool {
            name: "camera_set",
            description: "Move the view camera. Give position and/or target, or focus on an entity (frames it at a distance based on its size). While the scene has an active camera entity this moves that entity too (the view is driven by it), so it edits scene data: to just look around, use screenshot with camera {position, target} instead.",
            schema: object(json!({
                "position": vec3("Eye position"),
                "target": vec3("Point to look at"),
                "focus": entity_ref(),
                "distance": { "type": "number", "description": "Distance from the focus entity" },
                "fov_deg": { "type": "number" },
                "near": { "type": "number" },
                "far": { "type": "number" }
            }), &[]),
            action: Action::Engine("camera.set"),
        },
        Tool {
            name: "environment_get",
            description: "Background clear color, ambient intensity and HDR sky (path, intensity). Exposure, tonemapping and color grading are post-processing: see postprocess_get.",
            schema: empty(),
            action: Action::Engine("environment.get"),
        },
        Tool {
            name: "environment_set",
            description: "Change environment settings; only given fields change. Creates an Environment entity if the scene has none, because that is how scenes save their environment.",
            schema: object(json!({
                "clear_color": color("Background color"),
                "ambient_intensity": { "type": "number" },
                "hdr": { "type": "object", "properties": {
                    "enabled": { "type": "boolean" }, "intensity": { "type": "number" }, "path": { "type": "string", "description": ".hdr image" },
                    "background": { "type": "boolean", "description": "Draw the sky behind the scene (default true); false keeps it to lighting and reflections, with clear_color behind" }
                }}
            }), &[]),
            action: Action::Engine("environment.set"),
        },
        Tool {
            name: "light_probes_bake",
            description: "Bake the scene's light probe volumes (light_probe_volume), or one of them (entity), at high quality, one after another: warm-up sweeps for the bounced light to build up, then the exact mean of many sweeps with up to 256 rays per probe (about 65k rays per probe by default). Runs over the next frames (one update per frame; the result says how many): wait that long, then light_probes_status. A volume asked for during a bake waits its turn. Each bake is saved as content/lighting/<volume name>.probes and the volume's baked path is set; project_save keeps them. Baked probes light the scene where the probes cannot trace (web builds, GPUs without ray tracing) and stay as baked; with ray tracing they are the starting point (use_baked: true shows only the bake). Needs hardware ray tracing.",
            schema: object(json!({
                "entity": entity_ref(),
                "warmup_sweeps": { "type": "integer", "description": "Sweeps before averaging, for the bounces to build up (default 32)" },
                "sweeps": { "type": "integer", "description": "Sweeps averaged (default 256)" },
                "rays_per_probe": { "type": "integer", "description": "Rays per probe per sweep, 32-256 (default 256)" }
            }), &[]),
            action: Action::Engine("light_probes.bake"),
        },
        Tool {
            name: "light_probes_status",
            description: "The light probe volumes in use, in blending order (probe counts, spacing, rotation, live or baked, the baked file each shows, their regions of probes.irradiance / probes.distance), whether the probes can trace here, a running bake's progress and queue, and the last bake's outcome.",
            schema: empty(),
            action: Action::Engine("light_probes.status"),
        },
        Tool {
            name: "light_probes_debug",
            description: "Show or hide the light probe debug view: a sphere at every probe of the light probe volumes (where relocation moved it), lit by the light the probe holds from each direction. Dark red: inside geometry (gives nothing); yellow rim: moved out of geometry. Drawn over the image in the editor and player until hidden; screenshot to see it.",
            schema: object(json!({
                "show": { "type": "boolean", "description": "Default true; false hides it" },
                "radius": { "type": "number", "description": "Sphere radius in meters (default 0.06; kept until changed)" },
                "exposure": { "type": "number", "description": "Factor on the light shown before a simple tone curve (default 3)" }
            }), &[]),
            action: Action::Engine("light_probes.debug"),
        },
        Tool {
            name: "graphics_get",
            description: "The player's graphics settings (what the engine's settings page shows: display mode, window size, vsync, anti-aliasing, render scale and upscaler, the shadow atlas size and shadow updates per frame, the ray tracing master switch, ray-traced shadows/reflections/AO/refraction, FPS readout), whether the page is open, and what the GPU supports. Saved per game and user; they replace the scene volumes' anti_aliasing, upscaling and ray-tracing choices.",
            schema: empty(),
            action: Action::Engine("graphics.get"),
        },
        Tool {
            name: "graphics_set",
            description: "Change the player's graphics settings (saved per game and user, like a player using the settings page) and open or close the settings page. Only the given fields change. They win over post-process volumes for anti-aliasing, upscaling and ray-traced effects; use visual_capture variants to compare those without changing the player's choice.",
            schema: object(json!({
                "display_mode": { "type": "string", "enum": ["windowed", "fullscreen"], "description": "fullscreen: borderless on the window's monitor (not in the editor, whose window is not the game's)" },
                "window_size": { "type": ["array", "null"], "items": { "type": "integer" }, "description": "[width, height] in pixels while windowed; null keeps the size the app opened with" },
                "vsync": { "type": "boolean" },
                "anti_aliasing": { "type": "string", "enum": ["none", "fxaa", "taa"] },
                "render_scale": { "type": "number", "description": "0.25-1: 1 native, 0.67 FSR quality, 0.5 performance" },
                "upscaler": { "type": "string", "enum": ["fsr", "taau"] },
                "sharpness": { "type": "number", "description": "FSR sharpening 0-1" },
                "shadow_atlas_size": { "type": "integer", "enum": [1024, 2048, 4096, 8192], "description": "Most texels across the point/spot light shadow atlas (default 8192 native, 4096 web; 8 bytes a texel): room for more shadowed lights at higher resolution. engine_stats renderer.lights shows its use" },
                "shadow_updates_per_frame": { "type": "integer", "description": "Point/spot light shadow views redrawn per frame at most (default 48; views wait at most a few frames). engine_stats renderer.shadow_views_deferred counts the waiting ones" },
                "shadow_cascades": { "type": "integer", "minimum": 1, "maximum": 4, "description": "Shadow map cascades per directional light (default 4): more keep sun shadows sharp further from the camera. engine_stats renderer.lights shows the cascades' split depths" },
                "shadow_cascade_size": { "type": "integer", "enum": [1024, 2048, 4096], "description": "Texels across each directional shadow cascade (default 2048; 8 bytes a texel with its static casters' copy)" },
                "ray_tracing": { "type": "boolean", "description": "Hardware ray tracing at all (default true). false: every effect takes its raster version (shadow maps, screen-space reflections and AO, see-through glass, no traced GI or path tracing) and light probes show their bakes, as on a GPU without ray tracing (or the web); the individual choices are kept for when it is switched back on" },
                "ray_traced_shadows": { "type": "boolean" },
                "reflections": { "type": "string", "enum": ["off", "screen_space", "ray_traced"] },
                "ambient_occlusion": { "type": "string", "enum": ["off", "screen_space", "ray_traced"] },
                "ray_traced_refraction": { "type": "boolean" },
                "global_illumination": { "type": "string", "enum": ["off", "ray_traced"], "description": "ray_traced: raster direct light plus ray-traced, denoised indirect diffuse light (bounce light, emissive surfaces, the sky only through openings) in place of the environment's diffuse light and flat ambient; ambient occlusion stands down. Between raster and path tracing in cost and look; path_tracing wins when both are on. Needs capabilities.global_illumination. Cost: engine_stats renderer.gpu global_illumination/gi_denoise/gi_composite" },
                "path_tracing": { "type": "boolean", "description": "Path-traced lighting with a denoiser instead of raster lighting (soft shadows, bounced light, emissive surfaces light the scene; reflections and AO included). Needs capabilities.path_tracing (hardware ray tracing, no MSAA). Demanding: check engine_stats renderer.gpu path_tracing/denoise" },
                "path_tracing_bounces": { "type": "integer", "description": "Indirect bounces per path, 1-8 (default 2)" },
                "path_tracing_samples": { "type": "integer", "description": "Paths per pixel per frame, 1-8 (default 1)" },
                "path_tracing_denoiser": { "type": "boolean", "description": "Denoise the paths (default true); off shows the raw noisy samples" },
                "path_tracing_caustics": { "type": "boolean", "description": "Caustics: light focused through glass (transmission > 0) and off smooth metals onto other surfaces, traced as photons from the lights (default true); glass then casts a shadow with the focused light in it" },
                "show_fps": { "type": "boolean", "description": "Yellow frame rate in the corner of the view" },
                "open": { "type": "boolean", "description": "Show or hide the settings page" },
                "reset": { "type": "boolean", "description": "Restore the defaults first" },
                "save": { "type": "boolean", "description": "Save the change as the player's choice (default true). false applies it to this run only, for tests and A/B comparisons: the saved settings come back with the next launch, or set them back yourself" }
            }), &[]),
            action: Action::Engine("graphics.set"),
        },
        Tool {
            name: "postprocess_get",
            description: "Post-processing in effect at the view camera (exposure, local exposure, tonemapping, color grading, bloom, SSAO, vignette, chromatic aberration, film grain, sharpening, anti-aliasing) and the post-process volumes it is blended from, with each volume's influence at the camera. The scene renders in HDR; these turn it into the displayed image.",
            schema: empty(),
            action: Action::Engine("postprocess.get"),
        },
        Tool {
            name: "postprocess_set",
            description: "Change post-processing on a volume: entity, or else the scene's lowest-priority infinite volume (created if there is none). Only given fields change. Settings are saved with the scene. For an area that looks different (a dark cave, underwater), spawn a post_process_volume entity with a box or sphere shape, a higher priority and a blend_distance instead.",
            schema: object(json!({
                "entity": { "type": ["integer", "string"], "description": "Volume entity id or unique name (default: the infinite volume)" },
                "volume": { "type": "object", "description": "Volume properties", "properties": {
                    "enabled": { "type": "boolean" },
                    "shape": { "type": "object", "description": "{kind: infinite} | {kind: box, size: [x,y,z]} | {kind: sphere, radius}" },
                    "priority": { "type": "integer" },
                    "weight": { "type": "number", "description": "0-1" },
                    "blend_distance": { "type": "number", "description": "Fade distance outside a bounded shape, meters" },
                    "overrides": { "type": ["array", "null"], "description": "Setting paths this volume sets (\"bloom.intensity\", or a whole group \"bloom\"); the rest comes from volumes below. null = everything. Settings you set on a selective volume are ticked automatically." }
                }},
                "exposure": { "type": "object", "properties": {
                    "mode": { "type": "string", "enum": ["manual", "auto"], "description": "auto adapts to scene brightness like an eye" },
                    "compensation": { "type": "number", "description": "Stops (EV); +1 doubles brightness" },
                    "min_ev": { "type": "number" }, "max_ev": { "type": "number" },
                    "speed_up": { "type": "number", "description": "EV/s when the scene gets brighter" },
                    "speed_down": { "type": "number", "description": "EV/s when it gets darker" },
                    "low_percent": { "type": "number" }, "high_percent": { "type": "number" }
                }},
                "local_exposure": { "type": "object", "description": "Exposure that varies across the frame like an eye adapting to where it looks: each region's brightness (from a bilateral grid, so regions end at edges) moves toward middle grey and pixels keep their detail around it. Opens up dark corners of a sunlit interior without flattening the sunlit parts. Cost: engine_stats renderer.gpu local_exposure", "properties": {
                    "enabled": { "type": "boolean" },
                    "shadows": { "type": "number", "description": "0-1 (default 0.5): how far regions darker than middle grey move toward it; 0.5 halves their distance in stops (6 under comes up 3)" },
                    "highlights": { "type": "number", "description": "0-1 (default 0.25): how far regions brighter than middle grey move toward it" },
                    "detail": { "type": "number", "description": "0-2 (default 1): contrast of the detail within each region; 1 keeps it" }
                }},
                "tonemapping": { "type": "string", "enum": ["rolloff", "neutral", "aces", "agx", "reinhard", "none"], "description": "rolloff (default) keeps colors below 0.76 exactly and softly compresses highlights; neutral is Khronos PBR Neutral (also darkens darks slightly); aces is contrasty and filmic; agx desaturates highlights naturally; none clips" },
                "color": { "type": "object", "properties": {
                    "post_exposure": { "type": "number", "description": "Stops" },
                    "temperature": { "type": "number", "description": "-100 cool .. 100 warm" },
                    "tint": { "type": "number", "description": "-100 green .. 100 magenta" },
                    "color_filter": color("Multiplies the image"),
                    "hue_shift": { "type": "number", "description": "Degrees" },
                    "saturation": { "type": "number", "description": "1 = unchanged" },
                    "vibrance": { "type": "number", "description": "-1..1" },
                    "contrast": { "type": "number", "description": "1 = unchanged" },
                    "lift": { "type": "array", "description": "Shadows [r,g,b tint (1 = neutral), offset]" },
                    "gamma": { "type": "array", "description": "Midtones [r,g,b, offset]" },
                    "gain": { "type": "array", "description": "Highlights [r,g,b, offset]" }
                }},
                "lut": { "type": "object", "description": "3D colour lookup table (.cube from Resolve, Photoshop, ...) applied to the finished image", "properties": {
                    "enabled": { "type": "boolean" },
                    "path": { "type": "string", "description": ".cube file, project-relative" },
                    "intensity": { "type": "number", "description": "0-1" }
                }},
                "bloom": { "type": "object", "properties": {
                    "enabled": { "type": "boolean" }, "intensity": { "type": "number" },
                    "threshold": { "type": "number", "description": "HDR brightness where glow starts; 0 = physically based glow on everything" },
                    "knee": { "type": "number" }, "scatter": { "type": "number", "description": "0-1, how far the glow spreads" },
                    "tint": { "type": "array" }
                }},
                "ssao": { "type": "object", "properties": {
                    "enabled": { "type": "boolean" }, "radius": { "type": "number" }, "bias": { "type": "number" }, "intensity": { "type": "number" }, "power": { "type": "number" },
                    "ray_traced": { "type": "boolean", "description": "Trace occlusion with hardware ray tracing (sees occluders off screen; radius is the ray length); falls back to screen space when engine_info surface.ray_tracing is unavailable" }
                }},
                "depth_of_field": { "type": "object", "description": "Bokeh depth of field", "properties": {
                    "enabled": { "type": "boolean" },
                    "mode": { "type": "string", "enum": ["physical", "manual"], "description": "physical: real lens from the camera fov + aperture (distant scenes need a low f-number); manual: blur ramps by distance ranges (tilt-shift looks)" },
                    "focus_distance": { "type": "number", "description": "Meters (physical)" },
                    "autofocus": { "type": "boolean", "description": "Focus on what is at the centre of the view (physical)" },
                    "aperture": { "type": "number", "description": "f-number, lower = more blur (physical)" },
                    "near_start": { "type": "number" }, "near_end": { "type": "number" },
                    "far_start": { "type": "number" }, "far_end": { "type": "number" },
                    "max_blur": { "type": "number", "description": "Largest blur radius in pixels at 1080p" }
                }},
                "vignette": { "type": "object", "properties": {
                    "enabled": { "type": "boolean" }, "intensity": { "type": "number" }, "smoothness": { "type": "number" }, "roundness": { "type": "number", "description": "1 round corners, 0 square" }, "rounded": { "type": "boolean", "description": "circular on screen instead of following the frame" }, "color": { "type": "array" }, "center": { "type": "array" }
                }},
                "chromatic_aberration": { "type": "object", "properties": { "enabled": { "type": "boolean" }, "intensity": { "type": "number" } }},
                "film_grain": { "type": "object", "properties": { "enabled": { "type": "boolean" }, "intensity": { "type": "number" }, "response": { "type": "number" } }},
                "sharpen": { "type": "object", "properties": { "enabled": { "type": "boolean" }, "strength": { "type": "number" } }},
                "refraction": { "type": "object", "description": "Light through transmissive materials (material transmission > 0)", "properties": {
                    "ray_traced": { "type": "boolean", "description": "Refract with hardware ray tracing (bending, internal reflection, absorption; default true); off or without engine_info surface.ray_tracing they are drawn see-through" }
                }},
                "global_illumination": { "type": "object", "description": "Ray-traced global illumination: the path tracer traces only the indirect diffuse light (bounces, emissive surfaces, the sky through openings) at the primary surface, denoised and added to the main pass's lighting, which then leaves out the environment's diffuse light and flat ambient (and, with ray-traced reflections, the sky's reflection, which the reflection pass adds where its rays reach the sky). SSAO stands down. Path tracing wins when both are enabled. Needs engine_info surface.path_tracing; engine_stats renderer.global_illumination says whether a frame used it. The player's graphics setting global_illumination controls enabled", "properties": {
                    "enabled": { "type": "boolean" },
                    "half_resolution": { "type": "boolean", "description": "Trace one pixel of every 2x2 and upsample by normal and depth (default true): about a quarter of the rays" },
                    "bounces": { "type": "integer", "description": "Surfaces a path bounces off, 1-8 (default 4); bounce rays that land on what the screen showed last frame take its finished light, which carries every earlier bounce" },
                    "max_history": { "type": "integer", "description": "Most frames the denoiser accumulates, 1-256 (default 32)" }
                }},
                "path_tracing": { "type": "object", "description": "Path-traced lighting for opaque surfaces instead of the main pass's (direct light with soft shadows sized by rt_shadows.softness/light_radius, multi-bounce diffuse and specular light, emissive surfaces lighting the scene, glass), denoised spatiotemporally (ReLAX-style). Reflections and SSAO stand down while it runs. Needs engine_info surface.path_tracing; engine_stats renderer.path_traced says whether a frame used it. The player's graphics settings control enabled/bounces/samples/denoiser", "properties": {
                    "enabled": { "type": "boolean" },
                    "bounces": { "type": "integer", "description": "Indirect bounces per path, 1-8 (default 2)" },
                    "samples": { "type": "integer", "description": "Paths per pixel per frame, 1-8 (default 1)" },
                    "denoiser": { "type": "boolean", "description": "Denoise (default true); off shows raw samples" },
                    "max_history": { "type": "integer", "description": "Most frames the denoiser accumulates, 1-256 (default 32): more is smoother, slower to follow changing light" },
                    "caustics": { "type": "boolean", "description": "Caustic photons through glass and off smooth metals (default true); point, spot and directional lights only (not emissive surfaces)" },
                    "caustic_photons": { "type": "integer", "description": "Photons per frame shared by every light and every glass/mirror object, 1024-4194304 (default 262144); engine_stats renderer.gpu caustics shows their cost" }
                }},
                "rt_shadows": { "type": "object", "description": "Shadows of shadow-casting lights (up to 4 directional, 4 point, 4 spot) traced per pixel with hardware ray tracing instead of shadow maps: exact at any distance, no acne or leaks. Needs engine_info surface.ray_tracing and no MSAA; otherwise shadow maps", "properties": {
                    "enabled": { "type": "boolean" },
                    "softness": { "type": "number", "description": "Directional lights' apparent size in degrees: 0 hard shadows, 0.5 like the sun (soft edges smoothed by TAA)" },
                    "light_radius": { "type": "number", "description": "Point and spot lights' radius in meters for soft shadows (default 0.05, 0 hard)" }
                }},
                "ssr": { "type": "object", "description": "Reflections: smooth surfaces (low material roughness) reflect their surroundings as a blurred, glossy sheen. Screen-space (only what is on screen) unless ray_traced", "properties": {
                    "enabled": { "type": "boolean" },
                    "ray_traced": { "type": "boolean", "description": "Trace against the scene with hardware ray tracing (sees off-screen and hidden objects); falls back to screen space when engine_info surface.ray_tracing is unavailable" },
                    "full_resolution": { "type": "boolean", "description": "One ray per pixel instead of per 2x2 pixels: sharp mirrors at 4x the trace cost (default false)" },
                    "intensity": { "type": "number", "description": "1 = physically based for the material (0-4)" },
                    "max_distance": { "type": "number", "description": "How far a reflected ray travels, meters" },
                    "thickness": { "type": "number", "description": "Screen space only: how far behind a surface a ray may pass and still hit it, meters" },
                    "blur": { "type": "number", "description": "How much material roughness blurs reflections: 0 all sharp .. 1 rough surfaces very glossy (smooth ones stay sharp)" }
                }},
                "motion_blur": { "type": "object", "description": "Camera and per-object motion blur", "properties": {
                    "enabled": { "type": "boolean" },
                    "intensity": { "type": "number", "description": "Shutter: share of the frame it is open, 0.5 = 180 degrees" },
                    "max_blur": { "type": "number", "description": "Longest streak in pixels at 1080p" }
                }},
                "anti_aliasing": { "type": "string", "enum": ["none", "fxaa", "taa"], "description": "taa: temporal (jittered, accumulated over frames): smoothest and most stable, can trail on fast motion" },
                "upscaling": { "type": "object", "description": "Render the scene below the output resolution and upscale it: 3D and ray-traced passes cost less, post effects run at output resolution. Ignored with MSAA", "properties": {
                    "render_scale": { "type": "number", "description": "Share of the output resolution per axis, 0.25-1 (1 = native, 0.67 FSR quality, 0.5 performance)" },
                    "method": { "type": "string", "enum": ["fsr", "taau"], "description": "fsr (default): AMD FSR 3.1 temporal upscaling ported to WGSL, sharpest and most stable (jitters the camera itself); taau: the engine's cheaper TAA upsampling, accumulating only with anti_aliasing taa" },
                    "sharpness": { "type": "number", "description": "FSR's RCAS sharpening, 0 (off) to 1 (default 0.5)" }
                }}
            }), &[]),
            action: Action::Engine("postprocess.set"),
        },
        Tool {
            name: "visual_capture",
            description: "Render one frozen view under several post-processing variants, save the frames and compare them, for visual comparisons (does TAA ghost? does SSAO cause the halo?) without editing the scene. The variants are render-only overrides: the scene is never touched, the camera is locked, play mode is frozen (time scale 0) while it runs, and the override and time scale are put back afterwards. Per variant: the override is applied, temporal anti-aliasing settles for settle_frames, then `frames` consecutive frames are saved as full-resolution PNGs of the scene viewport (the editor UI and grid are left out). The result and dir/manifest.json record per image the engine frame, TAA jitter index and scene time, and per variant the settings that differ from the baseline (manifest.json has the full effective settings). Compared by default (compare: false to skip): each variant's last image against the baseline's (the first variant's) as max difference, mean difference, `smooth` (mean difference of 8x8 block averages, which averages out the output dither that makes every pair of frames differ by about 0.8 levels per pixel, so it is the number for faint or broad changes) and the share of pixels over `threshold` levels, for the whole image and a grid of regions (the reply lists the regions most above the noise, manifest.json every one), plus an amplified <name>_diff.png. The same numbers between a variant's own consecutive frames (dither and TAA jitter) give the noise and a <name>_noise.png: `differs` says whether a difference clearly exceeds that noise (needs frames >= 2) and `score` by how many times. `settle` says whether the last 16 settle frames still changed (`converged`, with warnings; needs settle_frames of 16), meaning settle_frames should be raised. Each TAA variant's first image waits up to one jitter period (7 frames, up to 63 when upscaling) to share the baseline's jitter phase. A few seconds of rendering per variant.",
            schema: object(json!({
                "variants": { "type": "array", "maxItems": 16, "description": "What to compare against the baseline", "items": { "type": "object", "properties": {
                    "name": { "type": "string", "description": "Letters, digits, - and _; names the image files" },
                    "postprocess": { "type": "object", "description": "Settings to change, as for postprocess_set (e.g. {anti_aliasing: \"none\"}, {ssao: {enabled: false}}); names are checked" },
                    "base": { "type": "string", "enum": ["scene", "defaults"], "default": "scene", "description": "scene changes the scene's own settings; defaults changes the engine defaults, to isolate one effect" }
                }, "required": ["name"] }},
                "include_baseline": { "type": "boolean", "default": true, "description": "Capture the scene's own post-processing first, as \"baseline\"" },
                "frames": { "type": "integer", "minimum": 1, "maximum": 32, "default": 2, "description": "Consecutive frames to capture per variant (each has a different TAA jitter)" },
                "settle_frames": { "type": "integer", "minimum": 0, "maximum": 600, "default": 16, "description": "Frames rendered after applying a variant before capturing, for TAA history to converge" },
                "compare": { "type": "boolean", "default": true, "description": "Compare the frames (differences to the baseline, jitter noise, settle check). false only saves the images" },
                "grid": { "anyOf": [{ "type": "integer", "minimum": 1, "maximum": 16 }, { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 16 }, "minItems": 2, "maxItems": 2 }], "default": 4, "description": "Regions to compare: N for N x N, or [columns, rows]" },
                "threshold": { "type": "integer", "minimum": 0, "maximum": 255, "default": 4, "description": "A pixel counts as different when its largest R/G/B difference is greater than this many 8-bit levels. Keep it above the dither (up to about 3 levels), or every pixel counts" },
                "diff_gain": { "type": "number", "minimum": 1, "maximum": 255, "default": 16, "description": "Amplification of the difference images (3 levels of difference show as 48 at the default)" },
                "settle_check": { "type": "boolean", "default": true, "description": "Read back the last 16 settle frames of each variant to see whether TAA history stopped changing (costs one GPU readback per frame; false skips it)" },
                "camera": { "anyOf": [{ "type": "integer" }, { "type": "string" }, { "type": "object" }], "description": "The view to lock: \"game\", a camera entity, or a pose {position, target, fov_deg} (as for screenshot). Default: the current view camera." },
                "dir": { "type": "string", "description": "Directory for the PNGs and manifest.json (default: a new folder under the system temp directory)" },
                "max_size": { "type": "integer", "minimum": 16, "maximum": 8192, "default": 8192, "description": "Longest side in pixels; larger frames are scaled down" },
                "freeze_noise": { "type": "boolean", "default": false, "description": "Hold the renderer's per-frame noise still while capturing (TAA jitter phase, grain and dither pattern, live light probe updates), so an unchanged view renders the same image every frame and captures compare exactly, also across builds (visual_compare). Temporal history still needs settle_frames" },
                "overlays": { "type": "boolean", "default": false, "description": "Keep the FPS readout and editor grid in the frames (off by default, so they do not count as differences)" }
            }), &[]),
            action: Action::Engine("visual.capture"),
        },
        Tool {
            name: "visual_compare",
            description: "Compare two saved captures without rendering, with visual_capture's metrics: a and b are PNG files or visual_capture directories (variant picks one of a directory's variants, else the first). b is compared with a as a variant with its baseline: mean and smooth (8x8 block mean) difference in 8-bit levels, max, share of pixels over threshold, per region; with two or more images a side, the noise between consecutive images and whether b differs beyond it. Writes b_diff.png (amplified) to dir. For comparing builds: capture the same frozen view in each (visual_capture, ideally freeze_noise: true, or screenshot {save_path, overlays: false}), then compare the files or directories.",
            schema: object(json!({
                "a": { "type": "string", "description": "Reference: a PNG file or a visual_capture directory" },
                "b": { "type": "string", "description": "The capture to compare with it" },
                "variant": { "type": "string", "description": "For directories: the variant to take the images from (default the first, usually baseline)" },
                "grid": { "anyOf": [{ "type": "integer", "minimum": 1, "maximum": 16 }, { "type": "array", "items": { "type": "integer", "minimum": 1, "maximum": 16 }, "minItems": 2, "maxItems": 2 }], "default": 4, "description": "Regions to compare: N for N x N, or [columns, rows]" },
                "threshold": { "type": "integer", "minimum": 0, "maximum": 255, "default": 4, "description": "A pixel counts as different above this many 8-bit levels" },
                "diff_gain": { "type": "number", "minimum": 1, "maximum": 255, "default": 16, "description": "Amplification of the difference image" },
                "dir": { "type": "string", "description": "Where the difference image goes (default: a new folder under the system temp directory)" }
            }), &["a", "b"]),
            action: Action::Engine("visual.compare"),
        },
        Tool {
            name: "render_freeze_noise",
            description: "Hold the renderer's per-frame noise still (enabled: true) or let it run again (false): the TAA jitter stays at its phase, film grain and dither keep their pattern, and light probes stop tracing new rays (a bake pauses too). With the scene and camera unchanged every frame then renders the same image, so screenshots taken in different builds or sessions compare exactly (visual_compare). Temporal history keeps accumulating, so wait a few frames after freezing. Capture-only: anti-aliasing is slightly worse while on, so turn it off afterwards.",
            schema: object(json!({ "enabled": { "type": "boolean", "default": true } }), &[]),
            action: Action::Engine("render.freeze_noise"),
        },
        Tool {
            name: "texture_fetch",
            description: "Read a render target back from the GPU and look at it with numbers: the G-buffer (normals, motion, world position, pick ids), depth, the HDR scene before tonemapping, shadow maps (array layers), the ray-traced shadow mask, SSAO, reflections, TAA, and the path tracer's buffers (raw and denoised diffuse/specular, denoiser history and moments, radiance cache, caustic photon sums). Without texture it lists every target with its size, format and what each channel holds. With one it reads a rectangle (default all) and returns per-channel min, max, mean and NaN/infinity counts, the values at `points`, and an image of up to three channels. Targets hold what the last frame left in them; nothing on screen changes. Use it to debug rendering (is the normal right, did the history reset, where are the NaNs, how bright is that pixel before tonemapping) instead of adding debug code. Native builds only.",
            schema: object(json!({
                "texture": { "type": "string", "description": "Target name, e.g. gbuffer.normal, scene, depth, pt.history.specular; omit to list them" },
                "rect": { "type": "array", "items": { "type": "integer" }, "minItems": 4, "maxItems": 4, "description": "[x, y, width, height] in the texture's pixels (default the whole texture). Path tracing targets are the scene region's size (half of it with half-resolution global illumination)" },
                "layer": { "type": "integer", "description": "Array layer (shadows.directional: light * cascades + cascade, the nearest cascade first)" },
                "points": { "type": "array", "items": { "type": "array", "items": { "type": "integer" } }, "description": "[[x, y], ...] pixels (inside rect) whose raw values to return" },
                "channels": { "type": "string", "description": "One to three of r, g, b, a for the image (default rgb, or the texture's own channels when fewer)" },
                "mode": { "type": "string", "enum": ["range", "tonemap", "false_color"], "description": "range (default): linear from range low to high; tonemap: HDR colour through Reinhard and sRGB; false_color: one channel through a Turbo colour map" },
                "range": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2, "description": "[low, high] values mapped to black and white (default the 0.5th to 99.5th percentile)" },
                "image": { "type": "boolean", "description": "Return the image (default true); false for numbers only" },
                "max_size": { "type": "integer", "description": "Longest image side in pixels (default 1024; shrunk nearest-neighbour so values are not blended)" },
                "save_path": { "type": "string", "description": "Also write the PNG here" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "shader_reload",
            description: "Read the renderer's WGSL from src/shader on disk again and rebuild the passes that use it, without a rebuild or restart: the main pass (PBR, shader materials, depth prepass, background), every post-processing pass including ray-traced reflections, RT AO and the path tracer with its denoiser, and ray-traced shadows. A running engine built from this repository also does this by itself when a .wgsl file is saved (log target \"shader\"). A shader that fails to compile changes nothing: the old pipelines keep running and the error comes back with naga's message. Returns the changed files, and not_reloaded: changed files only used by passes that are not rebuilt (shadow maps, skinning, particles, environment, HDR output), which need engine_launch {restart: true}. Rust-side changes (bind group layouts, uniforms, new passes) still need a rebuild. Native builds only.",
            schema: empty(),
            action: Action::Engine("shader.reload"),
        },
        Tool {
            name: "logs",
            description: "Recent engine log lines (scripts log with target \"script\"; script errors appear here too). Pass after = latest_seq from a previous call to get only new lines.",
            schema: object(json!({
                "after": { "type": "integer" },
                "since_play_start": { "type": "boolean", "description": "Only lines logged since the most recent play session started, so errors from before it (already fixed) do not show up" },
                "min_level": { "type": "string", "enum": ["error", "warn", "info", "debug", "trace"], "description": "Default info" },
                "contains": { "type": "string" },
                "target": { "type": "string", "description": "Log target prefix, e.g. script, remote, wgpu" },
                "limit": { "type": "integer", "description": "Most recent N (default 100)" }
            }), &[]),
            action: Action::Engine("logs"),
        },
        // ---- Editor: projects and scenes ----
        Tool {
            name: "editor_state",
            description: "Editor only: open project (directory, content and scripts folders), scene documents, open scene tabs, selection, undo/redo availability and whether the startup dialog is blocking.",
            schema: empty(),
            action: Action::Engine("editor.state"),
        },
        Tool {
            name: "project_create",
            description: "Editor only: create a new project at location/name and open it; returns once it is open, with the editor state. Its content/ folder holds scenes, scripts (content/scripts), audio and textures. By default the scene starts with a starter set-up (dark backdrop, ambient light, a shadowed sun, an active GameCamera looking at the origin, a 40 m Floor with a collider) and is saved.",
            schema: object(json!({
                "name": { "type": "string" },
                "location": { "type": "string", "description": "Parent directory" },
                "template": { "type": "string", "enum": ["starter", "empty"], "description": "starter (default) or empty" }
            }), &["name", "location"]),
            action: Action::Local,
        },
        Tool {
            name: "project_open",
            description: "Editor only: open an existing project directory (containing project.json); returns once it is open.",
            schema: object(json!({ "path": { "type": "string" } }), &["path"]),
            action: Action::Local,
        },
        Tool {
            name: "project_save",
            description: "Editor only: save scenes, environment and editor camera to the project directory or path. Stop play first. Returns after writing, with project/scene paths and a written-file count. Files whose content did not change are not rewritten, and each scene file keeps the layout it has on disk (compact or indented; new files are indented) unless compact is given. verbose includes all changed paths.",
            schema: object(json!({
                "path": { "type": "string" },
                "verbose": { "type": "boolean", "default": false },
                "compact": { "type": "boolean", "description": "true writes every scene file compact, false indents every one; omit to keep each file's layout. Same data and embedded meshes either way" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "scene_new",
            description: "Editor only: open a new empty scene tab and make it active.",
            schema: empty(),
            action: Action::Engine("scene.new"),
        },
        Tool {
            name: "scene_activate",
            description: "Editor only: switch to another open scene or a scene document of the project.",
            schema: object(json!({ "document_id": { "type": "string" } }), &["document_id"]),
            action: Action::Engine("scene.activate"),
        },
        Tool {
            name: "scene_close",
            description: "Editor only: close an open scene tab.",
            schema: object(json!({ "document_id": { "type": "string" } }), &["document_id"]),
            action: Action::Engine("scene.close"),
        },
        Tool {
            name: "scene_rename",
            description: "Editor only: rename a scene document.",
            schema: object(json!({ "document_id": { "type": "string" }, "name": { "type": "string" } }), &["document_id", "name"]),
            action: Action::Engine("scene.rename"),
        },
        Tool {
            name: "scene_instance",
            description: "Editor only: instance another scene document (or a node path inside it) into the active scene as a prefab.",
            schema: object(json!({
                "document_id": { "type": "string" },
                "node_path": { "type": "array", "items": { "type": "string" } }
            }), &["document_id"]),
            action: Action::Engine("scene.instance"),
        },
        Tool {
            name: "asset_import",
            description: "Editor only: bring a file into the project. A glTF/GLB model is copied in, saved as a scene document and instanced into the active scene (example models are under web/assets/; skinned characters are under web/assets/skinned/ and get an animator playing their first clip in play mode; scale the '(Instance)' root for models authored in centimetres). FBX, .blend and OBJ files are converted with Blender in the background first (Mixamo FBX included; watch logs). A file with animations but no meshes (or kind: animation, e.g. Mixamo's \"Without Skin\" downloads) is a clip library instead: it is copied to content/animations, named after the file, and its clips are listed, ready for entity_update {animator: {add_clips}}; pass entity to add it to that character's animator right away. Images, audio, fonts, .lua and .wgsl files are copied into content/textures, audio, fonts, scripts or shaders (or dest) and the result gives the project-relative path to reference. Replacing a texture the scene uses reloads it.",
            schema: object(json!({
                "path": { "type": "string", "description": "Source file" },
                "dest": { "type": "string", "description": "Non-model files: target path relative to the project's content folder, e.g. textures/ui/logo.png" },
                "overwrite": { "type": "boolean", "description": "Replace an existing file at the target (default false)" },
                "kind": { "type": "string", "enum": ["model", "animation"], "description": "Import as a model (instanced) or as an animation clip library. Default: animation when the file has animations and no meshes" },
                "entity": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "Animations: add the clip library to this character's animator (its root, model or the entity its AnimationTree is on)" }
            }), &["path"]),
            action: Action::Engine("asset.import"),
        },
        Tool {
            name: "character_make",
            description: "Editor only, while editing: make an imported model playable, like the inspector's Make Character. The model goes under a new entity (default name Player) with a CharacterController sized to it, an AnimationTree generated from its clips (idle/walk/run/strafe/back/jump/fall/land by name), and the active camera gets a SpringCamera following it (or a camera is made). A model facing -Z is turned to face +Z. script: true also writes content/scripts/<name>.lua with the engine's third-person controls (require(\"engine.third_person\")). Returns the entities, size, tree states and notes (e.g. that nothing has a Collider, so the character would fall: give the floor one).",
            schema: object(json!({
                "entity": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "The model: its '(Instance)' root or any entity with its meshes below" },
                "name": { "type": "string", "description": "Name of the character entity (default Player, or '<model> Character' when taken)" },
                "camera": { "type": "boolean", "description": "Have the active camera follow it with a SpringCamera (default true)" },
                "script": { "anyOf": [{ "type": "boolean" }, { "type": "string" }], "description": "true writes content/scripts/<name>.lua with third-person controls; a project-relative path writes (or attaches an existing) script there" }
            }), &["entity"]),
            action: Action::Engine("character.make"),
        },
        Tool {
            name: "animation_inspect",
            description: "List the skeletal animation clips in a glTF/GLB file (name, duration, animated bones) without importing or attaching it. Paths are absolute or project-relative.",
            schema: object(json!({ "path": { "type": "string" } }), &["path"]),
            action: Action::Engine("animation.inspect"),
        },
        Tool {
            name: "debug_skeleton",
            description: "Draw the bones of every skinned mesh as orange overlays (in the viewport and in screenshots), to check that a rig sits inside its mesh and follows the animation. Returns the bone count.",
            schema: object(json!({ "enabled": { "type": "boolean" } }), &[]),
            action: Action::Engine("debug.skeleton"),
        },
        Tool {
            name: "debug_physics",
            description: "Draw every collider the physics simulation has as wireframes (green fixed, orange dynamic, blue kinematic, cyan characters, yellow sensors), in the viewport and screenshots. The simulation exists only in play mode (bodies is 0 when editing); the editor always outlines the selected entity's collider. Returns body and step counts and gravity.",
            schema: object(json!({ "enabled": { "type": "boolean" } }), &[]),
            action: Action::Engine("debug.physics"),
        },
        Tool {
            name: "navmesh_bake",
            description: "Bake the navmesh of a nav_mesh_surface (entity, or the scene's first) from the scene's static geometry, in the background, and wait for it: saved as content/navigation/<surface name>.navmesh with the surface's baked path set (project_save keeps it). Returns the result (tiles, polygons, triangles, milliseconds) and the navigation status. Editing only (stop play first).",
            schema: object(json!({
                "entity": entity_ref(),
                "timeout_seconds": { "type": "number", "description": "How long to wait (default 180)" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "navigation_status",
            description: "The scene's navmeshes (tiles, polygons, baked or built at play start, stale settings, load errors), agents, obstacles (and how many carve), the debug view, a running bake and the last one.",
            schema: empty(),
            action: Action::Engine("navigation.status"),
        },
        Tool {
            name: "navigation_debug",
            description: "Draw navigation in the view and screenshots (depth tested): mesh (polygons tinted by area, walls dark, tiles checkered), agents (bodies coloured by state: grey idle, green walking, blue arrived, red failed; velocity arrows), paths (agents' corners to their destinations), obstacles (red carving, orange moving), x_ray (show what scenery hides, faintly). all: true or false sets every layer. Only given fields change.",
            schema: object(json!({
                "all": { "type": "boolean" },
                "mesh": { "type": "boolean" },
                "agents": { "type": "boolean" },
                "paths": { "type": "boolean" },
                "obstacles": { "type": "boolean" },
                "x_ray": { "type": "boolean" }
            }), &[]),
            action: Action::Engine("navigation.debug"),
        },
        Tool {
            name: "navigation_path",
            description: "Find a path over the navmesh between two points ([x,y,z]) or entities, and draw it (magenta; red when the goal cannot be reached and it stops as near as it can). Returns complete, length and the corner points. clear: true removes the drawn path.",
            schema: object(json!({
                "from": { "anyOf": [{ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, { "type": "integer" }, { "type": "string" }] },
                "to": { "anyOf": [{ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }, { "type": "integer" }, { "type": "string" }] },
                "show": { "type": "boolean", "description": "Draw it (default true)" },
                "clear": { "type": "boolean" }
            }), &[]),
            action: Action::Engine("navigation.path"),
        },
        Tool {
            name: "navigation_agents",
            description: "Every nav_agent with its state (idle, planning, moving, arrived, failed, off_mesh, lost), position, speed, destination, distance left and whether its path is blocked (partial). Agents walk only in play mode.",
            schema: empty(),
            action: Action::Engine("navigation.agents"),
        },
        Tool {
            name: "ai_status",
            description: "Every AI agent (ai_agent component, play mode): its brain, state path (machines: outer/inner), current action (the running tree leaf), tags, group and last error; counts of agents per brain and state; the brains defined; and step stats (agents, ticked, looked, events, step_ms).",
            schema: empty(),
            action: Action::Engine("ai.status"),
        },
        Tool {
            name: "ai_inspect",
            description: "One AI agent in depth: its blackboard (and the group and global boards), its brain as it runs (a tree's nodes with their last results, what is running, utility scores and time left; a machine's states, transitions and recent transition history with reasons), its senses (what it sees, remembers and heard) and events waiting for its next tick.",
            schema: object(json!({ "entity": entity_ref() }), &["entity"]),
            action: Action::Engine("ai.inspect"),
        },
        Tool {
            name: "ai_define",
            description: "Define (or redefine) a brain from JSON; agents running that name start it again. A tree is its root node: {\"type\": \"selector\", \"children\": [...], \"name\", \"when\": \"expr\", \"abort\": \"self|lower|both\", \"cooldown\", \"timeout\", \"chance\", \"score\", \"weight\"} with composites sequence, selector, reactive_sequence, reactive_selector, parallel {success, failure}, shuffle_sequence, shuffle_selector, random, utility {interval, hysteresis}; decorators invert, force_success, force_failure, repeat {count, until}, retry {count}, timeout, delay, cooldown {time}, once, chance {p}, guard {when, abort}; leaves wait {time}, log, set {key, value | expr}, clear, add, check {expr}, wait_until, lua {run, enter, exit: function names}, send {event, to, data}, broadcast, noise {tag, radius}, emit, wait_event {event, store}, move_to {target, radius, speed, run}, wander, flee {from, distance}, follow {target, distance}, face {target | yaw}, stop, patrol {points}, pick_point, play {clip, layer, time, loop}, stop_animation, anim_param, anim_trigger, find {tag, store}, subtree {tree}. A machine is {\"initial\", \"states\": {name: {enter, update, exit, tree, machine, set, final, transitions: [{to, event, when, after, on, chance, store, store_from, store_position}]}}, \"transitions\": [machine-wide, with except]}. Expressions read the blackboard (hunger > 0.5 and not group.alarm) and call dist, sees, heard, count, nearest, in_state, time, state_time, random, chance. Lives until play stops (scripts usually define brains in on_created). See docs/ai.md.",
            schema: object(json!({
                "name": { "type": "string" },
                "brain": { "type": "object", "description": "A tree's root node or a machine" }
            }), &["name", "brain"]),
            action: Action::Engine("ai.define"),
        },
        Tool {
            name: "ai_validate",
            description: "Check a brain definition (as for ai_define) without defining it: its nodes or states, or what is wrong and where.",
            schema: object(json!({ "brain": { "type": "object" } }), &["brain"]),
            action: Action::Engine("ai.validate"),
        },
        Tool {
            name: "ai_send",
            description: "Send an AI event: to one agent (entity), or to every agent, to those within radius of center (with tag), or to a group. Machines take transitions with event = name; wait_event leaves finish. data becomes the event's data (stored by transitions with store).",
            schema: object(json!({
                "entity": entity_ref(),
                "event": { "type": "string" },
                "data": {},
                "radius": { "type": "number" },
                "center": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "tag": { "type": "string" },
                "group": { "type": "string" }
            }), &["event"]),
            action: Action::Engine("ai.send"),
        },
        Tool {
            name: "ai_set",
            description: "Set a blackboard value of an agent (entity, key, value); key group.x sets its group's board, global.x the scene's (no entity needed), or pass group to set that group's board. [x,y,z] arrays become positions. Returns the board.",
            schema: object(json!({
                "entity": entity_ref(),
                "key": { "type": "string" },
                "value": {},
                "group": { "type": "string" }
            }), &["key"]),
            action: Action::Engine("ai.set"),
        },
        Tool {
            name: "ai_noise",
            description: "Make a noise at a point: agents within radius and their hearing range get hear and hear:<tag> events and remember it (heard('tag') in expressions).",
            schema: object(json!({
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "radius": { "type": "number" },
                "tag": { "type": "string" },
                "data": {}
            }), &["position"]),
            action: Action::Engine("ai.noise"),
        },
        Tool {
            name: "ai_control",
            description: "Restart an agent's brain (restart: true), pause or resume it (paused).",
            schema: object(json!({
                "entity": entity_ref(),
                "restart": { "type": "boolean" },
                "paused": { "type": "boolean" }
            }), &["entity"]),
            action: Action::Local,
        },
        Tool {
            name: "ai_debug",
            description: "Draw what agents think, in the view and screenshots: senses (view cones, near range; the selected agent's hearing range), goals (lines to where they walk), percepts (lines to what they see, green, or remember, grey; heard noises), states (a ring at each agent's feet in its state's colour). all: true or false sets every layer. Selected agents always show theirs.",
            schema: object(json!({
                "all": { "type": "boolean" },
                "senses": { "type": "boolean" },
                "goals": { "type": "boolean" },
                "percepts": { "type": "boolean" },
                "states": { "type": "boolean" }
            }), &[]),
            action: Action::Engine("ai.debug"),
        },
        Tool {
            name: "asset_reload",
            description: "Re-read textures (and scripts) from disk after changing the files, so the scene shows the new images without renaming them. Without path, every texture loaded from a file is reloaded. The editor also does this by itself when an image in the project changes.",
            schema: object(json!({ "path": { "type": "string", "description": "A file or folder (absolute, or relative to the project)" } }), &[]),
            action: Action::Engine("asset.reload"),
        },
        Tool {
            name: "editor_select",
            description: "Editor only: select an entity (shows it in the inspector), or null to clear. With uv instead, click the Scene view there: the same GPU pick a mouse click makes (what is visible under that point, cut-out holes and gizmos included); the selection follows a few frames later in editor_state.selected. gizmo / gizmo_space switch the transform tool (the W/E/R shortcuts), alone or together with a selection.",
            schema: object(json!({
                "entity": { "anyOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }] },
                "gizmo": { "type": "string", "enum": ["translate", "rotate", "scale"], "description": "Transform tool shown on the selection" },
                "gizmo_space": { "type": "string", "enum": ["local", "world"], "description": "Whether the gizmo follows the entity's rotation" },
                "uv": { "type": "array", "items": { "type": "number", "minimum": 0, "maximum": 1 }, "minItems": 2, "maxItems": 2, "description": "[u, v] across the Scene view, 0..1 from its top-left corner" }
            }), &[]),
            action: Action::Engine("editor.select"),
        },
        Tool {
            name: "editor_undo",
            description: "Editor only: undo the last scene change.",
            schema: empty(),
            action: Action::Engine("editor.undo"),
        },
        Tool {
            name: "editor_redo",
            description: "Editor only: redo the last undone change.",
            schema: empty(),
            action: Action::Engine("editor.redo"),
        },
    ];
    tools.extend(procedural_tools());
    tools.extend(geometry_tools());
    tools.extend(additional_tools());
    for tool in &mut tools {
        match tool.name {
            "batch" => {
                tool.description="Run tool calls sequentially; atomic true validates scene-only calls and commits as one undo step, with rollback on failure. dry_run validates without committing. Calls may name results with as and reference prior results using {$ref: alias/id}. Normal batches stop on error by default.";
                tool.schema["properties"]["atomic"] = json!({"type":"boolean","description":"Run scene-only calls in one engine transaction with rollback and one undo step"});
                tool.schema["properties"]["dry_run"] = json!({"type":"boolean","description":"Validate a speculative scene transaction without committing (implies atomic)"});
                tool.schema["properties"]["calls"]["items"]["properties"]["as"] = json!({"type":"string","description":"Name this result for later {$ref: alias/id} references"});
            }
            "screenshot" => {
                tool.schema["properties"]["overlays"] = json!({"type":"boolean","default":true,"description":"false keeps the FPS readout and the editor grid out of the frame (captured a frame later, so the UI can be rebuilt without them): for clean images and build-to-build comparisons with visual_compare"});
            }
            "runtime_set" => {
                tool.schema["properties"]["manual"] = json!({"type":"boolean","description":"Before play: freeze automatic Lua/animation updates; advance using runtime_step"});
                tool.schema["properties"]["seed"] = json!({"type":"integer","minimum":0,"description":"Seed Lua's PRNG before scripts load and on_created runs"});
            }
            "playtest" => {
                tool.description="Playtest with wall-clock seconds or exact steps/dt and seeded Lua randomness. Optional setup runs after initialization; assertions check boolean Lua expressions at checkpoints. Returns state/eval, failed assertions/scripts, logs, sounds, scene diff and optional screenshot, then stops. scenario/save_scenario load/save JSON test parameters and input. Manual stepping covers Lua gameplay and scene animation; rendering/UI/GPU particles/custom app systems are independent.";
                tool.schema["properties"]["steps"] = json!({"type":"integer","minimum":1,"maximum":10000,"description":"Use fixed simulation steps instead of wall-clock seconds"});
                tool.schema["properties"]["dt"] = json!({"type":"number","minimum":0.000001,"maximum":1,"default":0.016666666666666666});
                tool.schema["properties"]["seed"] = json!({"type":"integer","minimum":0});
                tool.schema["properties"]["setup"] = json!({"type":"string","description":"Lua setup hook after on_created, before input and simulation"});
                tool.schema["properties"]["assertions"] = json!({"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"code":{"type":"string","description":"Lua expression returning true"},"at_step":{"type":"integer","minimum":0}},"required":["code"]}});
                tool.schema["properties"]["scenario"] = json!({"type":"string","description":"Read test parameters and saved input from a JSON file; explicit arguments override it"});
                tool.schema["properties"]["save_scenario"] = json!({"type":"string","description":"Save resolved test parameters before simulation; creates parent directories and refuses existing files"});
            }
            _ => {}
        }
    }
    tools
}

fn additional_tools() -> Vec<Tool> {
    let region = json!({"type":"object","properties":{"min":vec3("Minimum world corner"),"max":vec3("Maximum world corner")},"required":["min","max"]});
    let ray = json!({"origin":vec3("World ray origin"),"direction":vec3("Nonzero ray direction"),"max_distance":{"type":"number","exclusiveMinimum":0,"default":10000},"exclude":{"type":"array","items":entity_ref()},"visible_only":{"type":"boolean","default":true},"name_contains":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":1000,"default":1}});
    let mut placement = ray.clone();
    placement["entity"] = entity_ref();
    placement["offset"] = json!({"type":"number","default":0});
    placement["align_normal"] = json!({"type":"boolean","default":false});
    placement["avoid_overlap"] = json!({"type":"boolean","default":false});
    let mut sweep = entity_fields();
    sweep.extend(json!({"entity":entity_ref(),"path":{"type":"array","items":vec3("Polyline point"),"minItems":2,"maxItems":4096},"profile":{"type":"array","items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"description":"Optional simple XY cross-section; defaults to a circle"},"radius":{"type":"number","exclusiveMinimum":0,"default":0.1},"segments":{"type":"integer","minimum":3,"maximum":512,"default":24},"caps":{"type":"boolean","default":true}}).as_object().unwrap().clone());
    let mut terrain = entity_fields();
    terrain.extend(json!({"entity":entity_ref(),"resolution":{"type":"array","items":{"type":"integer","minimum":2,"maximum":512},"minItems":2,"maxItems":2,"default":[64,64]},"size":{"type":"array","items":{"type":"number","exclusiveMinimum":0},"minItems":2,"maxItems":2,"default":[20,20]},"heights":{"type":"array","items":{"type":"number"},"description":"Row-major values, x fastest; mutually exclusive with source"},"source":{"type":"string","description":"Height image, resized to resolution; luminance values multiplied by height_scale"},"height_scale":{"type":"number","default":3},"seed":{"type":"integer","minimum":0},"noise_scale":{"type":"number","exclusiveMinimum":0,"default":4},"octaves":{"type":"integer","minimum":1,"maximum":12,"default":4}}).as_object().unwrap().clone());
    vec![
        Tool{name:"runtime_step",description:"Advance exactly N Lua/gameplay and scene-animation steps while rendering remains independent. Requires manual play mode. Input delay and tap frames are simulation-step indices within this call. GPU particles and custom app systems are outside the deterministic scope.",schema:object(json!({"steps":{"type":"integer","minimum":1,"maximum":10000,"default":1},"dt":{"type":"number","minimum":0.000001,"maximum":1,"default":0.016666666666666666},"input":{"type":"array","items":{"type":"object"}}}),&[]),action:Action::Engine("runtime.step")},
        Tool{name:"scene_raycast",description:"Intersect transformed mesh world AABBs; returns distance-sorted hits, positions and box-face normals. This is an approximate bounding-box query, not triangle collision.",schema:object(ray,&["origin","direction"]),action:Action::Engine("scene.raycast")},
        Tool{name:"scene_query",description:"Find visible mesh bounds overlapping a world region or within a radius of center; returns paginated entity summaries and world bounds. Hidden ancestors are respected.",schema:object(json!({"bounds":region,"center":vec3("Distance-query center"),"radius":{"type":"number","minimum":0},"exclude":{"type":"array","items":entity_ref()},"name_contains":{"type":"string"},"visible_only":{"type":"boolean","default":true},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":1000}}),&[]),action:Action::Engine("scene.query")},
        Tool{name:"entity_place",description:"Place an entity hierarchy on the closest ray-hit bounding-box surface, using its support point. Supports normal alignment, offset and optional overlap rejection; parent transforms are respected.",schema:object(placement,&["entity","origin","direction"]),action:Action::Engine("entity.place")},
        Tool{name:"scene_scatter",description:"Seeded copies of an entity hierarchy within XZ bounds, optionally dropped onto world AABB surfaces. Enforces spacing, slope and overlap constraints. All requested placements must fit or no copies are made.",schema:object(json!({"entity":entity_ref(),"bounds":region,"count":{"type":"integer","minimum":1,"maximum":1000,"default":10},"seed":{"type":"integer","minimum":0},"spacing":{"type":"number","minimum":0,"default":1},"on_surface":{"type":"boolean","default":true},"max_slope":{"type":"number","minimum":0,"maximum":90,"default":45},"avoid_overlap":{"type":"boolean","default":true}}),&["entity","bounds"]),action:Action::Engine("scene.scatter")},
        Tool{name:"prefab_create",description:"Editor: save an entity hierarchy as a portable scene document, embedding procedural geometry and resetting its root transform. Registers the document for scene_instance. Save the project to persist registration.",schema:object(json!({"entity":entity_ref(),"name":{"type":"string"}}),&["entity","name"]),action:Action::Engine("prefab.create")},
        Tool{name:"prefab_inspect",description:"Editor: inspect a prefab source, linked instance entities and serialized instance overrides.",schema:object(json!({"document_id":{"type":"string"}}),&["document_id"]),action:Action::Engine("prefab.inspect")},
        Tool{name:"prefab_update",description:"Editor: replace a prefab source with a selected hierarchy. New instances use the new source; existing instances keep current values until recreated or reopened. Save the project to persist registration.",schema:object(json!({"entity":entity_ref(),"document_id":{"type":"string"}}),&["entity","document_id"]),action:Action::Engine("prefab.update")},
        Tool{name:"profile_capture",description:"Sample consecutive engine frames and report median/p95/p99/max of CPU frame and stage times, of the GPU time per render stage and in total (gpu_stages_ms, gpu_total_ms, from timestamp queries; native only), the renderer counters' min/max/last (draw calls, shadow views rendered/cached, culled instances), script cost and entity/asset growth. Optional baseline compares frame latency. Prefer the GPU percentiles over a single engine_stats reading when another process shares the GPU.",schema:object(json!({"frames":{"type":"integer","minimum":2,"maximum":10000,"default":300},"baseline":{"type":"object"}}),&[]),action:Action::Engine("profile.capture")},
        Tool{name:"renderdoc_capture",description:"Capture the next frame with RenderDoc and return the .rdc path: every pass, draw and dispatch with its pipeline state, bound resources, shaders and buffer/texture contents, for the renderdoc MCP server (open_capture, then e.g. get_frame_overview, get_draw_call_state, pixel_history) or qrenderdoc. Use it when a screenshot shows something wrong but not why. The engine must run under RenderDoc: engine_launch with renderdoc: true. Captures are tens to hundreds of MB; delete them when done.",schema:empty(),action:Action::Engine("renderdoc.capture")},
        Tool{name:"mesh_sweep",description:"Sweep a circular or polygon cross-section along an open 3D polyline, with transported frames, UVs, normals, tangents and caps. Sharp bends may self-intersect. Geometry is persisted by project_save.",schema:object(Value::Object(sweep),&["path"]),action:Action::Engine("mesh.sweep")},
        Tool{name:"mesh_heightfield",description:"Generate XZ terrain with upward slope normals from explicit heights, an image or seeded fractal noise. UVs/tangents and embedded geometry persist with project_save. Shares the procedural vertex budget.",schema:object(Value::Object(terrain),&[]),action:Action::Engine("mesh.heightfield")},
        Tool{name:"project_validate",description:"Through the running native engine: validate a saved project's scene JSON, asset references, portable paths, camera configuration and Lua syntax. Reports errors and warnings with file locations; does not execute scripts or write files. The browser block is a preflight for web builds: assets web_build would not serve (only project.json and content/ are staged), files differing only in case, and Lua calls that need a file system.",schema:object(json!({"path":{"type":"string","description":"Project directory; defaults to running engine's active project"}}),&[]),action:Action::Engine("project.validate")},
        Tool {
            name: "project_content",
            description: "What a build of a saved project carries: the files its main scene (the first in project.json), the prefabs it instances, their materials, model packages and scripts use, followed through scripts' string literals (play_sound(\"content/audio/x.wav\"), require modules; a literal naming a folder such as \"content/audio/\" keeps the whole folder, for paths built at run time). Lists files, scenes, bytes, what is left out (unused) and problems (missing files, files outside the project). The editor's Build menu, web_build and build_web.sh copy only these.",
            schema: object(json!({
                "path": { "type": "string", "description": "Project directory; default the open project" },
                "list_unused": { "type": "boolean", "description": "List the unused files too (default true)" }
            }), &[]),
            action: Action::Engine("project.content"),
        },
        Tool{name:"web_build",description:"Validate and package a saved project using the existing wasm build pipeline; only the content the game uses is staged (see project_content). Returns its output directory. Stops before building when the browser preflight finds errors (see project_validate) unless ignore_browser_issues is true. Does not launch a server or publish. Existing artifacts require overwrite true.",schema:object(json!({"project":{"type":"string"},"overwrite":{"type":"boolean","default":false},"ignore_browser_issues":{"type":"boolean","default":false,"description":"Package even when the browser preflight (validation.browser) reports errors, such as assets outside content/"}}),&["project"]),action:Action::Local},
        Tool {
            name: "library_search",
            description: "Editor only: search the asset library, the characters, animations and models shared by every project (Mixamo, Quaternius CC0, your own; in ~/.local/share/wgpu-cube/library). Words must all appear in the id, name or tags (categories: locomotion, idle, combat, hit, emote, interaction, loop, root-motion). Rows give id, kind, name, tags, skeleton (bone naming: mixamo, unreal, rigify; clips work on any humanoid rig, they bind by role), duration, root_speed (m/s over the ground), in_place, height and a thumbnail path. Then library_add takes what a game needs into its project.",
            schema: object(json!({
                "query": { "type": "string", "description": "e.g. \"sword attack\", \"walk\", \"idle\"" },
                "kind": { "type": "string", "enum": ["character", "animation", "model"] },
                "tags": { "type": "array", "items": { "type": "string" } },
                "skeleton": { "type": "string", "enum": ["mixamo", "unreal", "rigify", "custom"] },
                "limit": { "type": "integer", "description": "Default 50" },
                "offset": { "type": "integer" }
            }), &[]),
            action: Action::Engine("library.search"),
        },
        Tool {
            name: "library_info",
            description: "Editor only: the full catalog entries of library assets (clips with duration and ground speed, height, triangles, bones, source, license, file path).",
            schema: object(json!({
                "id": { "type": "string" },
                "ids": { "type": "array", "items": { "type": "string" } }
            }), &[]),
            action: Action::Engine("library.info"),
        },
        Tool {
            name: "library_add",
            description: "Editor only: take library assets into the open project. An animation is copied to content/animations/<name>.glb and, with entity, added to that character's animator (its root, model or the entity its AnimationTree is on), playable under that name; a character or model is imported into the project and the active scene like asset_import ('<name> (Instance)'). Only what is added becomes part of the game, so its export carries only those. Then animation_tree {generate: true} or character_make use the clips.",
            schema: object(json!({
                "id": { "type": "string" },
                "ids": { "type": "array", "items": { "type": "string" } },
                "entity": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "Character to add animations to" }
            }), &[]),
            action: Action::Engine("library.add"),
        },
        Tool {
            name: "library_fetch",
            description: "Download assets into the library (no editor needed; Blender required). source mixamo fetches named Mixamo animations (or characters with characters: true) with the user's own token from ~/.config/wgpu-cube/mixamo-token, measuring and thumbnailing each; Mixamo files are for the user's projects only (never commit or redistribute them). source quaternius-ual1 / quaternius-ual2 fetch Quaternius's CC0 animation libraries. A long list takes about 10 s per animation; already-present assets are skipped.",
            schema: object(json!({
                "source": { "type": "string", "enum": ["mixamo", "quaternius-ual1", "quaternius-ual2"] },
                "names": { "type": "array", "items": { "type": "string" }, "description": "Mixamo: animation (or character) names as Mixamo lists them, e.g. \"Walking\", \"Sword And Shield Slash\". Without names, the curated set in tools/library/mixamo_set.json" },
                "characters": { "type": "boolean", "description": "Mixamo: fetch characters (rigged, T-pose) instead of animations" }
            }), &["source"]),
            action: Action::Local,
        },
        Tool {
            name: "mixamo_catalog",
            description: "What Mixamo offers, to choose names for library_fetch (uses the user's Mixamo token; no editor needed): every character (characters: true; all share Mixamo's skeleton, so every library animation plays on them), or animations matching query with Mixamo's descriptions (several animations share a name; the description tells them apart).",
            schema: object(json!({
                "characters": { "type": "boolean" },
                "query": { "type": "string", "description": "e.g. \"sword\", \"climb\", \"zombie walk\"" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "library_ingest",
            description: "Add your own files to the library (no editor needed; Blender required): FBX, glTF/GLB, .blend or OBJ. Each animation becomes its own asset (measured, thumbnailed on a library character with the same rig), meshes become a character (rigged) or model asset.",
            schema: object(json!({
                "paths": { "type": "array", "items": { "type": "string" } },
                "name": { "type": "string", "description": "Name for a file's single animation or its character" },
                "provider": { "type": "string", "description": "Where it came from (default local)" },
                "license": { "type": "string" },
                "author": { "type": "string" },
                "url": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } }
            }), &["paths"]),
            action: Action::Local,
        },
        Tool{name:"web_playtest",description:"Browser smoke test of a web_build artifact using Node 22+ and Chromium's DevTools protocol, with no npm dependencies. Returns console/page/network failures and a screenshot; sends input to unlock audio. No deployment.",schema:object(json!({"directory":{"type":"string"},"seconds":{"type":"number","minimum":0,"maximum":60,"default":3},"viewport_size":{"type":"array","items":{"type":"integer","minimum":64,"maximum":4096},"minItems":2,"maxItems":2},"browser":{"type":"string","description":"Chromium/Brave executable, or ENGINE_BROWSER"},"save_path":{"type":"string","description":"Optional screenshot file; otherwise returned inline"}}),&["directory"]),action:Action::Local},
    ]
}

fn geometry_tools() -> Vec<Tool> {
    let point = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2});
    let lathe = json!({
        "profile": {"type": "array", "items": point, "minItems": 2, "maxItems": 512, "description": "[radius, height] points ordered bottom to top; equal heights make ledges, radius 0 closes a pole. Y is the turning axis"},
        "segments": {"type": "integer", "minimum": 3, "maximum": 512, "default": 64},
        "caps": {"type": "boolean", "default": true},
        "crease_angle": {"type": "number", "minimum": 0, "maximum": 180, "default": 45, "description": "Profile edges meeting above this angle stay sharp; others share smooth normals"},
        "bevel": {"type": "number", "minimum": 0, "default": 0, "description": "Trim distance along edges at each profile corner; clamped to fit short edges. Softens turned bases, collars and crowns"},
        "bevel_segments": {"type": "integer", "minimum": 1, "maximum": 8, "default": 3, "description": "1 chamfers profile corners; 2..8 rounds them"},
        "thickness": {"type": "number", "minimum": 0, "default": 0, "description": "Wall thickness for a hollow shell: the profile is the outer wall and the inner wall is offset this far inward, joined by flat rims at open ends. A profile starting at radius 0 gets a floor (cups, glasses, bowls, vases); one ending at radius 0 is a dome open below (bells, lampshades). caps is ignored. Must stay below the smallest radius and feature size"}
    });
    let extrude = json!({
        "polygon": {"type": "array", "items": point, "minItems": 3, "maxItems": 256, "description": "Simple XY polygon in either winding; concave outlines supported, holes/self-intersections rejected. Optional repeated closing point"},
        "depth": {"type": "number", "exclusiveMinimum": 0, "default": 1, "description": "Length along Z, centered at Z=0"},
        "caps": {"type": "boolean", "default": true},
        "bevel": {"type": "number", "minimum": 0, "default": 0, "description": "Inset at both cap rims, less than depth/2 and small enough to preserve the outline"},
        "bevel_segments": {"type": "integer", "minimum": 1, "maximum": 8, "default": 1},
        "steps": {"type": "integer", "minimum": 1, "maximum": 256, "default": 1},
        "twist": {"type": "number", "default": 0, "description": "Total rotation in degrees from negative to positive Z; increase steps for a smooth twist"},
        "taper": {"type": "number", "exclusiveMinimum": 0, "maximum": 100, "default": 1, "description": "Positive Z end scale relative to negative Z end; around the XY origin"}
    });
    let mut common = entity_fields();
    // Only geometry editing fields supported by the handler are advertised.
    common.retain(|key, _| {
        [
            "name",
            "translation",
            "position",
            "rotation",
            "rotation_quat",
            "scale",
            "look_at",
            "parent",
            "visible",
            "material",
            "cast_shadow",
        ]
        .contains(&key.as_str())
    });
    common.insert("entity".into(), entity_ref());
    let mut lathe_props = common.clone();
    lathe_props.extend(lathe.as_object().unwrap().clone());
    let mut extrude_props = common.clone();
    extrude_props.extend(extrude.as_object().unwrap().clone());
    let transform = json!({
        "translation": vec3("Local translation, meters"),
        "rotation": vec3("Euler degrees [pitch, yaw, roll]"),
        "scale": vec3("Nonzero per-axis scale, default [1,1,1]; negative values mirror with corrected winding")
    });
    let part_schema = |shape: &str, properties: &Value, required: &[&str]| {
        let mut props = transform.as_object().unwrap().clone();
        props.extend(properties.as_object().unwrap().clone());
        props.insert("shape".into(), json!({"const": shape, "type": "string"}));
        let mut required_fields = vec!["shape"];
        required_fields.extend(required);
        object(Value::Object(props), &required_fields)
    };
    common.insert("parts".into(), json!({
        "type": "array", "minItems": 1, "maxItems": 128,
        "items": {"oneOf": [part_schema("lathe", &lathe, &["profile"]), part_schema("extrude", &extrude, &["polygon"]),
            part_schema("primitive", &json!({"primitive": {"type": "string", "enum": ["cube", "sphere", "plane", "cylinder", "cone", "torus"]}}), &["primitive"])]}
    }));
    vec![
        Tool {
            name: "mesh_lathe",
            description: "Turn a radius/height profile around Y into a procedural mesh with UVs, normals, tangents, end caps, configurable sharp creases and optional rounded profile bevels. Ideal for chess bases, stems, finials, vases and balusters; thickness makes hollow vessels (cups, glasses, bowls, bells). Creates one entity, or replaces entity's mesh while preserving its material. Accepts transforms/material/parent. Identical geometry shares a GPU mesh; project_save persists it. Returns bounds, counts and a reproducible recipe.",
            schema: object(Value::Object(lathe_props), &["profile"]),
            action: Action::Engine("mesh.lathe"),
        },
        Tool {
            name: "mesh_extrude",
            description: "Extrude a simple concave XY outline along centered Z, with triangulated caps, optional rim bevel, taper and twist. Useful for knight silhouettes, crosses, crenellations and plaques. UVs/normals/tangents support procedural textures. Creates an entity or replaces entity's geometry; accepts material/transforms/parent. Holes and self-intersections are rejected. project_save persists the mesh.",
            schema: object(Value::Object(extrude_props), &["polygon"]),
            action: Action::Engine("mesh.extrude"),
        },
        Tool {
            name: "mesh_compose",
            description: "Build one mesh from up to 128 transformed lathe, extrusion or primitive parts. Each part has shape plus local translation/rotation/scale. Ideal for complete chess pieces with turned bodies, extruded heads and ornaments. Corrects normals and mirrored winding. Joins overlapping surfaces without boolean union. Primitives use entity_spawn sizes/axes; lathe is Y-up and extrusion is along Z. Creates an entity or replaces entity's mesh; project_save persists geometry.",
            schema: object(Value::Object(common), &["parts"]),
            action: Action::Engine("mesh.compose"),
        },
    ]
}

fn procedural_tools() -> Vec<Tool> {
    let common = json!({
        "name": { "type": "string", "minLength": 1, "maxLength": 80, "pattern": "^[A-Za-z0-9_-]+$", "description": "Asset filename stem; letters, digits, underscores and hyphens" },
        "directory": { "type": "string", "description": "Output folder, absolute or relative to the project; default content/textures or content/audio. Required when no project is open" },
        "overwrite": { "type": "boolean", "default": false, "description": "Replace existing outputs under this name" },
        "image": { "type": "boolean", "default": true, "description": "Return a map preview or waveform/spectrogram image" }
    });
    let mut texture = common.clone();
    texture.as_object_mut().unwrap().extend(json!({
        "maps": { "type": "array", "minItems": 1, "maxItems": 7, "uniqueItems": true, "items": { "type": "string", "enum": ["base_color", "height", "normal", "roughness", "metallic_roughness", "occlusion", "orm"] }, "description": "Generated maps; texture_generate defaults to base_color, height, normal, metallic_roughness, occlusion. texture_maps defaults to normal, metallic_roughness, occlusion" },
        "colors": { "type": "array", "minItems": 2, "maxItems": 2, "items": color("Color"), "description": "Two colors for low/high heights, default earthy dark/light. Used for base_color only" },
        "normal_strength": { "type": "number", "minimum": 0, "maximum": 10, "default": 0.05, "description": "Bump height relative to UV extent; independent of image resolution, 0 gives flat normals" },
        "flip_y": { "type": "boolean", "default": false, "description": "Flip the normal map's green channel for the opposite tangent-space Y convention" },
        "roughness": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.7 },
        "roughness_variation": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.2, "description": "Height-based roughness variation; 0 gives uniform roughness" },
        "metallic": { "type": "number", "minimum": 0, "maximum": 1, "default": 0 },
        "ao_strength": { "type": "number", "minimum": 0, "maximum": 10, "default": 1, "description": "Darkening of height-field depressions; 0 disables occlusion" },
        "ao_radius": { "type": "integer", "minimum": 1, "maximum": 32, "default": 4, "description": "Neighborhood radius in pixels for approximate occlusion" },
        "entity": entity_ref()
    }).as_object().unwrap().clone());
    let mut generate = texture.clone();
    generate.as_object_mut().unwrap().extend(json!({
        "pattern": { "type": "string", "enum": ["noise", "bricks", "checker", "wood", "marble", "voronoi"], "default": "noise" },
        "size": { "type": "array", "minItems": 2, "maxItems": 2, "items": { "type": "integer", "minimum": 1, "maximum": 2048 }, "default": [256, 256], "description": "[width, height] in pixels" },
        "seed": { "type": "integer", "minimum": 0, "default": 0 },
        "scale": { "type": "integer", "minimum": 1, "maximum": 64, "default": 4, "description": "Pattern repetitions / base noise cells per tile" },
        "octaves": { "type": "integer", "minimum": 1, "maximum": 8, "default": 5, "description": "Noise layers" },
        "mortar": { "type": "number", "minimum": 0.001, "maximum": 0.3, "default": 0.06, "description": "Brick joint width relative to a brick cell" }
    }).as_object().unwrap().clone());
    let mut derive = texture;
    derive.as_object_mut().unwrap().extend(json!({
        "source": { "type": "string", "description": "Height image, absolute or project-relative; max 2048 pixels per axis. Image values are treated as linear height, not inferred physical depth" },
        "channel": { "type": "string", "enum": ["luminance", "red", "green", "blue", "alpha"], "default": "luminance" },
        "invert": { "type": "boolean", "default": false, "description": "Treat dark areas as raised" },
        "tileable": { "type": "boolean", "default": true, "description": "Wrap height sampling across the edges; false clamps for non-tileable images. This does not make a nonperiodic source tileable" }
    }).as_object().unwrap().clone());
    let mut sound = common;
    sound.as_object_mut().unwrap().extend(json!({
        "preset": { "type": "string", "enum": ["tone", "blip", "laser", "explosion", "pickup", "jump", "hit", "powerup"], "default": "tone" },
        "seed": { "type": "integer", "minimum": 0, "default": 0 },
        "waveform": { "type": "string", "enum": ["sine", "square", "saw", "triangle", "noise"], "description": "Override preset oscillator" },
        "duration": { "type": "number", "minimum": 0.02, "maximum": 10, "description": "Seconds (default from preset)" },
        "sample_rate": { "type": "integer", "enum": [8000, 16000, 22050, 44100, 48000, 96000], "default": 44100 },
        "frequency": { "type": "number", "minimum": 20, "description": "Starting Hz; must be <= 0.45 * sample_rate" },
        "end_frequency": { "type": "number", "minimum": 20, "description": "Ending Hz, logarithmic pitch sweep; tone defaults to frequency" },
        "noise_mix": { "type": "number", "minimum": 0, "maximum": 1, "description": "Blend seeded noise into the oscillator (default from preset)" },
        "attack": { "type": "number", "minimum": 0, "description": "Fade-in seconds; minimum anti-click fade 1 ms" },
        "release": { "type": "number", "minimum": 0, "description": "Fade-out seconds; attack + release <= duration, minimum anti-click fade 1 ms" },
        "decay": { "type": "number", "minimum": 0, "maximum": 20, "description": "Exponential falloff over clip (0 is sustained)" },
        "volume": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.8, "description": "Peak-normalized output level, with clipping headroom" },
        "vibrato_hz": { "type": "number", "minimum": 0, "maximum": 40, "default": 0 },
        "vibrato_depth": { "type": "number", "minimum": 0, "maximum": 0.5, "default": 0, "description": "Fractional frequency modulation" },
        "lowpass_hz": { "type": "number", "minimum": 20, "description": "Low-pass cutoff <= 0.49 * sample_rate (default 0.4 * sample_rate)" },
        "duty_cycle": { "type": "number", "minimum": 0.05, "maximum": 0.95, "default": 0.5, "description": "Square-wave pulse width" }
    }).as_object().unwrap().clone());
    vec![
        Tool {
            name: "texture_generate",
            description: "Generate deterministic tileable textures and matching PBR maps as PNG assets, using noise, bricks, checker, wood, marble or Voronoi. Includes tangent-space normal maps, linear height, roughness, occlusion and packed metallic/roughness or ORM. Saves into the project's content/textures, with a generation recipe. Returns paths, an image preview and material fields ready for entity_update; optional entity applies the maps immediately. Existing loaded textures are reloaded. Roughness/AO are height-based approximations; height is an export map, not displacement. Same seed and parameters reproduce the same images.",
            schema: object(generate, &["name"]),
            action: Action::Local,
        },
        Tool {
            name: "texture_maps",
            description: "Derive procedural normal, roughness, occlusion and packed PBR maps from an existing height image. Choose height channel, inversion, bump strength, wrap/clamp and normal Y convention. Saves PNGs and a generation recipe, returns a preview and material bindings, optionally applies to entity. Wrap sampling preserves tileable inputs but does not repair source seams. An albedo image can supply luminance height, but this is an approximation, not recovered geometry.",
            schema: object(derive, &["name", "source"]),
            action: Action::Local,
        },
        Tool {
            name: "sound_generate",
            description: "Synthesize a deterministic sound effect directly into the project's content/audio as mono 16-bit PCM WAV. Presets: tone, blip, laser, explosion, pickup, jump, hit and powerup. Customize waveform, pitch sweep, envelope, seeded noise, vibrato, pulse width and low-pass filter. Peak normalization prevents clipping; short endpoint fades prevent clicks. Saves a recipe and automatically returns sound_inspect measurements and a waveform/spectrogram image. Use the returned path with sound_play or Lua play_sound. Replacing a sound invalidates its playback cache.",
            schema: object(sound, &["name"]),
            action: Action::Local,
        },
    ]
}
