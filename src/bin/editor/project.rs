use std::fmt;
use std::path::PathBuf;

use wgpu_cube::project::{ProjectMetadata, SceneDocument, CONTENT_DIR};

#[cfg(not(target_arch = "wasm32"))]
const BUILD_PLATFORM_COMBO_ID: &str = "project_build_platform";

pub struct ProjectController {
    metadata: ProjectMetadata,
    current_dir: Option<PathBuf>,
    pending_save: Option<PathBuf>,
    pending_load: Option<PathBuf>,
    pending_build: Option<ProjectBuildRequest>,
    pending_create: Option<NewProjectRequest>,
    settings_open: bool,
    startup_dialog: StartupDialogState,
    build_dialog: BuildDialogState,
    scene_documents: Vec<SceneDocument>,
}

impl ProjectController {
    pub fn new() -> Self {
        Self {
            metadata: ProjectMetadata::default(),
            current_dir: None,
            pending_save: None,
            pending_load: None,
            pending_build: None,
            pending_create: None,
            settings_open: false,
            startup_dialog: StartupDialogState::new(),
            build_dialog: BuildDialogState::default(),
            scene_documents: Vec::new(),
        }
    }

    pub fn metadata(&self) -> &ProjectMetadata {
        &self.metadata
    }

    pub fn set_metadata(&mut self, metadata: ProjectMetadata) {
        self.metadata = metadata;
    }

    pub(super) fn add_scene_document(&mut self, document: wgpu_cube::project::SceneDocument) {
        self.scene_documents.push(document);
    }

    pub(super) fn scene_documents(&self) -> &[wgpu_cube::project::SceneDocument] {
        &self.scene_documents
    }

    pub fn current_dir(&self) -> Option<&PathBuf> {
        self.current_dir.as_ref()
    }

    pub fn set_scene_documents(&mut self, documents: Vec<SceneDocument>) {
        self.scene_documents = documents;
    }

    pub fn primary_scene_document(&self) -> Option<&SceneDocument> {
        self.scene_documents.first()
    }

    pub fn set_current_dir(&mut self, dir: PathBuf) {
        self.current_dir = Some(dir);
        self.startup_dialog.on_project_loaded();
    }

    pub fn content_root(&self) -> Option<PathBuf> {
        let dir = self.current_dir()?;
        let path = dir.join(CONTENT_DIR);

        #[cfg(not(target_arch = "wasm32"))]
        {
            if !path.exists() {
                if let Err(err) = std::fs::create_dir_all(&path) {
                    log::error!(
                        "Failed to create missing project content directory {:?}: {err}",
                        path
                    );
                    return None;
                }
            }
        }

        Some(path)
    }

    pub fn is_startup_dialog_visible(&self) -> bool {
        self.startup_dialog.visible
    }

