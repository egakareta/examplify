use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use egui::{Color32, RichText, ScrollArea, Ui};
use log::{Level, LevelFilter, Log, Metadata, Record};

/// Maximum number of recent log records retained in memory.
pub const MAX_LOG_ENTRIES: usize = 1_000;

static LOGGER: ExamplifyLogger = ExamplifyLogger;
static LOG_BUFFER: OnceLock<Mutex<LogBuffer>> = OnceLock::new();
static REPAINT_CONTEXT: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();
static REPAINT_PENDING: Mutex<bool> = Mutex::new(false);

/// A captured log record, including its level and source metadata when available.
#[derive(Clone, Debug)]
pub struct LogEntry {
    /// Time when the record was captured.
    pub timestamp: SystemTime,
    /// Severity of the record.
    pub level: Level,
    /// Log target, usually the Rust module path.
    pub target: String,
    /// Formatted log message.
    pub message: String,
    /// Rust module that emitted the record, when provided by the logging facade.
    pub module_path: Option<String>,
    /// Source file that emitted the record, when provided by the logging facade.
    pub file: Option<String>,
    /// Source line that emitted the record, when provided by the logging facade.
    pub line: Option<u32>,
}

/// Install Examplify as the process-wide logger.
pub fn init() {
    let _ = try_init();
}

/// Launch Examplify's default eframe diagnostics application.
///
/// On native targets this runs the application event loop until the window
/// closes. On wasm32 it starts the app on the `canvas#examplify` element.
/// This also installs Examplify as the logger if no other logger is active.
pub fn run() {
    let _ = try_init();
    start_default_app();
}

/// Fallible version of [`init`], useful when the caller needs to know whether
/// another global logger was already installed.
pub fn try_init() -> Result<(), log::SetLoggerError> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(LevelFilter::Trace);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn start_default_app() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Examplify",
        options,
        Box::new(|cc| Ok(Box::new(ExamplifyApp::new(cc.egui_ctx.clone())))),
    )
    .expect("failed to launch Examplify");
}

#[cfg(target_arch = "wasm32")]
fn start_default_app() {
    use wasm_bindgen::JsCast;

    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("examplify"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok());

        let Some(canvas) = canvas else {
            log::error!("Could not find canvas#examplify to start Examplify");
            return;
        };

        if let Err(error) = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(ExamplifyApp::new(cc.egui_ctx.clone())))),
            )
            .await
        {
            log::error!("Failed to launch Examplify: {error:?}");
        }
    });
}

struct ExamplifyApp {
    console: Console,
}

impl ExamplifyApp {
    fn new(context: egui::Context) -> Self {
        *lock_repaint_context() = Some(context);
        Self {
            console: Console::default(),
        }
    }
}

impl eframe::App for ExamplifyApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.console.show(ui);
    }
}

fn lock_repaint_context() -> MutexGuard<'static, Option<egui::Context>> {
    REPAINT_CONTEXT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn request_repaint() {
    let Some(context) = lock_repaint_context().clone() else {
        return;
    };

    let should_schedule = {
        let mut pending = lock_repaint_pending();
        if *pending {
            false
        } else {
            *pending = true;
            true
        }
    };

    if should_schedule {
        defer_repaint(context);
    }
}

fn lock_repaint_pending() -> MutexGuard<'static, bool> {
    REPAINT_PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(not(target_arch = "wasm32"))]
fn defer_repaint(context: egui::Context) {
    // Log callbacks can run while egui holds its context lock. Calling
    // request_repaint synchronously would re-enter that lock, so do it after
    // the current callback has returned.
    let result = std::thread::Builder::new()
        .name("examplify-repaint".to_owned())
        .spawn(move || {
            *lock_repaint_pending() = false;
            context.request_repaint();
        });

    if result.is_err() {
        *lock_repaint_pending() = false;
    }
}

#[cfg(target_arch = "wasm32")]
fn defer_repaint(context: egui::Context) {
    // `spawn_local` runs on a later microtask, after egui's current callback
    // has released the context lock.
    wasm_bindgen_futures::spawn_local(async move {
        *lock_repaint_pending() = false;
        context.request_repaint();
    });
}

/// Return a snapshot of the recent captured records, oldest first.
pub fn entries() -> Vec<LogEntry> {
    lock_buffer().entries.iter().cloned().collect()
}

/// Remove all currently captured records.
pub fn clear() {
    lock_buffer().entries.clear();
}

struct ExamplifyLogger;

impl Log for ExamplifyLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Trace
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let entry = LogEntry {
            timestamp: SystemTime::now(),
            level: record.level(),
            target: record.target().to_owned(),
            message: record.args().to_string(),
            module_path: record.module_path().map(str::to_owned),
            file: record.file().map(str::to_owned),
            line: record.line(),
        };
        lock_buffer().push(entry);
        request_repaint();
    }

    fn flush(&self) {}
}

#[derive(Default)]
struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

