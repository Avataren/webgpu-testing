//! Tabbed code editor for Lua scripts, shaders and other text files.
//!
//! Documents are either files on disk (saved in place; the file watcher hot-reloads
//! scripts that use them) or inline scripts stored in an entity's script component.

use std::io;
#[cfg(target_arch = "wasm32")]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant, SystemTime};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use hecs::Entity;

use wgpu_cube::scripting::{LuaScriptComponent, LuaScriptSource};

use crate::theme::{self, colors, icons};

/// Lua 5.4 reserved words.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Font used for code and the line-number gutter; both must match so rows line up.
const CODE_FONT_SIZE: f32 = 14.0;

/// How often clean file documents check whether the file changed on disk.
#[cfg(not(target_arch = "wasm32"))]
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(1);

pub enum ScriptEditorEvent {
    None,
    SaveInline {
        entity: Entity,
        name: String,
        contents: String,
        message: String,
    },
}

#[derive(Clone, PartialEq, Eq)]
enum DocumentSource {
    Inline { entity: Entity, name: String },
    File { path: PathBuf },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Lua,
    Wgsl,
    Plain,
}

struct Document {
    source: DocumentSource,
    buffer: String,
    original: String,
    status: Option<StatusMessage>,
    target_missing: bool,
    save_in_progress: bool,
    #[cfg(not(target_arch = "wasm32"))]
    disk_modified: Option<SystemTime>,
    #[cfg(not(target_arch = "wasm32"))]
    last_disk_check: Option<Instant>,
}

impl Document {
    fn inline(entity: Entity, name: &str, source: &str) -> Self {
        Self::new(
            DocumentSource::Inline {
                entity,
                name: name.to_string(),
            },
            source.to_string(),
            None,
        )
    }

    fn file(path: &Path) -> Self {
        let (contents, status) = match read_file_to_string(path) {
            Ok(contents) => (contents, None),
            Err(err) => (
                String::new(),
                Some(StatusMessage::error(format!("Failed to load: {err}"))),
            ),
        };
        let mut document = Self::new(
            DocumentSource::File {
                path: path.to_path_buf(),
            },
            contents,
            status,
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            document.disk_modified = modified_time(path);
        }
        document
    }

    fn new(source: DocumentSource, contents: String, status: Option<StatusMessage>) -> Self {
        Self {
            source,
            original: contents.clone(),
            buffer: contents,
            status,
            target_missing: false,
            save_in_progress: false,
            #[cfg(not(target_arch = "wasm32"))]
            disk_modified: None,
            #[cfg(not(target_arch = "wasm32"))]
            last_disk_check: None,
        }
    }

    fn is_dirty(&self) -> bool {
        self.buffer != self.original
    }

    fn title(&self) -> String {
        match &self.source {
            DocumentSource::Inline { name, .. } => name.clone(),
            DocumentSource::File { path } => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        }
    }

    fn description(&self) -> String {
        match &self.source {
            DocumentSource::Inline { entity, .. } => {
                format!("Inline script on entity {entity:?}")
            }
            DocumentSource::File { path } => path.display().to_string(),
        }
    }

    /// Like [`Self::description`], with project files relative to the content folder.
    fn short_description(&self, content_root: Option<&Path>) -> String {
        match (&self.source, content_root) {
            (DocumentSource::File { path }, Some(root)) => path
                .strip_prefix(root)
                .map(|relative| relative.display().to_string())
                .unwrap_or_else(|_| self.description()),
            _ => self.description(),
        }
    }

    fn language(&self) -> Language {
        match &self.source {
            DocumentSource::Inline { .. } => Language::Lua,
            DocumentSource::File { path } => match path.extension().and_then(|ext| ext.to_str()) {
                Some(ext) if ext.eq_ignore_ascii_case("lua") => Language::Lua,
                Some(ext) if ext.eq_ignore_ascii_case("wgsl") => Language::Wgsl,
                _ => Language::Plain,
            },
        }
    }

    fn inline_entity(&self) -> Option<Entity> {
        match self.source {
            DocumentSource::Inline { entity, .. } => Some(entity),
            DocumentSource::File { .. } => None,
        }
    }

