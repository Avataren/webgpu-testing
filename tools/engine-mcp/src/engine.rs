//! Connection to a running engine, and building and launching engine processes.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Port the editor listens on by default (`wgpu_cube::remote::DEFAULT_EDITOR_PORT`).
pub const DEFAULT_PORT: u16 = 47811;

/// Lines of engine output kept in memory.
const OUTPUT_LINES: usize = 4000;

/// How long a single request may take; the engine itself gives up after 300 s.
const READ_TIMEOUT: Duration = Duration::from_secs(320);

/// What went wrong talking to the engine.
#[derive(Debug)]
pub enum CallError {
    /// No engine is listening; it needs to be launched first.
    NotConnected(String),
    /// The engine answered with an error.
    Engine(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NotConnected(message) => write!(
                f,
                "{message}. Start the engine with engine_launch (or run the editor yourself; it listens on port {DEFAULT_PORT})."
            ),
            CallError::Engine(message) => f.write_str(message),
        }
    }
}

struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

/// An engine process started by this server.
pub struct ManagedProcess {
    child: Child,
    target: String,
    started: Instant,
    output: Arc<Mutex<VecDeque<String>>>,
    log_path: PathBuf,
}

/// Toolchain used to build the engine.
#[derive(Clone)]
pub struct Toolchain {
    description: String,
    bin_dir: Option<PathBuf>,
    target_dir: PathBuf,
}

pub struct Engine {
    pub repo: PathBuf,
    pub port: u16,
    connection: Option<Connection>,
    next_id: u64,
    process: Option<ManagedProcess>,
    last_exit: Option<String>,
}

impl Engine {
    /// Package a saved project through the repository's canonical web-build script.
    pub fn build_web(&self, project: &Path, overwrite: bool) -> Result<Value, String> {
        let project = project.canonicalize().map_err(|e| e.to_string())?;
        if !project.join("project.json").is_file() {
            return Err("project has no project.json".into());
        }
        let output_dir = self
            .repo
            .join("target/web")
            .join(project.file_name().ok_or("project directory has no name")?);
        if output_dir.exists() && !overwrite {
            return Err(format!(
                "{} exists; pass overwrite: true",
                output_dir.display()
            ));
        }
        let toolchain = self.toolchain()?;
        let mut command = Command::new("bash");
        command
            .current_dir(&self.repo)
            .args(["build_web.sh", "--project"])
            .arg(&project)
            .arg("--no-serve")
            .env("CARGO_TARGET_DIR", &toolchain.target_dir);
        if let Some(bin) = toolchain.bin_dir {
            let mut path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .collect::<Vec<_>>();
            path.insert(0, bin);
            command.env(
                "PATH",
                std::env::join_paths(path).map_err(|e| e.to_string())?,
            );
        }
        let started = Instant::now();
        let output = command.output().map_err(|e| e.to_string())?;
        let log = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(format!(
                "web build failed:\n{}",
                log.lines()
                    .rev()
                    .take(100)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        Ok(
            json!({"ok":true,"directory":output_dir.display().to_string(),"seconds":started.elapsed().as_secs_f64(),"summary":log,"entry":"index.html?example=player"}),
        )
    }
    pub fn new(repo: PathBuf, port: u16) -> Self {
        Self {
            repo,
            port,
            connection: None,
            next_id: 1,
            process: None,
            last_exit: None,
        }
    }

    fn address(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.port))
    }

    fn connect(&mut self) -> Result<&mut Connection, CallError> {
        if self.connection.is_none() {
            let stream = TcpStream::connect_timeout(&self.address(), Duration::from_secs(2))
                .map_err(|err| {
                    let mut message =
                        format!("no engine is listening on 127.0.0.1:{} ({err})", self.port);
                    if let Some(exit) = self.poll_exit() {
                        message.push_str(&format!("; the engine process {exit}"));
                    }
                    CallError::NotConnected(message)
                })?;
            stream
                .set_read_timeout(Some(READ_TIMEOUT))
                .map_err(|err| CallError::NotConnected(err.to_string()))?;
            let _ = stream.set_nodelay(true);
            let writer = stream
                .try_clone()
                .map_err(|err| CallError::NotConnected(err.to_string()))?;
            self.connection = Some(Connection {
                reader: BufReader::new(stream),
                writer,
            });
        }
        Ok(self.connection.as_mut().expect("connected above"))
    }

