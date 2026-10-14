use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub enum Event {
    Info { generation: u64, text: String },
    Best { generation: u64, uci: String },
    Error { generation: u64, message: String },
}

struct Search {
    generation: u64,
    moves: Vec<String>,
    millis: u64,
}
pub struct Engine {
    requests: Option<Sender<Search>>,
    pub events: Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Engine {
    pub fn new(path: PathBuf) -> Self {
        let (requests, incoming) = mpsc::channel::<Search>();
        let (events, outgoing) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut session: Option<Session> = None;
            let mut previous = None;
            while let Ok(request) = incoming.recv() {
                let result = (|| {
                    if session.is_none() {
                        session = Some(Session::spawn(&path)?);
                        previous = None;
                    }
                    let s = session.as_mut().unwrap();
                    if previous != Some(request.generation) {
                        s.send("ucinewgame\nisready")?;
                        s.until("readyok")?;
                        previous = Some(request.generation);
                    }
                    s.send(&format!(
                        "position startpos moves {}\ngo movetime {}",
                        request.moves.join(" "),
                        request.millis
                    ))?;
                    let deadline = Instant::now() + Duration::from_secs(15);
                    loop {
                        let line = s.line(deadline)?;
                        if let Some(rest) = line.strip_prefix("bestmove ") {
                            let uci = rest
                                .split_whitespace()
                                .next()
                                .ok_or("Missing bestmove")?
                                .to_owned();
                            let _ = events.send(Event::Best {
                                generation: request.generation,
                                uci,
                            });
                            return Ok::<(), String>(());
                        }
                        if line.starts_with("info depth ") {
                            let _ = events.send(Event::Info {
                                generation: request.generation,
                                text: line,
                            });
                        }
                    }
                })();
                if let Err(message) = result {
                    let _ = events.send(Event::Error {
                        generation: request.generation,
                        message,
                    });
                    session = None;
                    previous = None;
                }
            }
        });
        Self {
            requests: Some(requests),
            events: outgoing,
            worker: Some(worker),
        }
    }

    pub fn search(&self, generation: u64, moves: &[String], millis: u64) -> Result<(), String> {
        self.requests
            .as_ref()
            .ok_or("Engine stopped")?
            .send(Search {
                generation,
                moves: moves.to_vec(),
                millis,
            })
            .map_err(|_| "Engine worker exited".to_owned())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Result<String, String>>,
}

// One identical strength configuration serves both colors. Root noise must stay
// disabled: its alternative move selection can promote bounded PVS scores over
// the true best move, including moves that drop attacked material.
const STRONG_OPTIONS: &str = "setoption name Threads value 16\nsetoption name Hash value 128\nsetoption name Ponder value false\nsetoption name NeuralEval value false\nsetoption name DirichletAlpha value 0.0\nisready";

impl Session {
    fn spawn(path: &Path) -> Result<Self, String> {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let stdin = child.stdin.take().ok_or("No engine stdin")?;
        let stdout = child.stdout.take().ok_or("No engine stdout")?;
        let stderr = child.stderr.take().ok_or("No engine stderr")?;
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.map_err(|e| e.to_string())).is_err() {
                    return;
                }
            }
            let _ = tx.send(Err("Engine exited".to_owned()));
        });
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log::debug!("XavChess: {line}");
            }
        });
        let mut s = Self {
            child,
            stdin,
            lines,
        };
        s.send("uci")?;
        s.until("uciok")?;
        s.send(STRONG_OPTIONS)?;
        s.until("readyok")?;
        Ok(s)
    }
    fn send(&mut self, text: &str) -> Result<(), String> {
        log::debug!("UCI send: {text}");
        writeln!(self.stdin, "{text}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| e.to_string())
    }
    fn line(&self, deadline: Instant) -> Result<String, String> {
        let line = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("UCI response: {e}"))??;
        log::debug!("UCI received: {line}");
        Ok(line)
    }
    fn until(&self, token: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self.line(deadline)? == token {
                return Ok(());
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.send("stop\nquit");
        // Reap the process even when the engine crashes or stops responding.
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Game;

    #[test]
    fn full_strength_search_saves_attacked_queens_for_both_colors() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/chess-uci/release/xavchess-hce");
        let mut session = Session::spawn(&path).unwrap();
        // Mirror the exact same threat for each color. Both must move their queen,
        // rather than choosing a quiet king move and surrendering the queen.
        for (fen, source) in [
            ("6k1/8/6p1/7Q/8/8/8/6K1 w - - 0 1", "h5"),
            ("6k1/8/8/8/7q/6P1/8/6K1 b - - 0 1", "h4"),
        ] {
            session.send("ucinewgame\nisready").unwrap();
            session.until("readyok").unwrap();
            session
                .send(&format!("position fen {fen}\ngo movetime 1000"))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let line = session.line(deadline).unwrap();
                if let Some(best) = line.strip_prefix("bestmove ") {
                    assert!(
                        best.starts_with(source),
                        "Queen left hanging in {fen}: {line}"
                    );
                    break;
                }
            }
        }
    }

    #[test]
    fn full_strength_search_finds_free_material_for_both_colors() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/chess-uci/release/xavchess-hce");
        let mut session = Session::spawn(&path).unwrap();
        for (fen, expected) in [
            ("r3k3/8/8/8/8/2N5/8/Q3K3 w - - 0 1", "a1a8"),
            ("q3k3/8/2n5/8/8/8/8/R3K3 b - - 0 1", "a8a1"),
        ] {
            session.send("ucinewgame\nisready").unwrap();
            session.until("readyok").unwrap();
            session
                .send(&format!("position fen {fen}\ngo movetime 1000"))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let line = session.line(deadline).unwrap();
                if let Some(best) = line.strip_prefix("bestmove ") {
                    assert_eq!(
                        best.split_whitespace().next().unwrap(),
                        expected,
                        "Failed to take free rook in {fen}"
                    );
                    break;
                }
            }
        }
    }

    #[test]
    fn actual_hce_uci_search_and_generation_barriers() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/chess-uci/release/xavchess-hce");
        assert!(
            path.exists(),
            "Build the HCE UCI target before running tests"
        );
        let engine = Engine::new(path);
        let mut game = Game::new();
        for _ in 0..12 {
            engine.search(1, &game.history, 25).unwrap();
            loop {
                match engine.events.recv_timeout(Duration::from_secs(20)).unwrap() {
                    Event::Best { generation, uci } => {
                        assert_eq!(generation, 1);
                        game.apply(&uci).unwrap();
                        break;
                    }
                    Event::Error { message, .. } => panic!("{message}"),
                    Event::Info { .. } => {}
                }
            }
        }
        // Queue a new game while a search in the old game is still outstanding.
        engine.search(1, &game.history, 80).unwrap();
        engine.search(2, &[], 25).unwrap();
        let mut generations = Vec::new();
        while generations.len() < 2 {
            match engine.events.recv_timeout(Duration::from_secs(20)).unwrap() {
                Event::Best { generation, uci } => {
                    generations.push(generation);
                    if generation == 2 {
                        Game::new().apply(&uci).unwrap();
                    }
                }
                Event::Error { message, .. } => panic!("{message}"),
                Event::Info { .. } => {}
            }
        }
        assert_eq!(generations, [1, 2]);
        // Engine::drop joins its worker and reaps the child process.
    }
}
