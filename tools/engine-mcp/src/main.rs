//! MCP (Model Context Protocol) server for the wgpu-cube engine.
//!
//! Speaks JSON-RPC over stdio to an MCP client (such as Claude Code) and forwards tool
//! calls to a running engine's remote-control socket (`wgpu_cube::remote`). It can
//! also build and launch the engine.
//!
//! Usage: `engine-mcp [--repo DIR] [--port PORT]`. The repository defaults to the one
//! this binary was built in; the port defaults to `ENGINE_REMOTE_PORT` or 47811.

// The tool schemas are large json! literals.
#![recursion_limit = "256"]

mod engine;
mod guide;
mod tools;
mod web;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use engine::{CallError, Engine, DEFAULT_PORT};
use tools::{Action, Tool};

const SERVER_NAME: &str = "engine";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Results longer than this are cut so they do not flood the client's context.
const MAX_TEXT: usize = 120_000;

const INSTRUCTIONS: &str = "Controls the wgpu-cube engine. Read engine://workflow for the authoring workflow and lua_api_reference for scripting. Start with engine_launch and engine_info; create/open a project before editing. Check screenshots after visual changes and save with project_save.";

const WORKFLOW_URI: &str = "engine://workflow";
const WORKFLOW: &str = "\
Controls the wgpu-cube game engine (Rust, wgpu, hecs ECS, Lua scripting).

Workflow:
1. engine_launch (target editor by default) builds and starts the engine, or attaches \
to one already running. engine_info shows its state and value conventions.
2. In the editor, create or open a project first (project_create / project_open); the \
startup dialog blocks the editor until then. editor_state shows the project's scripts folder.
3. Build scenes with entity_spawn / entity_update (meshes, materials, lights, camera, \
particles, environment). Units are meters, Y is up, -Z is forward, rotations are Euler \
degrees. Entities are referenced by id or unique name. voxel_spawn builds procedural voxel \
models (text art or a noise fill expression) as optimized meshes.
mesh_lathe turns radius/height profiles around Y; mesh_extrude builds beveled concave \
polygon extrusions along Z, with taper and twist; mesh_compose assembles transformed \
parts into one mesh. These include UVs, normals and tangents and persist with project_save.
texture_generate creates tileable PNG material maps; texture_maps derives normal and \
PBR maps from a height image. Both return ready-to-use material bindings and can apply \
them to an entity. sound_generate writes seeded WAV effects and inspects their levels.
4. Gameplay is Lua: read lua_api_reference, write .lua files into the project's scripts \
folder with your file tools, attach them with entity_update {script: {path}}, then \
runtime_set play, wait, and check script_status, logs and screenshot. runtime_set stop \
restores the pre-play scene.
5. lua_eval runs arbitrary Lua against the live world for queries and bulk edits.
6. project_save persists the work. engine_output shows crashes and GPU validation errors.

Look at the result (screenshot) after visual changes instead of assuming they worked.

Measuring: engine_stats gives one frame (renderer.gpu holds GPU milliseconds per stage from \
timestamp queries, the stat to trust when another process shares the GPU); \
engine_stats {history: N} the last N frames, one row each, to see the frames around a change; \
profile_capture {frames} percentiles over many frames, CPU stages and GPU stages alike; \
engine_profile_cpu the hottest CPU symbols. wait {until_frame} waits for an absolute frame. \
Comparing images: visual_capture compares post-process variants of one frozen view; to compare \
builds or sessions, capture the same view in each (visual_capture with freeze_noise: true, or \
screenshot {save_path, overlays: false} after render_freeze_noise) and run visual_compare on \
the files or directories. engine_launch {binary, window_size} runs a baseline build from \
another tree in a small window.

Troubleshooting: an error reading \"MCP tool call requires approval, but approval policy is never\" \
comes from the MCP client, not the engine: its approval policy forbids calls that need \
confirmation (reads may still work while edits are refused). Allow this server's tools in the \
client's approval settings and retry. A real connection problem looks different: engine_status \
reports whether the engine process is running and reachable, and engine_output shows its log.
For web builds, project_validate's browser block lists what the browser would not find (web_build \
stages only project.json and content/); web_build stops on those errors before the slow wasm build.";

fn main() {
    let mut repo: Option<PathBuf> = std::env::var_os("ENGINE_REPO").map(PathBuf::from);
    let mut port = std::env::var("ENGINE_REMOTE_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo" => repo = args.next().map(PathBuf::from),
            "--port" => {
                if let Some(value) = args.next().and_then(|value| value.parse().ok()) {
                    port = value;
                }
            }
            "-h" | "--help" => {
                eprintln!("usage: engine-mcp [--repo DIR] [--port PORT]");
                return;
            }
            other => eprintln!("engine-mcp: ignoring unknown argument {other}"),
        }
    }
    let repo = repo
        .or_else(find_repo)
        .unwrap_or_else(|| PathBuf::from("."));
    let repo = repo.canonicalize().unwrap_or(repo);
    eprintln!("engine-mcp: repo {} port {port}", repo.display());

    let mut server = Server {
        engine: Engine::new(repo, port),
        tools: tools::tools(),
        started: SystemTime::now(),
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(&message),
            Err(err) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {err}") }
            })),
        };
        if let Some(response) = response {
            let mut encoded = response.to_string();
            encoded.push('\n');
            if stdout.write_all(encoded.as_bytes()).is_err() || stdout.flush().is_err() {
                break;
            }
        }
    }
}

/// The repository this binary was built in: the nearest ancestor of the executable
/// whose Cargo.toml is the engine's.
fn find_repo() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let is_engine = |dir: &Path| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|manifest| manifest.contains("name = \"wgpu-cube\""))
    };
    exe.ancestors()
        .find(|dir| is_engine(dir))
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::current_dir()
                .ok()?
                .ancestors()
                .find(|dir| is_engine(dir))
                .map(Path::to_path_buf)
        })
}

struct Server {
    engine: Engine,
    tools: Vec<Tool>,
    /// When this process started; sources or a binary newer than this mean the tools
    /// this server offers are out of date.
    started: SystemTime,
}

/// How `playtest` reports the sounds a run played.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SoundReport {
    Off,
    Summary,
    List,
}

/// Content returned from a tool call.
enum Output {
    Json(Value),
    Text(String),
    Image { png_base64: String, caption: Value },
}

impl Server {
    fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        // Notifications (no id) never get a response.
        let id = id?;