    pub fn is_listening(&self) -> bool {
        TcpStream::connect_timeout(&self.address(), Duration::from_millis(300)).is_ok()
    }

    /// An external process may have replaced the engine on the same port.
    /// Discard the old socket before attaching, without replaying any mutations.
    pub fn reconnect(&mut self) -> Result<(), CallError> {
        self.connection = None;
        self.connect().map(|_| ())
    }

    /// Send one request and wait for its answer.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = json!({ "id": id, "method": method, "params": params }).to_string();
        line.push('\n');

        // A stale connection fails on write; reconnect once and resend.
        let mut sent = false;
        for _ in 0..2 {
            let connection = self.connect()?;
            if connection.writer.write_all(line.as_bytes()).is_ok()
                && connection.writer.flush().is_ok()
            {
                sent = true;
                break;
            }
            self.connection = None;
        }
        if !sent {
            return Err(CallError::NotConnected(
                "lost the connection to the engine".into(),
            ));
        }

        let mut response = String::new();
        let read = self
            .connection
            .as_mut()
            .expect("connected above")
            .reader
            .read_line(&mut response);
        match read {
            Ok(0) | Err(_) => {
                self.connection = None;
                let mut message = "the engine closed the connection".to_string();
                if let Some(exit) = self.poll_exit() {
                    message.push_str(&format!("; the engine process {exit}"));
                }
                return Err(CallError::NotConnected(message));
            }
            Ok(_) => {}
        }

