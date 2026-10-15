//! Child processes for game scripts: start a program, write lines to its stdin and
//! collect the lines it prints, without ever blocking a frame. Made for line protocols
//! such as UCI chess engines.
//!
//! Only programs inside the working directory or the open project can be started, with
//! no shell in between. Every process is killed when play mode stops (see
//! [`kill_all`]). The web has no processes: [`spawn`] fails there.

use std::path::PathBuf;

/// The most lines [`read_lines`] returns per call; the rest wait for the next call.
pub const MAX_LINES_PER_READ: usize = 4096;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
    use std::thread;

    struct Process {
        child: Child,
        stdin: Option<ChildStdin>,
        lines: Receiver<String>,
    }

    impl Drop for Process {
        fn drop(&mut self) {
            self.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[derive(Default)]
    struct Registry {
        next: u64,
        processes: HashMap<u64, Process>,
    }

    fn registry() -> MutexGuard<'static, Registry> {
        static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
        REGISTRY
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn spawn(path: &str, args: &[String]) -> Result<u64, String> {
        let program = resolve_program(path)?;
        let mut child = Command::new(&program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("{}: {err}", program.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("the process has no stdout")?;
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        let mut registry = registry();
        registry.next += 1;
        let id = registry.next;
        registry.processes.insert(
            id,
            Process {
                child,
                stdin,
                lines,
            },
        );
        Ok(id)
    }

    pub fn write_line(id: u64, line: &str) -> bool {
        let mut registry = registry();
        let Some(process) = registry.processes.get_mut(&id) else {
            return false;
        };
        let Some(stdin) = process.stdin.as_mut() else {
            return false;
        };
        let ok = writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .is_ok();
        if !ok {
            process.stdin = None;
        }
        ok
    }

    pub fn read_lines(id: u64) -> Vec<String> {
        let registry = registry();
        let Some(process) = registry.processes.get(&id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while out.len() < MAX_LINES_PER_READ {
            match process.lines.try_recv() {
                Ok(line) => out.push(line),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        out
    }

    pub fn running(id: u64) -> bool {
        let mut registry = registry();
        registry
            .processes
            .get_mut(&id)
            .is_some_and(|process| matches!(process.child.try_wait(), Ok(None)))
    }

    pub fn kill(id: u64) -> bool {
        registry().processes.remove(&id).is_some()
    }

    pub fn kill_all() {
        let processes = std::mem::take(&mut registry().processes);
        drop(processes);
    }

    pub fn count() -> usize {
        registry().processes.len()
    }
}

#[cfg(target_arch = "wasm32")]
mod native {
    pub fn spawn(_path: &str, _args: &[String]) -> Result<u64, String> {
        Err("processes are not available on the web".to_owned())
    }
    pub fn write_line(_id: u64, _line: &str) -> bool {
        false
    }
    pub fn read_lines(_id: u64) -> Vec<String> {
        Vec::new()
    }
    pub fn running(_id: u64) -> bool {
        false
    }
    pub fn kill(_id: u64) -> bool {
        false
    }
    pub fn kill_all() {}
    pub fn count() -> usize {
        0
    }
}

/// Start `path` with `args` and return its id. Relative paths are looked up in the
/// working directory, then the open project and its content folder; the program must
/// be inside one of those (no system programs, no shell).
pub fn spawn(path: &str, args: &[String]) -> Result<u64, String> {
    native::spawn(path, args)
}

/// Write `line` and a newline to the process's stdin. False once it is gone.
pub fn write_line(id: u64, line: &str) -> bool {
    native::write_line(id, line)
}

/// The lines the process printed since the last call (up to [`MAX_LINES_PER_READ`]).
pub fn read_lines(id: u64) -> Vec<String> {
    native::read_lines(id)
}

/// Whether the process is still running.
pub fn running(id: u64) -> bool {
    native::running(id)
}

/// Kill the process and forget it. False if there was no such process.
pub fn kill(id: u64) -> bool {
    native::kill(id)
}

/// Kill every process scripts started (play mode stopped).
pub fn kill_all() {
    // Other tests stop play mode while the process tests run; let those finish first.
    #[cfg(test)]
    let _guard = tests::GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    native::kill_all()
}

/// How many processes are open.
pub fn count() -> usize {
    native::count()
}

/// Find the program and check that it lives in the working directory or the project.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn resolve_program(path: &str) -> Result<PathBuf, String> {
    let raw = std::path::Path::new(path);
    let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
    let project = crate::project::active_project_root();
    let mut candidates = vec![if raw.is_relative() {
        cwd.join(raw)
    } else {
        raw.to_path_buf()
    }];
    if raw.is_relative() {
        if let Some(root) = &project {
            candidates.push(root.join(raw));
            candidates.push(root.join(crate::project::CONTENT_DIR).join(raw));
        }
    }
    let found = candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("program not found: {path}"))?;
    let found = found.canonicalize().map_err(|err| err.to_string())?;
    let allowed: Vec<PathBuf> = [Some(cwd), project]
        .into_iter()
        .flatten()
        .filter_map(|root| root.canonicalize().ok())
        .collect();
    if !allowed.iter().any(|root| found.starts_with(root)) {
        return Err(format!(
            "{} is outside the working directory and the project",
            found.display()
        ));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use std::time::{Duration, Instant};

    /// Held by the process tests; `kill_all` waits for it.
    pub(super) static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(unix)]
    #[test]
    fn lines_round_trip_through_a_child_and_kill_cleans_up() {
        let _guard = guard();
        // `cat` lives outside the working directory, so copy it in first.
        let dir = std::env::current_dir().unwrap().join("target/process-test");
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("echo-lines");
        std::fs::copy("/bin/cat", &program).unwrap();
        let id = spawn(program.to_str().unwrap(), &[]).unwrap();
        assert!(running(id));
        assert!(write_line(id, "uci"));
        assert!(write_line(id, "isready"));
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut lines = Vec::new();
        while lines.len() < 2 && Instant::now() < deadline {
            lines.extend(read_lines(id));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(lines, ["uci", "isready"]);
        assert!(kill(id));
        assert!(!running(id));
        assert!(!write_line(id, "quit"));
        assert!(!kill(id));
    }

    #[cfg(unix)]
    #[test]
    fn programs_outside_the_working_directory_are_refused() {
        let err = spawn("/bin/cat", &[]).unwrap_err();
        assert!(err.contains("outside"), "{err}");
        assert!(spawn("no/such/program", &[])
            .unwrap_err()
            .contains("not found"));
    }
}
