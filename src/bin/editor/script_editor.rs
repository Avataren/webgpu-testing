use std::io;
#[cfg(target_arch = "wasm32")]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use hecs::Entity;

use wgpu_cube::scripting::{RuneScriptComponent, RuneScriptSource};

const KEYWORDS: &[&str] = &[
    "async", "await", "break", "const", "continue", "else", "enum", "extern", "fn", "for", "if",
    "impl", "let", "loop", "match", "mod", "mut", "pub", "return", "self", "static", "struct",
    "super", "trait", "true", "false", "type", "use", "while",
];

pub enum ScriptEditorEvent {
    None,
    SaveInline {
        entity: Entity,
        name: String,
        contents: String,
        message: String,
    },
    SaveFile {
        entity: Entity,
        message: String,
    },
    Closed,
}

pub struct ScriptEditorState {
    entity: Entity,
    source: ScriptEditorSource,
    buffer: String,
    original_buffer: String,
    status: Option<StatusMessage>,
    open: bool,
    target_missing: bool,
    save_in_progress: bool,
}

impl ScriptEditorState {
    pub fn new(entity: Entity, component: RuneScriptComponent) -> Self {
        let (source, buffer, status) = Self::extract_source(&component);
        Self {
            entity,
            source,
            original_buffer: buffer.clone(),
            buffer,
            status,
            open: true,
            target_missing: false,
            save_in_progress: false,
        }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn show(&mut self, ctx: &egui::Context) -> ScriptEditorEvent {
        let mut event = ScriptEditorEvent::None;
        let mut open = self.open;
        let window_title = format!("Script Editor - {}", self.source.window_label());
        egui::Window::new(window_title)
            .open(&mut open)
            .resizable(true)
            .default_size(egui::vec2(720.0, 520.0))
            .show(ctx, |ui| {
                if let Some(status) = &self.status {
                    let color = status.color(ui.visuals());
                    ui.colored_label(color, &status.text);
                    ui.add_space(6.0);
                }

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Entity: {:?}", self.entity)).monospace());
                    if self.save_in_progress {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label("Saving...");
                    }
                });
                ui.label(self.source.description());
                if self.target_missing {
                    ui.label(
                        egui::RichText::new("Target entity unavailable")
                            .color(ui.visuals().warn_fg_color),
                    );
                }

                ui.separator();

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    let dirty = self.buffer != self.original_buffer;
                    let can_save = dirty && !self.save_in_progress && !self.target_missing;

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(can_save, egui::Button::new("Save"))
                            .clicked()
                        {
                            let source_snapshot = self.source.clone();
                            match source_snapshot {
                                ScriptEditorSource::Inline { name } => {
                                    self.save_in_progress = true;
                                    self.set_status_info("Saving inline script...");
                                    event = ScriptEditorEvent::SaveInline {
                                        entity: self.entity,
                                        name: name.clone(),
                                        contents: self.buffer.clone(),
                                        message: format!(
                                            "Inline script `{}` saved and hot-reloaded.",
                                            name
                                        ),
                                    };
                                }
                                ScriptEditorSource::File { path } => {
                                    match write_file(&path, &self.buffer) {
                                        Ok(()) => {
                                            self.original_buffer = self.buffer.clone();
                                            self.set_status_info(format!(
                                                "Saved to {}",
                                                path.display()
                                            ));
                                            event = ScriptEditorEvent::SaveFile {
                                                entity: self.entity,
                                                message: format!(
                                                    "Script `{}` hot-reloaded.",
                                                    path.display()
                                                ),
                                            };
                                        }
                                        Err(err) => {
                                            self.set_status_error(format!(
                                                "Failed to save {}: {err}",
                                                path.display()
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        if ui
                            .add_enabled(
                                dirty && !self.save_in_progress,
                                egui::Button::new("Reset"),
                            )
                            .clicked()
                        {
                            self.buffer = self.original_buffer.clone();
                            self.set_status_info("Reverted changes.");
                        }
                    });

                    ui.add_space(8.0);

                    let mut layouter =
                        |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                            let job = highlight_rune(ui, text.as_str(), wrap_width);
                            ui.fonts_mut(|fonts| fonts.layout_job(job))
                        };

                    let editor = egui::TextEdit::multiline(&mut self.buffer)
                        .code_editor()
                        .lock_focus(true)
                        .layouter(&mut layouter);

                    let mut available = ui.available_size();
                    available.y = available.y.max(0.0);
                    let available_size = available;

                    ui.add_enabled_ui(!self.target_missing, |ui| {
                        ui.add_sized(available_size, editor);
                    });
                });
            });

        if !open && self.open && matches!(event, ScriptEditorEvent::None) {
            event = ScriptEditorEvent::Closed;
        }
        self.open = open;
        event
    }

    pub fn mark_target_missing(&mut self, message: impl Into<String>) {
        self.target_missing = true;
        self.save_in_progress = false;
        self.status = Some(StatusMessage::error(message));
    }

    pub fn clear_target_missing(&mut self) {
        self.target_missing = false;
    }

    pub fn sync_with_component(&mut self, component: &RuneScriptComponent) {
        if self.save_in_progress || self.buffer != self.original_buffer {
            return;
        }

        let (source, buffer, status) = Self::extract_source(component);
        let changed = self.source != source || self.original_buffer != buffer;
        if changed {
            self.source = source;
            self.original_buffer = buffer.clone();
            self.buffer = buffer;
            if let Some(status) = status {
                self.status = Some(status);
            } else {
                self.set_status_info("Script updated.");
            }
        } else if let Some(status) = status {
            self.status = Some(status);
        }
    }

    pub fn finish_save(&mut self, message: impl Into<String>) {
        self.save_in_progress = false;
        self.original_buffer = self.buffer.clone();
        self.set_status_info(message);
    }

    pub fn fail_save(&mut self, message: impl Into<String>) {
        self.save_in_progress = false;
        self.set_status_error(message);
    }

    fn extract_source(
        component: &RuneScriptComponent,
    ) -> (ScriptEditorSource, String, Option<StatusMessage>) {
        match component.source() {
            RuneScriptSource::Inline { name, source } => (
                ScriptEditorSource::Inline {
                    name: name.to_string(),
                },
                source.to_string(),
                None,
            ),
            RuneScriptSource::File { path } => match read_file_to_string(path) {
                Ok(contents) => (
                    ScriptEditorSource::File { path: path.clone() },
                    contents,
                    None,
                ),
                Err(err) => (
                    ScriptEditorSource::File { path: path.clone() },
                    String::new(),
                    Some(StatusMessage::error(format!(
                        "Failed to load {}: {err}",
                        path.display()
                    ))),
                ),
            },
        }
    }

    fn set_status_info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage::info(text));
    }

    fn set_status_error(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage::error(text));
    }
}

