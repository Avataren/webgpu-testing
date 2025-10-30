use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
use log::debug;
#[cfg(not(target_arch = "wasm32"))]
use log::warn;

#[derive(Default)]
pub struct AssetBrowserState {
    selected_folder: Option<PathBuf>,
    selected_item: Option<PathBuf>,
    renaming_item: Option<PathBuf>,
    pending_move: Option<PathBuf>,
    rename_buffer: String,
    rename_needs_focus: bool,
    new_folder_name: String,
    feedback: Option<Feedback>,
    last_root: Option<PathBuf>,
    directory_cache: HashMap<PathBuf, DirectoryContents>,
    #[cfg(not(target_arch = "wasm32"))]
    watcher: Option<FileWatcher>,
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
        let Some(root) = content_root else {
            ui.label("Open or create a project to browse assets.");
            return;
        };

        self.ensure_watcher(ui, root);

        if self
            .last_root
            .as_ref()
            .map(|previous| previous != root)
            .unwrap_or(true)
        {
            self.last_root = Some(root.to_path_buf());
            self.selected_folder = Some(root.to_path_buf());
            self.selected_item = None;
            self.renaming_item = None;
            self.pending_move = None;
            self.feedback = None;
            self.new_folder_name.clear();
            self.rename_buffer.clear();
            self.directory_cache.clear();
        }

        if let Some(selected) = self.selected_folder.clone() {
            if !selected.starts_with(root) || !selected.is_dir() {
                self.selected_folder = Some(root.to_path_buf());
            }
        } else {
            self.selected_folder = Some(root.to_path_buf());
        }

        if let Some(item) = self.selected_item.clone() {
            if !item.starts_with(root) || !item.exists() {
                if self.pending_move.as_deref() == Some(item.as_path()) {
                    self.pending_move = None;
                }
                self.selected_item = None;
            }
        }

        if let Some(pending) = self.pending_move.clone() {
            if !pending.exists() {
                self.pending_move = None;
            }
        }

        let selected_folder = self
            .selected_folder
            .clone()
            .unwrap_or_else(|| root.to_path_buf());

