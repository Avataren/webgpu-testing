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
            "emissive": { "type": "number", "minimum": 0, "maximum": 1, "description": "Emission strength (uses the base color)" },
            "unlit": { "type": "boolean" },
            "transparent": { "type": "boolean", "description": "Alpha blending; set the color's alpha below 1" },
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
    point["range"] = json!({ "type": "number" });
    let mut spot = point.clone();
    spot["inner_angle_deg"] = json!({ "type": "number" });
    spot["outer_angle_deg"] = json!({ "type": "number" });
    let mut directional = light_common;
    directional["shadow_size"] =
        json!({ "type": "number", "description": "Half-size of the shadow map area in meters" });

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
        "point_light": nullable_object("Add/update a point light, or null to remove", point),
        "spot_light": nullable_object("Add/update a spot light (shines along -Z), or null to remove", spot),
        "directional_light": nullable_object("Add/update a directional light (shines along -Z), or null to remove", directional),
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
        }))
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
                     "point_light", "directional_light", "spot_light", "camera", "particles", "environment"],
            "description": "What to create (default empty). Primitive sizes differ: the cube spans -0.5..0.5, the sphere has radius 1, cylinder and cone lie along the Z axis; entity_get shows local_bounds. The plane is 1x1 in XY facing +Z, so use rotation [-90, 0, 0] and a large scale for a floor."
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

    vec![
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
                "monitor": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "Monitor for the window: 1-based number, output name (e.g. HDMI-A-0) or primary. Defaults to settings.json / ENGINE_MONITOR; respect the user's choice and don't override it without reason." },
                "timeout_seconds": { "type": "number", "description": "How long to wait for the engine to start (default 120)" }
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
            description: "The engine's Lua scripting guide: lifecycle hooks (on_created, update, on_destroyed, on_ui), entity/transform/hierarchy/input/component/query/event/state/UI functions, coroutines and script modes. Read it before writing game scripts.",
            schema: empty(),
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
                "stop_on_error": { "type": "boolean", "description": "Default true" }
            }), &["calls"]),
            action: Action::Local,
        },
        // ---- Overview ----
        Tool {
            name: "engine_info",
            description: "Snapshot of the running engine: frame, fps, runtime mode, active scene, window size, project root, value conventions and, in the editor, project/scene/selection state. Start here.",
            schema: empty(),
            action: Action::Local,
        },
        Tool {
            name: "engine_stats",
            description: "Frame time, fps, entity/script counts, draw calls and asset counts.",
            schema: empty(),
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
            description: "Everything about one entity: local and world transform (translation, Euler rotation, quaternion, scale), mesh and bounds, material, lights, camera, script (source, mode, last error), particles, environment, animations, parent/children.",
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
            description: "Create an entity in the active scene: an empty, a primitive mesh, a light, a camera (made active by default), a GPU particle system or an environment. Accepts every entity_update field too (material, script, parent, lights, ...). Returns the new entity's id. Units are meters, Y is up, -Z is forward. To make many similar objects, spawn one and use entity_duplicate with count/step/columns.",
            schema: json!({ "type": "object", "properties": spawn_props }),
            action: Action::Engine("entity.spawn"),
        },
        Tool {
            name: "entity_update",
            description: "Change any aspect of an entity in one call: name, transform, parent, visibility, shadows, mesh, material, point/spot/directional light, camera, script, particle system/emitter, environment, built-in animations, billboard. Only the given fields change; component fields set to null remove the component. Pass entities (a list) or name_contains instead of entity to apply the same change to many entities at once. Changes made during play mode are discarded on stop (results then carry discarded_on_stop: true).",
            schema: json!({ "type": "object", "properties": update_props }),
            action: Action::Engine("entity.update"),
        },
        Tool {
            name: "entity_remove_component",
            description: "Remove one component from an entity.",
            schema: object(json!({
                "entity": entity_ref(),
                "component": { "type": "string", "enum": ["transform", "mesh", "material", "visible", "point_light", "directional_light", "spot_light", "cast_shadow", "camera", "script", "particle_system", "particle_emitter", "environment", "billboard", "depth_state", "rotate_animation", "orbit_animation", "gltf"] }
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
        // ---- Scripting & runtime ----
        Tool {
            name: "lua_eval",
            description: "Run Lua in the engine with the full scripting API (see lua_api_reference) and return the result and anything printed. A bare expression returns its value. Without entity, code runs in a persistent console environment (globals survive between calls). With entity, it runs inside that entity's script, so it can read or change the script's globals and state. World changes apply when the chunk finishes. Great for queries and for moving, hiding, recoloring (set_material_color) or re-parenting many entities, and for spawning primitives, lights and cameras (spawn_primitive, spawn_light, spawn_camera).",
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
            description: "Recompile scripts: one entity's, every user of a file, or all. Files under the project and scripts/ are also hot-reloaded automatically by the editor when saved.",
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
            description: "One-call playtest: enter play (optionally sped up with time_scale), send scripted input, run for `seconds`, then report failing scripts, new warnings/errors, the sounds played, what changed in the scene (added/removed entities and changed fields), the result of an `eval` Lua expression and a viewport screenshot, and stop again (the scene is restored). Replaces runtime_set + input_send + wait + lua_eval + screenshot + runtime_set.",
            schema: object(json!({
                "seconds": { "type": "number", "description": "Real seconds to play (default 3, up to 240); game time is seconds * time_scale" },
                "time_scale": { "type": "number", "minimum": 0, "maximum": 16, "description": "Play speed (default 1)" },
                "input": { "type": "array", "items": { "type": "object" }, "description": "Input events as for input_send (key/mouse_button/mouse_move/scroll, action, frames, delay in frames). Everything is released at the end." },
                "eval": { "type": "string", "description": "Lua evaluated at the end, e.g. \"return GAME.score\"" },
                "entity": entity_ref(),
                "screenshot": { "type": "boolean", "description": "Include a viewport screenshot (default true)" },
                "max_size": { "type": "integer", "description": "Screenshot longest side (default 1280)" },
                "diff": { "type": "boolean", "description": "Report what play changed in the scene (default true)" },
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
                "seconds": { "type": "number", "description": "Up to 240" }
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
                "loop": { "type": "boolean" }
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
            name: "audio_status",
            description: "Sounds playing now and the recently played ones (id, path, volume, pitch, time, audible). You cannot hear the game, so use this (or playtest's sounds_played) to check that scripts play the right sounds at the right moments.",
            schema: object(json!({ "since": { "type": "integer", "description": "Only plays with an id at least this (next_id from an earlier call)" } }), &[]),
            action: Action::Engine("audio.status"),
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
            description: "Capture what the engine window shows (including the editor UI), or only the scene viewport, as an image. Takes one frame. In editor mode the viewport shows the editor's view camera; pass camera: \"game\" (or a camera entity) to render that frame through the game camera instead, without gizmos and without moving the view.",
            schema: object(json!({
                "camera": { "anyOf": [{ "type": "integer" }, { "type": "string" }], "description": "\"game\" for the scene's active camera, or a camera entity id/name. Only this capture's frame uses it." },
                "target": { "type": "string", "enum": ["window", "viewport"], "description": "Default viewport in the editor is usually what you want to judge the game's look" },
                "max_size": { "type": "integer", "description": "Longest side in pixels (default 1280)" },
                "save_path": { "type": "string", "description": "Also write the PNG here" }
            }), &[]),
            action: Action::Local,
        },
        Tool {
            name: "camera_get",
            description: "The view camera: position, target, forward, projection and the active camera entity.",
            schema: empty(),
            action: Action::Engine("camera.get"),
        },
        Tool {
            name: "camera_set",
            description: "Move the view camera. Give position and/or target, or focus on an entity (frames it at a distance based on its size). In the editor this moves the active camera entity too.",
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
            description: "Background clear color, ambient intensity, HDR sky (path, intensity) and color grading (exposure, saturation, contrast).",
            schema: empty(),
            action: Action::Engine("environment.get"),
        },
        Tool {
            name: "environment_set",
            description: "Change environment settings; only given fields change. Creates an Environment entity if the scene has none, because that is how scenes save their environment.",
            schema: object(json!({
                "clear_color": color("Background color"),
                "ambient_intensity": { "type": "number" },
                "color_grading": { "type": "object", "properties": {
                    "exposure": { "type": "number" }, "saturation": { "type": "number" }, "contrast": { "type": "number" }
                }},
                "hdr": { "type": "object", "properties": {
                    "enabled": { "type": "boolean" }, "intensity": { "type": "number" }, "path": { "type": "string", "description": ".hdr image" }
                }}
            }), &[]),
            action: Action::Engine("environment.set"),
        },
        Tool {
            name: "postprocess_get",
            description: "Post-processing toggles and settings (SSAO, bloom, FXAA).",
            schema: empty(),
            action: Action::Engine("postprocess.get"),
        },
        Tool {
            name: "postprocess_set",
            description: "Toggle or tune SSAO, bloom and FXAA.",
            schema: object(json!({
                "ssao": { "type": "boolean" },
                "bloom": { "type": "boolean" },
                "fxaa": { "type": "boolean" },
                "ssao_settings": { "type": "object", "properties": {
                    "radius": { "type": "number" }, "bias": { "type": "number" }, "intensity": { "type": "number" }, "power": { "type": "number" }
                }},
                "bloom_settings": { "type": "object", "properties": {
                    "threshold": { "type": "number" }, "knee": { "type": "number" }, "scatter": { "type": "number" }
                }}
            }), &[]),
            action: Action::Engine("postprocess.set"),
        },
        Tool {
            name: "logs",
            description: "Recent engine log lines (scripts log with target \"script\"; script errors appear here too). Pass after = latest_seq from a previous call to get only new lines.",
            schema: object(json!({
                "after": { "type": "integer" },
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
            description: "Editor only: create a new project at location/name and open it; returns once it is open, with the editor state. Its content/ folder holds scenes, scripts and assets.",
            schema: object(json!({
                "name": { "type": "string" },
                "location": { "type": "string", "description": "Parent directory" }
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
            description: "Editor only: save the project (scenes, environment, editor camera) to its directory, or to path. Must be in editor mode, not playing.",
            schema: object(json!({ "path": { "type": "string" } }), &[]),
            action: Action::Engine("project.save"),
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
            description: "Editor only: bring a file into the project. A glTF/GLB model is copied in, saved as a scene document and instanced into the active scene (example models are under web/assets/). Images, audio, fonts, .lua and .wgsl files are copied into content/textures, audio, fonts, scripts or shaders (or dest) and the result gives the project-relative path to reference. Replacing a texture the scene uses reloads it.",
            schema: object(json!({
                "path": { "type": "string", "description": "Source file" },
                "dest": { "type": "string", "description": "Non-model files: target path relative to the project's content folder, e.g. textures/ui/logo.png" },
                "overwrite": { "type": "boolean", "description": "Replace an existing file at the target (default false)" }
            }), &["path"]),
            action: Action::Engine("asset.import"),
        },
        Tool {
            name: "asset_reload",
            description: "Re-read textures (and scripts) from disk after changing the files, so the scene shows the new images without renaming them. Without path, every texture loaded from a file is reloaded. The editor also does this by itself when an image in the project changes.",
            schema: object(json!({ "path": { "type": "string", "description": "A file or folder (absolute, or relative to the project)" } }), &[]),
            action: Action::Engine("asset.reload"),
        },
        Tool {
            name: "editor_select",
            description: "Editor only: select an entity (shows it in the inspector), or null to clear.",
            schema: object(json!({ "entity": { "anyOf": [{ "type": "integer" }, { "type": "string" }, { "type": "null" }] } }), &[]),
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
    ]
}
