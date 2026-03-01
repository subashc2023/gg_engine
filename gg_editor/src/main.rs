use gg_engine::prelude::*;

struct GGEditor;

impl Application for GGEditor {
    fn new() -> Self {
        info!("GGEditor initialized");
        GGEditor
    }
}

fn main() {
    run::<GGEditor>();
}
