#[cfg(not(target_arch = "wasm32"))]
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
use log::warn;

/// Watches for Lua script (.lua) file updates on native builds.
/// Supports debouncing to avoid triggering multiple reloads for rapid saves.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct ScriptWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<notify::Event>>,
    watched_roots: Vec<PathBuf>,
    last_change_time: std::collections::HashMap<PathBuf, Instant>,
    debounce_duration: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
impl ScriptWatcher {
    /// Creates a new script watcher without active roots.
    pub(super) fn new() -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let watcher = recommended_watcher(move |res| {
            let _ = sender.send(res);
        })?;

        Ok(Self {
            watcher,
            receiver,
            watched_roots: Vec::new(),
            last_change_time: std::collections::HashMap::new(),
            debounce_duration: Duration::from_millis(500), // 500ms debounce
        })
    }

    /// Ensures the watcher is monitoring the provided root directories.
    /// Typically called with both "examples/scripts/" and "scripts/" (if exists).
    pub(super) fn watch_roots(&mut self, roots: &[PathBuf]) -> notify::Result<()> {
        // Check if roots have changed
        if self.watched_roots.len() == roots.len()
            && self
                .watched_roots
                .iter()
                .zip(roots.iter())
                .all(|(a, b)| a == b)
        {
            return Ok(());
        }

        // Clear old watches
        self.clear();

        // Watch new roots
        for root in roots {
            if root.exists() {
                let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
                match self.watcher.watch(&canonical, RecursiveMode::NonRecursive) {
                    Ok(_) => {
                        self.watched_roots.push(canonical);
                    }
                    Err(err) => {
                        warn!(
                            "Failed to watch script directory {:?}: {}",
                            root.display(),
                            err
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Stops watching all current roots.
    pub(super) fn clear(&mut self) {
        for root in self.watched_roots.drain(..) {
            if let Err(err) = self.watcher.unwatch(&root) {
                warn!("Failed to unwatch script directory {:?}: {err}", root);
            }
        }
        self.last_change_time.clear();
    }

    /// Polls for file events and returns the set of .lua files that changed.
    /// Applies debouncing to avoid rapid reload cycles.
    pub(super) fn poll(&mut self) -> Vec<PathBuf> {
        let mut changed = HashSet::new();
        let now = Instant::now();

        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            for path in event.paths {
                                // Only process .lua files
                                if path
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
                                    && path.exists()
                                {
                                    // Check debounce
                                    if let Some(last_change) = self.last_change_time.get(&path) {
                                        if now.duration_since(*last_change) < self.debounce_duration
                                        {
                                            // Too soon, skip this change
                                            continue;
                                        }
                                    }

                                    // Record this change
                                    self.last_change_time.insert(path.clone(), now);
                                    changed.insert(path);
                                }
                            }
                        }
                        EventKind::Remove(_) => {
                            // Script was deleted - we could handle this by closing the plugin
                            // For now, ignore removals
                        }
                        _ => {}
                    }
                }
                Ok(Err(err)) => {
                    warn!("Script watcher error: {err}");
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("Script watcher disconnected");
                    break;
                }
            }
        }

        changed.into_iter().collect()
    }

    /// Sets the debounce duration (how long to wait after a change before allowing another reload).
    #[allow(dead_code)]
    pub(super) fn set_debounce_duration(&mut self, duration: Duration) {
        self.debounce_duration = duration;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ScriptWatcher {
    fn drop(&mut self) {
        self.clear();
    }
}

// Stub for WASM builds
#[cfg(target_arch = "wasm32")]
pub(super) struct ScriptWatcher;

#[cfg(target_arch = "wasm32")]
impl ScriptWatcher {
    pub(super) fn new() -> notify::Result<Self> {
        Ok(Self)
    }

    pub(super) fn watch_roots(&mut self, _roots: &[PathBuf]) -> notify::Result<()> {
        Ok(())
    }

    pub(super) fn clear(&mut self) {}

    pub(super) fn poll(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
}
