//! WGSL sources for the renderer's passes.
//!
//! Every shader in `src/shader` is built into the binary, and [`wgsl!`] returns that text
//! unless a reload replaced it. In a native build whose source tree is still on disk (a
//! development build run from the repository) the files can be read again while the
//! engine runs: [`Renderer::reload_shaders`](crate::renderer::Renderer::reload_shaders)
//! stages the disk text here, rebuilds the passes that can be rebuilt, and puts the
//! previous text back if any of them fails, so a broken edit never replaces a working
//! pipeline. Nothing is read from disk until the first reload, so startup always uses the
//! shaders the binary was built with.
//!
//! `ENGINE_SHADER_DIR` points the reload at another directory; `ENGINE_SHADER_RELOAD=0`
//! turns watching off.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant, SystemTime};

/// The text of `src/shader/<path>`: built in, or as last reloaded from disk.
macro_rules! wgsl {
    ($path:literal) => {
        $crate::renderer::shader_source::source(
            $path,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shader/", $path)),
        )
    };
}
pub(crate) use wgsl;

/// Text that replaced the built-in source, by path relative to the shader directory.
/// Leaked: a reload keeps its text for the life of the process, which is what lets
/// [`ShaderBuilder`](crate::renderer::ShaderBuilder) keep holding `&'static str` modules.
/// Only files whose text changed are leaked again.
static OVERRIDES: RwLock<Option<HashMap<String, &'static str>>> = RwLock::new(None);
/// The built-in text of each file asked for so far, to tell real changes apart.
static BUILTIN: Mutex<Option<HashMap<&'static str, &'static str>>> = Mutex::new(None);

thread_local! {
    /// Files asked for while a reload rebuilds the passes.
    static TOUCHED: RefCell<Option<BTreeSet<String>>> = const { RefCell::new(None) };
}

#[doc(hidden)]
pub fn source(path: &'static str, builtin: &'static str) -> &'static str {
    TOUCHED.with(|touched| {
        if let Some(touched) = touched.borrow_mut().as_mut() {
            touched.insert(path.to_owned());
        }
    });
    if let Ok(mut known) = BUILTIN.lock() {
        known.get_or_insert_with(HashMap::new).insert(path, builtin);
    }
    OVERRIDES
        .read()
        .ok()
        .and_then(|overrides| overrides.as_ref()?.get(path).copied())
        .unwrap_or(builtin)
}

/// Check composed WGSL with naga, returning its diagnostic (with the offending lines).
pub(crate) fn validate(source: &str) -> Result<(), String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let module =
        naga::front::wgsl::parse_str(source).map_err(|error| error.emit_to_string(source))?;
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map(|_| ())
        .map_err(|error| error.emit_to_string(source))
}

/// The directory shaders reload from, when reloading is possible here.
pub fn shader_dir() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = std::env::var_os("ENGINE_SHADER_DIR") {
            return Some(PathBuf::from(dir));
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("shader");
        dir.is_dir().then_some(dir)
    }
}

/// A reload's new text, staged until the rebuilt passes are known to work.
pub(crate) struct Staged {
    previous: Option<HashMap<String, &'static str>>,
    /// Files whose text differs from what was in effect.
    pub changed: Vec<String>,
}

/// Read every `.wgsl` file under the shader directory and make it the text [`wgsl!`]
/// returns, recording which passes ask for which files from now on. Undo with
/// [`restore`], or keep with [`finish`].
pub(crate) fn stage_from_disk() -> Result<Staged, String> {
    let dir = shader_dir().ok_or(
        "shader reloading needs the engine's source tree (src/shader) on disk, or \
         ENGINE_SHADER_DIR; this build only has its built-in shaders",
    )?;
    let mut files = Vec::new();
    collect_wgsl(&dir, &dir, &mut files)
        .map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
    if files.is_empty() {
        return Err(format!("no .wgsl files under {}", dir.display()));
    }

    let mut overrides = OVERRIDES
        .write()
        .map_err(|_| "shader sources are poisoned")?;
    let previous = overrides.clone();
    let builtin = BUILTIN.lock().ok().and_then(|known| known.clone());
    let mut next = previous.clone().unwrap_or_default();
    let mut changed = Vec::new();
    for (relative, text) in files {
        let current = next.get(&relative).copied().or_else(|| {
            builtin
                .as_ref()
                .and_then(|known| known.get(relative.as_str()).copied())
        });
        if current == Some(text.as_str()) {
            continue;
        }
        // A file nothing has asked for yet has no known built-in text; it only counts as
        // changed once something uses it.
        if current.is_some() {
            changed.push(relative.clone());
        }
        next.insert(relative, Box::leak(text.into_boxed_str()));
    }
    changed.sort();
    *overrides = Some(next);
    drop(overrides);
    TOUCHED.with(|touched| *touched.borrow_mut() = Some(BTreeSet::new()));
    Ok(Staged { previous, changed })
}

