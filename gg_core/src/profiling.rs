use std::cell::RefCell;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Per-frame egui profiling (thread-local drain) — always available
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProfileResult {
    pub name: &'static str,
    pub time_ms: f32,
}

thread_local! {
    static PROFILE_RESULTS: RefCell<Vec<ProfileResult>> = const { RefCell::new(Vec::new()) };
}

/// Global results from worker threads (thread-local storage is per-thread,
/// so worker threads push here instead).
static WORKER_RESULTS: Mutex<Vec<ProfileResult>> = Mutex::new(Vec::new());

/// Drain all profile results collected since the last call.
/// Collects from both the main thread's thread-local storage and the
/// global worker thread results.
pub fn drain_profile_results() -> Vec<ProfileResult> {
    let mut results = PROFILE_RESULTS.with(|r| r.borrow_mut().drain(..).collect::<Vec<_>>());
    if let Ok(mut worker) = WORKER_RESULTS.lock() {
        results.append(&mut worker);
    }
    results
}

// ===========================================================================
// Feature: profiling ENABLED
// ===========================================================================

#[cfg(feature = "profiling")]
mod inner {
    use super::{ProfileResult, PROFILE_RESULTS};
    use std::fs::File;
    use std::io::{BufWriter, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Monotonic thread ID counter (avoids relying on `ThreadId`'s Debug format).
    static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

    /// Fast-path flag: avoids taking the mutex when no session is active.
    static SESSION_ACTIVE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    thread_local! {
        static THREAD_ID: u64 = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
    }

    // -----------------------------------------------------------------------
    // Chrome Tracing JSON instrumentor
    // -----------------------------------------------------------------------

    struct InstrumentorSession {
        writer: BufWriter<File>,
        start: Instant,
        event_count: usize,
    }

    static INSTRUMENTOR: Mutex<Option<InstrumentorSession>> = Mutex::new(None);

    /// Begin a profiling session that writes Chrome Tracing JSON to `filepath`.
    ///
    /// Open the resulting `.json` file in `chrome://tracing` (or `edge://tracing`)
    /// to visualize the timeline. Only one session can be active at a time; calling
    /// this while a session is already active will end the previous session first.
    pub fn begin_session(name: &str, filepath: &str) {
        let mut guard = match INSTRUMENTOR.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        // End any existing session before starting a new one.
        if let Some(mut prev) = guard.take() {
            if let Err(e) = write!(prev.writer, "]}}") {
                log::warn!(target: "gg_engine", "Failed to write profiling footer: {e}");
            }
            if let Err(e) = prev.writer.flush() {
                log::warn!(target: "gg_engine", "Failed to flush profiling output: {e}");
            }
            log::warn!(target: "gg_engine", "Ended previous profiling session before starting new one");
        }

        // Resolve relative paths against the executable's directory so profile
        // JSONs land next to the binary (e.g. target/debug/) instead of CWD.
        let resolved: PathBuf = {
            let path = Path::new(filepath);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|dir| dir.join(path)))
                    .unwrap_or_else(|| path.to_path_buf())
            }
        };

        let file = match File::create(&resolved) {
            Ok(f) => f,
            Err(e) => {
                log::warn!(target: "gg_engine",
                    "Cannot create profiling output '{}': {e}. Profiling disabled for this session.",
                    resolved.display()
                );
                return;
            }
        };
        let mut writer = BufWriter::new(file);
        if let Err(e) = write!(writer, r#"{{"otherData":{{}},"traceEvents":["#) {
            log::error!(target: "gg_engine", "Failed to write profiling header: {e}");
        }

        log::info!(target: "gg_engine", "Profiling session \"{name}\" -> {}", resolved.display());

