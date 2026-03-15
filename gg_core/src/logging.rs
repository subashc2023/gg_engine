use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Maximum number of log entries kept in the capture buffer.
const LOG_BUFFER_CAPACITY: usize = 1024;

/// A captured log entry for display in the editor console.
#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub timestamp: String,
    pub tag: String,
    pub message: String,
}

static LOG_BUFFER: OnceLock<Mutex<VecDeque<LogEntry>>> = OnceLock::new();

/// Read captured log entries. The closure receives a slice of all buffered entries.
pub fn with_log_buffer<R>(f: impl FnOnce(&VecDeque<LogEntry>) -> R) -> R {
    let buf = LOG_BUFFER.get_or_init(|| Mutex::new(VecDeque::new()));
    match buf.lock() {
        Ok(guard) => f(&guard),
        Err(poisoned) => f(&poisoned.into_inner()),
    }
}

/// Clear all captured log entries.
pub fn clear_log_buffer() {
    if let Some(buf) = LOG_BUFFER.get() {
        if let Ok(mut guard) = buf.lock() {
            guard.clear();
        }
    }
}

struct EngineLogger;

impl EngineLogger {
    fn level_color(level: Level) -> &'static str {
        match level {
            Level::Error => "\x1b[31m",
            Level::Warn => "\x1b[33m",
            Level::Info => "\x1b[32m",
            Level::Debug => "\x1b[36m",
            Level::Trace => "\x1b[0m",
        }
    }

    fn tag(target: &str) -> &str {
        if target.starts_with("gg_") {
            "GGEngine"
        } else {
            "APP"
        }
    }

    fn elapsed_string() -> String {
        let elapsed = START_TIME.get().map(|s| s.elapsed()).unwrap_or_default();
        let total_secs = elapsed.as_secs();
        let m = total_secs / 60;
        let s = total_secs % 60;
        let ms = elapsed.subsec_millis();
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

impl Log for EngineLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let color = Self::level_color(record.level());
        let tag = Self::tag(record.target());
        let ts = Self::elapsed_string();
        let msg = format!("{}", record.args());

        // Write to stderr.
        let _ = writeln!(
            std::io::stderr(),
            "{color}[{ts} {level} {tag}]: {msg}\x1b[0m",
            level = record.level(),
        );

        // Capture into ring buffer.
        let buf = LOG_BUFFER.get_or_init(|| Mutex::new(VecDeque::new()));
        if let Ok(mut guard) = buf.lock() {
            if guard.len() >= LOG_BUFFER_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(LogEntry {
                level: record.level(),
                timestamp: ts,
                tag: tag.to_string(),
                message: msg,
            });
        }
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

static LOGGER: EngineLogger = EngineLogger;

/// Initialize the engine's logging system.
///
/// Called automatically by [`gg_main!`] before the application starts.
pub fn init() {
    START_TIME.get_or_init(Instant::now);
    if log::set_logger(&LOGGER).is_err() {
        // Logger already initialized (e.g. in tests or restart scenarios).
        return;
    }
    log::set_max_level(LevelFilter::Trace);
}
