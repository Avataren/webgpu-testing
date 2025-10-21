use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct AssetBrowserState {
    selected_folder: Option<PathBuf>,
    new_folder_name: String,
    feedback: Option<Feedback>,
    last_root: Option<PathBuf>,
}

#[derive(Debug)]
enum Feedback {
    Info(String),
    Error(String),
}

impl AssetBrowserState {
    pub fn selected_folder(&self, content_root: &Path) -> PathBuf {
        self.selected_folder
            .as_ref()
            .filter(|selected| selected.starts_with(content_root))
            .cloned()
            .unwrap_or_else(|| content_root.to_path_buf())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn report_error(&mut self, message: impl Into<String>) {
        self.feedback = Some(Feedback::Error(message.into()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn report_info(&mut self, message: impl Into<String>) {
        self.feedback = Some(Feedback::Info(message.into()));
    }

    pub fn clear_feedback(&mut self) {
        self.feedback = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, content_root: Option<&Path>) {
        #[cfg(target_arch = "wasm32")]
        {
            ui.label("The asset browser is unavailable in the WebAssembly build.");
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.ui_native(ui, content_root);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ui_native(&mut self, ui: &mut egui::Ui, content_root: Option<&Path>) {
        use std::fs;

        let Some(root) = content_root else {
            ui.label("Open or create a project to browse assets.");
            return;
        };

        if self
            .last_root
            .as_ref()
            .map(|previous| previous != root)
            .unwrap_or(true)
        {
            self.last_root = Some(root.to_path_buf());
            self.selected_folder = Some(root.to_path_buf());
            self.feedback = None;
            self.new_folder_name.clear();
        }

        if let Some(selected) = self.selected_folder.clone() {
            if !selected.starts_with(root) || !selected.is_dir() {
                self.selected_folder = Some(root.to_path_buf());
            }
        } else {
            self.selected_folder = Some(root.to_path_buf());
        }

        let selected_folder = self
            .selected_folder
            .clone()
            .unwrap_or_else(|| root.to_path_buf());

        ui.horizontal(|ui| {
            ui.set_height(ui.available_height());

            ui.vertical(|ui| {
                ui.set_min_width(180.0);
                ui.heading("Folders");
                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("asset_browser_folders")
                    .show(ui, |ui| {
                        self.show_folder_node(ui, &selected_folder, root, root);
                    });
            });

            ui.separator();

            ui.vertical(|ui| {
                let relative = selected_folder
                    .strip_prefix(root)
                    .ok()
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| "content".to_string());

                ui.heading(format!("Folder: {relative}"));
                ui.separator();

                if let Some(feedback) = &self.feedback {
                    match feedback {
                        Feedback::Info(message) => {
                            ui.colored_label(egui::Color32::from_rgb(120, 180, 120), message);
                        }
                        Feedback::Error(message) => {
                            ui.colored_label(egui::Color32::from_rgb(200, 80, 80), message);
                        }
                    }
                }

                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.new_folder_name)
                            .hint_text("New folder name"),
                    );

                    if response.changed() {
                        self.feedback = None;
                    }

                    let create_clicked = ui.button("Create Folder").clicked()
                        || (response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter)));

                    if create_clicked {
                        self.create_folder(&selected_folder);
                    }
                });

                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("asset_browser_files")
                    .show(ui, |ui| {
                        let mut directories = Vec::new();
                        let mut files = Vec::new();

                        match fs::read_dir(&selected_folder) {
                            Ok(entries) => {
                                for entry in entries.flatten() {
                                    let path = entry.path();
                                    if path.is_dir() {
                                        directories.push(path);
                                    } else {
                                        files.push(path);
                                    }
                                }
                            }
                            Err(err) => {
                                ui.colored_label(
                                    egui::Color32::from_rgb(200, 80, 80),
                                    format!(
                                        "Failed to read folder {}: {err}",
                                        selected_folder
                                            .strip_prefix(root)
                                            .ok()
                                            .map(|path| path.display().to_string())
                                            .unwrap_or_else(|| "content".to_string())
                                    ),
                                );
                                return;
                            }
                        }

                        sort_paths(&mut directories);
                        sort_paths(&mut files);

                        if directories.is_empty() && files.is_empty() {
                            ui.label("This folder is empty.");
                            return;
                        }

                        for dir in &directories {
                            if let Some(name) = dir.file_name().and_then(|name| name.to_str()) {
                                let label = format!("📁 {name}");
                                let selected = self
                                    .selected_folder
                                    .as_ref()
                                    .map_or(false, |current| current == dir);
                                if ui.selectable_label(selected, label).clicked() {
                                    self.selected_folder = Some(dir.clone());
                                    self.feedback = None;
                                    self.new_folder_name.clear();
                                }
                            }
                        }

                        if !directories.is_empty() && !files.is_empty() {
                            ui.separator();
                        }

                        for file in files {
                            if let Some(name) = file.file_name().and_then(|name| name.to_str()) {
                                ui.label(format!("📄 {name}"));
                            }
                        }
                    });
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn show_folder_node(
        &mut self,
        ui: &mut egui::Ui,
        selected_folder: &Path,
        root: &Path,
        current: &Path,
    ) {
        use egui::collapsing_header::CollapsingState;
        use std::fs;

        let label = if current == root {
            "content".to_string()
        } else {
            current
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<folder>")
                .to_string()
        };

        let id = egui::Id::new(("asset_browser_folder", current.to_path_buf()));
        let state = CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            selected_folder.starts_with(current),
        );

        let is_selected = self
            .selected_folder
            .as_ref()
            .map_or(false, |selected| selected == current);

        let header_response = state.show_header(ui, |ui| {
            let response = ui.selectable_label(is_selected, label);
            if response.clicked() {
                self.selected_folder = Some(current.to_path_buf());
                self.feedback = None;
                self.new_folder_name.clear();
            }
            response
        });

        let _ = header_response.body(|ui| {
            let Ok(entries) = fs::read_dir(current) else {
                return;
            };

            let mut children: Vec<PathBuf> = entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect();

            sort_paths(&mut children);

            for child in children {
                self.show_folder_node(ui, selected_folder, root, &child);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn create_folder(&mut self, parent: &Path) {
        use std::fs;

        let name = self.new_folder_name.trim();
        if name.is_empty() {
            self.feedback = Some(Feedback::Error("Folder name cannot be empty.".to_string()));
            return;
        }

        if name.contains(['/', '\\', ':']) {
            self.feedback = Some(Feedback::Error(
                "Folder name cannot contain path separators or ':' characters.".to_string(),
            ));
            return;
        }

        let new_path = parent.join(name);

        if !parent.exists() {
            self.feedback = Some(Feedback::Error(
                "Selected folder no longer exists.".to_string(),
            ));
            return;
        }

        match fs::create_dir(&new_path) {
            Ok(()) => {
                self.feedback = Some(Feedback::Info(format!("Created folder '{name}'.")));
                self.new_folder_name.clear();
                self.selected_folder = Some(new_path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                self.feedback = Some(Feedback::Error(format!("Folder '{name}' already exists.")));
            }
            Err(err) => {
                self.feedback = Some(Feedback::Error(format!(
                    "Failed to create folder '{name}': {err}",
                )));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|a, b| {
        let a_name = a
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        let b_name = b
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        a_name.cmp(&b_name)
    });
}
