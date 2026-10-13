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

        let Some(action) = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .map(|tool| match tool.action {
                Action::Engine(method) => Some(method),
                Action::Local => None,
            })
        else {
            return error_result(format!("unknown tool {name:?}"));
        };

        let outcome = match action {
            Some(method) => self
                .engine
                .call(method, arguments)
                .map(Output::Json)
                .map_err(|err| err.to_string()),
            None => self.local_tool(name, &arguments),
        };

        match outcome {
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

    fn local_tool(&mut self, name: &str, args: &Value) -> Result<Output, String> {
        match name {
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
                     - Entity ids returned by the engine tools are the same i64 handles Lua uses.\n\
                     - lua_eval runs with this whole API; `return` a value (or write a bare expression) to get it back.\n\
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
            let process = self.engine.spawn(target, &binary, &process_args, timeout)?;
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
        let wanted = project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf());
        let loaded = |state: &Value| {
            state["project"]["directory"]
                .as_str()
                .map(|dir| {
                    PathBuf::from(dir)
                        .canonicalize()
                        .unwrap_or_else(|_| PathBuf::from(dir))
                })
                .is_some_and(|dir| dir == wanted)
        };

        let state = self
            .engine
            .call("editor.state", json!({}))
            .map_err(|err| err.to_string())?;
        if loaded(&state) {
            return Ok(json!({ "already_open": project.display().to_string() }));
        }
        self.engine
            .call(
                "project.open",
                json!({ "path": project.display().to_string() }),
            )
            .map_err(|err| err.to_string())?;

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
            if loaded(&state) {
                return Ok(json!({ "opened": project.display().to_string() }));
            }
            last_state = state;
        }
        Err(format!(
            "the project did not finish loading within 60s; check logs. Last editor state: {}",
            pretty(&last_state)
        ))
    }
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