fn lock_buffer() -> MutexGuard<'static, LogBuffer> {
    LOG_BUFFER
        .get_or_init(|| Mutex::new(LogBuffer::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An embeddable, stateful diagnostics console.
pub struct Console {
    auto_scroll: bool,
    show_trace: bool,
    show_debug: bool,
    show_info: bool,
    show_warn: bool,
    show_error: bool,
}

impl Default for Console {
    fn default() -> Self {
        Self {
            auto_scroll: true,
            show_trace: true,
            show_debug: true,
            show_info: true,
            show_warn: true,
            show_error: true,
        }
    }
}

impl Console {
    /// Create a console with every log level visible and auto-scroll enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether new output should keep the view pinned to the bottom.
    pub fn set_auto_scroll(&mut self, enabled: bool) {
        self.auto_scroll = enabled;
    }

    /// Render the console inside an existing UI.
    pub fn show(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.toggle_value(&mut self.show_trace, level_text("TRACE", Level::Trace));
            ui.toggle_value(&mut self.show_debug, level_text("DEBUG", Level::Debug));
            ui.toggle_value(&mut self.show_info, level_text("INFO", Level::Info));
            ui.toggle_value(&mut self.show_warn, level_text("WARN", Level::Warn));
            ui.toggle_value(&mut self.show_error, level_text("ERROR", Level::Error));
            if ui.button("All levels").clicked() {
                self.show_trace = true;
                self.show_debug = true;
                self.show_info = true;
                self.show_warn = true;
                self.show_error = true;
            }
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
            if ui.button("Clear").clicked() {
                clear();
            }
        });

        let snapshot = entries();
        let visible: Vec<_> = snapshot
            .iter()
            .filter(|entry| self.includes(entry.level))
            .collect();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} shown, {} total", visible.len(), snapshot.len()))
                    .small()
                    .weak(),
            );
        });

        ui.separator();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.auto_scroll)
            .max_height(ui.available_height().max(120.0))
            .show(ui, |ui| {
                if visible.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new("No matching log messages yet.").weak());
                }

                for entry in visible {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format_timestamp(entry.timestamp))
                                .monospace()
                                .small()
                                .weak(),
                        );
                        ui.label(
                            RichText::new(format!("{:>5}", entry.level))
                                .monospace()
                                .strong()
                                .color(level_color(entry.level)),
                        );
                        ui.label(RichText::new(&entry.target).monospace().small().weak());
                        ui.label(RichText::new(&entry.message).monospace());
                    });
                }
            });
    }

    fn includes(&self, level: Level) -> bool {
        match level {
            Level::Trace => self.show_trace,
            Level::Debug => self.show_debug,
            Level::Info => self.show_info,
            Level::Warn => self.show_warn,
            Level::Error => self.show_error,
        }
    }
}

fn level_text(text: &'static str, level: Level) -> RichText {
    RichText::new(text).monospace().color(level_color(level))
}

fn level_color(level: Level) -> Color32 {
    match level {
        Level::Error => Color32::from_rgb(255, 105, 97),
        Level::Warn => Color32::from_rgb(255, 190, 85),
        Level::Info => Color32::from_rgb(105, 190, 255),
        Level::Debug => Color32::from_rgb(165, 145, 255),
        Level::Trace => Color32::from_rgb(145, 155, 170),
    }
}

fn format_timestamp(timestamp: SystemTime) -> String {
    let elapsed = timestamp.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = elapsed.as_secs() % (24 * 60 * 60);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60,
        elapsed.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: impl Into<String>) -> LogEntry {
        LogEntry {
            timestamp: UNIX_EPOCH,
            level: Level::Info,
            target: "test".to_owned(),
            message: message.into(),
            module_path: None,
            file: None,
            line: None,
        }
    }

    #[test]
    fn buffer_keeps_only_the_most_recent_entries() {
        let mut buffer = LogBuffer::default();
        for index in 0..=MAX_LOG_ENTRIES {
            buffer.push(entry(index.to_string()));
        }

        assert_eq!(buffer.entries.len(), MAX_LOG_ENTRIES);
        assert_eq!(buffer.entries.front().unwrap().message, "1");
        assert_eq!(
            buffer.entries.back().unwrap().message,
            MAX_LOG_ENTRIES.to_string()
        );
    }

    #[test]
    fn clear_removes_all_entries_from_a_buffer() {
        let mut buffer = LogBuffer::default();
        buffer.push(entry("hello"));
        buffer.entries.clear();

        assert!(buffer.entries.is_empty());
    }

    #[test]
    fn repaint_requests_from_egui_callbacks_are_deferred() {
        let context = egui::Context::default();
        *lock_repaint_context() = Some(context.clone());
        context.set_request_repaint_callback(|_| request_repaint());

        // egui invokes its repaint callback while its context is locked.
        // Logging there must not synchronously re-enter Context.
        context.request_repaint();

        context.set_request_repaint_callback(|_| {});
        *lock_repaint_context() = None;
    }
}
