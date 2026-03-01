use gg_engine::prelude::*;

struct GGEditor;

impl Application for GGEditor {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("GGEditor initialized");
        GGEditor
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEditor".into(),
            ..Default::default()
        }
    }

    fn on_event(&mut self, event: &Event) {
        trace!("{event}");
    }
}

fn main() {
    run::<GGEditor>();
}
