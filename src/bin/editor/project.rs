use std::path::PathBuf;

use wgpu_cube::project::ProjectMetadata;

pub struct ProjectController {
    metadata: ProjectMetadata,
    current_dir: Option<PathBuf>,
    pending_save: Option<PathBuf>,
    pending_load: Option<PathBuf>,
    settings_open: bool,
}

impl ProjectController {
    pub fn new() -> Self {
        Self {
            metadata: ProjectMetadata::default(),
            current_dir: None,
            pending_save: None,
            pending_load: None,
            settings_open: false,
        }
    }

    pub fn metadata(&self) -> &ProjectMetadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut ProjectMetadata {
        &mut self.metadata
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
                    .current_dir
                    .clone()
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

    pub fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        egui::Window::new("Project Settings")
            .open(&mut self.settings_open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.metadata.name);
                ui.separator();

                ui.label("Description");
                ui.text_edit_multiline(&mut self.metadata.description);

                ui.separator();
                match &self.current_dir {
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
}
