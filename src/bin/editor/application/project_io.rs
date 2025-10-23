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

#[cfg(not(target_arch = "wasm32"))]
use gltf::{buffer::Source as BufferSource, image::Source as ImageSource, Gltf};

use log::{error, info, warn};
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
#[cfg(not(target_arch = "wasm32"))]
use wgpu_cube::io::percent_decode_uri;
use wgpu_cube::project::{ProjectError, ProjectManifest, CONTENT_DIR};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(not(target_arch = "wasm32"))]
use wgpu_cube::scene::SerializedRuneScriptSource;

use super::core::{EditorApplication, UndoRedoState};
use crate::history::EditorHistory;
use crate::project::{BuildPlatform, NewProjectRequest, ProjectBuildRequest};

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
#[derive(Debug, Error)]
enum NewProjectError {
    #[error("{0:?} already exists and is not a directory")]
    NotADirectory(PathBuf),
    #[error("{0:?} already exists and is not empty")]
    DirectoryNotEmpty(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
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
struct ImportedGltf {
    absolute_gltf: PathBuf,
    project_relative_gltf: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, thiserror::Error)]
enum ImportAssetError {
    #[error("File {path:?} is not a glTF asset (.gltf or .glb).")]
    UnsupportedExtension { path: PathBuf },
    #[error("Source file {path:?} does not exist.")]
    MissingSource { path: PathBuf },
    #[error("Failed to parse glTF file {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: gltf::Error,
    },
    #[error("The selected folder {destination:?} is outside the project content directory.")]
    DestinationOutside { destination: PathBuf },
    #[error("glTF reference '{uri}' escapes the target folder.")]
    DependencyEscapes { uri: String },
    #[error("glTF reference '{uri}' is not supported.")]
    UnsupportedReference { uri: String },
    #[error("glTF reference '{uri}' contains an invalid escape sequence.")]
    MalformedUri { uri: String },
    #[error("glTF reference '{uri}' could not be resolved at {resolved:?}.")]
    MissingDependency { uri: String, resolved: PathBuf },
    #[error("Failed to create folder {path:?}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to copy {source:?} -> {destination:?}: {error}")]
    Copy {
        source: PathBuf,
        destination: PathBuf,
        #[source]
        error: std::io::Error,
    },
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_gltf_into_project(
    project_dir: &Path,
    content_root: &Path,
    destination_root: &Path,
    source_path: &Path,
) -> Result<ImportedGltf, ImportAssetError> {
    if !destination_root.starts_with(content_root) {
        return Err(ImportAssetError::DestinationOutside {
            destination: destination_root.to_path_buf(),
        });
    }

    let source_path = source_path.to_path_buf();
    if !source_path.exists() {
        return Err(ImportAssetError::MissingSource { path: source_path });
    }

    let extension = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("gltf") | Some("glb") => {}
        _ => {
            return Err(ImportAssetError::UnsupportedExtension {
                path: source_path.clone(),
            })
        }
    }

    fs::create_dir_all(destination_root).map_err(|source| ImportAssetError::CreateDir {
        path: destination_root.to_path_buf(),
        source,
    })?;

    let base_name = source_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("asset");
    let asset_folder = unique_asset_folder(destination_root, base_name);

    fs::create_dir_all(&asset_folder).map_err(|source| ImportAssetError::CreateDir {
        path: asset_folder.clone(),
        source,
    })?;