    /// Save the buffer: files are written now, inline scripts go through the editor.
    fn save(&mut self) -> ScriptEditorEvent {
        if !self.is_dirty() || self.save_in_progress || self.target_missing {
            return ScriptEditorEvent::None;
        }
        match self.source.clone() {
            DocumentSource::Inline { entity, name } => {
                self.save_in_progress = true;
                self.status = Some(StatusMessage::info("Saving..."));
                ScriptEditorEvent::SaveInline {
                    entity,
                    contents: self.buffer.clone(),
                    message: format!("Saved inline script `{name}`."),
                    name,
                }
            }
            DocumentSource::File { path } => {
                match write_file(&path, &self.buffer) {
                    Ok(()) => {
                        self.original = self.buffer.clone();
                        self.status = Some(StatusMessage::info("Saved."));
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.disk_modified = modified_time(&path);
                        }
                    }
                    Err(err) => {
                        self.status = Some(StatusMessage::error(format!("Failed to save: {err}")));
                    }
                }
                ScriptEditorEvent::None
            }
        }
    }

    /// Pick up changes other programs made to the file.
    #[cfg(not(target_arch = "wasm32"))]
    fn check_disk(&mut self) {
        let DocumentSource::File { path } = &self.source else {
            return;
        };
        if self
            .last_disk_check
            .is_some_and(|checked| checked.elapsed() < DISK_CHECK_INTERVAL)
        {
            return;
        }
        self.last_disk_check = Some(Instant::now());

        let modified = modified_time(path);
        if modified.is_none() || modified == self.disk_modified {
            return;
        }
        if self.is_dirty() {
            self.status = Some(StatusMessage::error(
                "The file changed on disk. Saving will overwrite those changes.",
            ));
            self.disk_modified = modified;
            return;
        }
        if let Ok(contents) = read_file_to_string(path) {
            self.disk_modified = modified;
            if contents != self.original {
                self.original = contents.clone();
                self.buffer = contents;
                self.status = Some(StatusMessage::info("Reloaded: the file changed on disk."));
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn check_disk(&mut self) {}
}

/// What a pending close confirmation is for.
#[derive(Clone, Copy)]
enum CloseRequest {
    Tab(usize),
    Window,
}

#[derive(Default)]
pub struct ScriptEditorState {
    documents: Vec<Document>,
    active: usize,
    confirm_close: Option<CloseRequest>,
    /// Whether the editor had keyboard focus or the pointer last frame.
    wants_keyboard: bool,
}

impl ScriptEditorState {
    pub fn is_open(&self) -> bool {
        !self.documents.is_empty()
    }

    /// True while the editor should receive shortcuts such as Ctrl+S.
    pub fn wants_keyboard(&self) -> bool {
        self.is_open() && self.wants_keyboard
    }

    /// Open (or focus) an entity's inline script.
    pub fn open_inline(&mut self, entity: Entity, component: &LuaScriptComponent) {
        if let LuaScriptSource::File { path } = component.source() {
            self.open_file(path);
            return;
        }
        if let Some(index) = self
            .documents
            .iter()
            .position(|doc| doc.inline_entity() == Some(entity))
        {
            self.active = index;
            self.sync_inline(entity, Ok(component));
            return;
        }
        if let LuaScriptSource::Inline { name, source } = component.source() {
            self.documents.push(Document::inline(entity, name, source));
            self.active = self.documents.len() - 1;
        }
    }

    /// Open (or focus) a file.
    pub fn open_file(&mut self, path: &Path) {
        let wanted = canonical(path);
        if let Some(index) = self.documents.iter().position(|doc| match &doc.source {
            DocumentSource::File { path } => canonical(path) == wanted,
            DocumentSource::Inline { .. } => false,
        }) {
            self.active = index;
            return;
        }
        self.documents.push(Document::file(path));
        self.active = self.documents.len() - 1;
    }

    /// Entities whose inline scripts are open, to be checked with [`Self::sync_inline`].
    pub fn inline_entities(&self) -> Vec<Entity> {
        self.documents
            .iter()
            .filter_map(Document::inline_entity)
            .collect()
    }

    /// Update an inline document from its entity's script component, or mark it
    /// unavailable with the reason.
    pub fn sync_inline(&mut self, entity: Entity, component: Result<&LuaScriptComponent, &str>) {
        for doc in &mut self.documents {
            if doc.inline_entity() != Some(entity) {
                continue;
            }
            let component = match component {
                Ok(component) => component,
                Err(reason) => {
                    if !doc.target_missing {
                        doc.target_missing = true;
                        doc.save_in_progress = false;
                        doc.status = Some(StatusMessage::error(reason));
                    }
                    continue;
                }
            };
            let LuaScriptSource::Inline { name, source } = component.source() else {
                if !doc.target_missing {
                    doc.target_missing = true;
                    doc.status = Some(StatusMessage::error(
                        "The entity now uses a script file instead.",
                    ));
                }
                continue;
            };
            if doc.target_missing {
                doc.target_missing = false;
                doc.status = None;
            }
            if doc.save_in_progress || doc.is_dirty() {
                continue;
            }
            if doc.original != *source {
                doc.original = source.to_string();
                doc.buffer = source.to_string();
                doc.status = Some(StatusMessage::info("Script updated."));
            }
            if let DocumentSource::Inline { name: doc_name, .. } = &mut doc.source {
                if doc_name != name {
                    *doc_name = name.to_string();
                }
            }
        }
    }

    pub fn finish_save(&mut self, entity: Entity, message: impl Into<String>) {
        let message = message.into();
        for doc in &mut self.documents {
            if doc.inline_entity() == Some(entity) && doc.save_in_progress {
                doc.save_in_progress = false;
                doc.original = doc.buffer.clone();
                doc.status = Some(StatusMessage::info(message.clone()));
            }
        }
    }

    pub fn fail_save(&mut self, entity: Entity, message: impl Into<String>) {
        let message = message.into();
        for doc in &mut self.documents {
            if doc.inline_entity() == Some(entity) {
                doc.save_in_progress = false;
                doc.status = Some(StatusMessage::error(message.clone()));
            }
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        content_root: Option<PathBuf>,
    ) -> ScriptEditorEvent {
        if self.documents.is_empty() {
            self.wants_keyboard = false;
            return ScriptEditorEvent::None;
        }
        self.active = self.active.min(self.documents.len() - 1);
        for doc in &mut self.documents {
            doc.check_disk();
        }

        let mut event = ScriptEditorEvent::None;
        let mut open = true;
        let mut editor_focused = false;

        // Ctrl+S saves the active document while the editor has focus or the pointer.
        if self.wants_keyboard
            && ctx.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ))
            })
        {
            event = self.documents[self.active].save();
        }

        let window = egui::Window::new(format!("{}  Code Editor", icons::SCRIPT))
            .id(egui::Id::new("code_editor_window"))
            .open(&mut open)
            .resizable(true)
            .collapsible(true)
            .default_size(egui::vec2(780.0, 560.0))
            .min_size(egui::vec2(360.0, 220.0))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.viewport_rect().center())
            .show(ctx, |ui| {
                self.tab_strip(ui);
                ui.add_space(4.0);

                let doc = &mut self.documents[self.active];
                let description = doc.short_description(content_root.as_deref());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dirty = doc.is_dirty();
                        let can_save = dirty && !doc.save_in_progress && !doc.target_missing;
                        let save = if can_save {
                            theme::primary_button(format!("{}  Save", icons::SAVE))
                        } else {
                            egui::Button::new(format!("{}  Save", icons::SAVE))
                        };
                        if ui
                            .add_enabled(can_save, save)
                            .on_hover_text("Save (Ctrl+S)")
                            .clicked()
                        {
                            event = doc.save();
                        }
                        if ui
                            .add_enabled(
                                dirty && !doc.save_in_progress,
                                egui::Button::new(format!("{}  Revert", icons::REVERT)),
                            )
                            .on_hover_text("Discard unsaved changes")
                            .clicked()
                        {
                            doc.buffer = doc.original.clone();
                            doc.status = Some(StatusMessage::info("Reverted changes."));
                        }
                        if doc.save_in_progress {
                            ui.spinner();
                        }
                        // Fills the space left of the buttons, truncated if long.
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(theme::hint(description).monospace().small())
                                    .truncate(),
                            )
                            .on_hover_text(doc.description());
                        });
                    });
                });

                if let Some(status) = &doc.status {
                    ui.colored_label(status.color(), &status.text);
                }

                ui.add_space(2.0);
                editor_focused = code_view(ui, doc);
            });

        let hovered = window.as_ref().is_some_and(|window| {
            ctx.pointer_hover_pos()
                .is_some_and(|pos| window.response.rect.contains(pos))
        });
        self.wants_keyboard = editor_focused || hovered;

        if !open {
            self.request_close(CloseRequest::Window);
        }
        let close_event = self.close_confirmation(ctx);
        if matches!(event, ScriptEditorEvent::None) {
            event = close_event;
        }
        event
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("code_editor_tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (index, doc) in self.documents.iter().enumerate() {
                        let active = index == self.active;
                        let title = if doc.is_dirty() {
                            format!("● {}", doc.title())
                        } else {
                            doc.title()
                        };
                        let text = egui::RichText::new(title).color(if active {
                            colors::TEXT_STRONG
                        } else {
                            colors::TEXT_WEAK
                        });
                        let response = ui
                            .selectable_label(active, text)
                            .on_hover_text(doc.description());
                        if response.clicked() {
                            self.active = index;
                        }
                        if response.middle_clicked() {
                            close = Some(index);
                        }
                        if ui
                            .add(egui::Button::new(icons::CLOSE).frame(false).small())
                            .on_hover_text("Close")
                            .clicked()
                        {
                            close = Some(index);
                        }
                        ui.add_space(6.0);
                    }
                });
            });
        if let Some(index) = close {
            self.request_close(CloseRequest::Tab(index));
        }
    }

    fn request_close(&mut self, request: CloseRequest) {
        let dirty = match request {
            CloseRequest::Tab(index) => self.documents.get(index).is_some_and(Document::is_dirty),
            CloseRequest::Window => self.documents.iter().any(Document::is_dirty),
        };
        if dirty {
            self.confirm_close = Some(request);
        } else {
            self.close(request);
        }
    }

    fn close(&mut self, request: CloseRequest) {
        match request {
            CloseRequest::Tab(index) if index < self.documents.len() => {
                self.documents.remove(index);
                if self.active > index || self.active >= self.documents.len() {
                    self.active = self.active.saturating_sub(1);
                }
            }
            CloseRequest::Tab(_) => {}
            CloseRequest::Window => {
                self.documents.clear();
                self.active = 0;
            }
        }
    }

    fn close_confirmation(&mut self, ctx: &egui::Context) -> ScriptEditorEvent {
        let Some(request) = self.confirm_close else {
            return ScriptEditorEvent::None;
        };
        let names: Vec<String> = match request {
            CloseRequest::Tab(index) => self
                .documents
                .get(index)
                .map(Document::title)
                .into_iter()
                .collect(),
            CloseRequest::Window => self
                .documents
                .iter()
                .filter(|doc| doc.is_dirty())
                .map(Document::title)
                .collect(),
        };

        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("code_editor_close_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Unsaved changes");
            ui.add_space(4.0);
            ui.label(format!("Save changes to {}?", names.join(", ")));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Save")).clicked() {
                    choice = Some(true);
                }
                if ui.button("Don't Save").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_close = None;
                }
            });
        });
        if modal.should_close() {
            self.confirm_close = None;
        }

        let Some(save) = choice else {
            return ScriptEditorEvent::None;
        };
        self.confirm_close = None;
        let mut event = ScriptEditorEvent::None;
        if save {
            let indices: Vec<usize> = match request {
                CloseRequest::Tab(index) => vec![index],
                CloseRequest::Window => (0..self.documents.len()).collect(),
            };
            for index in indices {
                let doc = &mut self.documents[index];
                if !doc.is_dirty() {
                    continue;
                }
                if doc.inline_entity().is_some() {
                    // The save event carries the contents, so the tab can close now,
                    // but only one inline save goes out per frame.
                    if !matches!(event, ScriptEditorEvent::None) {
                        return event;
                    }
                    event = doc.save();
                } else {
                    let _ = doc.save();
                    if doc.is_dirty() {
                        // Saving failed; leave it open with the error.
                        return event;
                    }
                }
            }
        }
        self.close(request);
        event
    }
}

