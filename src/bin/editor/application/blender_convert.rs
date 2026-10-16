//! Models the engine cannot read itself (FBX, .blend, OBJ) are converted to GLB with
//! Blender, the way Godot imports .blend files: the editor runs Blender in the
//! background with `tools/blender/convert_to_gltf.py` (built into the editor) and imports
//! the GLB it writes. A file without meshes (Mixamo's "Without Skin" animations), or one
//! imported as animations, becomes a clip library instead.
//!
//! Blender is found through `BLENDER_PATH`, the `PATH`, or its usual install places
//! (Steam's included).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const SCRIPT: &str = include_str!("../../../../tools/blender/convert_to_gltf.py");

/// Extensions converted through Blender.
pub(super) const CONVERTIBLE: [&str; 3] = ["fbx", "blend", "obj"];

pub(super) fn is_convertible(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| CONVERTIBLE.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The Blender executable: `$BLENDER_PATH`, `blender` on the `PATH`, or a usual install.
pub(super) fn find_blender() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BLENDER_PATH").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let executable = if cfg!(windows) {
        "blender.exe"
    } else {
        "blender"
    };
    if let Some(paths) = std::env::var_os("PATH") {
        if let Some(found) = std::env::split_paths(&paths)
            .map(|dir| dir.join(executable))
            .find(|candidate| candidate.is_file())
        {
            return Some(found);
        }
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut candidates = vec![
        home.join(".local/share/Steam/steamapps/common/Blender/blender"),
        home.join(".steam/steam/steamapps/common/Blender/blender"),
        PathBuf::from("/opt/blender/blender"),
        PathBuf::from("/snap/bin/blender"),
        PathBuf::from("/var/lib/flatpak/exports/bin/org.blender.Blender"),
        home.join(".local/share/flatpak/exports/bin/org.blender.Blender"),
        PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender"),
        PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\Blender\blender.exe"),
    ];
    // Windows installs are versioned: "Blender Foundation\Blender 4.2\blender.exe".
    if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\Blender Foundation") {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path().join("blender.exe"))
            .collect();
        versions.sort();
        candidates.extend(versions.into_iter().rev());
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// What a finished conversion produced.
pub(super) enum Converted {
    /// A GLB with meshes, to import as a model.
    Model(PathBuf),
    /// A GLB with only animations, to add as a clip library.
    Clips(PathBuf),
    Failed(String),
}

/// A Blender process converting one file.
pub(super) struct Conversion {
    pub source: PathBuf,
    /// For animations: the entity whose animator gets them once imported.
    pub attach: Option<hecs::Entity>,
    child: Child,
    output: PathBuf,
    log: PathBuf,
}

impl Conversion {
    /// Start converting `source` in the background. `clips_for` imports only its armature
    /// and animations, for the animator of the entity it holds (if any).
    pub(super) fn start(
        source: &Path,
        clips_for: Option<Option<hecs::Entity>>,
    ) -> Result<Self, String> {
        let animations_only = clips_for.is_some();
        let blender = find_blender().ok_or(
            "FBX, .blend and OBJ files are converted with Blender, which was not found: \
             install it, or set BLENDER_PATH to its executable",
        )?;
        let stem = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .ok_or("the file has no name")?;
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_nanos())
                .unwrap_or_default()
        );
        let dir = std::env::temp_dir().join("wgpu-cube-import").join(unique);
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        let script = dir.join("convert_to_gltf.py");
        std::fs::write(&script, SCRIPT)
            .map_err(|err| format!("cannot write {}: {err}", script.display()))?;
        let output = dir.join(format!("{stem}.glb"));
        let log = dir.join("blender.log");
        let log_file =
            File::create(&log).map_err(|err| format!("cannot create {}: {err}", log.display()))?;
        let mut command = Command::new(&blender);
        command
            .args(["-b", "--factory-startup", "--python"])
            .arg(&script)
            .arg("--")
            .arg(source)
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(
                log_file
                    .try_clone()
                    .map_err(|err| format!("cannot log Blender: {err}"))?,
            )
            .stderr(log_file);
        if animations_only {
            command.arg("--animations-only");
        }
        let child = command
            .spawn()
            .map_err(|err| format!("cannot run {}: {err}", blender.display()))?;
        log::info!(
            "Converting {} with Blender ({})",
            source.display(),
            blender.display()
        );
        Ok(Self {
            source: source.to_path_buf(),
            attach: clips_for.flatten(),
            child,
            output,
            log,
        })
    }

    /// The result, once Blender has finished.
    pub(super) fn poll(&mut self) -> Option<Converted> {
        let status = match self.child.try_wait() {
            Ok(None) => return None,
            Ok(Some(status)) => status,
            Err(err) => return Some(Converted::Failed(format!("Blender: {err}"))),
        };
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let report = log
            .lines()
            .find_map(|line| line.strip_prefix("CONVERTED "))
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok());
        match report {
            Some(report) if status.success() && self.output.is_file() => {
                if report["clips_only"].as_bool() == Some(true) {
                    Some(Converted::Clips(self.output.clone()))
                } else {
                    Some(Converted::Model(self.output.clone()))
                }
            }
            _ => {
                let tail: Vec<&str> = log
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .rev()
                    .take(6)
                    .collect();
                let tail: Vec<&str> = tail.into_iter().rev().collect();
                Some(Converted::Failed(format!(
                    "Blender could not convert {} ({status}): {}",
                    self.source.display(),
                    tail.join(" | ")
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convertible_files_are_recognised() {
        assert!(is_convertible(Path::new("Running.FBX")));
        assert!(is_convertible(Path::new("level.blend")));
        assert!(!is_convertible(Path::new("robot.glb")));
        assert!(SCRIPT.contains("CONVERTED"));
    }
}
