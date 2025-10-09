#[cfg(feature = "egui")]
use egui::{Color32, Label, Layout, RichText, TextWrapMode};
#[cfg(feature = "egui")]
use egui::{ScrollArea, Ui};
#[cfg(feature = "egui")]
use log::{Level, LevelFilter, Log, Metadata, Record};
#[cfg(feature = "egui")]
use std::collections::{BTreeSet, VecDeque};
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(feature = "egui")]
use std::time::SystemTime;

#[cfg(all(feature = "egui", target_arch = "wasm32"))]
use console_log as wasm_console_log;
#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
use env_logger::Builder as EnvBuilder;
#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
use env_logger::Logger as EnvLogger;

#[cfg(feature = "egui")]
const MAX_LOG_ENTRIES: usize = 1_000;

#[cfg(feature = "egui")]
static LOG_HANDLE: OnceLock<LogBufferHandle> = OnceLock::new();
#[cfg(feature = "egui")]
static LOGGER_ONCE: OnceLock<()> = OnceLock::new();

#[cfg(feature = "egui")]
const LOG_LEVELS: [Level; 5] = [
    Level::Error,
    Level::Warn,
    Level::Info,
    Level::Debug,
    Level::Trace,
];

#[cfg(feature = "egui")]
/// A snapshot of a single log record captured by the in-app recorder.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: SystemTime,
    pub level: Level,
    pub target: String,
    pub message: String,
}

#[cfg(feature = "egui")]
impl LogEntry {
    fn from_record(record: &Record<'_>) -> Self {
        Self {
            timestamp: SystemTime::now(),
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        }
    }
}

#[cfg(feature = "egui")]
#[derive(Default)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

#[cfg(feature = "egui")]
impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
        }
    }

    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn handle() -> LogBufferHandle {
        Arc::new(Mutex::new(Self::new()))
    }
}

#[cfg(feature = "egui")]
pub type LogBufferHandle = Arc<Mutex<LogBuffer>>;

#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
struct LogRouter {
    buffer: LogBufferHandle,
    logger: EnvLogger,
}

#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
impl LogRouter {
    fn new(buffer: LogBufferHandle, logger: EnvLogger) -> Self {
        Self { buffer, logger }
    }
}

#[cfg(all(feature = "egui", not(target_arch = "wasm32")))]
impl Log for LogRouter {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.logger.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push(LogEntry::from_record(record));
        }

        self.logger.log(record);
    }

    fn flush(&self) {
        self.logger.flush();
    }
}

#[cfg(all(feature = "egui", target_arch = "wasm32"))]
struct LogRouter {
    buffer: LogBufferHandle,
    level: LevelFilter,
}

#[cfg(all(feature = "egui", target_arch = "wasm32"))]
impl LogRouter {
    fn new(buffer: LogBufferHandle, level: LevelFilter) -> Self {
        Self { buffer, level }
    }
}

#[cfg(all(feature = "egui", target_arch = "wasm32"))]
impl Log for LogRouter {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push(LogEntry::from_record(record));
        }

        wasm_console_log::log(record);
    }

    fn flush(&self) {}
}

#[cfg(feature = "egui")]
fn install_logger_once(handle: &LogBufferHandle) {
    LOGGER_ONCE.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut builder = EnvBuilder::from_default_env();
            builder.filter_level(LevelFilter::Info);
            let logger = builder.build();
            let level = logger.filter();
            if log::set_boxed_logger(Box::new(LogRouter::new(handle.clone(), logger))).is_ok() {
                log::set_max_level(level);
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            let level = LevelFilter::Info;
            if log::set_boxed_logger(Box::new(LogRouter::new(handle.clone(), level))).is_ok() {
                log::set_max_level(level);
            }
        }
    });
}

#[cfg(feature = "egui")]
/// Initialize the global log recorder used by [`LogWindow`].
///
/// # Examples
/// ```
/// # #[cfg(feature = "egui")] {
/// use wgpu_cube::ui::{init_log_recorder, LogWindow};
/// let handle = init_log_recorder();
/// let mut window = LogWindow::new(handle);
/// let _ = &mut window;
/// # }
/// ```
pub fn init_log_recorder() -> LogBufferHandle {
    let handle = LOG_HANDLE.get_or_init(LogBuffer::handle).clone();
    install_logger_once(&handle);
    handle
}

