//! MCP (Model Context Protocol) server for the wgpu-cube engine.
//!
//! Speaks JSON-RPC over stdio to an MCP client (such as Claude Code) and forwards tool
//! calls to a running engine's remote-control socket (`wgpu_cube::remote`). It can
//! also build and launch the engine.
//!
//! Usage: `engine-mcp [--repo DIR] [--port PORT]`. The repository defaults to the one
//! this binary was built in; the port defaults to `ENGINE_REMOTE_PORT` or 47811.

mod engine;
mod tools;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use engine::{CallError, Engine, DEFAULT_PORT};
use tools::{Action, Tool};

const SERVER_NAME: &str = "engine";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Results longer than this are cut so they do not flood the client's context.
const MAX_TEXT: usize = 120_000;

const INSTRUCTIONS: &str = "\
Controls the wgpu-cube game engine (Rust, wgpu, hecs ECS, Lua scripting).

Workflow:
1. engine_launch (target editor by default) builds and starts the engine, or attaches \
to one already running. engine_info shows its state and value conventions.
2. In the editor, create or open a project first (project_create / project_open); the \
startup dialog blocks the editor until then. editor_state shows the project's scripts folder.
3. Build scenes with entity_spawn / entity_update (meshes, materials, lights, camera, \
particles, environment). Units are meters, Y is up, -Z is forward, rotations are Euler \
degrees. Entities are referenced by id or unique name.
4. Gameplay is Lua: read lua_api_reference, write .lua files into the project's scripts \
folder with your file tools, attach them with entity_update {script: {path}}, then \
runtime_set play, wait, and check script_status, logs and screenshot. runtime_set stop \
restores the pre-play scene.
5. lua_eval runs arbitrary Lua against the live world for queries and bulk edits.
6. project_save persists the work. engine_output shows crashes and GPU validation errors.

Look at the result (screenshot) after visual changes instead of assuming they worked.";

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
                    "capabilities": { "tools": { "listChanged": false } },
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
                })).collect::<Vec<_>>()
            })),
            "tools/call" => Ok(self.call_tool(&params)),
            "resources/list" => Ok(json!({ "resources": [] })),
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
            Ok(Output::Json(value)) => text_result(pretty(&value), false),
            Ok(Output::Text(text)) => text_result(text, false),
            Ok(Output::Image {
                png_base64,
                caption,
            }) => json!({
                "content": [
                    { "type": "image", "data": png_base64, "mimeType": "image/png" },
                    { "type": "text", "text": caption.to_string() }
                ],
                "isError": false
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
                Ok(Output::Json(value)) => json!({ "tool": name, "result": value }),
                Ok(Output::Text(text)) => json!({ "tool": name, "result": text }),
                Ok(Output::Image { caption, .. }) => {
                    json!({ "tool": name, "result": caption, "note": "image left out; call screenshot directly to see it" })
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
        Ok(Output::Json(json!({
            "ran": results.len(),
            "of": calls.len(),
            "failed": failed,
            "results": results,
        })))
    }

    fn local_tool(&mut self, name: &str, args: &Value) -> Result<Output, String> {
        match name {
            "batch" => self.batch(args),
            "engine_launch" => self.launch(args).map(Output::Json),
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
                Ok(Output::Text(format!(
                    "{guide}\n\n## Notes for remote control\n\n\
                     - Entity ids returned by the engine tools are the same i64 handles Lua uses. They stay valid across play/stop and undo.\n\
                     - lua_eval runs with this whole API; `return` a value (or write a bare expression) to get it back.\n\
                     - lua_eval with `entity` runs inside that entity's script, so it sees the script's globals but not its `local` variables. \
                     To inspect game state while testing, keep it in a global table (e.g. `GAME = {{ score = 0 }}`) or in the state API (`set_i32`, ...).\n\
                     - Example scripts: scripts/*.lua and examples/scripts/*.lua in {}.\n",
                    self.engine.repo.display()
                )))
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
                Ok(Output::Json(json!({
                    "created": directory.display().to_string(),
                    "editor": self.engine.call("editor.state", json!({})).map_err(|err| err.to_string())?,
                })))
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
            "playtest" => self.playtest(args),
            "screenshot" => {
                let mut params = args.clone();
                params["inline"] = json!(true);
                let mut result = self
                    .engine
                    .call("screenshot", params)
                    .map_err(|err| err.to_string())?;
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

    /// Play for a while with scripted input, then report errors, the state `eval`
    /// returns and a screenshot, and (by default) stop again.
    fn playtest(&mut self, args: &Value) -> Result<Output, String> {
        let seconds = args
            .get("seconds")
            .and_then(Value::as_f64)
            .unwrap_or(3.0)
            .clamp(0.0, 240.0);
        let stop = args.get("stop").and_then(Value::as_bool).unwrap_or(true);
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
        let diff = args.get("diff").and_then(Value::as_bool).unwrap_or(true);
        if diff {
            call(&mut self.engine, "scene.mark", json!({}))?;
        }
        let mut start = json!({ "mode": "play" });
        if let Some(scale) = args.get("time_scale") {
            start["time_scale"] = scale.clone();
        }
        call(&mut self.engine, "runtime.set", start)?;
        // Play starts at the beginning of the next frame.
        call(&mut self.engine, "wait", json!({ "frames": 2 }))?;

        if let Some(events) = args.get("input").filter(|events| !events.is_null()) {
            call(&mut self.engine, "input.send", json!({ "events": events }))?;
        }
        let waited = call(
            &mut self.engine,
            "wait",
            json!({ "seconds": seconds, "frames": 1 }),
        )?;

        let mut report = serde_json::Map::new();
        report.insert("waited".into(), waited);
        let runtime = call(&mut self.engine, "runtime.get", json!({}))?;
        report.insert("time_scale".into(), runtime["time_scale"].clone());
        if let Some(code) = args.get("eval").and_then(Value::as_str) {
            let mut params = json!({ "code": code, "read_only": true });
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
        if let Some(since) = sounds_since {
            if let Ok(status) = self.engine.call("audio.status", json!({ "since": since })) {
                report.insert("sounds_played".into(), status["recent"].clone());
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
            .unwrap_or(true)
        {
            let mut params = json!({ "target": "viewport", "inline": true });
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
        self.validate_target(target)?;
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
        let timeout = Duration::from_secs_f64(
            args.get("timeout_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(120.0)
                .clamp(5.0, 900.0),
        );

        let mut report = serde_json::Map::new();
        if restart && self.engine.has_managed_process() {
            report.insert("stopped_previous".into(), self.engine.stop()?);
            // Give the OS a moment to release the port.
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.engine.is_listening() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
        }

        if self.engine.is_listening() {
            report.insert(
                "attached".into(),
                json!(if self.engine.has_managed_process() {
                    "the engine this server launched is already running; pass restart: true to relaunch it"
                } else {
                    "an engine started outside this server is already listening; attached to it"
                }),
            );
        } else {
            if target == "player" && project.is_none() {
                return Err("the player needs a project directory (project)".into());
            }
            let binary = if build {
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
            let process =
                self.engine
                    .spawn(target, &binary, &process_args, monitor.as_deref(), timeout)?;
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

/// Whether an `editor.state` result has `project` open.
fn project_is_open(state: &Value, project: &Path) -> bool {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    state["project"]["directory"]
        .as_str()
        .is_some_and(|dir| canonical(Path::new(dir)) == canonical(project))
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
