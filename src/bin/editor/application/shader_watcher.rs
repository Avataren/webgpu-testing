#[cfg(not(target_arch = "wasm32"))]
use notify::{recommended_watcher, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[cfg(not(target_arch = "wasm32"))]
use log::warn;

/// Watches the active project's content directory for WGSL shader updates on native builds.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct ShaderWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<notify::Event>>,
    watched_root: Option<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ShaderWatcher {
    /// Creates a new shader watcher without an active root.
    pub(super) fn new() -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let watcher = recommended_watcher(move |res| {
            let _ = sender.send(res);
        })?;

        Ok(Self {
            watcher,
            receiver,
            watched_root: None,
        })
    }

    /// Ensures the watcher is monitoring the provided root directory.
    pub(super) fn watch_root(&mut self, root: &Path) -> notify::Result<()> {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        if self
            .watched_root
            .as_ref()
            .map(|current| current == &canonical)
            .unwrap_or(false)
        {
            return Ok(());
        }

        self.clear();
        self.watcher.watch(&canonical, RecursiveMode::Recursive)?;
        self.watched_root = Some(canonical);
        Ok(())
    }

    /// Stops watching the current root, if any.
    pub(super) fn clear(&mut self) {
        if let Some(root) = self.watched_root.take() {
            if let Err(err) = self.watcher.unwatch(&root) {
                warn!("Failed to unwatch shader directory {:?}: {err}", root);
            }
        }
    }

    /// Polls for file events and returns the set of WGSL files that changed.
    pub(super) fn poll(&mut self) -> Vec<PathBuf> {
        let mut changed = HashSet::new();

        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            for path in event.paths {
                                if path
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .map_or(false, |ext| ext.eq_ignore_ascii_case("wgsl"))
                                    && path.exists()
                                {
                                    changed.insert(path);
                                }
                            }
                        }
                        EventKind::Remove(_) => {
                            // Ignore removals; without file contents we cannot perform hot reload.
                        }
                        _ => {}
                    }
                }
                Ok(Err(err)) => {
                    warn!("Shader watcher error: {err}");
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("Shader watcher disconnected");
                    break;
                }
            }
        }

        changed.into_iter().collect()
    }
}