/// The text area with a line-number gutter. Returns whether it has keyboard focus.
fn code_view(ui: &mut egui::Ui, doc: &mut Document) -> bool {
    let language = doc.language();
    let font = FontId::monospace(CODE_FONT_SIZE);
    let line_count = doc.buffer.lines().count().max(1) + usize::from(doc.buffer.ends_with('\n'));
    let gutter_digits = line_count.to_string().len().max(3);
    let mut focused = false;

    egui::Frame::new()
        .fill(colors::BG_DEEP)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(0, 4))
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt("code_editor_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let numbers: String = (1..=line_count)
                            .map(|line| format!("{line:>gutter_digits$}\n"))
                            .collect();
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(numbers)
                                    .font(font.clone())
                                    .color(colors::TEXT_WEAK.gamma_multiply(0.7)),
                            )
                            .selectable(false),
                        );
                        ui.add_space(8.0);

                        let mut layouter =
                            |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
                                let job = match language {
                                    Language::Lua => highlight_lua(ui, text.as_str()),
                                    Language::Wgsl => crate::shader_editor::highlight_wgsl(
                                        ui,
                                        text.as_str(),
                                        f32::INFINITY,
                                    ),
                                    Language::Plain => LayoutJob::simple(
                                        text.as_str().to_owned(),
                                        FontId::monospace(CODE_FONT_SIZE),
                                        colors::TEXT,
                                        f32::INFINITY,
                                    ),
                                };
                                ui.fonts_mut(|fonts| fonts.layout_job(job))
                            };

                        let min_height = ui.available_height().max(120.0);
                        let response = ui.add_enabled(
                            !doc.target_missing,
                            egui::TextEdit::multiline(&mut doc.buffer)
                                .code_editor()
                                .frame(egui::Frame::NONE)
                                .lock_focus(true)
                                .margin(egui::Margin::ZERO)
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(0.0, min_height))
                                .layouter(&mut layouter),
                        );
                        focused = response.has_focus();
                    });
                });
        });
    focused
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(not(target_arch = "wasm32"))]
fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