        let mut available_size = ui.available_size();
        if !available_size.y.is_finite() {
            available_size.y = ui.available_height();
        }
        let available_height = available_size.y.max(0.0);
        ui.allocate_ui_with_layout(
            available_size,
            egui::Layout::left_to_right(egui::Align::TOP),
            |ui| {
                ui.set_min_size(available_size);

                let folder_width = 200.0;
                ui.vertical(|ui| {
                    ui.set_width(folder_width);
                    ui.set_min_height(available_height);
                    ui.heading("Folders");
                    ui.separator();

                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .id_salt("asset_browser_folders")
                        .show(ui, |ui| {
                            self.show_folder_node(ui, &selected_folder, root, root);
                        });
                });

                ui.separator();

                ui.vertical(|ui| {
                    let mut files_width = ui.available_width();
                    if !files_width.is_finite() {
                        files_width = (available_size.x - folder_width
                            - ui.spacing().item_spacing.x)
                            .max(220.0);
                    }

                    if files_width > 0.0 && files_width.is_finite() {
                        ui.set_width(files_width);
                    } else {
                        ui.set_min_width(220.0);
                        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                        debug!(
                            "Asset browser file pane received non-positive width (available={:?})",
                            available_size
                        );
                    }

                    ui.set_min_height(available_height);
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
                        .auto_shrink([false; 2])
                        .id_salt("asset_browser_files")
                        .show(ui, |ui| {
                            if let Some(pending) = self.pending_move.as_ref() {
                                if let Some(name) = pending
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .map(|name| name.to_string())
                                {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("Moving: {name}"));
                                        if ui.button("Move Here").clicked()
                                            && self.apply_move(&selected_folder)
                                        {
                                            ui.ctx().request_repaint();
                                        }
                                        if ui.button("Cancel").clicked() {
                                            self.pending_move = None;
                                            self.feedback = None;
                                        }
                                    });
                                    ui.separator();
                                }
                            }

                            if let Some(selected_item) = self.selected_item.clone() {
                                if selected_item.starts_with(&selected_folder) {
                                    let display = display_name(&selected_item);
                                    ui.horizontal(|ui| {
                                        ui.label(format!("Selected: {display}"));
                                        if ui.button("Rename").clicked() {
                                            self.start_rename(&selected_item);
                                        }
                                        if ui.button("Duplicate").clicked()
                                            && self.duplicate_entry(&selected_item)
                                        {
                                            ui.ctx().request_repaint();
                                        }
                                        if ui.button("Move").clicked() {
                                            self.begin_move(&selected_item);
                                        }
                                        if ui.button("Delete").clicked()
                                            && self.delete_entry(root, &selected_item)
                                        {
                                            ui.ctx().request_repaint();
                                        }
                                    });
                                    ui.separator();
                                }
                            }

                            match self.load_directory_contents(&selected_folder) {
                                Ok(contents) => {
                                    if contents.directories.is_empty() && contents.files.is_empty()
                                    {
                                        ui.label("This folder is empty.");
                                        return;
                                    }

                                    for dir in &contents.directories {
                                        self.show_entry(ui, root, dir.as_path(), true);
                                    }

                                    if !contents.directories.is_empty()
                                        && !contents.files.is_empty()
                                    {
                                        ui.separator();
                                    }

                                    for file in &contents.files {
                                        self.show_entry(ui, root, file.as_path(), false);
                                    }
                                }
                                Err(message) => {
                                    ui.colored_label(egui::Color32::from_rgb(200, 80, 80), message);
                                }
                            }
                        });

                    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
                    if ui.button("Debug Layout Bounds").clicked() {
                        debug!(
                            "Asset browser layout → available: {:?}, folder_width: {folder_width}, files_width: {files_width}",
                            available_size
                        );
                        ui.ctx().set_debug_on_hover(true);
                    }
                });
            },
        );
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
            .is_some_and(|selected| selected == current);

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
            if let Ok(contents) = self.load_directory_contents(current) {
                for child in &contents.directories {
                    self.show_folder_node(ui, selected_folder, root, child);
                }
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

    #[cfg(not(target_arch = "wasm32"))]
    fn ensure_watcher(&mut self, ui: &egui::Ui, root: &Path) {
        let needs_new = self
            .watcher
            .as_ref()
            .map(|watcher| watcher.root() != root)
            .unwrap_or(true);

        if needs_new {
            match FileWatcher::new(root) {
                Ok(watcher) => {
                    self.watcher = Some(watcher);
                    self.directory_cache.clear();
                }
                Err(err) => {
                    self.feedback = Some(Feedback::Error(format!(
                        "Failed to watch content folder: {err}",
                    )));
                    self.watcher = None;
                }
            }
        }

        if let Some(watcher) = self.watcher.as_mut() {
            if watcher.poll_events() {
                self.directory_cache.clear();
                ui.ctx().request_repaint();
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_directory_contents(&mut self, folder: &Path) -> Result<DirectoryContents, String> {
        if let Some(contents) = self.directory_cache.get(folder) {
            return Ok(contents.clone());
        }

        let mut directories = Vec::new();
        let mut files = Vec::new();

        match std::fs::read_dir(folder) {
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
                return Err(format!("Failed to read folder {}: {err}", folder.display()));
            }
        }

        sort_paths(&mut directories);
        sort_paths(&mut files);

        let contents = DirectoryContents { directories, files };

        self.directory_cache
            .insert(folder.to_path_buf(), contents.clone());

        Ok(contents)
    }

    #[cfg(target_arch = "wasm32")]
    fn load_directory_contents(&mut self, _folder: &Path) -> Result<DirectoryContents, String> {
        Err("Asset browser is unavailable on WebAssembly.".to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn show_entry(&mut self, ui: &mut egui::Ui, root: &Path, path: &Path, is_directory: bool) {
        let is_selected = self
            .selected_item
            .as_ref()
            .is_some_and(|selected| selected == path);
        let is_renaming = self
            .renaming_item
            .as_ref()
            .is_some_and(|renaming| renaming == path);

        ui.horizontal(|ui| {
            if is_renaming {
                let response = ui
                    .add(egui::TextEdit::singleline(&mut self.rename_buffer).desired_width(200.0));
                if self.rename_needs_focus {
                    response.request_focus();
                    self.rename_needs_focus = false;
                }

                let commit =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));

                if commit {
                    if self.finish_rename(path) {
                        ui.ctx().request_repaint();
                    }
                } else if cancel {
                    self.cancel_rename();
                }
            } else {
                let icon = if is_directory { "📁" } else { "📄" };
                let label = format!("{icon} {}", display_name(path));
                let response = ui.selectable_label(is_selected, label);
                if response.clicked() {
                    self.selected_item = Some(path.to_path_buf());
                    self.feedback = None;
                }
                if response.double_clicked() && is_directory {
                    self.selected_folder = Some(path.to_path_buf());
                    self.selected_item = None;
                    self.feedback = None;
                    self.new_folder_name.clear();
                }
                response.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        self.selected_folder = Some(path.to_path_buf());
                        self.selected_item = None;
                        self.feedback = None;
                        self.new_folder_name.clear();
                        ui.close();
                    }
                    if ui.button("Rename").clicked() {
                        self.start_rename(path);
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() && self.duplicate_entry(path) {
                        ui.ctx().request_repaint();
                        ui.close();
                    }
                    if ui.button("Move").clicked() {
                        self.begin_move(path);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() && self.delete_entry(root, path) {
                        ui.ctx().request_repaint();
                        ui.close();
                    }
                });
            }
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn show_entry(&mut self, _ui: &mut egui::Ui, _root: &Path, _path: &Path, _is_directory: bool) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn start_rename(&mut self, path: &Path) {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            self.renaming_item = Some(path.to_path_buf());
            self.rename_buffer = name.to_string();
            self.rename_needs_focus = true;
            self.feedback = None;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn finish_rename(&mut self, original: &Path) -> bool {
        use std::fs;

        let Some(current) = self.renaming_item.clone() else {
            return false;
        };

        if current != original {
            return false;
        }

        let new_name = self.rename_buffer.trim();
        if new_name.is_empty() {
            self.feedback = Some(Feedback::Error("Name cannot be empty.".to_string()));
            return false;
        }

        if new_name.contains(['/', '\\', ':']) {
            self.feedback = Some(Feedback::Error(
                "Names cannot contain path separators or ':' characters.".to_string(),
            ));
            return false;
        }

        let Some(parent) = original.parent() else {
            self.feedback = Some(Feedback::Error("Cannot rename this item.".to_string()));
            return false;
        };

        let extension = original.extension().and_then(|ext| ext.to_str());
        let candidate = parent.join(apply_extension(new_name, extension));

        if candidate == original {
            self.cancel_rename();
            return false;
        }

        if candidate.exists() {
            self.feedback = Some(Feedback::Error(
                "Another item with that name already exists.".to_string(),
            ));
            return false;
        }

        match fs::rename(original, &candidate) {
            Ok(_) => {
                self.feedback = Some(Feedback::Info(format!(
                    "Renamed '{}' to '{}'",
                    display_name(original),
                    display_name(&candidate)
                )));
                self.renaming_item = None;
                self.rename_buffer.clear();
                if self.selected_item.as_deref() == Some(original) {
                    self.selected_item = Some(candidate.clone());
                }
                if self.selected_folder.as_deref() == Some(original) {
                    self.selected_folder = Some(candidate.clone());
                }
                if self.pending_move.as_deref() == Some(original) {
                    self.pending_move = Some(candidate.clone());
                }
                self.directory_cache.clear();
                true
            }
            Err(err) => {
                self.feedback = Some(Feedback::Error(format!(
                    "Failed to rename '{}': {err}",
                    display_name(original)
                )));
                false
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn cancel_rename(&mut self) {
        self.renaming_item = None;
        self.rename_buffer.clear();
        self.rename_needs_focus = false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn duplicate_entry(&mut self, path: &Path) -> bool {
        use std::fs;

        if !path.exists() {
            self.feedback = Some(Feedback::Error("Item no longer exists.".to_string()));
            return false;
        }

        let Some(parent) = path.parent() else {
            self.feedback = Some(Feedback::Error("Cannot duplicate this item.".to_string()));
            return false;
        };

        let (base, extension) = split_name(path);
        let copy_base = format!("{base} copy");
        let destination = unique_path(parent, &copy_base, extension);

        let result = if path.is_dir() {
            copy_directory(path, &destination)
        } else {
            fs::copy(path, &destination).map(|_| ())
        };

        match result {
            Ok(()) => {
                self.feedback = Some(Feedback::Info(format!(
                    "Duplicated '{}'",
                    display_name(path)
                )));
                self.selected_item = Some(destination.clone());
                self.directory_cache.clear();
                true
            }
            Err(err) => {
                self.feedback = Some(Feedback::Error(format!(
                    "Failed to duplicate '{}': {err}",
                    display_name(path)
                )));
                false
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn delete_entry(&mut self, root: &Path, path: &Path) -> bool {
        use std::fs;

        if !path.exists() {
            self.feedback = Some(Feedback::Error("Item no longer exists.".to_string()));
            return false;
        }

        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };

        match result {
            Ok(()) => {
                self.feedback = Some(Feedback::Info(format!("Deleted '{}'", display_name(path))));
                if self.selected_item.as_deref() == Some(path) {
                    self.selected_item = None;
                }
                if self.pending_move.as_deref() == Some(path) {
                    self.pending_move = None;
                }
                if self
                    .selected_folder
                    .as_deref()
                    .is_some_and(|folder| folder.starts_with(path))
                {
                    self.selected_folder = Some(root.to_path_buf());
                }
                self.directory_cache.clear();
                true
            }
            Err(err) => {
                self.feedback = Some(Feedback::Error(format!(
                    "Failed to delete '{}': {err}",
                    display_name(path)
                )));
                false
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn begin_move(&mut self, path: &Path) {
        if path.exists() {
            self.pending_move = Some(path.to_path_buf());
            self.feedback = Some(Feedback::Info(
                "Select a destination folder and click 'Move Here'.".to_string(),
            ));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn apply_move(&mut self, target_folder: &Path) -> bool {
        use std::fs;

        let Some(source) = self.pending_move.clone() else {
            return false;
        };

        if !source.exists() {
            self.feedback = Some(Feedback::Error("Item no longer exists.".to_string()));
            self.pending_move = None;
            return false;
        }

        if !target_folder.exists() {
            self.feedback = Some(Feedback::Error(
                "Destination folder no longer exists.".to_string(),
            ));
            return false;
        }

        if source == target_folder {
            self.feedback = Some(Feedback::Error(
                "Cannot move a folder into itself.".to_string(),
            ));
            return false;
        }

        if target_folder.starts_with(&source) {
            self.feedback = Some(Feedback::Error(
                "Cannot move a folder into one of its subfolders.".to_string(),
            ));
            return false;
        }

        if source
            .parent()
            .is_some_and(|parent| parent == target_folder)
        {
            self.feedback = Some(Feedback::Info(
                "Item is already in this folder.".to_string(),
            ));
            self.pending_move = None;
            return false;
        }

        if source.file_name().is_none() {
            self.feedback = Some(Feedback::Error("Cannot move this item.".to_string()));
            return false;
        }

        let (base, extension) = split_name(&source);
        let candidate_base = base.to_string();
        let mut destination = target_folder.join(apply_extension(&candidate_base, extension));
        if destination.exists() {
            destination = unique_path(target_folder, &candidate_base, extension);
        }

        match fs::rename(&source, &destination) {
            Ok(_) => {
                self.feedback = Some(Feedback::Info(format!("Moved '{}'", display_name(&source))));
                if self.selected_item.as_deref() == Some(source.as_path()) {
                    self.selected_item = Some(destination.clone());
                }
                if self.selected_folder.as_deref() == Some(source.as_path()) {
                    self.selected_folder = Some(destination.clone());
                }
                self.pending_move = None;
                self.directory_cache.clear();
                true
            }
            Err(err) => {
                self.feedback = Some(Feedback::Error(format!(
                    "Failed to move '{}': {err}",
                    display_name(&source)
                )));
                false
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sort_paths(paths: &mut [PathBuf]) {
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

#[derive(Clone, Default)]
struct DirectoryContents {
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
struct FileWatcher {
    _watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<notify::Event>>,
    root: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileWatcher {
    fn new(root: &Path) -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let mut watcher = recommended_watcher(move |res| {
            let _ = sender.send(res);
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;

        Ok(Self {
            _watcher: watcher,
            receiver,
            root: root.to_path_buf(),
        })
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn poll_events(&mut self) -> bool {
        let mut dirty = false;
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(_event)) => {
                    dirty = true;
                }
                Ok(Err(err)) => {
                    warn!("File watcher error: {err}");
                    dirty = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("File watcher disconnected");
                    break;
                }
            }
        }
        dirty
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn split_name(path: &Path) -> (&str, Option<&str>) {
    let base = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .unwrap_or("item");
    let extension = path.extension().and_then(|ext| ext.to_str());
    (base, extension)
}

#[cfg(not(target_arch = "wasm32"))]
fn unique_path(parent: &Path, base: &str, extension: Option<&str>) -> PathBuf {
    let mut counter = 1;
    loop {
        let name = if counter == 1 {
            apply_extension(base, extension)
        } else {
            apply_extension(&format!("{base} {counter}"), extension)
        };
        let candidate = parent.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_extension(base: &str, extension: Option<&str>) -> String {
    match extension {
        Some(ext) if !ext.is_empty() => format!("{base}.{ext}"),
        _ => base.to_string(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::fs;

    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target = destination.join(entry.file_name());
        if entry_path.is_dir() {
            copy_directory(&entry_path, &target)?;
        } else {
            fs::copy(&entry_path, &target)?;
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<item>")
        .to_string()
}