        *guard = Some(InstrumentorSession {
            writer,
            start: Instant::now(),
            event_count: 0,
        });
        SESSION_ACTIVE.store(true, Ordering::Release);
    }

    /// End the current profiling session and flush the JSON file.
    pub fn end_session() {
        SESSION_ACTIVE.store(false, Ordering::Release);
        let mut guard = match INSTRUMENTOR.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(mut session) = guard.take() {
            if let Err(e) = write!(session.writer, "]}}") {
                log::warn!(target: "gg_engine", "Failed to write profiling footer: {e}");
            }
            if let Err(e) = session.writer.flush() {
                log::warn!(target: "gg_engine", "Failed to flush profiling output: {e}");
            }
        }
    }

    /// Write a single Chrome Tracing "X" (complete) event to the active session.
    fn write_profile(name: &str, start: Instant, duration: Duration) {
        // Fast path: skip mutex entirely when no session is recording.
        if !SESSION_ACTIVE.load(Ordering::Acquire) {
            return;
        }

        let tid = THREAD_ID.with(|id| *id);

        let mut guard = match INSTRUMENTOR.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(session) = guard.as_mut() {
            let start_us = start.saturating_duration_since(session.start).as_micros() as u64;
            let dur_us = duration.as_micros() as u64;

            if session.event_count > 0 {
                let _ = write!(session.writer, ",");
            }
            let _ = write!(
                session.writer,
                r#"{{"cat":"function","dur":{dur_us},"name":"{name}","ph":"X","pid":0,"tid":{tid},"ts":{start_us}}}"#,
            );
            session.event_count += 1;
        }
    }

    // -----------------------------------------------------------------------
    // RAII scope timer
    // -----------------------------------------------------------------------

    /// RAII timer that records a [`ProfileResult`] when dropped and writes a
    /// Chrome Tracing event if an instrumentor session is active.
    ///
    /// Constructed via [`profile_scope!`]. Measures wall-clock time from
    /// creation to drop (or explicit [`stop`](ProfileTimer::stop) call).
    pub struct ProfileTimer {
        name: &'static str,
        start: Instant,
        stopped: bool,
    }

    impl ProfileTimer {
        #[inline]
        pub fn new(name: &'static str) -> Self {
            Self {
                name,
                start: Instant::now(),
                stopped: false,
            }
        }

        #[inline]
        pub fn stop(&mut self) {
            if !self.stopped {
                let elapsed = self.start.elapsed();
                let time_ms = elapsed.as_secs_f32() * 1000.0;

                // Record for egui display.
                // Worker threads (named "gg-worker-*") push to the global
                // WORKER_RESULTS so drain_profile_results() can collect them.
                let is_worker = std::thread::current()
                    .name()
                    .is_some_and(|n| n.starts_with("gg-worker"));
                if is_worker {
                    if let Ok(mut worker) = super::WORKER_RESULTS.lock() {
                        worker.push(ProfileResult {
                            name: self.name,
                            time_ms,
                        });
                    }
                } else {
                    PROFILE_RESULTS.with(|results| {
                        results.borrow_mut().push(ProfileResult {
                            name: self.name,
                            time_ms,
                        });
                    });
                }

                // Record for Chrome Tracing JSON output.
                write_profile(self.name, self.start, elapsed);

                self.stopped = true;
            }
        }
    }

    impl Drop for ProfileTimer {
        #[inline]
        fn drop(&mut self) {
            self.stop();
        }
    }

    /// Returns `true` if a Chrome Tracing session is currently recording.
    pub fn is_session_active() -> bool {
        SESSION_ACTIVE.load(Ordering::Acquire)
    }
}

#[cfg(feature = "profiling")]
pub use inner::*;

// ===========================================================================
// Feature: profiling DISABLED — zero-cost stubs
// ===========================================================================

#[cfg(not(feature = "profiling"))]
pub fn begin_session(_name: &str, _filepath: &str) {}

#[cfg(not(feature = "profiling"))]
pub fn end_session() {}

#[cfg(not(feature = "profiling"))]
pub fn is_session_active() -> bool {
    false
}

#[cfg(not(feature = "profiling"))]
pub struct ProfileTimer;

#[cfg(not(feature = "profiling"))]
impl ProfileTimer {
    #[inline]
    pub fn new(_name: &'static str) -> Self {
        Self
    }

    #[inline]
    pub fn stop(&mut self) {}
}

/// Instrument a scope with a named timer. The timer starts immediately
/// and records its duration when the enclosing scope ends.
///
/// When the `profiling` feature is disabled, this macro expands to nothing.
///
/// ```ignore
/// fn on_update(&mut self, dt: Timestep, input: &Input) {
///     profile_scope!("Sandbox2D::on_update");
///     // ... work ...
/// }
/// ```
#[cfg(feature = "profiling")]
#[macro_export]
macro_rules! profile_scope {
    ($name:expr) => {
        let _profile_timer = $crate::profiling::ProfileTimer::new($name);
    };
}

#[cfg(not(feature = "profiling"))]
#[macro_export]
macro_rules! profile_scope {
    ($name:expr) => {};
}