#[derive(Clone)]
struct StatusMessage {
    kind: StatusKind,
    text: String,
}

impl StatusMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Info,
            text: text.into(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Error,
            text: text.into(),
        }
    }

    fn color(&self) -> Color32 {
        match self.kind {
            StatusKind::Info => colors::TEXT_WEAK,
            StatusKind::Error => colors::WARNING,
        }
    }
}

#[derive(Clone, Copy)]
enum StatusKind {
    Info,
    Error,
}

fn highlight_lua(ui: &egui::Ui, text: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;

    let font_id = FontId::new(CODE_FONT_SIZE, egui::FontFamily::Monospace);
    let visuals = ui.visuals();

    for (range, kind) in lua_tokens(text) {
        let color = match kind {
            TokenKind::Plain => visuals.text_color(),
            TokenKind::Comment => visuals.weak_text_color(),
            TokenKind::Keyword => Color32::from_rgb(204, 120, 50),
            TokenKind::String => Color32::from_rgb(152, 195, 121),
            TokenKind::Number => Color32::from_rgb(209, 154, 102),
        };
        job.append(
            &text[range],
            0.0,
            TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    }

    job
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenKind {
    Plain,
    Comment,
    Keyword,
    String,
    Number,
}

/// Split Lua source into highlight tokens covering the whole input.
///
/// Handles line and block comments (`--`, `--[[ ]]`, `--[==[ ]==]`), quoted and long
/// strings (`[[ ]]`, `[=[ ]=]`), numbers (including hex and exponents) and keywords.
/// Unterminated strings and comments extend to the end of the line or input.
fn lua_tokens(text: &str) -> Vec<(std::ops::Range<usize>, TokenKind)> {
    let bytes = text.as_bytes();
    let mut tokens: Vec<(std::ops::Range<usize>, TokenKind)> = Vec::new();
    let mut push = |range: std::ops::Range<usize>, kind: TokenKind| match tokens.last_mut() {
        Some((last, last_kind)) if *last_kind == kind && last.end == range.start => {
            last.end = range.end;
        }
        _ => tokens.push((range, kind)),
    };

    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let rest = &text[i..];
        if rest.starts_with("--") {
            i += 2;
            i = match long_bracket_level(&text[i..]) {
                Some(level) => find_long_bracket_close(text, i, level),
                None => text[i..].find('\n').map_or(text.len(), |offset| i + offset),
            };
            push(start..i, TokenKind::Comment);
        } else if let Some(level) = long_bracket_level(rest) {
            i = find_long_bracket_close(text, i, level);
            push(start..i, TokenKind::String);
        } else if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote && bytes[i] != b'\n' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
            push(start..i, TokenKind::String);
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let hex = rest.starts_with("0x") || rest.starts_with("0X");
            i += if hex { 2 } else { 1 };
            while i < bytes.len() {
                let b = bytes[i];
                let exponent = if hex { b"pP" } else { b"eE" };
                if exponent.contains(&b) && matches!(bytes.get(i + 1), Some(b'+' | b'-')) {
                    i += 2;
                } else if b.is_ascii_alphanumeric() || b == b'.' {
                    i += 1;
                } else {
                    break;
                }
            }
            push(start..i, TokenKind::Number);
        } else if bytes[i] == b'_' || bytes[i].is_ascii_alphabetic() {
            while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let kind = if KEYWORDS.contains(&&text[start..i]) {
                TokenKind::Keyword
            } else {
                TokenKind::Plain
            };
            push(start..i, kind);
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
            push(start..i, TokenKind::Plain);
        }
    }

    tokens
}

