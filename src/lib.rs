#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, OnceLock},
    time::SystemTime,
};

pub use log;

/// Maximum number of recent log records retained in memory.
pub const MAX_LOG_ENTRIES: usize = 1_000;

/// A captured log record, including its level and source metadata when available.
#[derive(Clone, Debug)]
pub struct LogEntry {
    /// Time when the record was captured.
    pub timestamp: SystemTime,
    /// Severity of the record.
    pub level: log::Level,
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

/// Install Examplify's logger for the current platform.
///
/// On native targets, records are forwarded to `env_logger`. On WebAssembly,
/// records are shown in both the browser's developer console and Examplify's
/// DOM console. WebAssembly panics are also captured as error-level records,
/// including the browser's JavaScript stack trace when available.
pub fn init() -> Init {
    let _ = try_init();
    Init
}

/// Handle for configuring the logger after calling [`init`].
pub struct Init;

impl Init {
    /// Set the maximum log level forwarded on native targets, or the default
    /// level filter shown in the WebAssembly console.
    ///
    /// On WebAssembly, all records are still captured and sent to the browser
    /// developer console; this only changes which levels are initially shown
    /// in Examplify's DOM console.
    pub fn with_log_level(self, _level: log::LevelFilter) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        platform::set_log_level(_level);
        #[cfg(target_arch = "wasm32")]
        platform::set_default_log_level(_level);

        self
    }
}

/// Fallible version of [`init`], useful when another global logger may already
/// be installed.
pub fn try_init() -> Result<(), log::SetLoggerError> {
    platform::install()?;

    #[cfg(target_arch = "wasm32")]
    {
        platform::refresh();
    }

    Ok(())
}

/// Return a snapshot of the recent captured records, oldest first.
pub fn entries() -> Vec<LogEntry> {
    lock_buffer().entries.iter().cloned().collect()
}

#[cfg(target_arch = "wasm32")]
fn captured_entries() -> Vec<(u64, LogEntry)> {
    let buffer = lock_buffer();
    buffer
        .ids
        .iter()
        .copied()
        .zip(buffer.entries.iter().cloned())
        .collect()
}

/// Remove all currently captured records and clear the browser console.
pub fn clear() {
    lock_buffer().clear();
    #[cfg(target_arch = "wasm32")]
    platform::refresh();
}

#[derive(Default)]
struct LogBuffer {
    entries: VecDeque<LogEntry>,
    #[cfg(any(target_arch = "wasm32", test))]
    ids: VecDeque<u64>,
    #[cfg(any(target_arch = "wasm32", test))]
    next_id: u64,
}

impl LogBuffer {
    #[cfg(any(target_arch = "wasm32", test))]
    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == MAX_LOG_ENTRIES {
            self.entries.pop_front();
            self.ids.pop_front();
        }
        self.ids.push_back(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.entries.push_back(entry);
    }

    fn clear(&mut self) {
        self.entries.clear();
        #[cfg(any(target_arch = "wasm32", test))]
        self.ids.clear();
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
        collections::HashSet,
        sync::{Mutex, MutexGuard, OnceLock},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use log::{Level, LevelFilter, Log, Metadata, Record};
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{
        Clipboard, ClipboardEvent, Document, Element, Event, HtmlElement, HtmlInputElement,
        KeyboardEvent, MouseEvent,
    };

    use super::{LogEntry, captured_entries, clear, lock_buffer};

    const PANEL_ID: &str = "examplify-console";
    const REOPEN_ID: &str = "examplify-console-reopen";
    const FILTERS_ID: &str = "examplify-console-filters";
    const SEARCH_ID: &str = "examplify-console-search";
    const ENTRIES_ID: &str = "examplify-console-entries";
    const STATUS_ID: &str = "examplify-console-status";
    const PANEL_STYLE: &str = "position:fixed;inset:0;z-index:2147483647;box-sizing:border-box;width:100vw;height:100vh;height:100dvh;max-height:none;display:flex;flex-direction:column;gap:12px;margin:0;padding:clamp(10px,2vw,24px);border:0;border-radius:0;background:#101014;color:#eeeeee;box-shadow:none;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;";
    const REOPEN_STYLE: &str = "position:fixed;right:16px;bottom:16px;z-index:2147483647;display:none;padding:9px 14px;border:1px solid #ffffff38;border-radius:6px;background:#202028;color:#eeeeee;font:13px ui-monospace,SFMono-Regular,Menlo,monospace;cursor:pointer;box-shadow:0 4px 16px #0009;";

    static LOGGER: ExamplifyLogger = ExamplifyLogger;

    static OPTIONS: OnceLock<Mutex<ConsoleOptions>> = OnceLock::new();

    pub(super) fn set_default_log_level(level: LevelFilter) {
        {
            let mut options = lock_options();
            for console_level in [
                Level::Trace,
                Level::Debug,
                Level::Info,
                Level::Warn,
                Level::Error,
            ] {
                options.set_level(console_level, console_level.to_level_filter() <= level);
            }
        }
        refresh();
    }

    struct ExamplifyLogger;

    pub(super) fn install() -> Result<(), log::SetLoggerError> {
        log::set_logger(&LOGGER)?;
        log::set_max_level(LevelFilter::Trace);
        std::panic::set_hook(Box::new(|info| {
            let panic_message = info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                .unwrap_or("non-string panic payload");

            let mut message = if let Some(location) = info.location() {
                format!(
                    "panicked at {}:{}:{}: {}",
                    location.file(),
                    location.line(),
                    location.column(),
                    panic_message
                )
            } else {
                format!("panicked: {panic_message}")
            };

            if let Some(stack) = browser_stack_trace() {
                message.push_str("\n\nStack: ");
                message.push_str(&stack);
            }

            log::error!(target: "panic", "{message}");
        }));
        Ok(())
    }

    fn browser_stack_trace() -> Option<String> {
        let error = js_sys::Error::new("");
        js_sys::Reflect::get(error.as_ref(), &wasm_bindgen::JsValue::from_str("stack"))
            .ok()?
            .as_string()
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
            console_log::log(record);
            lock_buffer().push(entry);
            refresh();
        }

        fn flush(&self) {}
    }