    pub fn show_startup_dialog(&mut self, ctx: &egui::Context) {
        if !self.startup_dialog.visible {
            return;
        }

        let window = egui::Window::new("Select Project")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]);

        window.show(ctx, |ui| {
            ui.set_width(360.0);
            ui.vertical(|ui| {
                ui.heading("Welcome to the Editor");
                ui.label("Load an existing project or create a new one to get started.");
                ui.add_space(12.0);

                ui.heading("Load Project");

                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Open Project Folder...").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.startup_dialog.clear_error();
                        self.pending_load = Some(dir);
                        self.startup_dialog.dismiss();
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    ui.add_enabled(false, egui::Button::new("Open Project Folder..."));
                    ui.label("Project loading is unavailable in the WebAssembly editor.");
                }

                ui.add_space(16.0);
                ui.heading("Create Project");

                let name_response =
                    ui.text_edit_singleline(&mut self.startup_dialog.new_project_name);
                if name_response.changed() {
                    self.startup_dialog.clear_error();
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label("Location:");
                    ui.label(self.startup_dialog.location_display());
                });

                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Choose Folder...").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.startup_dialog.set_root(dir);
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    ui.add_enabled(false, egui::Button::new("Choose Folder..."));
                }

                ui.add_space(8.0);

                #[cfg(not(target_arch = "wasm32"))]
                {
                    if ui.button("Create Project").clicked() {
                        self.startup_dialog.clear_error();
                        match self.startup_dialog.build_creation_request() {
                            Ok(request) => {
                                self.pending_create = Some(request);
                                self.startup_dialog.dismiss();
                            }
                            Err(message) => self.startup_dialog.set_error(message),
                        }
                    }
                }

                #[cfg(target_arch = "wasm32")]
                {
                    ui.add_enabled(false, egui::Button::new("Create Project"));
                    ui.label("Project creation is unavailable in the WebAssembly editor.");
                }

                if let Some(err) = self.startup_dialog.error_message.as_deref() {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(200, 64, 64), err);
                }
            });
        });
    }

    pub fn menu_contents(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if ui.button("Open Project...").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.pending_load = Some(dir);
                    ui.close();
                }
            }

            if ui.button("Save Project").clicked() {
                if let Some(dir) = self
                    .current_dir()
                    .cloned()
                    .or_else(|| rfd::FileDialog::new().pick_folder())
                {
                    self.pending_save = Some(dir);
                    ui.close();
                }
            }

            if ui.button("Save Project As...").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.pending_save = Some(dir);
                    ui.close();
                }
            }

            ui.separator();
        }

        #[cfg(target_arch = "wasm32")]
        {
            ui.add_enabled(false, egui::Button::new("Open Project..."));
            ui.add_enabled(false, egui::Button::new("Save Project"));
            ui.add_enabled(false, egui::Button::new("Save Project As..."));
            ui.separator();
        }

        if ui.button("Project Settings...").clicked() {
            self.settings_open = true;
            ui.close();
        }
    }

    pub fn build_menu_contents(&mut self, ui: &mut egui::Ui) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if ui.button("Build Project...").clicked() {
                self.build_dialog.open(self.current_dir().cloned());
                ui.close();
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            ui.add_enabled(false, egui::Button::new("Build Project..."));
        }
    }

    pub fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let current_dir = self.current_dir().cloned();
        let metadata = &mut self.metadata;
        let settings_open = &mut self.settings_open;
        egui::Window::new("Project Settings")
            .open(settings_open)
            .resizable(false)
            .show(ctx, move |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut metadata.name);
                ui.separator();

                ui.label("Description");
                ui.text_edit_multiline(&mut metadata.description);

                ui.separator();
                match &current_dir {
                    Some(dir) => ui.label(format!("Location: {}", dir.display())),
                    None => ui.label("Location: (unsaved)"),
                };
            });
    }

    pub fn take_pending_save(&mut self) -> Option<PathBuf> {
        self.pending_save.take()
    }

    pub fn take_pending_load(&mut self) -> Option<PathBuf> {
        self.pending_load.take()
    }

    pub fn take_pending_create(&mut self) -> Option<NewProjectRequest> {
        self.pending_create.take()
    }

    pub fn take_pending_build(&mut self) -> Option<ProjectBuildRequest> {
        self.pending_build.take()
    }

    pub fn report_startup_error(&mut self, message: impl Into<String>) {
        self.startup_dialog.visible = true;
        self.startup_dialog.error_message = Some(message.into());
    }

    pub fn show_build_window(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        {
            if !self.build_dialog.open {
                return;
            }

            egui::Window::new("Build Project")
                .open(&mut self.build_dialog.open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Project builds are not supported in WebAssembly builds of the editor.",
                    );
                });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            if !self.build_dialog.open {
                return;
            }

            let mut open = self.build_dialog.open;
            egui::Window::new("Build Project")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label("Select the target platform and output folder for the build.");
                        ui.separator();

                        ui.label("Platform");
                        egui::ComboBox::from_id_salt(BUILD_PLATFORM_COMBO_ID)
                            .selected_text(self.build_dialog.platform.display_name())
                            .show_ui(ui, |ui| {
                                for platform in BuildPlatform::ALL {
                                    ui.selectable_value(
                                        &mut self.build_dialog.platform,
                                        platform,
                                        platform.display_name(),
                                    );
                                }
                            });

                        ui.separator();
                        ui.label("Output folder");
                        ui.horizontal(|ui| {
                            let display = self
                                .build_dialog
                                .output_dir
                                .as_ref()
                                .map(|dir| dir.display().to_string())
                                .unwrap_or_else(|| "(not selected)".to_string());
                            ui.label(display);
                            if ui.button("Select...").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.build_dialog.output_dir = Some(dir);
                                    self.build_dialog.last_error = None;
                                }
                            }
                        });

                        if let Some(error) = &self.build_dialog.last_error {
                            ui.add_space(8.0);
                            ui.colored_label(egui::Color32::from_rgb(200, 80, 80), error);
                        }

                        ui.add_space(12.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                self.build_dialog.close();
                                return;
                            }

                            let build_enabled = self.build_dialog.output_dir.is_some();
                            if ui
                                .add_enabled(build_enabled, egui::Button::new("Build"))
                                .clicked()
                            {
                                if let Some(dir) = self.build_dialog.output_dir.clone() {
                                    self.pending_build = Some(ProjectBuildRequest {
                                        platform: self.build_dialog.platform,
                                        output_dir: dir,
                                    });
                                    self.build_dialog.close();
                                } else {
                                    self.build_dialog.last_error = Some(
                                        "Please select an output folder before building."
                                            .to_string(),
                                    );
                                }
                            }
                        });
                    });
                });
            let keep_open = self.build_dialog.open;
            self.build_dialog.open = open && keep_open;
        }
    }
}

