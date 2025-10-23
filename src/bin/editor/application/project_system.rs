use std::path::PathBuf;

use egui::Ui;
use log::{error, info, warn};
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::project::{ProjectError, ProjectManifest};
use wgpu_cube::scene::{EntityBuilder, Transform};

use crate::project::{BuildPlatform, NewProjectRequest, ProjectBuildRequest, ProjectController};

use super::core::EditorApplication;
use super::{EditorContext, EditorSystem};

pub(super) struct ProjectSystem {
    controller: ProjectController,
    aux_windows_enabled: bool,
}

impl ProjectSystem {
    pub(super) fn new(controller: ProjectController) -> Self {
        Self {
            controller,
            aux_windows_enabled: true,
        }
    }

    pub(super) fn controller_mut(&mut self) -> &mut ProjectController {
        &mut self.controller
    }

    pub(super) fn is_startup_dialog_visible(&self) -> bool {
        self.controller.is_startup_dialog_visible()
    }

    pub(super) fn content_root(&self) -> Option<PathBuf> {
        self.controller.content_root()
    }

    pub(super) fn menu_contents(&mut self, ui: &mut Ui) {
        self.controller.menu_contents(ui);
    }

    pub(super) fn build_menu_contents(&mut self, ui: &mut Ui) {
        self.controller.build_menu_contents(ui);
    }

    pub(super) fn show_startup_dialog(&mut self, ctx: &egui::Context) {
        self.controller.show_startup_dialog(ctx);
    }

    pub(super) fn set_auxiliary_windows_enabled(&mut self, enabled: bool) {
        self.aux_windows_enabled = enabled;
    }
    pub(super) fn process_pending_imports(
        &mut self,
        ctx: &mut EditorContext<'_, '_, '_>,
        pending: Vec<PathBuf>,
    ) {
        if pending.is_empty() {
            return;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            warn!("glTF imports are not supported when running inside the browser editor");
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(project_dir) = self.controller.current_dir().cloned() else {
                ctx.with_update(|app, _| {
                    app.asset_browser
                        .report_error("Open or create a project before importing assets.");
                });
                warn!("Ignoring glTF import request: no project directory is active");
                return;
            };

            let Some(content_root) = self.controller.content_root() else {
                ctx.with_update(|app, _| {
                    app.asset_browser
                        .report_error("The project's content folder is unavailable.");
                });
                warn!("Ignoring glTF import request: project content folder missing");
                return;
            };

            let Some(()) = ctx.with_update(|app, update_ctx| {
                let destination_root = app.asset_browser.selected_folder(&content_root);
                let mut any_spawned = false;

                for source_path in pending {
                    match native::copy_gltf_into_project(
                        &project_dir,
                        &content_root,
                        &destination_root,
                        &source_path,
                    ) {
                        Ok(result) => {
                            let Some(script_source) = EditorApplication::create_import_script(
                                &result.project_relative_gltf,
                            ) else {
                                error!(
                                    "Failed to build import script for {:?}; skipping entity spawn",
                                    result.project_relative_gltf
                                );
                                app.asset_browser
                                    .report_error("Failed to prepare glTF import script.");
                                if let Some(folder) = result.absolute_gltf.parent() {
                                    let _ = std::fs::remove_dir_all(folder);
                                }
                                continue;
                            };

                            let entity_name = result
                                .absolute_gltf
                                .file_stem()
                                .map(|stem| stem.to_string_lossy().into_owned())
                                .filter(|name| !name.is_empty())
                                .unwrap_or_else(|| "Imported glTF".to_string());

                            let mut builder = EntityBuilder::new(update_ctx.scene.main_world_mut())
                                .with_name(format!("{entity_name} (glTF)"))
                                .with_transform(Transform::default())
                                .with_script(script_source);
                            builder.spawn();

                            any_spawned = true;

                            app.asset_browser.report_info(format!(
                                "Imported asset to {}",
                                result.project_relative_gltf.display()
                            ));
                        }
                        Err(err) => {
                            error!("Failed to import glTF asset {:?}: {}", source_path, err);
                            app.asset_browser.report_error(err.to_string());
                        }
                    }
                }

                if matches!(update_ctx.runtime, RuntimeMode::Editor) {
                    update_ctx.scene.set_animation_playback(false);
                    update_ctx.scene.update(0.0);
                }

                if any_spawned {
                    app.record_scene_change(update_ctx.scene);
                }
            }) else {
                return;
            };
        }
    }
    fn handle_project_create(
        &mut self,
        ctx: &mut EditorContext<'_, '_, '_>,
        request: NewProjectRequest,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let _ = request;
            warn!("Project creation is not supported when running inside the browser editor");
            self.controller.report_startup_error(
                "Project creation is not supported in WebAssembly builds of the editor.",
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = request.directory.clone();
            match native::create_new_project(&request) {
                Ok(()) => {
                    info!("Created new project at {:?}", dir);
                    self.handle_project_load(ctx, dir);
                }
                Err(err) => {
                    error!("Failed to create project at {:?}: {err}", dir);
                    self.controller.report_startup_error(err.to_string());
                }
            }
        }
    }

