pub mod command_buffer;
pub mod parallel;

use std::sync::OnceLock;

static THREAD_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();

/// Initialize the global job thread pool. Call once at engine startup.
/// Uses N-1 worker threads (main thread participates via rayon::scope).
pub fn init() {
    THREAD_POOL.get_or_init(|| {
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1).max(1))
            .unwrap_or(3);
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("gg-worker-{i}"))
            .build()
            .expect("Failed to create job thread pool")
    });
    log::info!(
        target: "gg_engine",
        "Job thread pool initialized ({} worker threads)",
        worker_count()
    );
}

/// Access the thread pool.
pub fn pool() -> &'static rayon::ThreadPool {
    THREAD_POOL.get().expect("jobs::init() not called")
}

/// Number of worker threads (excludes main thread).
pub fn worker_count() -> usize {
    pool().current_num_threads()
}