        let result = match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(SUPPORTED_PROTOCOLS[0]);
                let version = if SUPPORTED_PROTOCOLS.contains(&requested) {
                    requested
                } else {
                    SUPPORTED_PROTOCOLS[0]
                };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false }, "resources": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.tools.iter().map(|tool| json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.schema,
                    "outputSchema": tool.output_schema(),
                    "annotations": tool.annotations(),
                })).collect::<Vec<_>>()
            })),
            "tools/call" => Ok(self.call_tool(&params)),
            "resources/list" => Ok(json!({ "resources": [{
                "uri": WORKFLOW_URI, "name": "Engine authoring workflow",
                "description": "Shared workflow, conventions and verification guidance",
                "mimeType": "text/plain",
            }] })),
            "resources/read" if params["uri"] == WORKFLOW_URI => Ok(json!({
                "contents": [{"uri": WORKFLOW_URI, "mimeType": "text/plain", "text": WORKFLOW}]
            })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            other => {
                Err(json!({ "code": -32601, "message": format!("method not found: {other}") }))
            }
        };

        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        })
    }

    fn call_tool(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params
            .get("arguments")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));

        match self.run_tool(name, arguments) {
            Ok(Output::Json(value)) => {
                let failed = value.get("ok") == Some(&Value::Bool(false));
                let mut result = text_result(pretty(&value), failed);
                result["structuredContent"] = if value.is_object() {
                    value
                } else {
                    json!({"result": value})
                };
                result
            }
            Ok(Output::Text(text)) => {
                let mut result = text_result(text.clone(), false);
                result["structuredContent"] = json!({"text": text});
                result
            }
            Ok(Output::Image {
                png_base64,
                caption,
            }) => json!({
                "content": [
                    { "type": "image", "data": png_base64, "mimeType": "image/png" },
                    { "type": "text", "text": caption.to_string() }
                ],
                "isError": caption.get("ok") == Some(&Value::Bool(false)),
                "structuredContent": caption
            }),
            Err(message) => error_result(message),
        }
    }

    fn run_tool(&mut self, name: &str, arguments: Value) -> Result<Output, String> {
        let action = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .map(|tool| match tool.action {
                Action::Engine(method) => Some(method),
                Action::Local => None,
            })
            .ok_or_else(|| format!("unknown tool {name:?}"))?;
        match action {
            Some(method) => self
                .engine
                .call(method, arguments)
                .map(Output::Json)
                .map_err(|err| err.to_string()),
            None => self.local_tool(name, &arguments),
        }
    }

    /// Run several tool calls in order and collect their results. Images are left out.
    fn batch(&mut self, args: &Value) -> Result<Output, String> {
        let calls = args
            .get("calls")
            .and_then(Value::as_array)
            .ok_or("calls must be an array of {tool, arguments}")?;
        if args["atomic"].as_bool().unwrap_or(false) || args["dry_run"].as_bool().unwrap_or(false) {
            let transaction_calls=calls.iter().map(|call| {
                let name=call["tool"].as_str().ok_or("every call needs tool")?;
                let tool=self.tools.iter().find(|t|t.name==name).ok_or_else(||format!("unknown tool {name:?}"))?;
                let Action::Engine(method)=tool.action else {return Err(format!("{name} is not a scene-only engine tool"));};
                let mut mapped=json!({"method":method,"params":call.get("arguments").cloned().unwrap_or(json!({}))});
                if let Some(alias)=call.get("as") {mapped["as"]=alias.clone();}
                Ok(mapped)
            }).collect::<Result<Vec<_>,String>>()?;
            return self
                .engine
                .call(
                    "scene.transaction",
                    json!({"calls":transaction_calls,"dry_run":args["dry_run"].as_bool().unwrap_or(false)}),
                )
                .map(|mut result| {
                    shape_batch_report(&mut result, args, calls);
                    Output::Json(result)
                })
                .map_err(|e| e.to_string());
        }
        let stop_on_error = args
            .get("stop_on_error")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut results = Vec::new();
        let mut failed = 0;
        for (index, call) in calls.iter().enumerate() {
            let name = call
                .get("tool")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("call {index} has no \"tool\""))?;
            if name == "batch" {
                return Err("batch calls cannot nest".into());
            }
            let arguments = call
                .get("arguments")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let result = match self.run_tool(name, arguments) {
                Ok(Output::Json(value)) => {
                    if value["ok"] == false {
                        failed += 1;
                    }
                    json!({ "tool": name, "result": value })
                }
                Ok(Output::Text(text)) => json!({ "tool": name, "result": text }),
                Ok(Output::Image { caption, .. }) => {
                    json!({ "tool": name, "result": caption, "note": "image left out; call this tool directly to see it" })
                }
                Err(message) => {
                    failed += 1;
                    json!({ "tool": name, "error": message })
                }
            };
            let stop = failed > 0 && stop_on_error;
            results.push(result);
            if stop {
                break;
            }
        }
        let mut report = json!({
            "ok": failed == 0,
            "ran": results.len(),
            "of": calls.len(),
            "failed": failed,
            "results": results,
        });
        shape_batch_report(&mut report, args, calls);
        Ok(Output::Json(report))
    }

    /// Run tools/library/library.py with `args` (low priority) and return its JSON.
    fn run_library(&self, args: &[String]) -> Result<Value, String> {
        let script = self.engine.repo.join("tools/library/library.py");
        let output = std::process::Command::new("nice")
            .args(["-n", "10", "python3", "-I"])
            .arg(&script)
            .arg("--json")
            .args(args)
            .current_dir(&self.engine.repo)
            .output()
            .map_err(|err| format!("cannot run {}: {err}", script.display()))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let log: Vec<&str> = stderr.lines().rev().take(20).collect();
        let log: Vec<&str> = log.into_iter().rev().collect();
        match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(mut result) if output.status.success() => {
                if !log.is_empty() {
                    result = json!({ "result": result, "log": log });
                }
                Ok(result)
            }
            _ => Err(format!(
                "library.py {} failed ({}): {}{}",
                args.first().map(String::as_str).unwrap_or_default(),
                output.status,
                stdout.trim(),
                log.join("\n")
            )),
        }
    }

    fn local_tool(&mut self, name: &str, args: &Value) -> Result<Output, String> {
        match name {
            "web_build" => {
                let project = self
                    .engine
                    .repo
                    .join(args["project"].as_str().ok_or("project required")?);
                let validation = self
                    .engine
                    .call("project.validate", json!({"path":project}))
                    .map_err(|e| e.to_string())?;
                if validation["ok"] != true {
                    return Ok(Output::Json(json!({"ok":false,"validation":validation})));
                }
                // Assets web_build would not stage 404 in the browser however well the
                // project validates natively; stop before the slow wasm build.
                if validation["browser"]["ok"] == false
                    && !args["ignore_browser_issues"].as_bool().unwrap_or(false)
                {
                    return Ok(Output::Json(json!({
                        "ok": false,
                        "validation": validation,
                        "note": "The project is not browser-ready (validation.browser.errors). Fix them, or pass ignore_browser_issues: true to package it anyway.",
                    })));
                }
                let mut result = self
                    .engine
                    .build_web(&project, args["overwrite"].as_bool().unwrap_or(false))?;
                result["validation"] = validation;
                Ok(Output::Json(result))
            }
            "web_playtest" => self.web_playtest(args),
            "library_fetch" => {
                let source = args["source"].as_str().ok_or("source required")?;
                let names: Vec<String> = args["names"]
                    .as_array()
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|name| name.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut command = vec![];
                if source == "mixamo" {
                    command.push("fetch-mixamo".to_string());
                    if args["characters"].as_bool() == Some(true) {
                        command.push("--characters".into());
                    } else {
                        command.push("--animations".into());
                    }
                    if !names.is_empty() {
                        // Names outside the curated set are searched for as they are.
                        command.push("--query".into());
                        command.extend(names.iter().cloned());
                        command.push("--only".into());
                        command.extend(names);
                    }
                } else {
                    command.push("fetch-pack".into());
                    command.push(source.to_string());
                }
                self.run_library(&command).map(Output::Json)
            }
            "mixamo_catalog" => {
                let mut command = vec!["mixamo-catalog".to_string()];
                if args["characters"].as_bool() == Some(true) {
                    command.push("--characters".into());
                }
                if let Some(query) = args["query"].as_str() {
                    command.push("--query".into());
                    command.push(query.to_string());
                }
                self.run_library(&command).map(Output::Json)
            }
            "library_ingest" => {
                let mut command = vec!["ingest".to_string()];
                for path in args["paths"].as_array().ok_or("paths required")? {
                    command.push(path.as_str().ok_or("paths are strings")?.to_string());
                }
                for key in ["name", "provider", "license", "author", "url"] {
                    if let Some(value) = args[key].as_str() {
                        command.push(format!("--{key}"));
                        command.push(value.to_string());
                    }
                }
                if let Some(tags) = args["tags"].as_array() {
                    let tags: Vec<&str> = tags.iter().filter_map(Value::as_str).collect();
                    command.push("--tags".into());
                    command.push(tags.join(","));
                }
                self.run_library(&command).map(Output::Json)
            }
            "batch" => self.batch(args),
            "engine_launch" => self.launch(args).map(Output::Json),
            "engine_profile_cpu" => self.profile_cpu(args).map(Output::Json),
            "engine_stop" => self.engine.stop().map(Output::Json),
            "engine_status" => Ok(Output::Json(self.engine.status())),
            "engine_output" => {
                let lines = args.get("lines").and_then(Value::as_u64).unwrap_or(80) as usize;
                let contains = args.get("contains").and_then(Value::as_str);
                self.engine.output(lines, contains).map(Output::Json)
            }
            "engine_build" => {
                let target = args
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("editor");
                self.validate_target(target)?;
                let release = args
                    .get("release")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let (binary, summary) = self.engine.build(target, release)?;
                Ok(Output::Json(json!({
                    "summary": summary,
                    "binary": binary.display().to_string(),
                    "note": "A running engine keeps the old code until it is restarted (engine_launch with restart: true).",
                })))
            }
            "lua_api_reference" => {
                let path = self.engine.repo.join("scripts/LUA_SCRIPTING_README.md");
                let guide = std::fs::read_to_string(&path)
                    .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
                let guide = format!(
                    "{guide}\n\n## Notes for remote control\n\n\
                     - Entity ids returned by the engine tools are the same i64 handles Lua uses. They stay valid across play/stop and undo.\n\
                     - lua_eval runs with this whole API; `return` a value (or write a bare expression) to get it back.\n\
                     - lua_eval with `entity` runs inside that entity's script, so it sees the script's globals but not its `local` variables. \
                     To inspect game state while testing, keep it in a global table (e.g. `GAME = {{ score = 0 }}`) or in the state API (`set_i32`, ...). \
                     Without `entity`, a missing global's error names the script that defines it, and playtest's eval falls back to that script by itself.\n\
                     - Test hooks: next to `GAME`, expose a global table of functions that jump straight to the situation you want to look at, \
                     e.g. `RT = {{ jump = function(wave) ... end, spawn = function(kind, x, z) ... end, god = function(on) ... end, units = function() ... end }}`, \
                     and call them with lua_eval {{entity}}. One enemy type or wave can then be checked in seconds instead of playing up to it.\n\
                     - Measure performance in the player (engine_launch target player), not the editor: engine_stats' frame_breakdown shows where a frame's time goes.\n\
                     - Example scripts: scripts/*.lua and examples/scripts/*.lua in {}.\n",
                    self.engine.repo.display()
                );
                if args.get("outline").and_then(Value::as_bool) == Some(true) {
                    return Ok(Output::Text(format!(
                        "Sections of the Lua guide (ask for one with section, or grep with search):\n{}",
                        guide::outline(&guide)
                    )));
                }
                if let Some(query) = args.get("search").and_then(Value::as_str) {
                    return Ok(Output::Text(guide::search(&guide, query, 40)));
                }
                if let Some(query) = args.get("section").and_then(Value::as_str) {
                    return guide::section(&guide, query)
                        .map(Output::Text)
                        .ok_or_else(|| {
                            format!(
                                "no section title contains {query:?}; the sections are:\n{}",
                                guide::outline(&guide)
                            )
                        });
                }
                Ok(Output::Text(guide))
            }
            "engine_info" => {
                let mut info = self
                    .engine
                    .call("engine.info", json!({}))
                    .map_err(|err| err.to_string())?;
                if let Ok(methods) = self.engine.call("engine.methods", json!({})) {
                    let editor = methods["application"]
                        .as_array()
                        .is_some_and(|methods| methods.iter().any(|m| m == "editor.state"));
                    info["application_methods"] = methods["application"].clone();
                    if editor {
                        if let Ok(state) = self.engine.call("editor.state", json!({})) {
                            info["editor"] = state;
                        }
                    }
                }
                info["mcp_server"] = self.server_status();
                Ok(Output::Json(info))
            }
            "engine_call" => {
                let method = args
                    .get("method")
                    .and_then(Value::as_str)
                    .ok_or("missing \"method\"")?;
                let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                self.engine
                    .call(method, params)
                    .map(Output::Json)
                    .map_err(|err| err.to_string())
            }
            "project_create" => {
                let created = self
                    .engine
                    .call("project.create", args.clone())
                    .map_err(|err| err.to_string())?;
                let directory = created["directory"]
                    .as_str()
                    .ok_or("the editor did not say where the project goes")?;
                let directory = PathBuf::from(directory);
                self.wait_for_project(&directory)?;
                let template = args
                    .get("template")
                    .and_then(Value::as_str)
                    .unwrap_or("starter");
                let starter = match template {
                    "starter" => Some(self.build_starter_scene()?),
                    "empty" => None,
                    other => {
                        return Err(format!(
                            "unknown template {other:?}; use starter or empty (the project was created empty)"
                        ))
                    }
                };
                let mut result = json!({
                    "created": directory.display().to_string(),
                    "editor": self.engine.call("editor.state", json!({})).map_err(|err| err.to_string())?,
                });
                if let Some(starter) = starter {
                    result["starter_scene"] = starter;
                }
                Ok(Output::Json(result))
            }
            "project_open" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("missing \"path\"")?;
                let path = PathBuf::from(path);
                let path = if path.is_absolute() {
                    path
                } else {
                    self.engine.repo.join(path)
                };
                self.open_project(&path).map(Output::Json)
            }
            "project_save" => self.save_project(args).map(Output::Json),
            "playtest" => self.playtest(args),
            "ai_control" => {
                let entity = args.get("entity").cloned().unwrap_or(Value::Null);
                let mut out = json!({});
                if let Some(paused) = args.get("paused").and_then(Value::as_bool) {
                    out["pause"] = self
                        .engine
                        .call("ai.pause", json!({ "entity": entity, "paused": paused }))
                        .map_err(|err| err.to_string())?;
                }
                if args.get("restart").and_then(Value::as_bool) == Some(true) {
                    out["restart"] = self
                        .engine
                        .call("ai.restart", json!({ "entity": entity }))
                        .map_err(|err| err.to_string())?;
                }
                Ok(Output::Json(out))
            }
            "navmesh_bake" => {
                let mut params = json!({});
                if let Some(entity) = args.get("entity") {
                    params["entity"] = entity.clone();
                }
                let started = self
                    .engine
                    .call("navigation.bake", params)
                    .map_err(|err| err.to_string())?;
                let timeout = args
                    .get("timeout_seconds")
                    .and_then(Value::as_f64)
                    .unwrap_or(180.0);
                let begun = std::time::Instant::now();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    let status = self
                        .engine
                        .call("navigation.status", json!({}))
                        .map_err(|err| err.to_string())?;
                    if status["bake"].is_null() {
                        return Ok(Output::Json(json!({
                            "started": started["started"],
                            "result": status["last_bake"],
                            "status": status,
                        })));
                    }
                    if begun.elapsed().as_secs_f64() > timeout {
                        return Ok(Output::Json(json!({
                            "started": started["started"],
                            "still_baking": status["bake"],
                            "note": "Still baking; check navigation_status later.",
                        })));
                    }
                }
            }
            "sound_inspect" | "sound_generate" | "texture_generate" | "texture_maps"
            | "texture_fetch" => {
                let method = match name {
                    "sound_generate" => "audio.generate",
                    "texture_generate" => "texture.generate",
                    "texture_maps" => "texture.maps",
                    "texture_fetch" => "texture.fetch",
                    _ => "audio.inspect",
                };
                let mut report = self
                    .engine
                    .call(method, args.clone())
                    .map_err(|err| err.to_string())?;
                match report
                    .as_object_mut()
                    .and_then(|report| report.remove("png_base64"))
                    .and_then(|data| data.as_str().map(str::to_owned))
                {
                    Some(png_base64) => Ok(Output::Image {
                        png_base64,
                        caption: report,
                    }),
                    None => Ok(Output::Json(report)),
                }
            }
            "screenshot" => {
                let mut params = args.clone();
                params["inline"] = json!(true);
                // Another shape is laid out for real (scripts, cameras and HUDs see it)
                // for a few frames, captured, then the full view comes back.
                let preview = preview_params(args);
                if let Some(preview) = &preview {
                    self.set_preview(preview.clone(), 3)?;
                    if let Some(object) = params.as_object_mut() {
                        object.remove("viewport_size");
                        object.remove("aspect");
                    }
                    params["target"] = json!("viewport");
                }
                let shot = self
                    .engine
                    .call("screenshot", params)
                    .map_err(|err| err.to_string());
                if preview.is_some() {
                    self.set_preview(json!({}), 1)?;
                }
                let mut result = shot?;
                let png_base64 = result
                    .as_object_mut()
                    .and_then(|object| object.remove("png_base64"))
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or("the engine returned no image")?;
                Ok(Output::Image {
                    png_base64,
                    caption: result,
                })
            }
            other => Err(format!("tool {other} is not implemented")),
        }
    }

    /// Turn a viewport preview on (`{aspect}` / `{size}`) or off (`{}`) and let `frames`
    /// frames pass so scripts and the layout adapt.
    fn set_preview(&mut self, preview: Value, frames: u64) -> Result<Value, String> {
        let result = self
            .engine
            .call("view.preview", preview)
            .map_err(|err| err.to_string())?;
        self.engine
            .call("wait", json!({ "frames": frames }))
            .map_err(|err| err.to_string())?;
        Ok(result)
    }

    /// Whether this server is older than its sources, and the repository's commit, so a
    /// tool description that disagrees with the source can be explained.
    fn server_status(&self) -> Value {
        let crate_dir = self.engine.repo.join("tools/engine-mcp");
        let modified = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        };
        let newest_source = std::fs::read_dir(crate_dir.join("src"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| modified(&entry.path()))
            .chain(modified(&crate_dir.join("Cargo.toml")))
            .chain(modified(&crate_dir.join("web_playtest.mjs")))
            .max();
        let binary = std::env::current_exe().ok().and_then(|exe| modified(&exe));
        let newer = |time: Option<SystemTime>| time.is_some_and(|time| time > self.started);
        let restart_needed = newer(newest_source) || newer(binary);
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&self.engine.repo)
                .args(args)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        };
        let seconds_ago = |time: SystemTime| {
            SystemTime::now()
                .duration_since(time)
                .map_or(0, |elapsed| elapsed.as_secs())
        };
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "started_seconds_ago": seconds_ago(self.started),
            "restart_needed": restart_needed,
            "note": if restart_needed {
                "The server's sources or binary changed after it started, so its tool list and descriptions are out of date. Restart it (/mcp in Claude Code) to pick up new tools and parameters."
            } else {
                "The server is as new as its sources."
            },
            "repo_head": git(&["rev-parse", "--short", "HEAD"]),
            "repo_dirty": git(&["status", "--porcelain", "--untracked-files=no"]).map(|status| !status.is_empty()),
        })
    }

    /// Play for a while with scripted input, then report errors, the state `eval`
    /// returns and a screenshot, and (by default) stop again.
    fn playtest(&mut self, args: &Value) -> Result<Output, String> {
        let mut resolved = if let Some(path) = args["scenario"].as_str() {
            let path = self.engine.repo.join(path);
            serde_json::from_str::<Value>(
                &std::fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read scenario {}: {e}", path.display()))?,
            )
            .map_err(|e| e.to_string())?
        } else {
            json!({})
        };
        let object = resolved
            .as_object_mut()
            .ok_or("scenario must be a JSON object")?;
        object.extend(
            args.as_object()
                .ok_or("arguments must be an object")?
                .clone(),
        );
        object.remove("scenario");
        if let Some(path) = object
            .remove("save_scenario")
            .and_then(|v| v.as_str().map(str::to_owned))
        {
            use std::io::Write;
            let path = self.engine.repo.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("cannot create scenario directory {}: {e}", parent.display())
                })?;
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("cannot save scenario {}: {e}", path.display()))?;
            file.write_all(
                serde_json::to_string_pretty(&resolved)
                    .map_err(|e| e.to_string())?
                    .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
        }
        let mut touched_runtime = false;
        let result = self.playtest_inner(&resolved, &mut touched_runtime);
        if result.is_err() && touched_runtime {
            let _ = self.engine.call("input.clear", json!({}));
            if preview_params(&resolved).is_some() {
                let _ = self.set_preview(json!({}), 1);
            }
            let _ = self.engine.call("runtime.set", json!({"mode":"stop"}));
            let _ = self.engine.call("wait", json!({"frames":2}));
        }
        result
    }

    fn check_assertions(&mut self, args: &Value, step: Option<u64>) -> Result<Vec<Value>, String> {
        let mut results = Vec::new();
        if let Some(assertions) = args.get("assertions") {
            for assertion in assertions.as_array().ok_or("assertions must be an array")? {
                let at = assertion["at_step"].as_u64();
                if at != step && !(at.is_none() && step == args["steps"].as_u64()) {
                    continue;
                }
                let code = assertion["code"]
                    .as_str()
                    .ok_or("assertion code must be a Lua expression")?;
                let mut params = json!({"code":format!("return ({code})"),"read_only":true,"fallback_to_script":true});
                if let Some(entity) = args.get("entity") {
                    params["entity"] = entity.clone();
                }
                let value = self
                    .engine
                    .call("lua.eval", params)
                    .unwrap_or_else(|e| json!({"error":e.to_string()}));
                results.push(json!({"name":assertion.get("name").cloned().unwrap_or(json!(code)),"at_step":step,"passed":value["result"]==true,"observed":value}));
            }
        }
        Ok(results)
    }

    fn playtest_inner(
        &mut self,
        args: &Value,
        touched_runtime: &mut bool,
    ) -> Result<Output, String> {
        let steps = args
            .get("steps")
            .map(|v| {
                v.as_u64()
                    .filter(|n| (1..=10000).contains(n))
                    .ok_or("steps must be 1..=10000")
            })
            .transpose()?;
        if let Some(assertions) = args.get("assertions") {
            for assertion in assertions.as_array().ok_or("assertions must be an array")? {
                if assertion["code"].as_str().is_none() {
                    return Err("assertions need code".into());
                }
                if let Some(at) = assertion.get("at_step") {
                    let at = at.as_u64().ok_or("at_step must be a nonnegative integer")?;
                    if steps.is_none_or(|n| at > n) {
                        return Err("at_step requires steps and must be <= steps".into());
                    }
                }
            }
        }
        let seconds = args
            .get("seconds")
            .and_then(Value::as_f64)
            .unwrap_or(3.0)
            .clamp(0.0, 240.0);
        let stop = args.get("stop").and_then(Value::as_bool).unwrap_or(true);
        // A lean run skips everything that costs tokens unless it is asked for.
        let lean = args.get("lean").and_then(Value::as_bool).unwrap_or(false);
        let sounds = match args.get("sounds") {
            Some(Value::Bool(false)) => SoundReport::Off,
            Some(Value::Bool(true)) => SoundReport::List,
            Some(Value::String(mode)) if mode == "list" => SoundReport::List,
            Some(Value::String(mode)) if mode == "summary" => SoundReport::Summary,
            _ if lean => SoundReport::Off,
            _ => SoundReport::Summary,
        };
        let call = |engine: &mut Engine, method: &str, params: Value| {
            engine.call(method, params).map_err(|err| err.to_string())
        };

        let log_start =
            call(&mut self.engine, "logs", json!({ "limit": 1 }))?["latest_seq"].clone();
        let sounds_since = self
            .engine
            .call("audio.status", json!({}))
            .ok()
            .map(|status| status["next_id"].clone());
        let diff = args.get("diff").and_then(Value::as_bool).unwrap_or(!lean);
        if diff {
            call(&mut self.engine, "scene.mark", json!({}))?;
        }
        let preview = preview_params(args);
        if let Some(preview) = &preview {
            *touched_runtime = true;
            self.set_preview(preview.clone(), 1)?;
        }
        let mut start = json!({ "mode": "play" });
        if steps.is_some() || args.get("seed").is_some() {
            *touched_runtime = true;
            call(&mut self.engine, "runtime.set", json!({"mode":"stop"}))?;
            call(&mut self.engine, "wait", json!({"frames":2}))?;
            call(&mut self.engine, "input.clear", json!({}))?;
            start["manual"] = json!(steps.is_some());
        }
        if let Some(seed) = args.get("seed") {
            start["seed"] = seed.clone();
        }
        if let Some(scale) = args.get("time_scale").filter(|_| steps.is_none()) {
            start["time_scale"] = scale.clone();
        }
        call(&mut self.engine, "runtime.set", start)?;
        *touched_runtime = true;
        // Play starts at the beginning of the next frame.
        call(&mut self.engine, "wait", json!({ "frames": 2 }))?;

        if let Some(setup) = args["setup"].as_str() {
            let mut params = json!({"code":setup,"fallback_to_script":true});
            if let Some(entity) = args.get("entity") {
                params["entity"] = entity.clone();
            }
            call(&mut self.engine, "lua.eval", params)?;
        }
        let mut assertions = Vec::new();
        let waited = if let Some(total) = steps {
            let mut checkpoints = std::collections::BTreeSet::from([0, total]);
            if let Some(rows) = args["assertions"].as_array() {
                for row in rows {
                    if let Some(at) = row["at_step"].as_u64() {
                        checkpoints.insert(at);
                    }
                }
            }
            let mut previous = 0;
            let mut waited = json!({"steps":0});
            let mut input_sent = false;
            for checkpoint in checkpoints {
                if checkpoint > previous {
                    let mut params = json!({"steps":checkpoint-previous});
                    if let Some(dt) = args.get("dt") {
                        params["dt"] = dt.clone();
                    }
                    if !input_sent {
                        if let Some(events) = args.get("input") {
                            params["input"] = events.clone();
                        }
                        input_sent = true;
                    }
                    waited = call(&mut self.engine, "runtime.step", params)?;
                }
                assertions.extend(self.check_assertions(args, Some(checkpoint))?);
                if assertions.iter().any(|r| r["passed"] == false) {
                    break;
                }
                previous = checkpoint;
            }
            waited
        } else {
            if let Some(events) = args.get("input").filter(|events| !events.is_null()) {
                call(&mut self.engine, "input.send", json!({"events":events}))?;
            }
            let waited = call(
                &mut self.engine,
                "wait",
                json!({"seconds":seconds,"frames":1}),
            )?;
            assertions = self.check_assertions(args, None)?;
            waited
        };

        let mut report = serde_json::Map::new();
        report.insert("waited".into(), waited);
        report.insert(
            "ok".into(),
            json!(assertions.iter().all(|r| r["passed"] == true)),
        );
        report.insert(
            "first_failure".into(),
            assertions
                .iter()
                .find(|r| r["passed"] == false)
                .cloned()
                .unwrap_or(Value::Null),
        );
        report.insert("assertions".into(), json!(assertions));
        let runtime = call(&mut self.engine, "runtime.get", json!({}))?;
        report.insert("time_scale".into(), runtime["time_scale"].clone());
        if let Some(code) = args.get("eval").and_then(Value::as_str) {
            // Without an entity, a global only a script defines (GAME) is looked up there.
            let mut params = json!({ "code": code, "read_only": true, "fallback_to_script": true });
            if let Some(entity) = args.get("entity") {
                params["entity"] = entity.clone();
            }
            report.insert(
                "eval".into(),
                self.engine
                    .call("lua.eval", params)
                    .unwrap_or_else(|err| json!({ "error": err.to_string() })),
            );
        }
        let scripts = call(&mut self.engine, "script.status", json!({}))?;
        let failing: Vec<Value> = scripts["scripts"]
            .as_array()
            .map(|scripts| {
                scripts
                    .iter()
                    .filter(|script| !script["last_error"].is_null())
                    .map(|script| json!({ "entity": script["entity"], "name": script["name"], "error": script["last_error"] }))
                    .collect()
            })
            .unwrap_or_default();
        if !failing.is_empty() {
            report.insert("ok".into(), json!(false));
        }
        report.insert("failing_scripts".into(), json!(failing));
        let logs = call(
            &mut self.engine,
            "logs",
            json!({ "after": log_start, "min_level": "warn", "limit": 30 }),
        )?;
        let warnings: Vec<Value> = logs["entries"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| {
                        json!(format!(
                            "{} {}: {}",
                            entry["level"].as_str().unwrap_or(""),
                            entry["target"].as_str().unwrap_or(""),
                            entry["message"].as_str().unwrap_or("")
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        report.insert("warnings".into(), json!(warnings));
        if let (Some(since), true) = (sounds_since, sounds != SoundReport::Off) {
            let summary = sounds == SoundReport::Summary;
            if let Ok(status) = self.engine.call(
                "audio.status",
                json!({ "since": since, "summary": summary }),
            ) {
                if summary {
                    report.insert("sounds_summary".into(), status["summary"].clone());
                } else {
                    report.insert("sounds_played".into(), status["recent"].clone());
                }
            }
        }
        if diff {
            let limit = args.get("diff_limit").and_then(Value::as_u64).unwrap_or(30);
            report.insert(
                "scene_diff".into(),
                call(&mut self.engine, "scene.diff", json!({ "limit": limit }))?,
            );
        }

        let screenshot = if args
            .get("screenshot")
            .and_then(Value::as_bool)
            .unwrap_or(!lean)
        {
            let mut params = json!({ "target": "viewport", "inline": true });
            // The preview is still on here, so the capture shows the previewed shape.
            if let Some(size) = args.get("max_size") {
                params["max_size"] = size.clone();
            }
            let mut shot = call(&mut self.engine, "screenshot", params)?;
            shot.as_object_mut()
                .and_then(|shot| shot.remove("png_base64"))
                .and_then(|data| data.as_str().map(str::to_owned))
        } else {
            None
        };

        call(&mut self.engine, "input.clear", json!({}))?;
        if preview.is_some() {
            self.set_preview(json!({}), 1)?;
        }
        if stop {
            call(&mut self.engine, "runtime.set", json!({ "mode": "stop" }))?;
            call(&mut self.engine, "wait", json!({ "frames": 2 }))?;
            report.insert(
                "stopped".into(),
                json!("the scene is back to its state before play"),
            );
        } else {
            report.insert("stopped".into(), json!(false));
        }

        let report = Value::Object(report);
        Ok(match screenshot {
            Some(png_base64) => Output::Image {
                png_base64,
                caption: report,
            },
            None => Output::Json(report),
        })
    }

    /// Sample the engine process with `perf` and return its hottest symbols.
    fn profile_cpu(&mut self, args: &Value) -> Result<Value, String> {
        let pid = match args.get("pid").and_then(Value::as_u64) {
            Some(pid) => u32::try_from(pid).map_err(|_| "pid is out of range".to_string())?,
            None => self.engine.managed_pid().ok_or(
                "no engine process launched by this server is running; pass pid to sample \
                 another one",
            )?,
        };
        let seconds = args
            .get("seconds")
            .and_then(Value::as_f64)
            .unwrap_or(4.0)
            .clamp(1.0, 30.0);
        let top = args
            .get("top")
            .and_then(Value::as_u64)
            .unwrap_or(25)
            .clamp(1, 100) as usize;
        let callers = args
            .get("callers")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if std::process::Command::new("perf")
            .arg("--version")
            .output()
            .is_err()
        {
            return Err("perf is not installed (Linux perf tools); CPU sampling needs it".into());
        }
        let dir = self.engine.repo.join("target/engine-mcp");
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        let data = dir.join(format!("perf-{pid}.data"));
        let record = std::process::Command::new("perf")
            .args(["record", "-F", "999", "-g", "-p"])
            .arg(pid.to_string())
            .arg("-o")
            .arg(&data)
            .args(["--", "sleep"])
            .arg(format!("{seconds}"))
            .output()
            .map_err(|err| format!("failed to run perf record: {err}"))?;
        if !record.status.success() {
            let stderr = String::from_utf8_lossy(&record.stderr);
            return Err(format!(
                "perf record failed ({}): {}",
                record.status,
                stderr.trim().lines().last().unwrap_or("")
            ));
        }
        let mut report = std::process::Command::new("perf");
        report.args(["report", "-i"]).arg(&data).args([
            "--stdio",
            "-g",
            "none",
            "--percent-limit",
            "0.2",
        ]);
        report.arg(if callers {
            "--children"
        } else {
            "--no-children"
        });
        let output = report
            .output()
            .map_err(|err| format!("failed to run perf report: {err}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let (samples, rows) = parse_perf_report(&text, callers);
        Ok(json!({
            "pid": pid,
            "seconds": seconds,
            "samples": samples,
            "callers": callers,
            "top": rows.into_iter().take(top).collect::<Vec<_>>(),
            "perf_data": data.display().to_string(),
            "note": if callers {
                "percent is the share of samples in the symbol or anything it called (children), self_percent in the symbol itself; an idle engine samples little, so look at render-related symbols rather than waits"
            } else {
                "percent is the share of CPU samples spent in the symbol itself (self time) over the sampling window, by shared object; the engine sleeps between frames, so a small total share can still be the whole frame cost"
            },
        }))
    }

    fn validate_target(&self, target: &str) -> Result<(), String> {
        if matches!(target, "editor" | "player") {
            return Ok(());
        }
        let examples: Vec<String> = std::fs::read_dir(self.engine.repo.join("examples"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        name.strip_suffix(".rs").map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if examples.iter().any(|example| example == target) {
            Ok(())
        } else {
            let mut examples = examples;
            examples.sort();
            Err(format!(
                "unknown target {target:?}; use editor, player or an example: {}",
                examples.join(", ")
            ))
        }
    }

    fn launch(&mut self, args: &Value) -> Result<Value, String> {
        let target = args
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("editor");
        let binary_override = args.get("binary").and_then(Value::as_str).map(|path| {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                self.engine.repo.join(path)
            }
        });
        if binary_override.is_none() {
            self.validate_target(target)?;
        }
        let window_size = match args.get("window_size").filter(|v| !v.is_null()) {
            Some(value) => Some(parse_window_size(value)?),
            None => None,
        };
        let project = args.get("project").and_then(Value::as_str).map(|path| {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                self.engine.repo.join(path)
            }
        });
        let build = args.get("build").and_then(Value::as_bool).unwrap_or(true);
        let release = args
            .get("release")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let restart = args
            .get("restart")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let renderdoc = args
            .get("renderdoc")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let timeout = Duration::from_secs_f64(
            args.get("timeout_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(120.0)
                .clamp(5.0, 900.0),
        );

        let mut report = serde_json::Map::new();
        if restart && self.engine.has_managed_process() {
            engine::launch_environment(args.get("env"))?;
            report.insert("stopped_previous".into(), self.engine.stop()?);
            // Give the OS a moment to release the port.
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.engine.is_listening() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        if self.engine.is_listening() {
            let managed = self.engine.has_managed_process();
            self.engine.reconnect().map_err(|err| err.to_string())?;
            report.insert(
                "attached".into(),
                json!(if managed {
                    "the engine this server launched is already running; pass restart: true to relaunch it"
                } else {
                    "an engine started outside this server is already listening; attached to it"
                }),
            );
            if renderdoc {
                report.insert(
                    "renderdoc".into(),
                    json!("ignored: the running engine keeps how it was started; pass restart: true to relaunch it under RenderDoc"),
                );
            }
        } else {
            if target == "player" && project.is_none() {
                return Err("the player needs a project directory (project)".into());
            }
            let mut env = engine::launch_environment(args.get("env"))?;
            if let Some((width, height)) = window_size {
                env.insert("ENGINE_WINDOW_SIZE".into(), format!("{width}x{height}"));
            }
            if renderdoc {
                // RenderDoc's implicit Vulkan layer loads it into the process; it reads
                // the capture path template from RENDERDOC_CAPFILE as it loads. WGPU_DEBUG
                // passes wgpu's labels on, so captures name passes and resources.
                let captures = self.engine.repo.join("target/engine-mcp/captures");
                env.insert("ENABLE_VULKAN_RENDERDOC_CAPTURE".into(), "1".into());
                env.entry("WGPU_DEBUG".into()).or_insert_with(|| "1".into());
                env.entry("RENDERDOC_CAPFILE".into())
                    .or_insert_with(|| captures.join(target).display().to_string());
                report.insert(
                    "renderdoc".into(),
                    json!({ "captures": captures.display().to_string() }),
                );
            }
            let binary = if let Some(binary) = binary_override {
                if !binary.is_file() {
                    return Err(format!("binary {} does not exist", binary.display()));
                }
                // A binary from another tree must not pick up this tree's shaders.
                env.entry("ENGINE_SHADER_RELOAD".into())
                    .or_insert_with(|| "0".into());
                report.insert("binary".into(), json!(binary.display().to_string()));
                binary
            } else if build {
                let (binary, summary) = self.engine.build(target, release)?;
                report.insert("build".into(), json!(summary));
                binary
            } else {
                self.engine.binary_path(target, release)?
            };
            let mut process_args = Vec::new();
            if target == "player" {
                if let Some(project) = &project {
                    process_args.push(project.display().to_string());
                }
            }
            let monitor = args.get("monitor").and_then(|monitor| match monitor {
                Value::String(name) => Some(name.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            });
            let process = self.engine.spawn(
                target,
                &binary,
                &process_args,
                monitor.as_deref(),
                &env,
                timeout,
            )?;
            report.insert("process".into(), process);
        }

        if target == "editor" {
            if let Some(project) = &project {
                report.insert("project".into(), self.open_project(project)?);
            }
        }

        let info = self
            .engine
            .call("engine.info", json!({}))
            .map_err(|err| err.to_string())?;
        report.insert("engine".into(), info);
        Ok(Value::Object(report))
    }

    /// Open `project` in the editor and wait until it has loaded.
    fn open_project(&mut self, project: &Path) -> Result<Value, String> {
        if !project.join("project.json").exists() {
            return Err(format!(
                "{} has no project.json; create it with project_create",
                project.display()
            ));
        }
        let state = self
            .engine
            .call("editor.state", json!({}))
            .map_err(|err| err.to_string())?;
        if project_is_open(&state, project) {
            return Ok(json!({ "already_open": project.display().to_string() }));
        }
        self.engine
            .call(
                "project.open",
                json!({ "path": project.display().to_string() }),
            )
            .map_err(|err| err.to_string())?;
        self.wait_for_project(project)?;
        Ok(json!({ "opened": project.display().to_string() }))
    }

    /// Fill a new project's scene with what every game needs first: a dark backdrop with
    /// usable ambient light, a post-process volume with slight grading, a shadowed sun, an active camera looking
    /// down at the origin and a 40 m floor; then save it.
    fn build_starter_scene(&mut self) -> Result<Value, String> {
        let calls = [
            (
                "environment.set",
                json!({
                    "clear_color": "#05070f",
                    "ambient_intensity": 0.35,
                }),
            ),
            (
                "postprocess.set",
                json!({ "color": { "saturation": 1.15, "contrast": 1.05 } }),
            ),
            (
                "entity.spawn",
                json!({
                    "kind": "directional_light", "name": "Sun",
                    "translation": [0, 10, 0], "rotation": [-55, 25, 0],
                    "directional_light": { "color": [1.0, 0.96, 0.9], "intensity": 1.6, "shadow_size": 30 },
                    "quiet": true,
                }),
            ),
            (
                "entity.spawn",
                json!({
                    "kind": "camera", "name": "GameCamera",
                    "translation": [0, 14, 12], "look_at": [0, 0, 0],
                    "camera": { "fov_deg": 50, "near": 0.3, "far": 300 },
                    "quiet": true,
                }),
            ),
            (
                "entity.spawn",
                json!({
                    "kind": "plane", "name": "Floor",
                    "rotation": [-90, 0, 0], "scale": [40, 40, 1],
                    "material": { "color": "#262b36", "roughness": 0.9, "metallic": 0.0 },
                    "collider": {},
                    "quiet": true,
                }),
            ),
        ];
        for (method, params) in calls {
            self.engine
                .call(method, params)
                .map_err(|err| format!("starter scene: {method} failed: {err}"))?;
        }
        let saved = self.save_project(&json!({}))?;
        Ok(json!({
            "entities": ["Post Process Volume (infinite: tonemapping, bloom, SSAO, FXAA, slight grading)", "Sun (directional light, shadows)", "GameCamera (active, 14 m up, looking at the origin)", "Floor (40 x 40 m plane at y = 0, with a collider)"],
            "environment": "dark blue backdrop, ambient 0.35",
            "note": "Post-processing lives on the Post Process Volume entity (postprocess_get/postprocess_set). Change or delete any of this; pass template: \"empty\" for a bare scene.",
            "saved": saved,
        }))
    }

    /// Save the project, wait for the editor to finish, and list the files it wrote.
    fn save_project(&mut self, args: &Value) -> Result<Value, String> {
        // File times come from a coarse kernel clock that can lag this one by a tick.
        let started = SystemTime::now() - Duration::from_millis(20);
        let queued = self
            .engine
            .call("project.save", args.clone())
            .map_err(|err| err.to_string())?;
        let seq = queued["save_seq"].as_u64().unwrap_or(0);
        let directory = PathBuf::from(queued["path"].as_str().unwrap_or_default());
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            self.engine
                .call("wait", json!({ "frames": 1 }))
                .map_err(|err| err.to_string())?;
            let state = self
                .engine
                .call("editor.state", json!({}))
                .map_err(|err| err.to_string())?;
            let save = &state["last_save"];
            if save["seq"].as_u64().is_some_and(|done| done >= seq) {
                if let Some(error) = save["error"].as_str() {
                    return Err(format!("saving {} failed: {error}", directory.display()));
                }
                break;
            }
            if Instant::now() > deadline {
                return Err("the editor did not finish saving within 60s; check logs".into());
            }
        }
        let mut written = Vec::new();
        collect_modified(&directory, &directory, started, &mut written);
        written.sort();
        let mut report = json!({
            "saved": directory.display().to_string(),
            "project": directory.join("project.json").display().to_string(),
            "scenes": written.iter().filter(|path| Path::new(path).starts_with(Path::new("content").join("scenes"))).collect::<Vec<_>>(),
            "written_count": written.len(),
            "layout": match args["compact"].as_bool() {
                Some(true) => "compact",
                Some(false) => "indented",
                None => "as found on disk",
            },
        });
        if args["verbose"].as_bool().unwrap_or(false) {
            report["written"] = json!(written);
        }
        Ok(report)
    }

    /// Wait until the editor reports `project` as its open project.
    fn wait_for_project(&mut self, project: &Path) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut last_state = Value::Null;
        while Instant::now() < deadline {
            self.engine
                .call("wait", json!({ "frames": 5 }))
                .map_err(|err| err.to_string())?;
            let state = self
                .engine
                .call("editor.state", json!({}))
                .map_err(|err| err.to_string())?;
            if project_is_open(&state, project) {
                return Ok(());
            }
            last_state = state;
        }
        Err(format!(
            "the project did not finish loading within 60s; check logs. Last editor state: {}",
            pretty(&last_state)
        ))
    }
}

/// Apply the batch's `aggregate` / `quiet` options to its finished report.
fn shape_batch_report(report: &mut Value, args: &Value, calls: &[Value]) {
    if args["aggregate"].as_bool().unwrap_or(false) {
        aggregate_batch(report, calls);
    } else if args["quiet"].as_bool().unwrap_or(false) {
        summarize_batch(report);
    }
}

/// Most entities a batch aggregate lists one by one; larger sets give a count, first and last.
const AGGREGATE_LIST_LIMIT: usize = 24;

/// Replace a batch's per-call results with totals: calls per tool, the entities they
/// returned, the aliases that were named and every failed call in full. The report's own
/// verdict (`ok`, `failure`, `rolled_back`, `committed`...) is left as it is.
fn aggregate_batch(report: &mut Value, calls: &[Value]) {
    let Some(Value::Array(rows)) = report.as_object_mut().and_then(|r| r.remove("results")) else {
        return;
    };
    let mut by_tool = serde_json::Map::new();
    let mut entities: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut aliases = serde_json::Map::new();
    let mut failed_calls = Vec::new();

    let mut note_entity = |entity: &Value| {
        if let Some(id) = entity.get("id").filter(|id| id.is_number()) {
            if seen.insert(id.to_string()) {
                entities.push(
                    json!({ "id": id, "name": entity.get("name").cloned().unwrap_or(Value::Null) }),
                );
            }
        }
    };

    for (index, row) in rows.iter().enumerate() {
        let call = calls.get(index);
        let tool = call
            .and_then(|call| call["tool"].as_str())
            .or_else(|| row["tool"].as_str())
            .or_else(|| row["method"].as_str())
            .unwrap_or("?");
        let result = &row["result"];
        if row.get("error").is_some() || result["ok"] == false {
            let mut failed = json!({ "index": index, "tool": tool });
            for key in ["error", "result"] {
                if let Some(value) = row.get(key) {
                    failed[key] = value.clone();
                }
            }
            failed_calls.push(failed);
            continue;
        }
        let count = by_tool.get(tool).and_then(Value::as_u64).unwrap_or(0);
        by_tool.insert(tool.to_string(), json!(count + 1));

        note_entity(result);
        for key in ["first", "last"] {
            note_entity(&result[key]);
        }
        for copy in result["copies"].as_array().into_iter().flatten() {
            note_entity(copy);
        }
        if let Some(alias) = call.and_then(|call| call["as"].as_str()) {
            aliases.insert(
                alias.to_string(),
                result.get("id").cloned().unwrap_or(Value::Null),
            );
        }
    }

    let mut summary = json!({ "calls": by_tool });
    if !entities.is_empty() {
        summary["entities"] = if entities.len() <= AGGREGATE_LIST_LIMIT {
            json!({ "count": entities.len(), "list": entities })
        } else {
            json!({
                "count": entities.len(),
                "first": entities[0],
                "last": entities[entities.len() - 1],
                "note": "Address them by name; ids need not be consecutive.",
            })
        };
    }
    if !aliases.is_empty() {
        summary["aliases"] = Value::Object(aliases);
    }
    report["summary"] = summary;
    if !failed_calls.is_empty() {
        report["failed_calls"] = Value::Array(failed_calls);
    }
}

/// Tools whose results a quiet batch keeps whole (minus the notes): they are the data the
/// batch was run for, not an echo of what it created.
const QUIET_KEEPS: &[&str] = &[
    "engine_stats",
    "profile_capture",
    "engine_info",
    "editor_state",
    "camera_get",
    "graphics_get",
    "postprocess_get",
    "environment_get",
    "entity_get",
    "light_probes_status",
    "visual_compare",
    "engine_profile_cpu",
    "renderdoc_capture",
    "input_state",
    "script_status",
];

/// Drop explanatory `note` / `*_note` keys from a result, at any depth.
fn strip_notes(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, _| key != "note" && !key.ends_with("_note"));
            object.values_mut().for_each(strip_notes);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_notes),
        _ => {}
    }
}

/// Keep batch failures and result identities without echoing generated geometry.
fn summarize_batch(report: &mut Value) {
    if let Some(results) = report["results"].as_array_mut() {
        for row in results {
            let keep = row["tool"]
                .as_str()
                .is_some_and(|tool| QUIET_KEEPS.contains(&tool));
            if let Some(result) = row.get_mut("result") {
                if result.get("ok") == Some(&Value::Bool(false)) {
                    continue;
                }
                if keep {
                    strip_notes(result);
                    continue;
                }
                let mut summary =
                    json!({"ok": result.get("ok").and_then(Value::as_bool).unwrap_or(true)});
                for key in [
                    "id", "name", "entity", "root", "count", "total", "created", "updated",
                    "deleted",
                ] {
                    if let Some(value) = result.get(key).filter(|v| !v.is_array() && !v.is_object())
                    {
                        summary[key] = value.clone();
                    }
                }
                if let Some(object) = result.as_object() {
                    let counts: serde_json::Map<String, Value> = object
                        .iter()
                        .filter_map(|(key, value)| {
                            value
                                .as_array()
                                .map(|items| (key.clone(), json!(items.len())))
                        })
                        .collect();
                    if !counts.is_empty() {
                        summary["counts"] = Value::Object(counts);
                    }
                }
                if let Some(geometry) = result.get("geometry") {
                    summary["geometry"] = json!({
                        "vertices": geometry["vertices"], "triangles": geometry["triangles"],
                        "local_bounds": geometry["local_bounds"], "key": geometry["key"],
                    });
                }
                *result = summary;
            }
        }
    }
}

/// The `view.preview` parameters for a tool's `aspect` or `viewport_size`, if any.
fn preview_params(args: &Value) -> Option<Value> {
    if let Some(size) = args.get("viewport_size").filter(|size| !size.is_null()) {
        return Some(json!({ "size": size }));
    }
    args.get("aspect")
        .filter(|aspect| !aspect.is_null())
        .map(|aspect| json!({ "aspect": aspect }))
}

/// Files under `dir` modified at or after `since`, as paths relative to `root`.
fn collect_modified(root: &Path, dir: &Path, since: SystemTime, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.') {
                collect_modified(root, &path, since, out);
            }
        } else if meta.modified().is_ok_and(|modified| modified >= since) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            out.push(relative.display().to_string());
        }
    }
}

