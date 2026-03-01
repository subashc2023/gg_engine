pub trait Application {
    fn new() -> Self
    where
        Self: Sized;

    fn run(&mut self) {
        loop {
            // TODO: will be replaced with a proper game loop
            std::thread::yield_now();
        }
    }
}

/// Initialize the engine and run the application.
pub fn run<T: Application>() {
    crate::log_init();
    log::info!(target: "gg_engine", "Engine v{}", crate::engine_version());
    let mut app = T::new();
    app.run();
    log::info!(target: "gg_engine", "Shutting down");
}