/// Put back the text that was in effect before `staged`.
pub(crate) fn restore(staged: Staged) {
    TOUCHED.with(|touched| touched.borrow_mut().take());
    if let Ok(mut overrides) = OVERRIDES.write() {
        *overrides = staged.previous;
    }
}

/// Keep the staged text; returns the files the rebuilt passes asked for.
pub(crate) fn finish() -> BTreeSet<String> {
    TOUCHED
        .with(|touched| touched.borrow_mut().take())
        .unwrap_or_default()
}

fn collect_wgsl(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_wgsl(root, &path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((relative, std::fs::read_to_string(&path)?));
        }
    }
    Ok(())
}

/// Log what a reload did: the files it applied, and those that need a restart; or the
/// compiler's message when it was refused.
pub fn log_reload(result: &Result<crate::renderer::ShaderReload, String>) {
    match result {
        Ok(reload) if reload.changed.is_empty() => {}
        Ok(reload) => {
            log::info!(target: "shader", "Reloaded shaders: {}", reload.changed.join(", "));
            if !reload.not_reloaded.is_empty() {
                log::warn!(
                    target: "shader",
                    "Changed, but used only by passes that are not rebuilt (restart to apply): {}",
                    reload.not_reloaded.join(", ")
                );
            }
        }
        Err(error) => log::error!(
            target: "shader",
            "Shader reload failed; the previous shaders stay in use:\n{error}"
        ),
    }
}

/// Notices edits to the shader directory, by modification time, at most a few times a
/// second.
#[cfg(not(target_arch = "wasm32"))]
pub struct ShaderWatcher {
    dir: PathBuf,
    stamps: HashMap<PathBuf, SystemTime>,
    next_poll: Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl ShaderWatcher {
    const INTERVAL: Duration = Duration::from_millis(400);

    /// A watcher, when shaders can be reloaded here and watching is not turned off.
    pub fn new() -> Option<Self> {
        if std::env::var("ENGINE_SHADER_RELOAD").is_ok_and(|value| value == "0") {
            return None;
        }
        let dir = shader_dir()?;
        let mut watcher = Self {
            stamps: HashMap::new(),
            dir,
            next_poll: Instant::now(),
        };
        watcher.stamps = watcher.scan();
        log::info!(
            target: "shader",
            "Watching {} for shader edits (ENGINE_SHADER_RELOAD=0 turns this off)",
            watcher.dir.display()
        );
        Some(watcher)
    }

    /// Whether a shader file was added, removed or saved since the last call.
    pub fn poll(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next_poll {
            return false;
        }
        self.next_poll = now + Self::INTERVAL;
        let stamps = self.scan();
        let changed = stamps != self.stamps;
        self.stamps = stamps;
        changed
    }

    fn scan(&self) -> HashMap<PathBuf, SystemTime> {
        let mut stamps = HashMap::new();
        let mut pending = vec![self.dir.clone()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "wgsl") {
                    if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                        stamps.insert(path, modified);
                    }
                }
            }
        }
        stamps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_text_is_returned_and_records_what_was_asked_for() {
        let text = wgsl!("postprocess/fullscreen.wgsl");
        assert!(text.contains("vs_fullscreen"));
        assert!(BUILTIN
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains_key("postprocess/fullscreen.wgsl"));
    }
}