/// Whether an `editor.state` result has `project` open.
fn project_is_open(state: &Value, project: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    state["project"]["directory"]
        .as_str()
        .is_some_and(|dir| canonical(Path::new(dir)) == canonical(project))
}

/// `[width, height]` for `engine_launch`'s window_size.
fn parse_window_size(value: &Value) -> Result<(u32, u32), String> {
    let pair = value
        .as_array()
        .filter(|items| items.len() == 2)
        .and_then(|items| {
            let side = |item: &Value| {
                item.as_u64()
                    .filter(|side| (16..=16384).contains(side))
                    .map(|side| side as u32)
            };
            side(&items[0]).zip(side(&items[1]))
        });
    pair.ok_or_else(|| "window_size must be [width, height] in pixels, each 16 to 16384".into())
}

/// The sample count and the symbol rows of a `perf report --stdio -g none` listing.
/// With `children`, each row has two percentages (children, self).
fn parse_perf_report(text: &str, children: bool) -> (Option<String>, Vec<Value>) {
    let mut samples = None;
    let mut rows = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# Samples:") {
            samples = rest
                .split_whitespace()
                .next()
                .map(|count| count.trim_end_matches(',').to_owned());
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut fields = trimmed.split_whitespace();
        let percent = |field: Option<&str>| {
            field
                .and_then(|field| field.strip_suffix('%'))
                .and_then(|field| field.parse::<f64>().ok())
        };
        let Some(first) = percent(fields.next()) else {
            continue;
        };
        let self_percent = if children {
            match percent(fields.next()) {
                Some(value) => Some(value),
                None => continue,
            }
        } else {
            None
        };
        // command, shared object, [.]/[k], then the symbol (which may hold spaces).
        let Some(_command) = fields.next() else {
            continue;
        };
        let Some(object) = fields.next() else {
            continue;
        };
        let rest: Vec<&str> = fields.collect();
        let symbol = rest
            .iter()
            .position(|field| field.starts_with('[') && field.ends_with(']') && field.len() == 3)
            .map(|index| rest[index + 1..].join(" "))
            .unwrap_or_else(|| rest.join(" "));
        if symbol.is_empty() {
            continue;
        }
        let mut row = json!({ "percent": first, "object": object, "symbol": symbol });
        if let Some(self_percent) = self_percent {
            row["self_percent"] = json!(self_percent);
        }
        rows.push(row);
    }
    (samples, rows)
}

