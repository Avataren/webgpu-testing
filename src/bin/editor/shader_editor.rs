use std::io;
#[cfg(target_arch = "wasm32")]
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use hecs::Entity;

use wgpu_cube::asset::{Handle, MaterialAsset, ShaderMaterialMetadata};

// WGSL keywords - comprehensive list from the WGSL spec
const KEYWORDS: &[&str] = &[
    "alias", "break", "case", "const", "const_assert", "continue", "continuing", "default",
    "diagnostic", "discard", "else", "enable", "false", "fn", "for", "if", "let", "loop",
    "override", "requires", "return", "struct", "switch", "true", "var", "while",
    // Storage classes
    "private", "workgroup", "uniform", "storage",
    // Access modes
    "read", "write", "read_write",
];

// WGSL built-in types
const TYPES: &[&str] = &[
    "bool", "i32", "u32", "f32", "f16",
    "vec2", "vec3", "vec4",
    "vec2i", "vec2u", "vec2f", "vec2h",
    "vec3i", "vec3u", "vec3f", "vec3h",
    "vec4i", "vec4u", "vec4f", "vec4h",
    "mat2x2", "mat2x3", "mat2x4",
    "mat3x2", "mat3x3", "mat3x4",
    "mat4x2", "mat4x3", "mat4x4",
    "mat2x2f", "mat2x3f", "mat2x4f",
    "mat3x2f", "mat3x3f", "mat3x4f",
    "mat4x2f", "mat4x3f", "mat4x4f",
    "atomic", "array", "ptr",
    "texture_1d", "texture_2d", "texture_2d_array", "texture_3d",
    "texture_cube", "texture_cube_array",
    "texture_multisampled_2d",
    "texture_storage_1d", "texture_storage_2d", "texture_storage_2d_array", "texture_storage_3d",
    "texture_depth_2d", "texture_depth_2d_array", "texture_depth_cube", "texture_depth_cube_array",
    "texture_depth_multisampled_2d",
    "sampler", "sampler_comparison",
];

// WGSL attributes
const ATTRIBUTES: &[&str] = &[
    "@binding", "@builtin", "@compute", "@const", "@fragment", "@group", "@id",
    "@interpolate", "@invariant", "@location", "@must_use", "@size", "@align",
    "@stride", "@vertex", "@workgroup_size",
];

pub enum ShaderEditorEvent {
    None,
    Save {
        entity: Entity,
        handle: Handle<MaterialAsset>,
        contents: String,
        message: String,
    },
    Closed,
}

pub struct ShaderEditorState {
    entity: Entity,
    handle: Handle<MaterialAsset>,
    path: Option<PathBuf>,
    buffer: String,
    original_buffer: String,
    status: Option<StatusMessage>,
    open: bool,
    target_missing: bool,
    save_in_progress: bool,
    validation_error: Option<String>,
}

impl ShaderEditorState {
    pub fn new(
        entity: Entity,
        handle: Handle<MaterialAsset>,
        metadata: &ShaderMaterialMetadata,
    ) -> Self {
        let (path, buffer, status) = Self::extract_source(metadata);
        Self {
            entity,
            handle,
            path,
            original_buffer: buffer.clone(),
            buffer,
            status,
            open: true,
            target_missing: false,
            save_in_progress: false,
            validation_error: None,
        }
    }

    pub fn entity(&self) -> Entity {
        self.entity
    }

    pub fn handle(&self) -> Handle<MaterialAsset> {
        self.handle
    }

