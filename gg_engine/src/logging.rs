use log::{Level, LevelFilter, Log, Metadata, Record};
use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

static START_TIME: OnceLock<Instant> = OnceLock::new();

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

    fn elapsed() -> impl std::fmt::Display {
        let elapsed = START_TIME
            .get()
            .map(|s| s.elapsed())
            .unwrap_or_default();
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
        let ts = Self::elapsed();
        let _ = writeln!(
            std::io::stderr(),
            "{color}[{ts} {level} {tag}]: {args}\x1b[0m",
            level = record.level(),
            args = record.args(),
        );
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
    log::set_logger(&LOGGER).expect("Logger already initialized");
    log::set_max_level(LevelFilter::Trace);
}