/// If `text` starts with a Lua long bracket (`[[`, `[=[`, ...), return its level.
fn long_bracket_level(text: &str) -> Option<usize> {
    let after_open = text.strip_prefix('[')?;
    let level = after_open.bytes().take_while(|&b| b == b'=').count();
    after_open[level..].starts_with('[').then_some(level)
}

/// Index just past the long bracket close `]=*]` matching `level`, or the end of text.
fn find_long_bracket_close(text: &str, from: usize, level: usize) -> usize {
    let close = format!("]{}]", "=".repeat(level));
    // Skip the opening bracket itself so `[[]]` closes correctly.
    let search_from = from + level + 2;
    text.get(search_from..)
        .and_then(|rest| rest.find(&close))
        .map_or(text.len(), |offset| search_from + offset + close.len())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file_to_string(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

#[cfg(target_arch = "wasm32")]
fn read_file_to_string(path: &Path) -> io::Result<String> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        format!("File I/O is not supported: {}", path.display()),
    ))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    std::fs::write(path, contents)
}

#[cfg(target_arch = "wasm32")]
fn write_file(path: &Path, _contents: &str) -> io::Result<()> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        format!("File I/O is not supported: {}", path.display()),
    ))
}

#[cfg(test)]
mod highlight_tests {
    use super::{lua_tokens, TokenKind};