    #[derive(Clone)]
    struct ConsoleOptions {
        auto_scroll: bool,
        minimized: bool,
        search_query: String,
        selected_ids: HashSet<u64>,
        selection_anchor: Option<u64>,
        visible_ids: Vec<u64>,
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
                search_query: String::new(),
                selected_ids: HashSet::new(),
                selection_anchor: None,
                visible_ids: Vec::new(),
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

        let snapshot = captured_entries();
        let mut options = lock_options().clone();
        let valid_ids: HashSet<_> = snapshot.iter().map(|(id, _)| *id).collect();
        options.selected_ids.retain(|id| valid_ids.contains(id));
        if options
            .selection_anchor
            .is_some_and(|id| !valid_ids.contains(&id))
        {
            options.selection_anchor = None;
        }
        lock_options().selected_ids = options.selected_ids.clone();
        lock_options().selection_anchor = options.selection_anchor;
        update_filter_controls(&document, &options);
        update_visibility(&document, options.minimized);

        let Some(list) = document.get_element_by_id(ENTRIES_ID) else {
            return;
        };
        remove_children(&list);

        let visible: Vec<_> = snapshot
            .iter()
            .filter(|(_, entry)| {
                options.includes(entry.level) && matches_search(entry, &options.search_query)
            })
            .map(|(id, entry)| (*id, entry))
            .collect();
        let visible_ids: Vec<_> = visible.iter().map(|(id, _)| *id).collect();
        lock_options().visible_ids = visible_ids.clone();

        update_status(
            &document,
            visible.len(),
            snapshot.len(),
            options.selected_ids.len(),
        );

