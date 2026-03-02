use crate::events::Event;
use crate::input::Input;
use crate::timestep::Timestep;

// ---------------------------------------------------------------------------
// Layer trait
// ---------------------------------------------------------------------------

pub trait Layer {
    /// Debug name for logging.
    fn name(&self) -> &str {
        "Layer"
    }

    /// Called when this layer is pushed onto the stack.
    fn on_attach(&mut self) {}

    /// Called when this layer is removed from the stack.
    fn on_detach(&mut self) {}

    /// Called every frame. Layers are updated bottom-to-top.
    /// `dt` is the time elapsed since the last frame.
    /// `input` provides pollable keyboard/mouse state.
    fn on_update(&mut self, _dt: Timestep, _input: &Input) {}

    /// Called for each event. Layers receive events top-to-bottom.
    /// Return `true` if the event was handled and should not propagate further.
    /// `input` provides pollable keyboard/mouse state.
    fn on_event(&mut self, _event: &Event, _input: &Input) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// LayerStack
// ---------------------------------------------------------------------------

/// Ordered collection of layers with two zones:
/// `[layer_0 .. layer_N | overlay_0 .. overlay_M]`
///
/// Normal layers are inserted before the `insert_index`; overlays are
/// appended to the end. Updates iterate forward (bottom-to-top); event
/// dispatch iterates backward (top-to-bottom).
pub struct LayerStack {
    layers: Vec<Box<dyn Layer>>,
    insert_index: usize,
}

impl LayerStack {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            insert_index: 0,
        }
    }

    /// Push a normal layer. Inserted before overlays.
    pub fn push_layer(&mut self, mut layer: Box<dyn Layer>) {
        log::trace!(target: "gg_engine", "Pushing layer: {}", layer.name());
        layer.on_attach();
        self.layers.insert(self.insert_index, layer);
        self.insert_index += 1;
    }

    /// Push an overlay. Always appended to the end (rendered last, receives events first).
    pub fn push_overlay(&mut self, mut layer: Box<dyn Layer>) {
        log::trace!(target: "gg_engine", "Pushing overlay: {}", layer.name());
        layer.on_attach();
        self.layers.push(layer);
    }

    /// Update all layers forward (bottom-to-top).
    pub fn update_all(&mut self, dt: Timestep, input: &Input) {
        for layer in self.layers.iter_mut() {
            layer.on_update(dt, input);
        }
    }

    /// Dispatch an event through the stack in reverse order (top-to-bottom).
    /// Returns `true` if any layer handled the event.
    pub fn dispatch_event(&mut self, event: &Event, input: &Input) -> bool {
        for layer in self.layers.iter_mut().rev() {
            if layer.on_event(event, input) {
                return true;
            }
        }
        false
    }
}

impl Default for LayerStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LayerStack {
    fn drop(&mut self) {
        for layer in self.layers.iter_mut().rev() {
            log::trace!(target: "gg_engine", "Detaching layer: {}", layer.name());
            layer.on_detach();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{MouseButton, MouseEvent};
    use crate::input::Input;
    use crate::timestep::Timestep;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    struct TestLayer {
        label: String,
        log: Log,
    }

    impl TestLayer {
        fn new(label: &str, log: &Log) -> Self {
            Self {
                label: label.into(),
                log: Rc::clone(log),
            }
        }
    }

    impl Layer for TestLayer {
        fn name(&self) -> &str {
            &self.label
        }
        fn on_attach(&mut self) {
            self.log.borrow_mut().push(format!("{} attach", self.label));
        }
        fn on_detach(&mut self) {
            self.log.borrow_mut().push(format!("{} detach", self.label));
        }
        fn on_update(&mut self, _dt: Timestep, _input: &Input) {
            self.log.borrow_mut().push(format!("{} update", self.label));
        }
        fn on_event(&mut self, _event: &Event, _input: &Input) -> bool {
            self.log
                .borrow_mut()
                .push(format!("{} event", self.label));
            false
        }
    }

    struct HandlingLayer {
        label: String,
        log: Log,
    }

    impl HandlingLayer {
        fn new(label: &str, log: &Log) -> Self {
            Self {
                label: label.into(),
                log: Rc::clone(log),
            }
        }
    }

    impl Layer for HandlingLayer {
        fn name(&self) -> &str {
            &self.label
        }
        fn on_event(&mut self, _event: &Event, _input: &Input) -> bool {
            self.log
                .borrow_mut()
                .push(format!("{} handled", self.label));
            true
        }
    }

    fn dummy_event() -> Event {
        Event::Mouse(MouseEvent::ButtonPressed(MouseButton::Left))
    }

    #[test]
    fn push_layer_calls_on_attach() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_layer(Box::new(TestLayer::new("A", &log)));
        assert_eq!(&*log.borrow(), &["A attach"]);
    }

    #[test]
    fn push_overlay_calls_on_attach() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_overlay(Box::new(TestLayer::new("O", &log)));
        assert_eq!(&*log.borrow(), &["O attach"]);
    }

    #[test]
    fn update_iterates_forward() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_layer(Box::new(TestLayer::new("A", &log)));
        stack.push_layer(Box::new(TestLayer::new("B", &log)));
        stack.push_overlay(Box::new(TestLayer::new("X", &log)));
        log.borrow_mut().clear();

        let input = Input::new();
        stack.update_all(Timestep::from_seconds(0.016), &input);
        assert_eq!(&*log.borrow(), &["A update", "B update", "X update"]);
    }

    #[test]
    fn events_iterate_backward() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_layer(Box::new(TestLayer::new("A", &log)));
        stack.push_layer(Box::new(TestLayer::new("B", &log)));
        stack.push_overlay(Box::new(TestLayer::new("X", &log)));
        log.borrow_mut().clear();

        let input = Input::new();
        let handled = stack.dispatch_event(&dummy_event(), &input);
        assert!(!handled);
        assert_eq!(&*log.borrow(), &["X event", "B event", "A event"]);
    }

    #[test]
    fn event_propagation_stops_on_handled() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_layer(Box::new(TestLayer::new("A", &log)));
        stack.push_overlay(Box::new(HandlingLayer::new("X", &log)));
        log.borrow_mut().clear();

        let input = Input::new();
        let handled = stack.dispatch_event(&dummy_event(), &input);
        assert!(handled);
        // A should never receive the event
        assert_eq!(&*log.borrow(), &["X handled"]);
    }

    #[test]
    fn layers_inserted_before_overlays() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let mut stack = LayerStack::new();
        stack.push_overlay(Box::new(TestLayer::new("X", &log)));
        stack.push_layer(Box::new(TestLayer::new("A", &log)));
        log.borrow_mut().clear();

        // Update order should be: A (layer) then X (overlay)
        let input = Input::new();
        stack.update_all(Timestep::from_seconds(0.016), &input);
        assert_eq!(&*log.borrow(), &["A update", "X update"]);
    }

    #[test]
    fn drop_calls_on_detach_reverse() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        {
            let mut stack = LayerStack::new();
            stack.push_layer(Box::new(TestLayer::new("A", &log)));
            stack.push_layer(Box::new(TestLayer::new("B", &log)));
            stack.push_overlay(Box::new(TestLayer::new("X", &log)));
            log.borrow_mut().clear();
        } // stack dropped here
        assert_eq!(&*log.borrow(), &["X detach", "B detach", "A detach"]);
    }

    #[test]
    fn empty_stack_operations() {
        let mut stack = LayerStack::new();
        let input = Input::new();
        assert!(!stack.dispatch_event(&dummy_event(), &input));
        stack.update_all(Timestep::from_seconds(0.016), &input); // should not panic
    }
}