    fn kinds(text: &str) -> Vec<(&str, TokenKind)> {
        lua_tokens(text)
            .into_iter()
            .map(|(range, kind)| (&text[range], kind))
            .filter(|(token, _)| !token.trim().is_empty())
            .collect()
    }

    #[test]
    fn highlights_lua_keywords_not_rust_ones() {
        let tokens = kinds("local fn = function() end");
        assert!(tokens.contains(&("local", TokenKind::Keyword)));
        assert!(tokens.contains(&("function", TokenKind::Keyword)));
        assert!(tokens
            .iter()
            .any(|(t, k)| t.contains("fn") && *k == TokenKind::Plain));
    }

    #[test]
    fn handles_comments_and_long_strings() {
        let text = "x = 1 -- note\n--[[ block\nstill ]] y = [==[ a ]] b ]==] .. 'q\\'s'";
        let tokens = kinds(text);
        assert!(tokens.contains(&("-- note", TokenKind::Comment)));
        assert!(tokens.contains(&("--[[ block\nstill ]]", TokenKind::Comment)));
        assert!(tokens.contains(&("[==[ a ]] b ]==]", TokenKind::String)));
        assert!(tokens.contains(&("'q\\'s'", TokenKind::String)));
    }

    #[test]
    fn tokens_cover_input_exactly() {
        let text = "for i = 0x1F, 1.5e-3 do print(\"é\") end --[[ unterminated";
        let covered: String = lua_tokens(text)
            .into_iter()
            .map(|(range, _)| &text[range])
            .collect();
        assert_eq!(covered, text);
        assert!(kinds(text).contains(&("0x1F", TokenKind::Number)));
        assert!(kinds(text).contains(&("1.5e-3", TokenKind::Number)));
    }
}