    pub fn show(&mut self, ctx: &egui::Context) -> ShaderEditorEvent {
        let mut event = ShaderEditorEvent::None;
        let mut open = self.open;
        let window_title = format!("Shader Editor - {}", self.window_label());
        egui::Window::new(window_title)
            .open(&mut open)
            .resizable(true)
            .movable(true)
            .constrain(true)
            .default_size(egui::vec2(900.0, 400.0))
            .max_height(600.0)
            .show(ctx, |ui| {
                if let Some(status) = &self.status {
                    let color = status.color(ui.visuals());
                    ui.colored_label(color, &status.text);
                    ui.add_space(6.0);
                }

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("Entity: {:?}", self.entity)).monospace());
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("Material: {:?}", self.handle)).monospace(),
                    );
                    if self.save_in_progress {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label("Saving...");
                    }
                });
                ui.label(self.description());
                if self.target_missing {
                    ui.label(
                        egui::RichText::new("Target entity or material unavailable")
                            .color(ui.visuals().warn_fg_color),
                    );
                }

                // Show validation error if present
                if let Some(error) = &self.validation_error {
                    ui.separator();
                    ui.colored_label(ui.visuals().error_fg_color, "Shader Compilation Error:");
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .show(ui, |ui| {
                            ui.monospace(error);
                        });
                }

                ui.separator();

                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    let dirty = self.buffer != self.original_buffer;
                    let can_save = dirty && !self.save_in_progress && !self.target_missing;

                    ui.horizontal(|ui| {
                        if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                            // Validate shader before saving
                            match validate_shader(&self.buffer) {
                                Ok(()) => {
                                    self.validation_error = None;
                                    self.save_in_progress = true;
                                    self.set_status_info("Saving shader...");
                                    let message = if let Some(path) = &self.path {
                                        format!("Shader '{}' saved and hot-reloaded.", path.display())
                                    } else {
                                        "Shader saved and hot-reloaded.".to_string()
                                    };
                                    event = ShaderEditorEvent::Save {
                                        entity: self.entity,
                                        handle: self.handle,
                                        contents: self.buffer.clone(),
                                        message,
                                    };
                                }
                                Err(error) => {
                                    self.validation_error = Some(error.clone());
                                    self.set_status_error("Shader validation failed. See error above.");
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
                            self.validation_error = None;
                            self.set_status_info("Reverted changes.");
                        }

                        ui.separator();

                        if ui.button("Validate").clicked() {
                            match validate_shader(&self.buffer) {
                                Ok(()) => {
                                    self.validation_error = None;
                                    self.set_status_info("Shader is valid!");
                                }
                                Err(error) => {
                                    self.validation_error = Some(error);
                                    self.set_status_error("Shader validation failed.");
                                }
                            }
                        }
                    });

                    ui.add_space(8.0);

                    let mut layouter =
                        |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                            let job = highlight_wgsl(ui, text.as_str(), wrap_width);
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

        if !open && self.open && matches!(event, ShaderEditorEvent::None) {
            event = ShaderEditorEvent::Closed;
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

    pub fn sync_with_metadata(&mut self, metadata: &ShaderMaterialMetadata) {
        if self.save_in_progress || self.buffer != self.original_buffer {
            return;
        }

        let (path, buffer, status) = Self::extract_source(metadata);
        let changed = self.path != path || self.original_buffer != buffer;
        if changed {
            self.path = path;
            self.original_buffer = buffer.clone();
            self.buffer = buffer;
            self.validation_error = None;
            if let Some(status) = status {
                self.status = Some(status);
            } else {
                self.set_status_info("Shader updated.");
            }
        } else if let Some(status) = status {
            self.status = Some(status);
        }
    }

    pub fn finish_save(&mut self, message: impl Into<String>) {
        self.save_in_progress = false;
        self.original_buffer = self.buffer.clone();
        self.validation_error = None;
        self.set_status_info(message);
    }

    pub fn fail_save(&mut self, message: impl Into<String>) {
        self.save_in_progress = false;
        self.set_status_error(message);
    }

    fn extract_source(
        metadata: &ShaderMaterialMetadata,
    ) -> (Option<PathBuf>, String, Option<StatusMessage>) {
        let path = metadata.source_path().map(|p| p.to_path_buf());
        let source = metadata.wgsl_source().to_string();

        if let Some(path) = &path {
            match read_file_to_string(path) {
                Ok(contents) => (Some(path.clone()), contents, None),
                Err(err) => (
                    Some(path.clone()),
                    source,
                    Some(StatusMessage::error(format!(
                        "Failed to load {}: {err}",
                        path.display()
                    ))),
                ),
            }
        } else {
            (None, source, None)
        }
    }

    fn description(&self) -> String {
        if let Some(path) = &self.path {
            format!("File: {}", path.display())
        } else {
            "Inline shader source".to_string()
        }
    }

    fn window_label(&self) -> String {
        if let Some(path) = &self.path {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        } else {
            format!("Material {:?}", self.handle)
        }
    }

    fn set_status_info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage::info(text));
    }

    fn set_status_error(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMessage::error(text));
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