    let result = (|| -> Result<ImportedGltf, ImportAssetError> {
        let dependencies = collect_gltf_dependencies(&source_path)?;
        let source_dir = source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        for uri in dependencies {
            let decoded = percent_decode_uri(&uri)
                .map_err(|_| ImportAssetError::MalformedUri { uri: uri.clone() })?;
            if decoded.contains("://") || decoded.starts_with("//") {
                return Err(ImportAssetError::UnsupportedReference { uri });
            }

            if decoded.starts_with("data:") {
                continue;
            }

            let relative_path = Path::new(&decoded);
            if relative_path.is_absolute() {
                return Err(ImportAssetError::UnsupportedReference { uri });
            }

            let source_dependency = source_dir.join(relative_path);
            if !source_dependency.exists() {
                return Err(ImportAssetError::MissingDependency {
                    uri,
                    resolved: source_dependency,
                });
            }

            let destination_dependency = safe_join(&asset_folder, relative_path, &uri)?;
            if let Some(parent) = destination_dependency.parent() {
                fs::create_dir_all(parent).map_err(|source| ImportAssetError::CreateDir {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }

            fs::copy(&source_dependency, &destination_dependency).map_err(|error| {
                ImportAssetError::Copy {
                    source: source_dependency.clone(),
                    destination: destination_dependency.clone(),
                    error,
                }
            })?;
        }

        let destination_file = asset_folder.join(source_path.file_name().ok_or_else(|| {
            ImportAssetError::UnsupportedExtension {
                path: source_path.clone(),
            }
        })?);

        fs::copy(&source_path, &destination_file).map_err(|error| ImportAssetError::Copy {
            source: source_path.clone(),
            destination: destination_file.clone(),
            error,
        })?;

        let project_relative = destination_file
            .strip_prefix(project_dir)
            .map(Path::to_path_buf)
            .map_err(|_| ImportAssetError::DestinationOutside {
                destination: destination_file.clone(),
            })?;

        Ok(ImportedGltf {
            absolute_gltf: destination_file,
            project_relative_gltf: project_relative,
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&asset_folder);
    }

    result
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_gltf_dependencies(path: &Path) -> Result<BTreeSet<String>, ImportAssetError> {
    let mut dependencies = BTreeSet::new();
    let document = Gltf::open(path).map_err(|source| ImportAssetError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    for buffer in document.buffers() {
        if let BufferSource::Uri(uri) = buffer.source() {
            let trimmed = uri.trim();
            if !trimmed.starts_with("data:") {
                dependencies.insert(trimmed.to_string());
            }
        }
    }

    for image in document.images() {
        match image.source() {
            ImageSource::View { .. } => {}
            ImageSource::Uri { uri, .. } => {
                let trimmed = uri.trim();
                if !trimmed.starts_with("data:") {
                    dependencies.insert(trimmed.to_string());
                }
            }
        }
    }

    Ok(dependencies)
}

#[cfg(not(target_arch = "wasm32"))]
fn safe_join(base: &Path, relative: &Path, original: &str) -> Result<PathBuf, ImportAssetError> {
    use std::path::Component;

    let mut result = base.to_path_buf();
    let mut depth = 0usize;
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                result.push(part);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return Err(ImportAssetError::DependencyEscapes {
                        uri: original.to_string(),
                    });
                }
                result.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ImportAssetError::DependencyEscapes {
                    uri: original.to_string(),
                });
            }
        }
    }

    Ok(result)
}

#[cfg(not(target_arch = "wasm32"))]
fn unique_asset_folder(destination_root: &Path, base_name: &str) -> PathBuf {
    let sanitized = if base_name.is_empty() {
        "asset"
    } else {
        base_name
    };
    let mut candidate = sanitized.to_string();
    let mut counter = 1usize;
    loop {
        let path = destination_root.join(&candidate);
        if !path.exists() {
            return path;
        }
        candidate = format!("{sanitized}_{counter:02}");
        counter += 1;
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

#[cfg(not(target_arch = "wasm32"))]
fn create_new_project(request: &NewProjectRequest) -> Result<(), NewProjectError> {
    let dir = &request.directory;

    if dir.exists() {
        if !dir.is_dir() {
            return Err(NewProjectError::NotADirectory(dir.clone()));
        }

        let mut entries = fs::read_dir(dir)?;
        if let Some(entry) = entries.next() {
            entry?;
            return Err(NewProjectError::DirectoryNotEmpty(dir.clone()));
        }
    } else {
        if let Some(parent) = dir.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(dir)?;
    }

    fs::create_dir_all(dir.join(CONTENT_DIR))?;

    let manifest = ProjectManifest::new_empty(request.metadata.clone());
    manifest.save_to_dir(dir)?;

    Ok(())
}

impl EditorApplication {
    pub(super) fn handle_project_create(
        &mut self,
        ctx: &mut GpuUpdateContext,
        request: NewProjectRequest,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let _ = request;
            warn!("Project creation is not supported when running inside the browser editor");
            self.project.report_startup_error(
                "Project creation is not supported in WebAssembly builds of the editor.",
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = request.directory.clone();
            match create_new_project(&request) {
                Ok(()) => {
                    info!("Created new project at {:?}", dir);
                    self.handle_project_load(ctx, dir);
                }
                Err(err) => {
                    error!("Failed to create project at {:?}: {err}", dir);
                    self.project.report_startup_error(err.to_string());
                }
            }
        }
    }

    pub(super) fn process_pending_imports(
        &mut self,
        ctx: &mut UpdateContext,
        pending: Vec<PathBuf>,
    ) {
        if pending.is_empty() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            if !pending.is_empty() {
                warn!("glTF imports are not supported when running inside the browser editor");
            }
            let _ = ctx;
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(project_dir) = self.project.current_dir().cloned() else {
                self.asset_browser
                    .report_error("Open or create a project before importing assets.");
                warn!("Ignoring glTF import request: no project directory is active");
                return;
            };

            let Some(content_root) = self.project.content_root() else {
                self.asset_browser
                    .report_error("The project's content folder is unavailable.");
                warn!("Ignoring glTF import request: project content folder missing");
                return;
            };

            let destination_root = self.asset_browser.selected_folder(&content_root);
            let mut any_spawned = false;

            for source_path in pending {
                match copy_gltf_into_project(
                    &project_dir,
                    &content_root,
                    &destination_root,
                    &source_path,
                ) {
                    Ok(result) => {
                        let Some(script_source) =
                            Self::create_import_script(&result.project_relative_gltf)
                        else {
                            error!(
                                "Failed to build import script for {:?}; skipping entity spawn",
                                result.project_relative_gltf
                            );
                            self.asset_browser
                                .report_error("Failed to prepare glTF import script.");
                            if let Some(folder) = result.absolute_gltf.parent() {
                                let _ = fs::remove_dir_all(folder);
                            }
                            continue;
                        };

                        let entity_name = result
                            .absolute_gltf
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

                        let display_path =
                            result.project_relative_gltf.to_string_lossy().to_string();
                        self.asset_browser
                            .report_info(format!("Imported asset to {display_path}"));
                    }
                    Err(err) => {
                        error!("Failed to import glTF asset {:?}: {}", source_path, err);
                        self.asset_browser.report_error(err.to_string());
                    }
                }
            }

            if matches!(ctx.runtime, RuntimeMode::Editor) {
                ctx.scene.set_animation_playback(false);
                ctx.scene.update(0.0);
            }

            if any_spawned {
                self.record_scene_change(ctx.scene);
            }
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
                        self.ensure_editor_scene_basics(ctx.scene, ctx.renderer);
                        if textures_changed {
                            ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
                        }

                        self.project.set_current_dir(dir);
                        self.project.set_metadata(metadata);
                        self.commands.clear();
                        {
                            let selection = self.selection_system_mut();
                            selection.set_selected(None);
                            selection.set_highlighted(None);
                            selection.clear_pending_pick();
                            selection.request_override(None);
                        }
                        self.undo_redo = UndoRedoState::default();
                        self.history = EditorHistory::new();
                        self.initialize_history_state(ctx.scene);
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