    fn handle_project_save(&mut self, ctx: &mut EditorContext<'_, '_, '_>, dir: PathBuf) {
        let Some(gpu_ctx) = ctx.gpu_context_mut() else {
            return;
        };

        match ProjectManifest::capture(gpu_ctx.scene, self.controller.metadata().clone()) {
            Ok(manifest) => {
                if let Err(err) = manifest.save_to_dir(&dir) {
                    error!("Failed to save project to {:?}: {err}", dir);
                } else {
                    self.controller.set_current_dir(dir);
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

    fn handle_project_load(&mut self, ctx: &mut EditorContext<'_, '_, '_>, dir: PathBuf) {
        let Some(()) = ctx.with_gpu(|app, gpu_ctx| match ProjectManifest::load_from_dir(&dir) {
            Ok(manifest) => {
                let metadata = manifest.metadata.clone();
                match manifest.instantiate_into(gpu_ctx.scene, gpu_ctx.renderer, &dir) {
                    Ok(textures_changed) => {
                        app.ensure_editor_scene_basics(gpu_ctx.scene, gpu_ctx.renderer);
                        if textures_changed {
                            gpu_ctx
                                .renderer
                                .update_texture_bind_group(&gpu_ctx.scene.assets);
                        }

                        self.controller.set_current_dir(dir.clone());
                        self.controller.set_metadata(metadata);
                        app.commands.clear();
                        {
                            let selection = app.selection_system_mut();
                            selection.set_selected(None);
                            selection.set_highlighted(None);
                            selection.clear_pending_pick();
                            selection.request_override(None);
                        }
                        app.history_system_mut().reset();
                        app.initialize_history_state(gpu_ctx.scene);
                        app.runtime_state.request_mode(RuntimeMode::Editor);
                    }
                    Err(err) => {
                        error!("Failed to instantiate project scene: {err}");
                    }
                }
            }
            Err(err) => {
                error!("Failed to load project from {:?}: {err}", dir);
            }
        }) else {
            return;
        };
    }

    fn handle_project_build(
        &mut self,
        ctx: &mut EditorContext<'_, '_, '_>,
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
            if let Err(err) = std::fs::create_dir_all(&request.output_dir) {
                error!(
                    "Failed to prepare build output directory {:?}: {err}",
                    request.output_dir
                );
                return;
            }

            let content_dir = request.output_dir.join("content");
            if content_dir.exists() {
                if let Err(err) = std::fs::remove_dir_all(&content_dir) {
                    error!(
                        "Failed to clean previous content directory {:?}: {err}",
                        content_dir
                    );
                    return;
                }
            }

            let Some(()) = ctx.with_gpu(|_app, gpu_ctx| {
                match ProjectManifest::capture(gpu_ctx.scene, self.controller.metadata().clone()) {
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
                                native::build_desktop_artifacts(&manifest, &request.output_dir)
                            }
                            BuildPlatform::Web => {
                                native::build_web_artifacts(&manifest, &request.output_dir)
                            }
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
            }) else {
                return;
            };
        }
    }
}
impl EditorSystem for ProjectSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        if let Some(request) = self.controller_mut().take_pending_create() {
            self.handle_project_create(ctx, request);
        }

        if let Some(dir) = self.controller_mut().take_pending_load() {
            self.handle_project_load(ctx, dir);
        }

        if let Some(dir) = self.controller_mut().take_pending_save() {
            self.handle_project_save(ctx, dir);
        }

        if let Some(request) = self.controller_mut().take_pending_build() {
            self.handle_project_build(ctx, request);
        }

        let _ = ctx.with_gpu(|_, gpu_ctx| {
            gpu_ctx.scene.process_pending_gltf_imports(gpu_ctx.renderer);
        });
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        if !self.aux_windows_enabled {
            return;
        }

        if let Some(ui_ctx) = ctx.ui_context() {
            self.controller.show_settings_window(ui_ctx.egui());
            self.controller.show_build_window(ui_ctx.egui());
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::path::{Component, Path, PathBuf};
    use std::process::{Command, ExitStatus, Stdio};
    use std::thread;

    use gltf::{buffer::Source as BufferSource, image::Source as ImageSource, Gltf};
    use log::{error, info};
    use thiserror::Error;
    use wgpu_cube::io::percent_decode_uri;
    use wgpu_cube::project::{ProjectError, ProjectManifest, CONTENT_DIR};
    use wgpu_cube::scene::SerializedRuneScriptSource;

    use crate::project::NewProjectRequest;

    #[derive(Debug, Error)]
    pub(super) enum BuildCommandError {
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

    #[derive(Debug, Error)]
    pub(super) enum NewProjectError {
        #[error("{0:?} already exists and is not a directory")]
        NotADirectory(PathBuf),
        #[error("{0:?} already exists and is not empty")]
        DirectoryNotEmpty(PathBuf),
        #[error(transparent)]
        Io(#[from] std::io::Error),
        #[error(transparent)]
        Project(#[from] ProjectError),
    }

    #[derive(Debug, thiserror::Error)]
    pub(super) enum ImportAssetError {
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

    pub(super) struct ImportedGltf {
        pub(super) absolute_gltf: PathBuf,
        pub(super) project_relative_gltf: PathBuf,
    }

    pub(super) fn create_new_project(request: &NewProjectRequest) -> Result<(), NewProjectError> {
        let dir = &request.directory;
        if dir.exists() && !dir.is_dir() {
            return Err(NewProjectError::NotADirectory(dir.clone()));
        }

        if dir
            .read_dir()
            .map(|mut r| r.next().is_some())
            .unwrap_or(false)
        {
            return Err(NewProjectError::DirectoryNotEmpty(dir.clone()));
        }

        fs::create_dir_all(dir)?;
        fs::create_dir_all(dir.join(CONTENT_DIR))?;

        let manifest = ProjectManifest::new_empty(request.metadata.clone());
        manifest.save_to_dir(dir)?;

        Ok(())
    }

    pub(super) fn copy_gltf_into_project(
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

    fn safe_join(
        base: &Path,
        relative: &Path,
        original: &str,
    ) -> Result<PathBuf, ImportAssetError> {
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

    pub(super) fn build_desktop_artifacts(
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

    pub(super) fn build_web_artifacts(
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

    fn build_web_index(title: &str) -> String {
        format!(
            "<!DOCTYPE html>\n<html lang='en'>\n  <head>\n    <meta charset='utf-8' />\n    <meta name='viewport' content='width=device-width, initial-scale=1.0' />\n    <title>{title}</title>\n    <style>\n      body {{\n        margin: 0;\n        padding: 0;\n        font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;\n        background: #141414;\n        color: #e0e0e0;\n        display: flex;\n        flex-direction: column;\n        min-height: 100vh;\n        align-items: center;\n        justify-content: center;\n      }}\n\n      #status {{\n        margin-top: 1rem;\n        font-size: 0.9rem;\n        opacity: 0.75;\n      }}\n\n      canvas {{\n        width: min(90vw, 960px);\n        height: min(90vh, 540px);\n        border: 1px solid #2a2a2a;\n        background: #050505;\n        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);\n      }}\n    </style>\n  </head>\n  <body>\n    <canvas id='wgpu-app'></canvas>\n    <div id='status'>Loading project...</div>\n    <script type='module'>\n      import init from './pkg/player.js';\n\n      async function start() {{\n        const status = document.getElementById('status');\n        try {{\n          await init();\n          status.textContent = 'Project loaded successfully.';\n        }} catch (err) {{\n          console.error('Failed to start project', err);\n          status.textContent = 'Failed to start project: ' + err;\n        }}\n      }}\n\n      start();\n    </script>\n  </body>\n</html>\n"
        )
    }

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

    fn copy_required_scripts(
        manifest: &ProjectManifest,
        output_dir: &Path,
    ) -> Result<(), std::io::Error> {
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

    fn run_command_with_logging(
        label: &str,
        mut command: Command,
    ) -> Result<(), BuildCommandError> {
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

    fn spawn_logging_thread<R>(
        label: String,
        reader: R,
        level: log::Level,
    ) -> thread::JoinHandle<()>
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
}