fn pretty(value: &Value) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    truncate(text)
}

fn truncate(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n... (truncated; narrow the query)");
    }
    text
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn error_result(message: String) -> Value {
    text_result(message, true)
}

impl From<CallError> for String {
    fn from(err: CallError) -> Self {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_report_rows_and_window_sizes_parse() {
        let text = "# Samples: 1K of event 'cpu/cycles/Pu'\n#\n     3.57%  sponza  libc.so.6  [.] cfree\n     1.96%  sponza  sponza  [.] <wgpu_cube::asset::mesh::Mesh as core::clone::Clone>::clone\n";
        let (samples, rows) = parse_perf_report(text, false);
        assert_eq!(samples.as_deref(), Some("1K"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["percent"], 3.57);
        assert_eq!(rows[1]["object"], "sponza");
        assert_eq!(
            rows[1]["symbol"],
            "<wgpu_cube::asset::mesh::Mesh as core::clone::Clone>::clone"
        );
        let text =
            "    12.00%     0.50%  editor  editor  [.] wgpu_cube::renderer::Renderer::render\n";
        let (_, rows) = parse_perf_report(text, true);
        assert_eq!(rows[0]["percent"], 12.0);
        assert_eq!(rows[0]["self_percent"], 0.5);
        assert_eq!(parse_window_size(&json!([800, 450])).unwrap(), (800, 450));
        assert!(parse_window_size(&json!([8, 450])).is_err());
        assert!(parse_window_size(&json!("800x450")).is_err());
    }

    #[test]
    fn quiet_batches_keep_read_only_results_without_notes() {
        let mut report = json!({"results": [
            {"tool": "engine_stats", "result": {"fps": 60.0, "frame_breakdown_note": "x", "renderer": {"gpu": {"total_ms": 1.0, "note": "y"}}}},
            {"tool": "entity_spawn", "result": {"id": 7, "name": "Cube", "components": ["a", "b"]}}
        ]});
        summarize_batch(&mut report);
        let rows = report["results"].as_array().unwrap();
        assert_eq!(rows[0]["result"]["fps"], 60.0);
        assert!(rows[0]["result"].get("frame_breakdown_note").is_none());
        assert!(rows[0]["result"]["renderer"]["gpu"].get("note").is_none());
        assert_eq!(rows[0]["result"]["renderer"]["gpu"]["total_ms"], 1.0);
        assert_eq!(rows[1]["result"]["counts"]["components"], 2);
    }

    #[test]
    fn sequential_batch_errors_set_mcp_error_and_stop() {
        let mut server = Server {
            engine: Engine::new(PathBuf::new(), 0),
            tools: tools::tools(),
            started: SystemTime::now(),
        };
        let result = server.call_tool(&json!({"name":"batch","arguments":{
            "quiet":true,"calls":[
                {"tool":"engine_status"}, {"tool":"missing_tool"}, {"tool":"engine_status"}
            ]
        }}));
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["ran"], 2);
        assert_eq!(result["structuredContent"]["failed"], 1);
        assert!(result["structuredContent"]["results"][1]["error"].is_string());
    }

    #[test]
    fn quiet_batch_keeps_failure_and_ids_without_mesh_payloads() {
        let mut report = json!({"ok":false,"committed":false,"rolled_back":true,
            "failure":{"index":1,"message":"compose needs 1..=128 parts"},
            "results":[{"method":"mesh.compose","result":{"id":42,"name":"Wall",
                "geometry":{"vertices":12000,"triangles":6000,"key":"abc",
                    "recipe":{"parameters":{"parts":vec![json!({"large":"recipe"});128]}}},
                "mesh":{"vertices":vec![0;12000]}}}]});
        summarize_batch(&mut report);
        assert_eq!(report["results"][0]["result"]["id"], 42);
        assert_eq!(
            report["results"][0]["result"]["geometry"]["vertices"],
            12000
        );
        assert_eq!(report["failure"]["index"], 1);
        assert_eq!(report["rolled_back"], true);
        assert!(!report["results"][0]["result"]
            .as_object()
            .unwrap()
            .contains_key("mesh"));
        assert!(report.to_string().len() < 600);
    }

    #[test]
    fn aggregate_batch_totals_calls_and_lists_entities_and_failures() {
        let calls = vec![
            json!({"tool":"entity_spawn","as":"root","arguments":{}}),
            json!({"tool":"entity_duplicate","arguments":{}}),
            json!({"tool":"entity_update","arguments":{}}),
            json!({"tool":"mesh_lathe","arguments":{}}),
        ];
        let mut report = json!({"ok":false,"committed":false,"rolled_back":true,
            "failure":{"index":3,"method":"mesh.lathe","message":"bad profile"},
            "results":[
                {"method":"entity.spawn","result":{"id":7,"name":"Group","components":vec![json!({"big":1});50]}},
                {"method":"entity.duplicate","result":{"count":2,"first":{"id":8,"name":"Wall 1"},
                    "last":{"id":9,"name":"Wall 2"}}},
                {"method":"entity.update","result":{"id":7,"name":"Group"}},
                {"method":"mesh.lathe","error":"bad profile"}]});
        aggregate_batch(&mut report, &calls);
        assert!(report.get("results").is_none());
        assert_eq!(report["rolled_back"], true);
        assert_eq!(report["failure"]["message"], "bad profile");
        let summary = &report["summary"];
        assert_eq!(summary["calls"]["entity_spawn"], 1);
        assert_eq!(summary["calls"]["entity_update"], 1);
        assert!(summary["calls"].get("mesh_lathe").is_none());
        // Entity 7 comes back twice and is listed once.
        assert_eq!(summary["entities"]["count"], 3);
        assert_eq!(summary["entities"]["list"][2]["name"], "Wall 2");
        assert_eq!(summary["aliases"]["root"], 7);
        assert_eq!(report["failed_calls"][0]["index"], 3);
        assert_eq!(report["failed_calls"][0]["tool"], "mesh_lathe");
        assert!(report.to_string().len() < 1200, "{report}");
    }

    #[test]
    fn aggregate_batch_collapses_large_entity_sets_to_first_and_last() {
        let calls: Vec<Value> = (0..100)
            .map(|_| json!({"tool":"entity_spawn","arguments":{}}))
            .collect();
        let rows: Vec<Value> = (0..100)
            .map(|i| json!({"tool":"entity_spawn","result":{"id":100 + i,"name":format!("Brick {i}")}}))
            .collect();
        let mut report = json!({"ok":true,"ran":100,"of":100,"failed":0,"results":rows});
        aggregate_batch(&mut report, &calls);
        assert_eq!(report["summary"]["calls"]["entity_spawn"], 100);
        assert_eq!(report["summary"]["entities"]["count"], 100);
        assert!(report["summary"]["entities"].get("list").is_none());
        assert_eq!(report["summary"]["entities"]["first"]["name"], "Brick 0");
        assert_eq!(report["summary"]["entities"]["last"]["name"], "Brick 99");
        assert!(report.get("failed_calls").is_none());
        assert!(report.to_string().len() < 700);
    }

    #[test]
    fn scenario_creates_parents_before_runtime_and_refuses_overwrite() {
        let dir = std::env::temp_dir().join(format!(
            "engine-mcp-scenario-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut server = Server {
            engine: Engine::new(dir.clone(), 0),
            tools: tools::tools(),
            started: SystemTime::now(),
        };
        let args = json!({"save_scenario":"nested/tests/run.json", "steps":1, "seed":123});
        // No engine is listening, but the resolved scenario is saved before touching it.
        assert!(server.playtest(&args).is_err());
        let path = dir.join("nested/tests/run.json");
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&saved).unwrap(),
            json!({"steps":1,"seed":123})
        );
        let error = server
            .playtest(&json!({"save_scenario":"nested/tests/run.json","steps":2}))
            .err()
            .unwrap();
        assert!(error.starts_with("cannot save scenario"), "{error}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn workflow_is_a_discoverable_resource() {
        let mut server = Server {
            engine: Engine::new(PathBuf::new(), 0),
            tools: tools::tools(),
            started: SystemTime::now(),
        };
        let listed = server
            .handle(&json!({"id":1,"method":"resources/list"}))
            .unwrap();
        assert_eq!(listed["result"]["resources"][0]["uri"], WORKFLOW_URI);
        let read = server
            .handle(&json!({"id":2,"method":"resources/read","params":{"uri":WORKFLOW_URI}}))
            .unwrap();
        assert_eq!(read["result"]["contents"][0]["text"], WORKFLOW);
        assert!(INSTRUCTIONS.len() < 400);
    }
}