#[derive(Clone, PartialEq, Eq)]
enum ScriptEditorSource {
    Inline { name: String },
    File { path: PathBuf },
}

impl ScriptEditorSource {
    fn description(&self) -> String {
        match self {
            Self::Inline { name } => format!("Inline script `{name}`"),
            Self::File { path } => format!("File: {}", path.display()),
        }
    }

    fn window_label(&self) -> String {
        match self {
            Self::Inline { name } => format!("Inline {name}"),
            Self::File { path } => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        }
    }
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

    fn color(&self, visuals: &egui::Visuals) -> Color32 {
        match self.kind {
            StatusKind::Info => visuals.text_color(),
            StatusKind::Error => visuals.error_fg_color,
        }
    }
}

#[derive(Clone, Copy)]
enum StatusKind {
    Info,
    Error,
}

fn highlight_rune(ui: &egui::Ui, text: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let font_id = FontId::new(14.0, egui::FontFamily::Monospace);
    let palette = HighlightPalette {
        default: ui.visuals().text_color(),
        comment: ui.visuals().weak_text_color(),
        keyword: Color32::from_rgb(204, 120, 50),
        string: Color32::from_rgb(152, 195, 121),
        number: Color32::from_rgb(209, 154, 102),
    };

    for line in text.split_inclusive('\n') {
        highlight_line(&mut job, line, &font_id, &palette);
    }

    job
}

fn highlight_line(job: &mut LayoutJob, line: &str, font_id: &FontId, palette: &HighlightPalette) {
    let mut index = 0;
    while index < line.len() {
        let ch = line[index..].chars().next().unwrap();
        let ch_len = ch.len_utf8();

        if line[index..].starts_with("//") {
            append(job, &line[index..], palette.comment, font_id);
            break;
        } else if ch == '"' {
            let end = find_string_end(line, index + ch_len);
            append(job, &line[index..end], palette.string, font_id);
            index = end;
        } else if is_ident_start(ch) {
            let end = find_ident_end(line, index + ch_len);
            let token = &line[index..end];
            let color = if is_keyword(token) {
                palette.keyword
            } else {
                palette.default
            };
            append(job, token, color, font_id);
            index = end;
        } else if is_number_start(line, index, ch) {
            let end = find_number_end(line, index + ch_len);
            append(job, &line[index..end], palette.number, font_id);
            index = end;
        } else {
            append(job, &line[index..index + ch_len], palette.default, font_id);
            index += ch_len;
        }
    }
}

fn append(job: &mut LayoutJob, text: &str, color: Color32, font_id: &FontId) {
    if text.is_empty() {
        return;
    }

    let format = TextFormat {
        font_id: font_id.clone(),
        color,
        ..Default::default()
    };
    job.append(text, 0.0, format);
}

struct HighlightPalette {
    default: Color32,
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
}

fn find_string_end(text: &str, mut index: usize) -> usize {
    let mut escaped = false;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        let len = ch.len_utf8();
        index += len;
        if ch == '"' && !escaped {
            break;
        }
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
    }
    index
}

fn find_ident_end(text: &str, mut index: usize) -> usize {
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        if is_ident_continue(ch) {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    index
}

fn find_number_end(text: &str, mut index: usize) -> usize {
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        if ch.is_ascii_digit() || ch == '.' {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    index
}

fn is_keyword(token: &str) -> bool {
    KEYWORDS.iter().any(|keyword| keyword == &token)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn is_number_start(line: &str, index: usize, ch: char) -> bool {
    if ch.is_ascii_digit() {
        return true;
    }

    if ch == '.' {
        let next_index = index + ch.len_utf8();
        if next_index < line.len() {
            if let Some(next_char) = line[next_index..].chars().next() {
                return next_char.is_ascii_digit();
            }
        }
    }

    false
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