fn highlight_wgsl(ui: &egui::Ui, text: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let font_id = FontId::new(14.0, egui::FontFamily::Monospace);
    let palette = HighlightPalette {
        default: ui.visuals().text_color(),
        comment: ui.visuals().weak_text_color(),
        keyword: Color32::from_rgb(204, 120, 50),
        type_color: Color32::from_rgb(86, 156, 214),
        string: Color32::from_rgb(152, 195, 121),
        number: Color32::from_rgb(209, 154, 102),
        attribute: Color32::from_rgb(197, 134, 192),
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

        // Comments
        if line[index..].starts_with("//") {
            append(job, &line[index..], palette.comment, font_id);
            break;
        }
        // Block comments
        else if line[index..].starts_with("/*") {
            let end = line[index..].find("*/").map(|pos| index + pos + 2).unwrap_or(line.len());
            append(job, &line[index..end], palette.comment, font_id);
            index = end;
        }
        // Strings
        else if ch == '"' {
            let end = find_string_end(line, index + ch_len);
            append(job, &line[index..end], palette.string, font_id);
            index = end;
        }
        // Attributes
        else if ch == '@' {
            let end = find_ident_end(line, index + ch_len);
            let token = &line[index..end];
            let color = if is_attribute(token) {
                palette.attribute
            } else {
                palette.default
            };
            append(job, token, color, font_id);
            index = end;
        }
        // Identifiers (keywords and types)
        else if is_ident_start(ch) {
            let end = find_ident_end(line, index + ch_len);
            let token = &line[index..end];
            let color = if is_keyword(token) {
                palette.keyword
            } else if is_type(token) {
                palette.type_color
            } else {
                palette.default
            };
            append(job, token, color, font_id);
            index = end;
        }
        // Numbers
        else if is_number_start(line, index, ch) {
            let end = find_number_end(line, index + ch_len);
            append(job, &line[index..end], palette.number, font_id);
            index = end;
        }
        // Everything else
        else {
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
    type_color: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    attribute: Color32,
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
    // Handle hex literals
    if text[index..].starts_with("0x") || text[index..].starts_with("0X") {
        index += 2;
        while index < text.len() {
            let ch = text[index..].chars().next().unwrap();
            if ch.is_ascii_hexdigit() {
                index += ch.len_utf8();
            } else {
                break;
            }
        }
        return index;
    }

    // Handle decimal numbers
    let mut has_dot = false;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        if ch.is_ascii_digit() {
            index += ch.len_utf8();
        } else if ch == '.' && !has_dot {
            has_dot = true;
            index += ch.len_utf8();
        } else if ch == 'e' || ch == 'E' {
            index += ch.len_utf8();
            if index < text.len() {
                let next = text[index..].chars().next().unwrap();
                if next == '+' || next == '-' {
                    index += next.len_utf8();
                }
            }
        } else if ch == 'f' || ch == 'h' || ch == 'u' || ch == 'i' {
            // Type suffix
            index += ch.len_utf8();
            break;
        } else {
            break;
        }
    }
    index
}

fn is_keyword(token: &str) -> bool {
    KEYWORDS.iter().any(|keyword| keyword == &token)
}

fn is_type(token: &str) -> bool {
    TYPES.iter().any(|t| t == &token)
}

fn is_attribute(token: &str) -> bool {
    ATTRIBUTES.iter().any(|attr| attr == &token)
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

/// Validates a WGSL shader source using naga
fn validate_shader(source: &str) -> Result<(), String> {
    use naga::front::wgsl;
    use naga::valid::{Capabilities, ValidationFlags, Validator};

    let module = wgsl::parse_str(source).map_err(|error| error.emit_to_string(source))?;
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    validator
        .validate(&module)
        .map(|_| ())
        .map_err(|error| error.emit_to_string(source))
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
