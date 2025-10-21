use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    thread,
};

use log::{error, info, warn};
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::project::{ProjectError, ProjectManifest};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(not(target_arch = "wasm32"))]
use wgpu_cube::scene::SerializedRuneScriptSource;

use super::core::{EditorApplication, UndoRedoState};
use crate::history::EditorHistory;
use crate::project::{BuildPlatform, ProjectBuildRequest};

#[cfg(not(target_arch = "wasm32"))]
use thiserror::Error;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Error)]
enum BuildCommandError {
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{command} exited with status {status}")]
    Failed { command: String, status: ExitStatus },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("missing build artifact at {0:?}")]
    MissingArtifact(PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
fn run_command_with_logging(label: &str, mut command: Command) -> Result<(), BuildCommandError> {
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    info!("Running {}: {:?}", label, command);

    let mut child = command.spawn().map_err(|source| BuildCommandError::Spawn {
        command: label.to_string(),
        source,
    })?;

    let stdout_handle = child.stdout.take().map(|stdout| {
        spawn_logging_thread(format!("{} (stdout)", label), stdout, log::Level::Info)
    });
    let stderr_handle = child.stderr.take().map(|stderr| {
        spawn_logging_thread(format!("{} (stderr)", label), stderr, log::Level::Warn)
    });

    let status = child.wait().map_err(BuildCommandError::Io)?;

    if let Some(handle) = stdout_handle {
        let _ = handle.join();
    }
    if let Some(handle) = stderr_handle {
        let _ = handle.join();
    }

    if !status.success() {
        return Err(BuildCommandError::Failed {
            command: label.to_string(),
            status,
        });
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_logging_thread<R>(label: String, reader: R, level: log::Level) -> thread::JoinHandle<()>
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let buf_reader = BufReader::new(reader);
        for line in buf_reader.lines() {
            match line {
                Ok(line) => log::log!(level, "{}: {}", label, line),
                Err(err) => log::error!("{}: failed to read process output: {}", label, err),
            }
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_project_stem(name: &str) -> String {
    let mut result = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
        } else if !result.ends_with('_') {
            result.push('_');
        }
    }

    let trimmed = result.trim_matches('_');
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_required_scripts(
    manifest: &ProjectManifest,
    output_dir: &Path,
) -> Result<(), BuildCommandError> {
    let mut scripts = BTreeSet::new();

    for entity in &manifest.scene.entities {
        if let Some(script) = &entity.script {
            if let SerializedRuneScriptSource::File { path } = &script.source {
                scripts.insert(path.clone());
            }
        }
    }

    for script_path in scripts {
        let source = if script_path.is_absolute() {
            script_path.clone()
        } else {
            PathBuf::from(&script_path)
        };

        let destination = output_dir.join(&script_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&source, &destination)?;
        info!("Copied script {:?} -> {:?}", source, destination);
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn build_desktop_artifacts(
    manifest: &ProjectManifest,
    output_dir: &Path,
) -> Result<(), BuildCommandError> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("player");
    run_command_with_logging("cargo build (desktop)", command)?;

    let binary_name = format!("player{}", std::env::consts::EXE_SUFFIX);
    let source_path = Path::new("target").join("release").join(&binary_name);
    if !source_path.exists() {
        return Err(BuildCommandError::MissingArtifact(source_path));
    }

    let target_name = format!(
        "{}{}",
        sanitize_project_stem(&manifest.metadata.name),
        std::env::consts::EXE_SUFFIX
    );
    let target_path = output_dir.join(target_name);
    fs::copy(&source_path, &target_path)?;
    info!("Copied desktop binary to {:?}", target_path);

    copy_required_scripts(manifest, output_dir)?;

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn build_web_artifacts(
    manifest: &ProjectManifest,
    output_dir: &Path,
) -> Result<(), BuildCommandError> {
    let mut cargo_cmd = Command::new("cargo");
    cargo_cmd
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("player")
        .arg("--target")
        .arg("wasm32-unknown-unknown");
    run_command_with_logging("cargo build (wasm)", cargo_cmd)?;

    let wasm_path = Path::new("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("player.wasm");
    if !wasm_path.exists() {
        return Err(BuildCommandError::MissingArtifact(wasm_path));
    }

    let pkg_dir = output_dir.join("pkg");
    if pkg_dir.exists() {
        fs::remove_dir_all(&pkg_dir)?;
    }
    fs::create_dir_all(&pkg_dir)?;

    let mut bindgen_cmd = Command::new("wasm-bindgen");
    bindgen_cmd
        .arg("--target")
        .arg("web")
        .arg("--no-typescript")
        .arg("--out-dir")
        .arg(&pkg_dir)
        .arg(&wasm_path);
    run_command_with_logging("wasm-bindgen", bindgen_cmd)?;

    let index_path = output_dir.join("index.html");
    fs::write(&index_path, build_web_index(&manifest.metadata.name))?;
    info!("Wrote web entry point to {:?}", index_path);

    copy_required_scripts(manifest, output_dir)?;

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn build_web_index(title: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{title}</title>
    <style>
      body {{
        margin: 0;
        padding: 0;
        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
        background: #141414;
        color: #e0e0e0;
        display: flex;
        align-items: center;
        justify-content: center;
        min-height: 100vh;
      }}

      #status {{
        font-size: 1rem;
        opacity: 0.85;
      }}
    </style>
  </head>
  <body>
    <div id="status">Loading {title}…</div>
    <script type="module">
      async function init() {{
        try {{
          const wasm = await import("./pkg/player.js");
          if (typeof wasm.default === "function") {{
            await wasm.default();
          }}
          wasm.start_app();
          const status = document.getElementById("status");
          if (status) {{
            status.textContent = "";
          }}
        }} catch (err) {{
          console.error("Failed to start project", err);
          const status = document.getElementById("status");
          if (status) {{
            status.textContent = "Failed to start project: " + err;
          }}
        }}
      }}

      init();
    </script>
  </body>
</html>
"#
    )
}

impl EditorApplication {
    pub(super) fn process_pending_imports(&mut self, ctx: &mut UpdateContext) {
        if self.pending_imports.is_empty() {
            return;
        }

        let imports = std::mem::take(&mut self.pending_imports);
        let mut any_spawned = false;
        for path in imports {
            let Some(script_source) = Self::create_import_script(&path) else {
                continue;
            };

            let entity_name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Imported glTF".to_string());

            let mut builder = EntityBuilder::new(ctx.scene.main_world_mut())
                .with_name(format!("{entity_name} (glTF)"))
                .with_transform(Transform::default())
                .with_script(script_source);
            builder.spawn();
            any_spawned = true;
        }

        if matches!(ctx.runtime, RuntimeMode::Editor) {
            ctx.scene.set_animation_playback(false);
            ctx.scene.update(0.0);
        }

        if any_spawned {
            self.record_scene_change(ctx.scene);
        }
    }

    pub(super) fn handle_project_save(&mut self, ctx: &mut GpuUpdateContext, dir: PathBuf) {
        match ProjectManifest::capture(ctx.scene, self.project.metadata().clone()) {
            Ok(manifest) => {
                if let Err(err) = manifest.save_to_dir(&dir) {
                    error!("Failed to save project to {:?}: {err}", dir);
                } else {
                    self.project.set_current_dir(dir);
                }
            }
            Err(ProjectError::EmptyScene) => {
                warn!("Skipping project save: no exportable scene data available");
            }
            Err(err) => {
                error!("Failed to prepare project for saving: {err}");
            }
        }
    }

    pub(super) fn handle_project_load(&mut self, ctx: &mut GpuUpdateContext, dir: PathBuf) {
        match ProjectManifest::load_from_dir(&dir) {
            Ok(manifest) => {
                let metadata = manifest.metadata.clone();
                match manifest.instantiate_into(ctx.scene, ctx.renderer, &dir) {
                    Ok(textures_changed) => {
                        if textures_changed {
                            ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
                        }

                        self.project.set_current_dir(dir);
                        self.project.set_metadata(metadata);
                        self.pending_imports.clear();
                        self.pending_entity_deletions.clear();
                        self.selection.set_selected(None);
                        self.selection.set_highlighted(None);
                        self.selection.clear_pending_pick();
                        self.undo_redo = UndoRedoState::default();
                        self.history = EditorHistory::new();
                        self.initialize_history_state(ctx.scene);
                        self.selection.request_override(None);
                        self.runtime_state.request_mode(RuntimeMode::Editor);
                    }
                    Err(err) => {
                        error!("Failed to instantiate project scene: {err}");
                    }
                }
            }
            Err(err) => {
                error!("Failed to load project from {:?}: {err}", dir);
            }
        }
    }

    pub(super) fn handle_project_build(
        &mut self,
        ctx: &mut GpuUpdateContext,
        request: ProjectBuildRequest,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let _ = request;
            warn!("Project builds are not supported when running inside the browser editor");
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Err(err) = fs::create_dir_all(&request.output_dir) {
                error!(
                    "Failed to prepare build output directory {:?}: {err}",
                    request.output_dir
                );
                return;
            }

            let content_dir = request.output_dir.join("content");
            if content_dir.exists() {
                if let Err(err) = fs::remove_dir_all(&content_dir) {
                    error!(
                        "Failed to clean previous content directory {:?}: {err}",
                        content_dir
                    );
                    return;
                }
            }

            match ProjectManifest::capture(ctx.scene, self.project.metadata().clone()) {
                Ok(manifest) => {
                    if let Err(err) = manifest.save_to_dir(&request.output_dir) {
                        error!(
                            "Failed to save project manifest to {:?}: {err}",
                            request.output_dir
                        );
                        return;
                    }

                    let build_result = match request.platform {
                        BuildPlatform::Desktop => {
                            build_desktop_artifacts(&manifest, &request.output_dir)
                        }
                        BuildPlatform::Web => build_web_artifacts(&manifest, &request.output_dir),
                    };

                    match build_result {
                        Ok(()) => {
                            info!(
                                "Project build for {:?} completed at {:?}",
                                request.platform, request.output_dir
                            );
                        }
                        Err(err) => {
                            error!("Failed to build project for {:?}: {err}", request.platform);
                        }
                    }
                }
                Err(ProjectError::EmptyScene) => {
                    warn!(
                        "Skipping project build: no exportable scene data available for {:?}",
                        request.platform
                    );
                }
                Err(err) => {
                    error!("Failed to prepare project for building: {err}");
                }
            }
        }
    }
}
