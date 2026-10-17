use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[cfg(not(target_arch = "wasm32"))]
use log::warn;

use crate::theme::{self, colors, icons};

/// Template for scripts created from the asset browser.
#[cfg(not(target_arch = "wasm32"))]
const NEW_SCRIPT_TEMPLATE: &str = "-- New Lua script\n\nfunction on_created(self_entity)\n    -- Initialization code here\nend\n\nfunction update(self_entity, dt)\n    -- Update code here\nend\n";

/// How long an info message stays in the status line.
#[cfg(not(target_arch = "wasm32"))]
const INFO_DURATION: Duration = Duration::from_secs(4);

/// Something the editor should do with a file the user opened.
#[derive(Debug, Clone)]
pub enum AssetBrowserAction {
    /// Open a text file (script, shader, JSON, ...) in the code editor.
    EditFile(PathBuf),
    /// Open a scene document in a tab.
    OpenScene(PathBuf),
    /// Import a model into the active scene.
    Import(PathBuf),
}

/// What a file is, judged by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Folder,
    Script,
    Scene,
    Material,
    Image,
    Model,
    Shader,
    Audio,
    Text,
    Other,
}

impl AssetKind {
    pub fn of(path: &Path, is_directory: bool) -> Self {
        if is_directory {
            return Self::Folder;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let in_scenes_folder = path
            .parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|parent| parent == "scenes");

        match extension.as_str() {
            "lua" => Self::Script,
            "wgsl" => Self::Shader,
            "json" if name.ends_with(".mat.json") => Self::Material,
            "json" if in_scenes_folder || name == "scene.json" => Self::Scene,
            "png" | "jpg" | "jpeg" | "tga" | "bmp" | "hdr" | "exr" | "ktx2" => Self::Image,
            "gltf" | "glb" | "fbx" | "blend" | "obj" => Self::Model,
            "wav" | "ogg" | "mp3" | "flac" | "ahx" | "hvl" | "a2m" | "asid" => Self::Audio,
            "json" | "txt" | "md" | "toml" | "ron" | "yaml" | "yml" | "csv" => Self::Text,
            _ => Self::Other,
        }
    }

    fn badge(self, path: &Path) -> (String, egui::Color32) {
        let extension = || {
            path.extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("?")
                .to_ascii_uppercase()
                .chars()
                .take(4)
                .collect::<String>()
        };
        match self {
            Self::Folder => (String::new(), colors::FOLDER),
            Self::Script => ("LUA".into(), colors::SCRIPT),
            Self::Scene => ("SCN".into(), colors::SCENE),
            Self::Material => ("MAT".into(), colors::MATERIAL),
            Self::Image => (extension(), colors::IMAGE),
            Self::Model => (extension(), colors::MODEL),
            Self::Shader => ("WGSL".into(), colors::SHADER),
            Self::Audio => (extension(), colors::AUDIO),
            Self::Text => (extension(), colors::TEXT_WEAK),
            Self::Other => (extension(), colors::WIDGET_ACTIVE),
        }
    }

    /// What double-clicking does, for tooltips.
    fn open_hint(self) -> Option<&'static str> {
        match self {
            Self::Folder => Some("Double-click to open the folder"),
            Self::Script | Self::Shader | Self::Text | Self::Material => {
                Some("Double-click to edit")
            }
            Self::Scene => Some("Double-click to open the scene"),
            Self::Model => Some("Double-click to import into the active scene"),
            Self::Image | Self::Audio | Self::Other => None,
        }
    }

    fn open_action(self, path: &Path) -> Option<AssetBrowserAction> {
        let path = path.to_path_buf();
        match self {
            Self::Script | Self::Shader | Self::Text | Self::Material => {
                Some(AssetBrowserAction::EditFile(path))
            }
            Self::Scene => Some(AssetBrowserAction::OpenScene(path)),
            Self::Model => Some(AssetBrowserAction::Import(path)),
            Self::Folder | Self::Image | Self::Audio | Self::Other => None,
        }
    }
}

#[derive(Default)]
pub struct AssetBrowserState {
    selected_folder: Option<PathBuf>,
    selected_item: Option<PathBuf>,
    renaming_item: Option<PathBuf>,
    rename_buffer: String,
    rename_needs_focus: bool,
    pending_delete: Option<PathBuf>,
    filter: String,
    feedback: Option<Feedback>,
    last_root: Option<PathBuf>,
    directory_cache: HashMap<PathBuf, DirectoryContents>,
    #[cfg(not(target_arch = "wasm32"))]
    watcher: Option<FileWatcher>,
}