        if visible.is_empty() {
            if let Ok(empty) = document.create_element("div") {
                let message = if snapshot.is_empty() {
                    "No log messages yet."
                } else {
                    "No matching log messages."
                };
                empty.set_text_content(Some(message));
                let _ =
                    empty.set_attribute("style", "padding:12px 0;color:#909098;font-style:italic");
                let _ = list.append_child(&empty);
            }
        } else {
            for (id, entry) in visible {
                if let Some(row) =
                    make_log_row(&document, id, entry, options.selected_ids.contains(&id))
                {
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

        let search = document.create_element("input").ok()?;
        search.set_id(SEARCH_ID);
        let _ = search.set_attribute("type", "search");
        let _ = search.set_attribute("placeholder", "Search logs…");
        let _ = search.set_attribute("aria-label", "Search log messages");
        set_style(
            &search,
            "box-sizing:border-box;flex:1 1 220px;width:auto;min-width:180px;height:28px;padding:3px 7px;border:1px solid #ffffff38;border-radius:4px;background:#ffffff0d;color:#eeeeee;font:inherit;line-height:1.5;",
        );
        let search_listener = Closure::<dyn FnMut(Event)>::new(|event: Event| {
            if let Some(query) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                .map(|input| input.value())
            {
                lock_options().search_query = query;
                refresh();
            }
        });
        add_listener(&search, "input", search_listener);

        let filters = document.create_element("div").ok()?;
        filters.set_id(FILTERS_ID);
        set_style(
            &filters,
            "display:flex;align-items:center;flex-wrap:wrap;gap:8px;",
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

        let level_controls = document.create_element("div").ok()?;
        set_style(
            &level_controls,
            "display:flex;align-items:center;flex-wrap:wrap;gap:8px;flex:0 1 auto;min-width:0;",
        );
        let _ = level_controls.append_child(&filters);
        let _ = level_controls.append_child(&all_button);

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
        let copy_link_listener = Closure::<dyn FnMut(Event)>::new(handle_copy_link);
        add_listener(&status, "click", copy_link_listener);

        let list = document.create_element("div").ok()?;
        list.set_id(ENTRIES_ID);
        let _ = list.set_attribute("role", "log");
        let _ = list.set_attribute("aria-live", "polite");
        let _ = list.set_attribute("aria-relevant", "additions");
        set_style(
            &list,
            "flex:1;min-height:0;overflow:auto;border-top:1px solid #ffffff24;padding-top:8px;",
        );
        let click_listener = Closure::<dyn FnMut(Event)>::new(handle_log_row_click);
        add_listener(&list, "click", click_listener);
        let keyboard_listener = Closure::<dyn FnMut(Event)>::new(handle_log_row_keydown);
        add_listener(&list, "keydown", keyboard_listener);

        let reopen_button = make_button(document, "Logs")?;
        reopen_button.set_id(REOPEN_ID);
        let _ = reopen_button.set_attribute("aria-label", "Reopen log console");
        set_style(&reopen_button, REOPEN_STYLE);
        let reopen_listener = Closure::<dyn FnMut(Event)>::new(|_| set_minimized(false));
        add_listener(&reopen_button, "click", reopen_listener);

        let _ = toolbar.append_child(&level_controls);
        let _ = toolbar.append_child(&search);
        let _ = toolbar.append_child(&auto_label);
        let _ = toolbar.append_child(&clear_button);
        let _ = panel.append_child(&header);
        let _ = panel.append_child(&toolbar);
        let _ = panel.append_child(&status);
        let _ = panel.append_child(&list);
        let copy_listener = Closure::<dyn FnMut(Event)>::new(handle_copy);
        add_listener(&panel, "copy", copy_listener);
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

    fn make_log_row(
        document: &Document,
        id: u64,
        entry: &LogEntry,
        selected: bool,
    ) -> Option<Element> {
        let row = document.create_element("div").ok()?;
        row.set_id(&format!("examplify-console-entry-{id}"));
        let _ = row.set_attribute("data-examplify-log-id", &id.to_string());
        set_style(&row, &log_row_style(selected));
        let _ = row.set_attribute("role", "button");
        let _ = row.set_attribute("aria-pressed", if selected { "true" } else { "false" });
        let _ = row.set_attribute("tabindex", "0");

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

    fn handle_log_row_click(event: Event) {
        let Some(row) = find_log_row(&event) else {
            return;
        };
        let Some(id) = row
            .get_attribute("data-examplify-log-id")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return;
        };
        let (visible_ids, shift, toggle) = {
            let options = lock_options();
            let (shift, toggle) = event
                .dyn_ref::<MouseEvent>()
                .map(|event| (event.shift_key(), event.ctrl_key() || event.meta_key()))
                .unwrap_or_default();
            (options.visible_ids.clone(), shift, toggle)
        };
        apply_selection(id, shift, toggle, &visible_ids);
    }

    fn handle_log_row_keydown(event: Event) {
        let Some(keyboard_event) = event.dyn_ref::<KeyboardEvent>() else {
            return;
        };
        if (keyboard_event.ctrl_key() || keyboard_event.meta_key())
            && keyboard_event.key().eq_ignore_ascii_case("a")
        {
            event.prevent_default();
            select_all_visible();
            return;
        }
        if keyboard_event.key() != "Enter" && keyboard_event.key() != " " {
            return;
        }
        let Some(row) = find_log_row(&event) else {
            return;
        };
        let Some(id) = row
            .get_attribute("data-examplify-log-id")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return;
        };
        event.prevent_default();
        let visible_ids = lock_options().visible_ids.clone();
        apply_selection(id, false, false, &visible_ids);
    }

    fn find_log_row(event: &Event) -> Option<Element> {
        let mut element = event.target()?.dyn_into::<Element>().ok()?;
        loop {
            if element.has_attribute("data-examplify-log-id") {
                return Some(element);
            }
            element = element.parent_element()?;
        }
    }

    fn apply_selection(id: u64, shift: bool, toggle: bool, visible_ids: &[u64]) {
        let (selected_ids, selected_count) = {
            let mut options = lock_options();
            let clicked_index = visible_ids.iter().position(|visible_id| *visible_id == id);
            let anchor_index = options.selection_anchor.and_then(|anchor| {
                visible_ids
                    .iter()
                    .position(|visible_id| *visible_id == anchor)
            });

            if shift {
                if let Some(clicked_index) = clicked_index {
                    let anchor_index = anchor_index.unwrap_or_else(|| {
                        options.selection_anchor = Some(id);
                        clicked_index
                    });
                    if !toggle {
                        options.selected_ids.clear();
                    }
                    for range_index in
                        anchor_index.min(clicked_index)..=anchor_index.max(clicked_index)
                    {
                        options.selected_ids.insert(visible_ids[range_index]);
                    }
                }
            } else if toggle {
                if !options.selected_ids.remove(&id) {
                    options.selected_ids.insert(id);
                }
                options.selection_anchor = Some(id);
            } else {
                options.selected_ids.clear();
                options.selected_ids.insert(id);
                options.selection_anchor = Some(id);
            }
            (options.selected_ids.clone(), options.selected_ids.len())
        };

        update_selection_display(visible_ids, &selected_ids, selected_count);
    }

    fn select_all_visible() {
        let (visible_ids, selected_ids, selected_count) = {
            let mut options = lock_options();
            let visible_ids = options.visible_ids.clone();
            options.selected_ids = visible_ids.iter().copied().collect();
            options.selection_anchor = visible_ids.first().copied();
            (
                visible_ids,
                options.selected_ids.clone(),
                options.selected_ids.len(),
            )
        };
        update_selection_display(&visible_ids, &selected_ids, selected_count);
    }

    fn update_selection_display(
        visible_ids: &[u64],
        selected_ids: &HashSet<u64>,
        selected_count: usize,
    ) {
        if let Some(document) = web_sys::window().and_then(|window| window.document()) {
            for visible_id in visible_ids {
                if let Some(row) =
                    document.get_element_by_id(&format!("examplify-console-entry-{visible_id}"))
                {
                    let selected = selected_ids.contains(visible_id);
                    let _ =
                        row.set_attribute("aria-pressed", if selected { "true" } else { "false" });
                    set_style(&row, &log_row_style(selected));
                }
            }
            update_status(
                &document,
                visible_ids.len(),
                captured_entries().len(),
                selected_count,
            );
        }
    }

    fn log_row_style(selected: bool) -> String {
        let mut style = String::from(
            "display:flex;align-items:baseline;flex-wrap:wrap;gap:4px 9px;padding:3px 0;border-bottom:1px solid #ffffff12;cursor:pointer;",
        );
        if selected {
            style.push_str("background:#1d4ed866;");
        }
        style
    }

    fn handle_copy(event: Event) {
        if event
            .target()
            .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
            .is_some_and(|input| input.type_() != "checkbox")
        {
            return;
        }

        let Some(clipboard) = event
            .dyn_ref::<ClipboardEvent>()
            .and_then(ClipboardEvent::clipboard_data)
        else {
            return;
        };
        let Some(copied) = selected_log_text() else {
            return;
        };

        if clipboard.set_data("text/plain", &copied).is_ok() {
            event.prevent_default();
        }
    }

    fn handle_copy_link(event: Event) {
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        else {
            return;
        };
        if !target.has_attribute("data-copy-selected") {
            return;
        }
        event.prevent_default();

        let Some(text) = selected_log_text() else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let clipboard = js_sys::Reflect::get(
            window.navigator().as_ref(),
            &wasm_bindgen::JsValue::from_str("clipboard"),
        )
        .ok()
        .and_then(|clipboard| clipboard.dyn_into::<Clipboard>().ok());

        if let Some(clipboard) = clipboard {
            let promise = clipboard.write_text(&text);
            wasm_bindgen_futures::spawn_local(async move {
                if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                    log::warn!("Could not copy selected log messages to the clipboard");
                }
            });
        } else {
            log::warn!("Could not copy selected log messages to the clipboard");
        }
    }

    fn selected_log_text() -> Option<String> {
        let selected_ids = lock_options().selected_ids.clone();
        if selected_ids.is_empty() {
            return None;
        }

        let text = captured_entries()
            .into_iter()
            .filter(|(id, _)| selected_ids.contains(id))
            .map(|(_, entry)| {
                format!(
                    "{} {:>5} {} {}",
                    format_timestamp(entry.timestamp),
                    entry.level,
                    entry.target,
                    entry.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        (!text.is_empty()).then_some(text)
    }

    fn update_status(document: &Document, shown: usize, total: usize, selected: usize) {
        if let Some(status) = document.get_element_by_id(STATUS_ID) {
            if selected == 0 {
                status.set_text_content(Some(&format!("{shown} shown, {total} total")));
            } else {
                status.set_text_content(Some(&format!(
                    "{shown} shown, {total} total, {selected} selected | "
                )));
                if let Ok(link) = document.create_element("a") {
                    let _ = link.set_attribute("href", "#");
                    let _ = link.set_attribute("data-copy-selected", "true");
                    let _ = link.set_attribute("aria-label", "Copy selected log messages");
                    link.set_text_content(Some("Copy selected"));
                    set_style(
                        &link,
                        "color:#8ab4f8;text-decoration:underline;cursor:pointer;",
                    );
                    let _ = status.append_child(&link);
                }
            }
        }
    }

    fn matches_search(entry: &LogEntry, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return true;
        }

        let query = query.to_lowercase();
        [
            entry.level.to_string(),
            entry.target.clone(),
            entry.message.clone(),
            entry.module_path.clone().unwrap_or_default(),
            entry.file.clone().unwrap_or_default(),
            entry.line.map(|line| line.to_string()).unwrap_or_default(),
        ]
        .iter()
        .any(|field| field.to_lowercase().contains(&query))
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

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    };

    use log::{LevelFilter, Log, Metadata, Record};

    static LOGGER: ExamplifyLogger = ExamplifyLogger;
    static BACKEND: OnceLock<env_logger::Logger> = OnceLock::new();
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static LOG_LEVEL: AtomicU8 = AtomicU8::new(level_filter_to_u8(LevelFilter::Debug));

    struct ExamplifyLogger;

    pub(super) fn install() -> Result<(), log::SetLoggerError> {
        BACKEND.get_or_init(|| {
            env_logger::Builder::new()
                .filter_level(LevelFilter::Trace)
                .build()
        });

        log::set_logger(&LOGGER)?;
        INSTALLED.store(true, Ordering::Release);
        log::set_max_level(current_log_level());
        Ok(())
    }

    pub(super) fn set_log_level(level: LevelFilter) {
        if !INSTALLED.load(Ordering::Acquire) {
            return;
        }

        LOG_LEVEL.store(level_filter_to_u8(level), Ordering::Relaxed);
        log::set_max_level(level);
    }

    impl Log for ExamplifyLogger {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            BACKEND.get().is_some() && metadata.level() <= current_log_level()
        }

        fn log(&self, record: &Record<'_>) {
            if self.enabled(record.metadata()) {
                if let Some(backend) = BACKEND.get() {
                    backend.log(record);
                }
            }
        }

        fn flush(&self) {
            if let Some(backend) = BACKEND.get() {
                backend.flush();
            }
        }
    }

    fn current_log_level() -> LevelFilter {
        level_filter_from_u8(LOG_LEVEL.load(Ordering::Relaxed))
    }

    const fn level_filter_to_u8(level: LevelFilter) -> u8 {
        match level {
            LevelFilter::Off => 0,
            LevelFilter::Error => 1,
            LevelFilter::Warn => 2,
            LevelFilter::Info => 3,
            LevelFilter::Debug => 4,
            LevelFilter::Trace => 5,
        }
    }

    const fn level_filter_from_u8(level: u8) -> LevelFilter {
        match level {
            0 => LevelFilter::Off,
            1 => LevelFilter::Error,
            2 => LevelFilter::Warn,
            3 => LevelFilter::Info,
            4 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn entry(message: impl Into<String>) -> LogEntry {
        LogEntry {
            timestamp: UNIX_EPOCH,
            level: log::Level::Info,
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
        assert_eq!(buffer.ids.len(), MAX_LOG_ENTRIES);
        assert_eq!(buffer.ids.front(), Some(&1));
        assert_eq!(buffer.ids.back(), Some(&(MAX_LOG_ENTRIES as u64)));
    }

    #[test]
    fn clearing_the_buffer_removes_all_records() {
        let mut buffer = LogBuffer::default();
        buffer.push(entry("hello"));
        let previous_id = buffer.ids.front().copied();
        buffer.clear();

        assert!(buffer.entries.is_empty());
        assert!(buffer.ids.is_empty());

        buffer.push(entry("after clear"));
        assert_ne!(buffer.ids.front().copied(), previous_id);
    }
}
