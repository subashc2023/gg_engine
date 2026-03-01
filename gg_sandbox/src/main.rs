use gg_engine::prelude::*;

struct Sandbox;

impl Application for Sandbox {
    fn new() -> Self {
        info!("Sandbox initialized");
        Sandbox
    }
}

fn main() {
    run::<Sandbox>();
}
