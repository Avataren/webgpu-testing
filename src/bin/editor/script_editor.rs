use std::io;
#[cfg(target_arch = "wasm32")]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use hecs::Entity;

use wgpu_cube::scripting::{LuaScriptComponent, LuaScriptSource};

/// Lua 5.4 reserved words.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
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
    pub fn new(entity: Entity, component: LuaScriptComponent) -> Self {
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
                            let job = highlight_lua(ui, text.as_str(), wrap_width);
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

    pub fn sync_with_component(&mut self, component: &LuaScriptComponent) {
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
        component: &LuaScriptComponent,
    ) -> (ScriptEditorSource, String, Option<StatusMessage>) {
        match component.source() {
            LuaScriptSource::Inline { name, source } => (
                ScriptEditorSource::Inline {
                    name: name.to_string(),
                },
                source.to_string(),
                None,
            ),
            LuaScriptSource::File { path } => match read_file_to_string(path) {
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

fn highlight_lua(ui: &egui::Ui, text: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let font_id = FontId::new(14.0, egui::FontFamily::Monospace);
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