#[derive(Debug)]
enum Feedback {
    #[cfg(not(target_arch = "wasm32"))]
    Info(String, Instant),
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
        self.feedback = Some(Feedback::Info(message.into(), Instant::now()));
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        content_root: Option<&Path>,
    ) -> Vec<AssetBrowserAction> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = content_root;
            theme::empty_state(
                ui,
                "Asset browser unavailable",
                "The web build has no file system access.",
            );
            Vec::new()
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(root) = content_root else {
                theme::empty_state(
                    ui,
                    "No project open",
                    "Open or create a project to browse assets.",
                );
                return Vec::new();
            };
            self.ui_native(ui, root)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ui_native(&mut self, ui: &mut egui::Ui, root: &Path) -> Vec<AssetBrowserAction> {
        self.ensure_watcher(ui, root);
        self.validate_state(root);

        let mut actions = Vec::new();
        let selected_folder = self.selected_folder(root);

        self.toolbar(ui, root, &selected_folder);
        ui.add_space(2.0);

        egui::Panel::left("asset_browser_folders")
            .resizable(true)
            .default_size(170.0)
            .size_range(110.0..=400.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 0,
                right: 6,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .id_salt("asset_browser_folders")
                    .show(ui, |ui| {
                        self.show_folder_node(ui, &selected_folder, root, root);
                    });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.status_line(ui);
            self.files_view(ui, root, &selected_folder, &mut actions);
        });

        self.handle_shortcuts(ui, &mut actions);
        self.delete_confirmation(ui.ctx(), root);

        actions
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn validate_state(&mut self, root: &Path) {
        if self.last_root.as_deref() != Some(root) {
            self.last_root = Some(root.to_path_buf());
            self.selected_folder = Some(root.to_path_buf());
            self.selected_item = None;
            self.renaming_item = None;
            self.pending_delete = None;
            self.feedback = None;
            self.filter.clear();
            self.rename_buffer.clear();
            self.directory_cache.clear();
        }

        if self
            .selected_folder
            .as_ref()
            .is_none_or(|selected| !selected.starts_with(root) || !selected.is_dir())
        {
            self.selected_folder = Some(root.to_path_buf());
        }

        if self
            .selected_item
            .as_ref()
            .is_some_and(|item| !item.starts_with(root) || !item.exists())
        {
            self.selected_item = None;
        }

        if let Some(Feedback::Info(_, shown_at)) = &self.feedback {
            if shown_at.elapsed() > INFO_DURATION {
                self.feedback = None;
            }
        }
    }

    /// Breadcrumbs for the open folder, a search field and the New menu.
    #[cfg(not(target_arch = "wasm32"))]
    fn toolbar(&mut self, ui: &mut egui::Ui, root: &Path, selected_folder: &Path) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;

            let up_enabled = selected_folder != root;
            if ui
                .add_enabled(up_enabled, egui::Button::new(icons::UP).frame(false))
                .on_hover_text("Parent folder")
                .clicked()
            {
                if let Some(parent) = selected_folder.parent() {
                    self.open_folder(parent);
                }
            }
            ui.add_space(4.0);

            let mut crumbs = vec![root.to_path_buf()];
            if let Ok(relative) = selected_folder.strip_prefix(root) {
                let mut current = root.to_path_buf();
                for part in relative.components() {
                    current.push(part);
                    crumbs.push(current.clone());
                }
            }
            let last = crumbs.len() - 1;
            for (index, crumb) in crumbs.iter().enumerate() {
                let name = if index == 0 {
                    "content".to_string()
                } else {
                    display_name(crumb)
                };
                let text = if index == last {
                    egui::RichText::new(name)
                        .strong()
                        .color(colors::TEXT_STRONG)
                } else {
                    egui::RichText::new(name).color(colors::TEXT_WEAK)
                };
                let response = ui.add(egui::Button::new(text).frame(false));
                if response.clicked() && index != last {
                    self.open_folder(crumb);
                }
                if let Some(source) = response.dnd_release_payload::<PathBuf>() {
                    self.move_entry(&source, crumb);
                }
                if index != last {
                    ui.label(theme::hint("›"));
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.menu_button(format!("{} New", icons::ADD), |ui| {
                    self.new_entry_menu(ui, selected_folder);
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(format!("{} Search", icons::SEARCH))
                        .desired_width(150.0),
                );
            });
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn new_entry_menu(&mut self, ui: &mut egui::Ui, folder: &Path) {
        if ui.button(format!("{}  Folder", icons::FOLDER)).clicked() {
            self.create_entry(folder, "New Folder", None);
            ui.close();
        }
        if ui
            .button(format!("{}  Lua Script", icons::SCRIPT))
            .clicked()
        {
            self.create_entry(folder, "new_script", Some(NEW_SCRIPT_TEMPLATE));
            ui.close();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let Some(feedback) = &self.feedback else {
            return;
        };
        let (text, color) = match feedback {
            Feedback::Info(message, _) => (message.clone(), colors::SUCCESS),
            Feedback::Error(message) => (format!("{} {message}", icons::WARNING), colors::ERROR),
        };
        let mut dismiss = false;
        ui.horizontal(|ui| {
            ui.colored_label(color, text);
            if matches!(feedback, Feedback::Error(_))
                && ui
                    .small_button(icons::CLOSE)
                    .on_hover_text("Dismiss")
                    .clicked()
            {
                dismiss = true;
            }
        });
        if dismiss {
            self.feedback = None;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn files_view(
        &mut self,
        ui: &mut egui::Ui,
        root: &Path,
        folder: &Path,
        actions: &mut Vec<AssetBrowserAction>,
    ) {
        let contents = match self.load_directory_contents(folder) {
            Ok(contents) => contents,
            Err(message) => {
                ui.colored_label(colors::ERROR, message);
                return;
            }
        };

        let filter = self.filter.trim().to_lowercase();
        let entries: Vec<(PathBuf, bool)> = if filter.is_empty() {
            contents
                .directories
                .iter()
                .map(|dir| (dir.clone(), true))
                .chain(contents.files.iter().map(|file| (file.clone(), false)))
                .collect()
        } else {
            // Searching covers the open folder and everything below it.
            let mut matches = Vec::new();
            self.collect_matches(folder, &filter, &mut matches);
            matches
        };

        let response = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .id_salt("asset_browser_files")
            .show(ui, |ui| {
                if entries.is_empty() {
                    let (title, detail) = if filter.is_empty() {
                        ("This folder is empty", "Use  ✚ New  or drop files here.")
                    } else {
                        ("No matches", "")
                    };
                    theme::empty_state(ui, title, detail);
                }
                for (path, is_directory) in &entries {
                    let label = if filter.is_empty() {
                        display_name(path)
                    } else {
                        path.strip_prefix(folder)
                            .map(|relative| relative.display().to_string())
                            .unwrap_or_else(|_| display_name(path))
                    };
                    self.show_entry(ui, root, path, &label, *is_directory, actions);
                }
                // The rest of the area takes clicks and context menus for the folder itself.
                ui.allocate_response(ui.available_size(), egui::Sense::click())
            });

        let background = response.inner;
        if background.clicked() {
            self.selected_item = None;
        }
        background.context_menu(|ui| self.new_entry_menu(ui, folder));
        if let Some(source) = background.dnd_release_payload::<PathBuf>() {
            self.move_entry(&source, folder);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn collect_matches(&mut self, folder: &Path, filter: &str, out: &mut Vec<(PathBuf, bool)>) {
        let Ok(contents) = self.load_directory_contents(folder) else {
            return;
        };
        let matches = |path: &Path| display_name(path).to_lowercase().contains(filter);
        for dir in &contents.directories {
            if matches(dir) {
                out.push((dir.clone(), true));
            }
        }
        for file in &contents.files {
            if matches(file) {
                out.push((file.clone(), false));
            }
        }
        for dir in &contents.directories {
            self.collect_matches(dir, filter, out);
        }
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
            display_name(current)
        };

        let id = egui::Id::new(("asset_browser_folder", current.to_path_buf()));
        let state = CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            current == root || selected_folder.starts_with(current),
        );

        let is_selected = selected_folder == current;
        let has_children = self
            .load_directory_contents(current)
            .is_ok_and(|contents| !contents.directories.is_empty());

        let show_label = |ui: &mut egui::Ui, this: &mut Self| {
            let text =
                egui::RichText::new(format!("{}  {label}", icons::FOLDER)).color(if is_selected {
                    colors::TEXT_STRONG
                } else {
                    colors::TEXT
                });
            let response = ui.selectable_label(is_selected, text);
            if response.clicked() {
                this.open_folder(current);
            }
            if let Some(source) = response.dnd_release_payload::<PathBuf>() {
                this.move_entry(&source, current);
            }
            if response.dnd_hover_payload::<PathBuf>().is_some() {
                ui.painter().rect_stroke(
                    response.rect,
                    4.0,
                    egui::Stroke::new(1.0, colors::ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
        };

        if !has_children {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                show_label(ui, self);
            });
            return;
        }

        state.show_header(ui, |ui| show_label(ui, self)).body(|ui| {
            if let Ok(contents) = self.load_directory_contents(current) {
                for child in &contents.directories {
                    self.show_folder_node(ui, selected_folder, root, child);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn show_entry(
        &mut self,
        ui: &mut egui::Ui,
        root: &Path,
        path: &Path,
        label: &str,
        is_directory: bool,
        actions: &mut Vec<AssetBrowserAction>,
    ) {
        let kind = AssetKind::of(path, is_directory);

        if self.renaming_item.as_deref() == Some(path) {
            self.show_rename_field(ui, path, kind);
            return;
        }

        let is_selected = self.selected_item.as_deref() == Some(path);
        let row_height = 22.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click_and_drag(),
        );

        let dropping_here = is_directory
            && response
                .dnd_hover_payload::<PathBuf>()
                .is_some_and(|source| source.as_path() != path);

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let fill = if is_selected {
                Some(colors::ACCENT_DIM)
            } else if response.hovered() || dropping_here {
                Some(colors::WIDGET)
            } else {
                None
            };
            if let Some(fill) = fill {
                painter.rect_filled(rect, 4.0, fill);
            }
            if dropping_here {
                painter.rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.0, colors::ACCENT),
                    egui::StrokeKind::Inside,
                );
            }

            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            if is_directory {
                child.label(egui::RichText::new(icons::FOLDER).color(colors::FOLDER));
            } else {
                let (badge, color) = kind.badge(path);
                theme::type_badge(&mut child, &badge, color);
            }
            child.add_space(2.0);
            child.add(
                egui::Label::new(egui::RichText::new(label).color(if is_selected {
                    colors::TEXT_STRONG
                } else {
                    colors::TEXT
                }))
                .selectable(false)
                .truncate(),
            );
        }

        if response.drag_started() {
            response.dnd_set_drag_payload(path.to_path_buf());
        }
        if response.dragged() {
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::new("asset_drag_preview"),
                egui::PopupAnchor::Pointer,
            )
            .gap(12.0)
            .show(|ui| {
                ui.label(format!("Move {}", display_name(path)));
            });
        }
        if is_directory {
            if let Some(source) = response.dnd_release_payload::<PathBuf>() {
                self.move_entry(&source, path);
            }
        }

        if response.clicked() || response.secondary_clicked() {
            self.selected_item = Some(path.to_path_buf());
        }
        if response.double_clicked() {
            self.open_entry(path, kind, actions);
        }

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let response = response.on_hover_ui(|ui| {
            ui.label(egui::RichText::new(&relative).monospace());
            if let Some(hint) = kind.open_hint() {
                ui.label(theme::hint(hint));
            }
        });

        response.context_menu(|ui| {
            if let Some(hint) = kind.open_hint() {
                let label = match kind {
                    AssetKind::Model => "Import into Scene",
                    _ if hint.ends_with("edit") => "Edit",
                    _ => "Open",
                };
                if ui.button(label).clicked() {
                    self.open_entry(path, kind, actions);
                    ui.close();
                }
                ui.separator();
            }
            if ui.button(format!("{}  Rename", icons::RENAME)).clicked() {
                self.start_rename(path);
                ui.close();
            }
            if ui
                .button(format!("{}  Duplicate", icons::DUPLICATE))
                .clicked()
            {
                self.duplicate_entry(path);
                ui.close();
            }
            if ui.button("Copy Path").clicked() {
                ui.ctx().copy_text(path.display().to_string());
                ui.close();
            }
            ui.separator();
            if ui
                .button(
                    egui::RichText::new(format!("{}  Delete", icons::DELETE)).color(colors::ERROR),
                )
                .clicked()
            {
                self.pending_delete = Some(path.to_path_buf());
                ui.close();
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn show_rename_field(&mut self, ui: &mut egui::Ui, path: &Path, kind: AssetKind) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if kind == AssetKind::Folder {
                ui.label(egui::RichText::new(icons::FOLDER).color(colors::FOLDER));
            } else {
                let (badge, color) = kind.badge(path);
                theme::type_badge(ui, &badge, color);
            }
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.rename_buffer)
                    .desired_width(ui.available_width().min(260.0)),
            );
            if self.rename_needs_focus {
                response.request_focus();
                self.rename_needs_focus = false;
            }

            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.cancel_rename();
            } else if response.lost_focus() {
                // Enter or clicking elsewhere both commit, like a file manager.
                self.finish_rename(path);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_shortcuts(&mut self, ui: &egui::Ui, actions: &mut Vec<AssetBrowserAction>) {
        if self.renaming_item.is_some() || !ui.ui_contains_pointer() {
            return;
        }
        if ui.ctx().memory(|memory| memory.focused().is_some()) {
            return;
        }
        let Some(item) = self.selected_item.clone() else {
            return;
        };
        ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::F2) {
                self.start_rename(&item);
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
                self.pending_delete = Some(item.clone());
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                let kind = AssetKind::of(&item, item.is_dir());
                self.open_entry(&item, kind, actions);
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn delete_confirmation(&mut self, ctx: &egui::Context, root: &Path) {
        let Some(path) = self.pending_delete.clone() else {
            return;
        };
        let is_directory = path.is_dir();
        let mut confirmed = false;
        let mut cancelled = false;

        let modal = egui::Modal::new(egui::Id::new("asset_delete_confirm")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading(format!("Delete {}?", display_name(&path)));
            ui.add_space(4.0);
            let detail = if is_directory {
                "The folder and everything in it will be deleted from disk."
            } else {
                "The file will be deleted from disk."
            };
            ui.label(detail);
            ui.label(theme::hint("This cannot be undone."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let delete =
                    egui::Button::new(egui::RichText::new("Delete").color(colors::TEXT_STRONG))
                        .fill(colors::ERROR.gamma_multiply(0.6));
                if ui.add(delete).clicked() || ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    confirmed = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

        if confirmed {
            self.delete_entry(root, &path);
            self.pending_delete = None;
        } else if cancelled || modal.should_close() {
            self.pending_delete = None;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_folder(&mut self, folder: &Path) {
        self.selected_folder = Some(folder.to_path_buf());
        self.selected_item = None;
        self.filter.clear();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_entry(&mut self, path: &Path, kind: AssetKind, actions: &mut Vec<AssetBrowserAction>) {
        if kind == AssetKind::Folder {
            self.open_folder(path);
        } else if let Some(action) = kind.open_action(path) {
            actions.push(action);
        } else {
            self.report_info(format!("No editor for {} files.", display_name(path)));
        }
    }

    /// Create a folder (no `contents`) or a file with a unique name, then rename it.
    #[cfg(not(target_arch = "wasm32"))]
    fn create_entry(&mut self, parent: &Path, base: &str, contents: Option<&str>) {
        let extension = contents.map(|_| "lua");
        let path = unique_path(parent, base, extension);
        let result = match contents {
            Some(contents) => std::fs::write(&path, contents),
            None => std::fs::create_dir(&path),
        };
        match result {
            Ok(()) => {
                self.directory_cache.clear();
                self.selected_item = Some(path.clone());
                self.start_rename(&path);
            }
            Err(err) => {
                self.report_error(format!("Failed to create {}: {err}", display_name(&path)));
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
                    self.report_error(format!("Failed to watch content folder: {err}"));
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
                    let hidden = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with('.'));
                    if hidden {
                        continue;
                    }
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

    #[cfg(not(target_arch = "wasm32"))]
    fn start_rename(&mut self, path: &Path) {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            // Edit the stem only; the extension is kept on commit.
            let editable = if path.is_dir() {
                name
            } else {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(name)
            };
            self.renaming_item = Some(path.to_path_buf());
            self.rename_buffer = editable.to_string();
            self.rename_needs_focus = true;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn finish_rename(&mut self, original: &Path) {
        use std::fs;

        if self.renaming_item.as_deref() != Some(original) {
            return;
        }

        let new_name = self.rename_buffer.trim().to_string();
        if new_name.is_empty() {
            self.cancel_rename();
            return;
        }

        if new_name.contains(['/', '\\', ':']) {
            self.report_error("Names cannot contain path separators or ':' characters.");
            self.rename_needs_focus = true;
            return;
        }

        let Some(parent) = original.parent() else {
            self.report_error("Cannot rename this item.");
            self.cancel_rename();
            return;
        };

        let extension = if original.is_dir() {
            None
        } else {
            original.extension().and_then(|ext| ext.to_str())
        };
        let candidate = parent.join(apply_extension(&new_name, extension));

        if candidate == original {
            self.cancel_rename();
            return;
        }

        if candidate.exists() {
            self.report_error(format!("'{}' already exists.", display_name(&candidate)));
            self.rename_needs_focus = true;
            return;
        }

        match fs::rename(original, &candidate) {
            Ok(_) => {
                self.cancel_rename();
                if self.selected_item.as_deref() == Some(original) {
                    self.selected_item = Some(candidate.clone());
                }
                if let Some(folder) = self.selected_folder.as_mut() {
                    if let Ok(rest) = folder.strip_prefix(original) {
                        *folder = candidate.join(rest);
                    }
                }
                self.directory_cache.clear();
            }
            Err(err) => {
                self.report_error(format!(
                    "Failed to rename '{}': {err}",
                    display_name(original)
                ));
                self.cancel_rename();
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
    fn duplicate_entry(&mut self, path: &Path) {
        use std::fs;

        if !path.exists() {
            self.report_error("Item no longer exists.");
            return;
        }

        let Some(parent) = path.parent() else {
            self.report_error("Cannot duplicate this item.");
            return;
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
                self.report_info(format!("Duplicated '{}'", display_name(path)));
                self.selected_item = Some(destination.clone());
                self.directory_cache.clear();
            }
            Err(err) => {
                self.report_error(format!(
                    "Failed to duplicate '{}': {err}",
                    display_name(path)
                ));
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn delete_entry(&mut self, root: &Path, path: &Path) {
        use std::fs;

        if !path.exists() {
            self.report_error("Item no longer exists.");
            return;
        }

        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };

        match result {
            Ok(()) => {
                self.report_info(format!("Deleted '{}'", display_name(path)));
                if self.selected_item.as_deref() == Some(path) {
                    self.selected_item = None;
                }
                if self
                    .selected_folder
                    .as_deref()
                    .is_some_and(|folder| folder.starts_with(path))
                {
                    self.selected_folder = Some(root.to_path_buf());
                }
                self.directory_cache.clear();
            }
            Err(err) => {
                self.report_error(format!("Failed to delete '{}': {err}", display_name(path)));
            }
        }
    }

    /// Move `source` into `target_folder` (drag and drop).
    #[cfg(not(target_arch = "wasm32"))]
    fn move_entry(&mut self, source: &Path, target_folder: &Path) {
        use std::fs;

        if !source.exists() || !target_folder.is_dir() {
            return;
        }
        if source == target_folder || source.parent() == Some(target_folder) {
            return;
        }
        if target_folder.starts_with(source) {
            self.report_error("Cannot move a folder into itself.");
            return;
        }

        let (base, extension) = split_name(source);
        let mut destination = target_folder.join(apply_extension(base, extension));
        if destination.exists() {
            destination = unique_path(target_folder, base, extension);
        }

        match fs::rename(source, &destination) {
            Ok(_) => {
                self.report_info(format!(
                    "Moved '{}' to '{}'",
                    display_name(source),
                    display_name(target_folder)
                ));
                if self.selected_item.as_deref() == Some(source) {
                    self.selected_item = Some(destination.clone());
                }
                if let Some(folder) = self.selected_folder.as_mut() {
                    if let Ok(rest) = folder.strip_prefix(source) {
                        *folder = destination.join(rest);
                    }
                }
                self.directory_cache.clear();
            }
            Err(err) => {
                self.report_error(format!("Failed to move '{}': {err}", display_name(source)));
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

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<item>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::AssetKind;
    use std::path::Path;

    #[test]
    fn classifies_project_files() {
        let kind = |path: &str| AssetKind::of(Path::new(path), false);
        assert_eq!(kind("content/scripts/spin.lua"), AssetKind::Script);
        assert_eq!(kind("content/scenes/Pj2AKgLtSVNj.json"), AssetKind::Scene);
        assert_eq!(kind("content/materials/a.mat.json"), AssetKind::Material);
        assert_eq!(kind("content/notes.json"), AssetKind::Text);
        assert_eq!(kind("content/models/ship.GLB"), AssetKind::Model);
        assert_eq!(kind("content/environment/sky.hdr"), AssetKind::Image);
        assert_eq!(kind("content/shaders/water.wgsl"), AssetKind::Shader);
        assert_eq!(kind("content/blob.bin"), AssetKind::Other);
        assert_eq!(
            AssetKind::of(Path::new("content/x"), true),
            AssetKind::Folder
        );
    }
}