#[derive(Clone)]
pub struct NewProjectRequest {
    pub directory: PathBuf,
    pub metadata: ProjectMetadata,
}

struct StartupDialogState {
    visible: bool,
    new_project_name: String,
    new_project_root: Option<PathBuf>,
    error_message: Option<String>,
}

impl StartupDialogState {
    fn new() -> Self {
        Self {
            // Disabled by default - now handled by Welcome Screen plugin
            visible: false,
            new_project_name: "NewProject".to_string(),
            new_project_root: None,
            error_message: None,
        }
    }

    fn dismiss(&mut self) {
        self.visible = false;
    }

    fn on_project_loaded(&mut self) {
        self.visible = false;
        self.error_message = None;
    }

    fn clear_error(&mut self) {
        self.error_message = None;
    }

    fn set_error(&mut self, message: String) {
        self.error_message = Some(message);
    }

    fn set_root(&mut self, dir: PathBuf) {
        self.new_project_root = Some(dir);
        self.clear_error();
    }

    fn location_display(&self) -> String {
        match &self.new_project_root {
            Some(root) => {
                let trimmed = self.new_project_name.trim();
                if trimmed.is_empty() {
                    root.display().to_string()
                } else {
                    root.join(trimmed).display().to_string()
                }
            }
            None => "(choose folder)".to_string(),
        }
    }

    fn build_creation_request(&self) -> Result<NewProjectRequest, String> {
        let name = self.new_project_name.trim();
        if name.is_empty() {
            return Err("Enter a project folder name.".to_string());
        }

        if name.chars().any(|ch| ch == '/' || ch == '\\') {
            return Err("Project folder name cannot contain path separators.".to_string());
        }

        let root = self
            .new_project_root
            .as_ref()
            .ok_or_else(|| "Choose a folder for the project.".to_string())?;

        let directory = root.join(name);
        let metadata = ProjectMetadata {
            name: name.to_string(),
            description: String::new(),
        };

        Ok(NewProjectRequest {
            directory,
            metadata,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildPlatform {
    Desktop,
    Web,
}

impl BuildPlatform {
    const ALL: [Self; 2] = [Self::Desktop, Self::Web];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Web => "Web",
        }
    }
}

impl Default for BuildPlatform {
    fn default() -> Self {
        Self::Desktop
    }
}

impl fmt::Display for BuildPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Clone, Debug)]
pub struct ProjectBuildRequest {
    pub platform: BuildPlatform,
    pub output_dir: PathBuf,
}

#[derive(Default)]
struct BuildDialogState {
    open: bool,
    platform: BuildPlatform,
    output_dir: Option<PathBuf>,
    last_error: Option<String>,
}

impl BuildDialogState {
    fn open(&mut self, default_output: Option<PathBuf>) {
        self.open = true;
        if self.output_dir.is_none() {
            self.output_dir = default_output;
        }
        self.last_error = None;
    }

    fn close(&mut self) {
        self.open = false;
        self.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn content_root_creates_missing_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).expect("project dir");

        let mut controller = ProjectController::new();
        controller.set_current_dir(project_dir.clone());

        let content_root = controller
            .content_root()
            .expect("content directory should be created");

        assert_eq!(content_root, project_dir.join(CONTENT_DIR));
        assert!(
            content_root.exists(),
            "content directory should exist after querying content_root"
        );
    }
}
