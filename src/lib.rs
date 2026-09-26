use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, OnceLock},
    time::SystemTime,
};

use log::Level;

/// Maximum number of recent log records retained in memory.
pub const MAX_LOG_ENTRIES: usize = 1_000;

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

static LOG_BUFFER: OnceLock<Mutex<LogBuffer>> = OnceLock::new();

/// Install Examplify's DOM console as the process-wide logger on WebAssembly.
///
/// On native targets this is a no-op. The browser console includes per-level
/// visibility filters, auto-scroll, and a clear button.
pub fn init() {
    let _ = try_init();
}

/// Fallible version of [`init`], useful when another global logger may already
/// be installed. On native targets it succeeds without installing a logger.
pub fn try_init() -> Result<(), log::SetLoggerError> {
    #[cfg(target_arch = "wasm32")]
    {
        platform::install()?;
        platform::refresh();
    }

    Ok(())
}

/// Return a snapshot of the recent captured records, oldest first.
pub fn entries() -> Vec<LogEntry> {
    lock_buffer().entries.iter().cloned().collect()
}

/// Remove all currently captured records and clear the browser console.
pub fn clear() {
    lock_buffer().entries.clear();
    #[cfg(target_arch = "wasm32")]
    platform::refresh();
}

#[derive(Default)]
struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    #[cfg(any(target_arch = "wasm32", test))]
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

#[cfg(target_arch = "wasm32")]
mod platform {
    use std::{
        sync::{Mutex, MutexGuard, OnceLock},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use log::{Level, LevelFilter, Log, Metadata, Record};
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Element, Event, HtmlElement, HtmlInputElement};

    use super::{LogEntry, clear, entries, lock_buffer};

    const PANEL_ID: &str = "examplify-console";
    const REOPEN_ID: &str = "examplify-console-reopen";
    const FILTERS_ID: &str = "examplify-console-filters";
    const ENTRIES_ID: &str = "examplify-console-entries";
    const STATUS_ID: &str = "examplify-console-status";
    const PANEL_STYLE: &str = "position:fixed;inset:0;z-index:2147483647;box-sizing:border-box;width:100vw;height:100vh;height:100dvh;max-height:none;display:flex;flex-direction:column;gap:12px;margin:0;padding:clamp(10px,2vw,24px);border:0;border-radius:0;background:#101014;color:#eeeeee;box-shadow:none;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;";
    const REOPEN_STYLE: &str = "position:fixed;right:16px;bottom:16px;z-index:2147483647;display:none;padding:9px 14px;border:1px solid #ffffff38;border-radius:6px;background:#202028;color:#eeeeee;font:13px ui-monospace,SFMono-Regular,Menlo,monospace;cursor:pointer;box-shadow:0 4px 16px #0009;";

    static LOGGER: ExamplifyLogger = ExamplifyLogger;

    static OPTIONS: OnceLock<Mutex<ConsoleOptions>> = OnceLock::new();

    struct ExamplifyLogger;

    pub(super) fn install() -> Result<(), log::SetLoggerError> {
        log::set_logger(&LOGGER)?;
        log::set_max_level(LevelFilter::Trace);
        Ok(())
    }

    impl Log for ExamplifyLogger {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.level() <= Level::Trace
        }

        fn log(&self, record: &Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }

            let entry = LogEntry {
                timestamp: browser_time(),
                level: record.level(),
                target: record.target().to_owned(),
                message: record.args().to_string(),
                module_path: record.module_path().map(str::to_owned),
                file: record.file().map(str::to_owned),
                line: record.line(),
            };
            lock_buffer().push(entry);
            refresh();
        }

        fn flush(&self) {}
    }

    #[derive(Clone)]
    struct ConsoleOptions {
        auto_scroll: bool,
        minimized: bool,
        show_trace: bool,
        show_debug: bool,
        show_info: bool,
        show_warn: bool,
        show_error: bool,
    }

    impl Default for ConsoleOptions {
        fn default() -> Self {
            Self {
                auto_scroll: true,
                minimized: false,
                show_trace: true,
                show_debug: true,
                show_info: true,
                show_warn: true,
                show_error: true,
            }
        }
    }

    impl ConsoleOptions {
        fn includes(&self, level: Level) -> bool {
            match level {
                Level::Trace => self.show_trace,
                Level::Debug => self.show_debug,
                Level::Info => self.show_info,
                Level::Warn => self.show_warn,
                Level::Error => self.show_error,
            }
        }

        fn set_level(&mut self, level: Level, show: bool) {
            match level {
                Level::Trace => self.show_trace = show,
                Level::Debug => self.show_debug = show,
                Level::Info => self.show_info = show,
                Level::Warn => self.show_warn = show,
                Level::Error => self.show_error = show,
            }
        }
    }

    fn lock_options() -> MutexGuard<'static, ConsoleOptions> {
        OPTIONS
            .get_or_init(|| Mutex::new(ConsoleOptions::default()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn refresh() {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(_panel) = ensure_console(&document) else {
            return;
        };

        let snapshot = entries();
        let options = lock_options().clone();
        update_filter_controls(&document, &options);
        update_visibility(&document, options.minimized);

        let Some(list) = document.get_element_by_id(ENTRIES_ID) else {
            return;
        };
        remove_children(&list);

        let visible: Vec<_> = snapshot
            .iter()
            .filter(|entry| options.includes(entry.level))
            .collect();

        if let Some(status) = document.get_element_by_id(STATUS_ID) {
            status.set_text_content(Some(&format!(
                "{} shown, {} total",
                visible.len(),
                snapshot.len()
            )));
        }

        if visible.is_empty() {
            if let Ok(empty) = document.create_element("div") {
                empty.set_text_content(Some("No matching log messages yet."));
                let _ =
                    empty.set_attribute("style", "padding:12px 0;color:#909098;font-style:italic");
                let _ = list.append_child(&empty);
            }
        } else {
            for entry in visible {
                if let Some(row) = make_log_row(&document, entry) {
                    let _ = list.append_child(&row);
                }
            }
        }

        if options.auto_scroll {
            if let Some(list) = list.dyn_ref::<HtmlElement>() {
                list.set_scroll_top(list.scroll_height());
            }
        }
    }

    fn ensure_console(document: &Document) -> Option<Element> {
        if let Some(panel) = document.get_element_by_id(PANEL_ID) {
            return Some(panel);
        }

        let panel = document.create_element("section").ok()?;
        panel.set_id(PANEL_ID);
        let _ = panel.set_attribute("role", "region");
        let _ = panel.set_attribute("aria-label", "Examplify log console");
        set_style(&panel, PANEL_STYLE);

        let header = document.create_element("header").ok()?;
        set_style(
            &header,
            "display:flex;align-items:center;justify-content:space-between;gap:12px;",
        );
        let title = document.create_element("strong").ok()?;
        title.set_text_content(Some("Logs"));
        let close_button = make_button(document, "×")?;
        let _ = close_button.set_attribute("aria-label", "Minimize log console");
        let _ = close_button.set_attribute("title", "Minimize");
        set_style(
            &close_button,
            "width:36px;height:36px;padding:0;border:1px solid #ffffff38;border-radius:6px;background:#ffffff12;color:#eeeeee;font:24px/1 ui-monospace,monospace;cursor:pointer;",
        );
        let close_listener = Closure::<dyn FnMut(Event)>::new(|_| set_minimized(true));
        add_listener(&close_button, "click", close_listener);
        let _ = header.append_child(&title);
        let _ = header.append_child(&close_button);

        let toolbar = document.create_element("div").ok()?;
        set_style(
            &toolbar,
            "display:flex;align-items:center;flex-wrap:wrap;gap:8px;",
        );

        let filters = document.create_element("div").ok()?;
        filters.set_id(FILTERS_ID);
        set_style(
            &filters,
            "display:flex;align-items:center;flex-wrap:wrap;gap:8px;flex:1;",
        );

        for (name, level) in [
            ("TRACE", Level::Trace),
            ("DEBUG", Level::Debug),
            ("INFO", Level::Info),
            ("WARN", Level::Warn),
            ("ERROR", Level::Error),
        ] {
            let label = document.create_element("label").ok()?;
            set_style(
                &label,
                "display:inline-flex;align-items:center;gap:3px;cursor:pointer;user-select:none;",
            );

            let input = document.create_element("input").ok()?;
            input.set_id(&filter_id(level));
            let _ = input.set_attribute("type", "checkbox");
            let _ = input.set_attribute("aria-label", &format!("Show {name} messages"));
            if let Some(input) = input.dyn_ref::<HtmlInputElement>() {
                input.set_checked(true);
            }
            let text = document.create_element("span").ok()?;
            text.set_text_content(Some(name));
            set_style(&text, &format!("color:{};", level_color(level)));
            let _ = label.append_child(&input);
            let _ = label.append_child(&text);
            let _ = filters.append_child(&label);
            add_filter_listener(&input, level);
        }

        let all_button = make_button(document, "All levels")?;
        let all_listener = Closure::<dyn FnMut(Event)>::new(|_| {
            {
                let mut options = lock_options();
                options.show_trace = true;
                options.show_debug = true;
                options.show_info = true;
                options.show_warn = true;
                options.show_error = true;
            }
            refresh();
        });
        add_listener(&all_button, "click", all_listener);

        let auto_label = document.create_element("label").ok()?;
        set_style(
            &auto_label,
            "display:inline-flex;align-items:center;gap:3px;cursor:pointer;white-space:nowrap;",
        );
        let auto_input = document.create_element("input").ok()?;
        auto_input.set_id("examplify-console-auto-scroll");
        let _ = auto_input.set_attribute("type", "checkbox");
        let _ = auto_input.set_attribute("aria-label", "Auto-scroll log output");
        if let Some(input) = auto_input.dyn_ref::<HtmlInputElement>() {
            input.set_checked(true);
        }
        let auto_text = document.create_element("span").ok()?;
        auto_text.set_text_content(Some("Auto-scroll"));
        let _ = auto_label.append_child(&auto_input);
        let _ = auto_label.append_child(&auto_text);
        let auto_listener = Closure::<dyn FnMut(Event)>::new(|event: Event| {
            if let Some(checked) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                .map(|input| input.checked())
            {
                lock_options().auto_scroll = checked;
                refresh();
            }
        });
        add_listener(&auto_input, "change", auto_listener);

        let clear_button = make_button(document, "Clear")?;
        let clear_listener = Closure::<dyn FnMut(Event)>::new(|_| clear());
        add_listener(&clear_button, "click", clear_listener);

        let status = document.create_element("div").ok()?;
        status.set_id(STATUS_ID);
        set_style(&status, "color:#909098;font-size:11px;");

        let list = document.create_element("div").ok()?;
        list.set_id(ENTRIES_ID);
        let _ = list.set_attribute("role", "log");
        let _ = list.set_attribute("aria-live", "polite");
        let _ = list.set_attribute("aria-relevant", "additions");
        set_style(
            &list,
            "flex:1;min-height:0;overflow:auto;border-top:1px solid #ffffff24;padding-top:8px;",
        );

        let reopen_button = make_button(document, "Logs")?;
        reopen_button.set_id(REOPEN_ID);
        let _ = reopen_button.set_attribute("aria-label", "Reopen log console");
        set_style(&reopen_button, REOPEN_STYLE);
        let reopen_listener = Closure::<dyn FnMut(Event)>::new(|_| set_minimized(false));
        add_listener(&reopen_button, "click", reopen_listener);

        let _ = toolbar.append_child(&filters);
        let _ = toolbar.append_child(&all_button);
        let _ = toolbar.append_child(&auto_label);
        let _ = toolbar.append_child(&clear_button);
        let _ = panel.append_child(&header);
        let _ = panel.append_child(&toolbar);
        let _ = panel.append_child(&status);
        let _ = panel.append_child(&list);
        let body = document.body()?;
        let _ = body.append_child(&panel).ok()?;
        let _ = body.append_child(&reopen_button).ok()?;
        Some(panel)
    }

    fn set_minimized(minimized: bool) {
        lock_options().minimized = minimized;
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            update_visibility(&document, minimized);
        }
    }

    fn update_visibility(document: &Document, minimized: bool) {
        if let Some(panel) = document.get_element_by_id(PANEL_ID) {
            let style = if minimized {
                format!("{PANEL_STYLE}display:none;")
            } else {
                PANEL_STYLE.to_owned()
            };
            set_style(&panel, &style);
        }
        if let Some(button) = document.get_element_by_id(REOPEN_ID) {
            let style = if minimized {
                REOPEN_STYLE.replace("display:none;", "display:flex;")
            } else {
                REOPEN_STYLE.to_owned()
            };
            set_style(&button, &style);
        }
    }

    fn make_button(document: &Document, text: &str) -> Option<Element> {
        let button = document.create_element("button").ok()?;
        let _ = button.set_attribute("type", "button");
        button.set_text_content(Some(text));
        set_style(
            &button,
            "padding:3px 7px;border:1px solid #ffffff38;border-radius:4px;background:#ffffff12;color:#eeeeee;font:inherit;cursor:pointer;",
        );
        Some(button)
    }

    fn make_log_row(document: &Document, entry: &LogEntry) -> Option<Element> {
        let row = document.create_element("div").ok()?;
        set_style(
            &row,
            "display:flex;align-items:baseline;flex-wrap:wrap;gap:4px 9px;padding:3px 0;border-bottom:1px solid #ffffff12;",
        );

        let timestamp = document.create_element("span").ok()?;
        timestamp.set_text_content(Some(&format_timestamp(entry.timestamp)));
        set_style(
            &timestamp,
            "color:#909098;font-size:11px;white-space:nowrap;",
        );

        let level = document.create_element("strong").ok()?;
        level.set_text_content(Some(&format!("{:>5}", entry.level)));
        set_style(
            &level,
            &format!(
                "min-width:40px;text-align:right;color:{};",
                level_color(entry.level)
            ),
        );

        let target = document.create_element("span").ok()?;
        target.set_text_content(Some(&entry.target));
        set_style(&target, "color:#a0a0a8;font-size:11px;");

        let message = document.create_element("span").ok()?;
        message.set_text_content(Some(&entry.message));
        set_style(&message, "white-space:pre-wrap;overflow-wrap:anywhere;");

        let _ = row.append_child(&timestamp);
        let _ = row.append_child(&level);
        let _ = row.append_child(&target);
        let _ = row.append_child(&message);
        Some(row)
    }

    fn add_filter_listener(input: &Element, level: Level) {
        let listener = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            if let Some(checked) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                .map(|input| input.checked())
            {
                lock_options().set_level(level, checked);
                refresh();
            }
        });
        add_listener(input, "change", listener);
    }

    fn add_listener(element: &Element, event_name: &str, listener: Closure<dyn FnMut(Event)>) {
        let _ =
            element.add_event_listener_with_callback(event_name, listener.as_ref().unchecked_ref());
        listener.forget();
    }

    fn update_filter_controls(document: &Document, options: &ConsoleOptions) {
        for (level, checked) in [
            (Level::Trace, options.show_trace),
            (Level::Debug, options.show_debug),
            (Level::Info, options.show_info),
            (Level::Warn, options.show_warn),
            (Level::Error, options.show_error),
        ] {
            if let Some(input) = document
                .get_element_by_id(&filter_id(level))
                .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
            {
                input.set_checked(checked);
            }
        }

        if let Some(input) = document
            .get_element_by_id("examplify-console-auto-scroll")
            .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
        {
            input.set_checked(options.auto_scroll);
        }
    }

    fn filter_id(level: Level) -> String {
        format!(
            "examplify-console-filter-{}",
            level.to_string().to_lowercase()
        )
    }

    fn remove_children(element: &Element) {
        while let Some(child) = element.first_child() {
            let _ = element.remove_child(&child);
        }
    }

    fn set_style(element: &Element, style: &str) {
        let _ = element.set_attribute("style", style);
    }

    fn level_color(level: Level) -> &'static str {
        match level {
            Level::Error => "#ff6961",
            Level::Warn => "#ffbe55",
            Level::Info => "#69beff",
            Level::Debug => "#a591ff",
            Level::Trace => "#919baa",
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

    fn browser_time() -> SystemTime {
        let millis = js_sys::Date::now();
        if !millis.is_finite() || millis <= 0.0 {
            return UNIX_EPOCH;
        }

        UNIX_EPOCH
            .checked_add(Duration::from_millis(millis as u64))
            .unwrap_or(UNIX_EPOCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

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
    fn clearing_the_buffer_removes_all_records() {
        let mut buffer = LogBuffer::default();
        buffer.push(entry("hello"));
        buffer.entries.clear();

        assert!(buffer.entries.is_empty());
    }
}
