use gg_engine::prelude::*;

struct Sandbox;

impl Application for Sandbox {
    fn new(_layers: &mut LayerStack) -> Self {
        info!("Sandbox initialized");
        Sandbox
    }

    fn window_config(&self) -> WindowConfig {
        WindowConfig {
            title: "GGEngine Sandbox".into(),
            ..Default::default()
        }
    }

    fn on_event(&mut self, event: &Event, _input: &Input) {
        trace!("{event}");
    }
}

fn main() {
    run::<Sandbox>();
}