        let response: Value = serde_json::from_str(&response)
            .map_err(|err| CallError::Engine(format!("invalid response from engine: {err}")))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| error.to_string(), str::to_owned);
            return Err(CallError::Engine(message));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Check whether the managed process has exited, remembering how.
    fn poll_exit(&mut self) -> Option<String> {
        if let Some(process) = &mut self.process {
            if let Ok(Some(status)) = process.child.try_wait() {
                let summary = format!(
                    "'{}' exited with {}",
                    process.target,
                    describe_status(status)
                );
                self.last_exit = Some(summary);
                self.process = None;
                self.connection = None;
            }
        }
        self.last_exit.clone()
    }

    pub fn status(&mut self) -> Value {
        self.poll_exit();
        let listening = self.is_listening();
        let process = self.process.as_ref().map(|process| {
            json!({
                "target": process.target,
                "pid": process.child.id(),
                "uptime_seconds": process.started.elapsed().as_secs(),
                "log_file": process.log_path.display().to_string(),
            })
        });
        json!({
            "port": self.port,
            "listening": listening,
            "managed_process": process,
            "last_exit": self.last_exit,
            "repo": self.repo.display().to_string(),
        })
    }

    /// The last `lines` lines the managed engine wrote to stdout/stderr.
    pub fn output(&mut self, lines: usize, contains: Option<&str>) -> Result<Value, String> {
        self.poll_exit();
        let log_path = self.repo.join("target/engine-mcp/engine.log");
        let all: Vec<String> = match &self.process {
            Some(process) => process
                .output
                .lock()
                .map(|output| output.iter().cloned().collect())
                .unwrap_or_default(),
            // After an exit, the log file still has everything.
            None => fs::read_to_string(&log_path)
                .map_err(|_| "no engine has been launched by this server yet".to_string())?
                .lines()
                .map(str::to_owned)
                .collect(),
        };
        let filtered: Vec<&String> = all
            .iter()
            .filter(|line| contains.is_none_or(|text| line.contains(text)))
            .collect();
        let tail: Vec<&String> = filtered
            .iter()
            .skip(filtered.len().saturating_sub(lines))
            .copied()
            .collect();
        Ok(json!({
            "lines": tail,
            "total_lines": filtered.len(),
            "last_exit": self.last_exit,
            "log_file": log_path.display().to_string(),
        }))
    }

    pub fn stop(&mut self) -> Result<Value, String> {
        self.connection = None;
        let Some(mut process) = self.process.take() else {
            return if self.is_listening() {
                Err(format!(
                    "the engine on port {} was not started by this server; close its window instead",
                    self.port
                ))
            } else {
                Ok(json!({ "stopped": false, "note": "no engine is running" }))
            };
        };
        let _ = process.child.kill();
        let status = process.child.wait().map_err(|err| err.to_string())?;
        self.last_exit = Some(format!(
            "'{}' was stopped ({})",
            process.target,
            describe_status(status)
        ));
        Ok(json!({ "stopped": true, "target": process.target }))
    }

    pub fn has_managed_process(&mut self) -> bool {
        self.poll_exit();
        self.process.is_some()
    }

    /// Find a cargo whose rustc satisfies the engine's `rust-version`.
    pub fn toolchain(&self) -> Result<Toolchain, String> {
        let manifest = fs::read_to_string(self.repo.join("Cargo.toml"))
            .map_err(|err| format!("cannot read {}/Cargo.toml: {err}", self.repo.display()))?;
        let required = manifest
            .lines()
            .find_map(|line| line.strip_prefix("rust-version = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .and_then(parse_version);

        let mut candidates: Vec<(String, Option<PathBuf>)> = Vec::new();
        if let Ok(cargo) = std::env::var("ENGINE_CARGO") {
            let bin = PathBuf::from(cargo).parent().map(Path::to_path_buf);
            candidates.push(("ENGINE_CARGO".into(), bin));
        }
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")));
        if let Some(toolchains) = rustup_home.map(|home| home.join("toolchains")) {
            let mut names: Vec<String> = fs::read_dir(&toolchains)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            // Prefer stable, then nightly, then anything else.
            names.sort_by_key(|name| {
                (
                    !name.starts_with("stable"),
                    !name.starts_with("nightly"),
                    name.clone(),
                )
            });
            for name in names {
                candidates.push((name.clone(), Some(toolchains.join(name).join("bin"))));
            }
        }
        candidates.push(("cargo on PATH".into(), None));

        let mut rejected = Vec::new();
        for (name, bin_dir) in candidates {
            let rustc = bin_dir
                .as_ref()
                .map_or_else(|| PathBuf::from("rustc"), |bin| bin.join("rustc"));
            let Some(version) = Command::new(&rustc)
                .arg("--version")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    let text = String::from_utf8_lossy(&output.stdout).into_owned();
                    text.split_whitespace().nth(1).and_then(parse_version)
                })
            else {
                continue;
            };
            if required.is_some_and(|required| version < required) {
                rejected.push(format!(
                    "{name} ({}.{}.{})",
                    version.0, version.1, version.2
                ));
                continue;
            }
            // Match smoke.sh: nightly builds live apart from the stable build cache.
            let target_dir = if name.starts_with("nightly") {
                self.repo.join("target/nightly")
            } else {
                std::env::var_os("CARGO_TARGET_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.repo.join("target"))
            };
            return Ok(Toolchain {
                description: format!("{name} (rustc {}.{}.{})", version.0, version.1, version.2),
                bin_dir,
                target_dir,
            });
        }
        Err(format!(
            "no Rust toolchain satisfies rust-version {}; rejected: {}",
            required.map_or("?".into(), |(a, b, c)| format!("{a}.{b}.{c}")),
            if rejected.is_empty() {
                "none found".into()
            } else {
                rejected.join(", ")
            }
        ))
    }

    /// Where the binary for `target` is (or would be) built.
    pub fn binary_path(&self, target: &str, release: bool) -> Result<PathBuf, String> {
        let toolchain = self.toolchain()?;
        let mut binary = toolchain
            .target_dir
            .join(if release { "release" } else { "debug" });
        if !matches!(target, "editor" | "player") {
            binary.push("examples");
        }
        binary.push(target);
        Ok(binary)
    }

    /// Build `target` (editor, player or an example). Returns the binary path and a log
    /// excerpt.
    pub fn build(&self, target: &str, release: bool) -> Result<(PathBuf, String), String> {
        let toolchain = self.toolchain()?;
        let cargo = toolchain
            .bin_dir
            .as_ref()
            .map_or_else(|| PathBuf::from("cargo"), |bin| bin.join("cargo"));
        let mut command = Command::new(&cargo);
        command
            .current_dir(&self.repo)
            .env("CARGO_TARGET_DIR", &toolchain.target_dir)
            .args(["build", "--all-features", "--color", "never"]);
        if let Some(bin) = &toolchain.bin_dir {
            let mut path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .collect::<Vec<_>>();
            path.insert(0, bin.clone());
            if let Ok(joined) = std::env::join_paths(path) {
                command.env("PATH", joined);
            }
        }
        if release {
            command.arg("--release");
        }
        let is_example = !matches!(target, "editor" | "player");
        if is_example {
            command.args(["--example", target]);
        } else {
            command.args(["--bin", target]);
        }

        let started = Instant::now();
        let output = command
            .output()
            .map_err(|err| format!("failed to run {}: {err}", cargo.display()))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            let errors: Vec<&str> = stderr
                .lines()
                .filter(|line| !line.trim_start().starts_with("Compiling"))
                .collect();
            let tail = errors[errors.len().saturating_sub(120)..].join("\n");
            return Err(format!(
                "build failed with {} using {}:\n{tail}",
                toolchain.description,
                describe_status(output.status)
            ));
        }

        let binary = self.binary_path(target, release)?;
        let warnings = stderr.matches("warning:").count();
        Ok((
            binary,
            format!(
                "built {target} with {} in {:.1}s ({warnings} warning lines)",
                toolchain.description,
                started.elapsed().as_secs_f32()
            ),
        ))
    }

    /// Start `binary` with remote control enabled and wait until it accepts connections.
    pub fn spawn(
        &mut self,
        target: &str,
        binary: &Path,
        args: &[String],
        monitor: Option<&str>,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<Value, String> {
        if !binary.exists() {
            return Err(format!(
                "{} does not exist; launch with build: true",
                binary.display()
            ));
        }
        self.connection = None;
        self.last_exit = None;

        let log_dir = self.repo.join("target/engine-mcp");
        fs::create_dir_all(&log_dir).map_err(|err| err.to_string())?;
        let log_path = log_dir.join("engine.log");
        let log_file = Arc::new(Mutex::new(
            File::create(&log_path).map_err(|err| err.to_string())?,
        ));

        let mut command = Command::new(binary);
        command
            .env(
                "RUST_LOG",
                std::env::var("ENGINE_RUST_LOG").unwrap_or_else(|_| "info".into()),
            )
            .env("RUST_BACKTRACE", "1")
            .envs(env)
            .env("ENGINE_REMOTE_PORT", self.port.to_string());
        if let Some(monitor) = monitor {
            // Read by the engine's RenderSettings; overrides settings.json.
            command.env("ENGINE_MONITOR", monitor);
        }
        let mut child = command
            .args(args)
            .current_dir(&self.repo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start {}: {err}", binary.display()))?;

        let output = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_LINES)));
        let pipes: [Box<dyn std::io::Read + Send>; 2] = [
            Box::new(child.stdout.take().expect("stdout piped")),
            Box::new(child.stderr.take().expect("stderr piped")),
        ];
        for pipe in pipes {
            let output = Arc::clone(&output);
            let log_file = Arc::clone(&log_file);
            thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if let Ok(mut file) = log_file.lock() {
                        let _ = writeln!(file, "{line}");
                    }
                    if let Ok(mut output) = output.lock() {
                        if output.len() == OUTPUT_LINES {
                            output.pop_front();
                        }
                        output.push_back(line);
                    }
                }
            });
        }

        self.process = Some(ManagedProcess {
            child,
            target: target.to_owned(),
            started: Instant::now(),
            output,
            log_path: log_path.clone(),
        });

        let deadline = Instant::now() + timeout;
        loop {
            if let Some(exit) = self.poll_exit() {
                let tail = self
                    .output(60, None)
                    .ok()
                    .and_then(|value| value.get("lines").cloned())
                    .unwrap_or_default();
                return Err(format!(
                    "the engine {exit} before accepting connections. Last output:\n{}",
                    tail.as_array()
                        .map(|lines| lines
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n"))
                        .unwrap_or_default()
                ));
            }
            if self.is_listening() {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "the engine did not open port {} within {}s; see engine_output",
                    self.port,
                    timeout.as_secs()
                ));
            }
            thread::sleep(Duration::from_millis(200));
        }

        Ok(json!({
            "target": target,
            "pid": self.process.as_ref().map(|process| process.child.id()),
            "log_file": log_path.display().to_string(),
        }))
    }
}

