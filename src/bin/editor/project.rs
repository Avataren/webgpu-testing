use std::fmt;
use std::path::PathBuf;

use wgpu_cube::project::ProjectMetadata;

#[cfg(not(target_arch = "wasm32"))]
const BUILD_PLATFORM_COMBO_ID: &str = "project_build_platform";

pub struct ProjectController {
    metadata: ProjectMetadata,
    current_dir: Option<PathBuf>,
    pending_save: Option<PathBuf>,
    pending_load: Option<PathBuf>,
    pending_build: Option<ProjectBuildRequest>,
    settings_open: bool,
    build_dialog: BuildDialogState,
}

impl ProjectController {
    pub fn new() -> Self {
        Self {
            metadata: ProjectMetadata::default(),
            current_dir: None,
            pending_save: None,
            pending_load: None,
            pending_build: None,
            settings_open: false,
            build_dialog: BuildDialogState::default(),
        }
    }

    pub fn metadata(&self) -> &ProjectMetadata {
        &self.metadata
    }

    pub fn set_metadata(&mut self, metadata: ProjectMetadata) {
        self.metadata = metadata;
    }

    pub fn current_dir(&self) -> Option<&PathBuf> {
        self.current_dir.as_ref()
    }

    pub fn set_current_dir(&mut self, dir: PathBuf) {
        self.current_dir = Some(dir);
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

    pub fn take_pending_build(&mut self) -> Option<ProjectBuildRequest> {
        self.pending_build.take()
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
                        egui::ComboBox::from_id_source(BUILD_PLATFORM_COMBO_ID)
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