#[cfg(feature = "egui")]
fn format_timestamp(ts: SystemTime) -> String {
    match ts.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let total_seconds = duration.as_secs();
            let hours = total_seconds / 3600;
            let minutes = (total_seconds % 3600) / 60;
            let seconds = total_seconds % 60;
            let millis = duration.subsec_millis();
            format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
        }
        Err(_) => "--:--:--.---".to_string(),
    }
}

#[cfg(feature = "egui")]
fn level_color(level: Level) -> Color32 {
    match level {
        Level::Error => Color32::from_rgb(255, 99, 99),
        Level::Warn => Color32::from_rgb(255, 192, 0),
        Level::Info => Color32::from_rgb(144, 202, 249),
        Level::Debug => Color32::from_rgb(158, 158, 158),
        Level::Trace => Color32::from_rgb(120, 144, 156),
    }
}

#[cfg(feature = "egui")]
fn render_entry(ui: &mut Ui, entry: &LogEntry) {
    ui.horizontal_wrapped(|ui| {
        // Timestamp - fixed width, no wrap
        ui.add(
            Label::new(RichText::new(format_timestamp(entry.timestamp)).monospace())
                .wrap_mode(TextWrapMode::Truncate),
        );
        ui.add_space(6.0);
        
        // Level - fixed width, no wrap
        ui.colored_label(
            level_color(entry.level),
            RichText::new(entry.level.as_str()).monospace(),
        );
        ui.add_space(6.0);
        
        // Target - can truncate if too long
        ui.add(
            Label::new(RichText::new(entry.target.as_str()).monospace())
                .wrap_mode(TextWrapMode::Truncate),
        );
        ui.add_space(12.0);
        
        // Message - this should wrap
        ui.add(Label::new(entry.message.as_str()).wrap_mode(TextWrapMode::Wrap));
    });
}

#[cfg(feature = "egui")]
/// An egui window that renders log records collected from [`init_log_recorder`].
pub struct LogWindow {
    handle: LogBufferHandle,
    title: String,
    enabled_levels: BTreeSet<Level>,
    auto_scroll: bool,
}

#[cfg(feature = "egui")]
impl LogWindow {
    /// Create a new window that renders log output from the global recorder.
    pub fn new(handle: LogBufferHandle) -> Self {
        let enabled_levels = LOG_LEVELS.iter().copied().collect();
        Self {
            handle,
            title: "Logs".to_string(),
            enabled_levels,
            auto_scroll: true,
        }
    }

    /// Display the log window using the provided egui context.
    ///
    /// If `open` is supplied the window will render a close button and update the
    /// handle when it is pressed.
    pub fn show(&mut self, ctx: &egui::Context, open: Option<&mut bool>) {
        let entries = self.entries_snapshot();
        let mut window = egui::Window::new(&self.title)
            .default_width(800.0)
            .min_width(360.0)
            .min_height(180.0);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.show(ctx, |ui| {
            self.level_controls(ui);
            ui.separator();
            let filtered: Vec<_> = entries
                .iter()
                .filter(|entry| self.enabled_levels.contains(&entry.level))
                .collect();
            ScrollArea::vertical()
                .stick_to_bottom(self.auto_scroll)
                .show(ui, |ui| {
                    for entry in filtered {
                        render_entry(ui, entry);
                        ui.add_space(4.0);
                    }
                });
        });
    }

    fn entries_snapshot(&self) -> Vec<LogEntry> {
        self.handle
            .lock()
            .map(|buffer| buffer.snapshot())
            .unwrap_or_default()
    }

    fn level_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for level in LOG_LEVELS {
                let mut enabled = self.enabled_levels.contains(&level);
                if ui.checkbox(&mut enabled, level.as_str()).changed() {
                    if enabled {
                        self.enabled_levels.insert(level);
                    } else {
                        self.enabled_levels.remove(&level);
                    }
                }
            }
            let mut auto_scroll = self.auto_scroll;
            if ui.checkbox(&mut auto_scroll, "Auto-scroll").changed() {
                self.auto_scroll = auto_scroll;
            }
            if ui.button("Clear").clicked() {
                if let Ok(mut buffer) = self.handle.lock() {
                    buffer.clear();
                }
            }
        });
    }
}