/// Validate child environment overrides before building or launching a window.
pub fn launch_environment(overrides: Option<&Value>) -> Result<BTreeMap<String, String>, String> {
    let env: BTreeMap<String, String> = match overrides {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "env must be an object with string values".to_string())?,
        None => BTreeMap::new(),
    };
    for (key, value) in &env {
        if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
            return Err("env contains an invalid environment name or value".into());
        }
    }
    #[cfg(target_os = "linux")]
    validate_desktop_environment(|key| env.get(key).cloned().or_else(|| std::env::var(key).ok()))?;
    Ok(env)
}

#[cfg(target_os = "linux")]
fn validate_desktop_environment(get: impl Fn(&str) -> Option<String>) -> Result<(), String> {
    if ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|key| get(key).is_some_and(|value| !value.is_empty()))
    {
        Ok(())
    } else {
        Err("no desktop session is configured: DISPLAY and WAYLAND_DISPLAY are empty or unset. Pass engine_launch env (for example {\"DISPLAY\":\":0\"}, or WAYLAND_DISPLAY and XDG_RUNTIME_DIR), or start the editor from a desktop terminal and attach. No build was started.".into())
    }
}

fn describe_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(signal) = status.signal() {
                    return format!("signal {signal}");
                }
            }
            "an unknown status".into()
        }
    }
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let core = text.split('-').next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_discards_socket_from_replaced_engine() {
        use std::net::TcpListener;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = thread::spawn(move || {
            for generation in [1, 2] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                assert_eq!(request["method"], "engine.info");
                writeln!(
                    stream,
                    "{}",
                    json!({"id":request["id"],"result":{"generation":generation}})
                )
                .unwrap();
            }
        });
        let mut engine = Engine::new(PathBuf::new(), port);
        assert_eq!(
            engine.call("engine.info", json!({})).unwrap()["generation"],
            1
        );
        engine.reconnect().unwrap();
        assert_eq!(
            engine.call("engine.info", json!({})).unwrap()["generation"],
            2
        );
        peer.join().unwrap();
    }

    #[test]
    fn launch_environment_validates_overrides() {
        assert_eq!(
            launch_environment(Some(&json!({"DISPLAY":":99", "ENGINE_TEST":"yes"}))).unwrap()
                ["ENGINE_TEST"],
            "yes"
        );
        for invalid in [
            json!([]),
            json!({"DISPLAY": 1}),
            json!({"bad=name":"x"}),
            json!({"":"x"}),
            json!({"DISPLAY":"x\0"}),
        ] {
            assert!(launch_environment(Some(&invalid)).is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_desktop_fails_preflight() {
        assert!(validate_desktop_environment(|_| None).is_err());
        assert!(validate_desktop_environment(|_| Some(String::new())).is_err());
        assert!(validate_desktop_environment(
            |key| (key == "WAYLAND_DISPLAY").then(|| "wayland-0".into())
        )
        .is_ok());
        assert!(
            validate_desktop_environment(|key| (key == "DISPLAY").then(|| ":0".into())).is_ok()
        );
    }

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("1.95"), Some((1, 95, 0)));
        assert_eq!(parse_version("1.97.0-nightly"), Some((1, 97, 0)));
        assert!(parse_version("1.94.1").unwrap() < parse_version("1.95").unwrap());
        assert_eq!(parse_version("garbage"), None);
    }
}
