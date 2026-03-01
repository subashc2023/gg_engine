use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowAttributes};

use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};

// ---------------------------------------------------------------------------
// WindowConfig
// ---------------------------------------------------------------------------

pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "GGEngine".into(),
            width: 1280,
            height: 720,
        }
    }
}

// ---------------------------------------------------------------------------
// Application trait
// ---------------------------------------------------------------------------

pub trait Application {
    fn new() -> Self
    where
        Self: Sized;

    fn window_config(&self) -> WindowConfig {
        WindowConfig::default()
    }

    fn on_event(&mut self, event: &Event) {
        log::trace!("{event}");
    }

    fn on_update(&mut self) {}
}

// ---------------------------------------------------------------------------
// EngineRunner (internal winit bridge)
// ---------------------------------------------------------------------------

struct EngineRunner<T: Application> {
    app: T,
    window: Option<Arc<Window>>,
    window_config: WindowConfig,
}

impl<T: Application> ApplicationHandler for EngineRunner<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let size = winit::dpi::LogicalSize::new(self.window_config.width, self.window_config.height);
        let attrs = WindowAttributes::default()
            .with_title(&self.window_config.title)
            .with_inner_size(size);

        match event_loop.create_window(attrs) {
            Ok(window) => {
                log::info!(target: "gg_engine", "Window created: \"{}\" ({}x{})",
                    self.window_config.title, self.window_config.width, self.window_config.height);
                self.window = Some(Arc::new(window));
            }
            Err(e) => {
                log::error!(target: "gg_engine", "Failed to create window: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let Some(engine_event) = map_window_event(&event) {
            self.app.on_event(&engine_event);

            if matches!(engine_event, Event::Window(WindowEvent::Close)) {
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.app.on_update();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// run()
// ---------------------------------------------------------------------------

pub fn run<T: Application>() {
    crate::log_init();
    log::info!(target: "gg_engine", "Engine v{}", crate::engine_version());

    let app = T::new();
    let window_config = app.window_config();

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut runner = EngineRunner {
        app,
        window: None,
        window_config,
    };

    event_loop.run_app(&mut runner).expect("event loop error");

    log::info!(target: "gg_engine", "Shutting down");
}

// ---------------------------------------------------------------------------
// Event mapping: winit → GGEngine
// ---------------------------------------------------------------------------

fn map_window_event(event: &winit::event::WindowEvent) -> Option<Event> {
    match event {
        winit::event::WindowEvent::CloseRequested => Some(Event::Window(WindowEvent::Close)),

        winit::event::WindowEvent::Resized(size) => Some(Event::Window(WindowEvent::Resize {
            width: size.width,
            height: size.height,
        })),

        winit::event::WindowEvent::KeyboardInput { event, .. } => {
            let PhysicalKey::Code(code) = event.physical_key else {
                return None;
            };
            let key_code = map_key_code(code);
            match event.state {
                ElementState::Pressed => Some(Event::Key(KeyEvent::Pressed {
                    key_code,
                    repeat: event.repeat,
                })),
                ElementState::Released => Some(Event::Key(KeyEvent::Released { key_code })),
            }
        }

        winit::event::WindowEvent::CursorMoved { position, .. } => {
            Some(Event::Mouse(MouseEvent::Moved {
                x: position.x,
                y: position.y,
            }))
        }

        winit::event::WindowEvent::MouseWheel { delta, .. } => {
            let (x_offset, y_offset) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x as f64, *y as f64),
                MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
            };
            Some(Event::Mouse(MouseEvent::Scrolled { x_offset, y_offset }))
        }

        winit::event::WindowEvent::MouseInput { state, button, .. } => {
            let btn = map_mouse_button(*button);
            match state {
                ElementState::Pressed => Some(Event::Mouse(MouseEvent::ButtonPressed(btn))),
                ElementState::Released => Some(Event::Mouse(MouseEvent::ButtonReleased(btn))),
            }
        }

        _ => None,
    }
}

fn map_key_code(code: winit::keyboard::KeyCode) -> KeyCode {
    use winit::keyboard::KeyCode as WK;
    match code {
        // Alphabetic
        WK::KeyA => KeyCode::A,
        WK::KeyB => KeyCode::B,
        WK::KeyC => KeyCode::C,
        WK::KeyD => KeyCode::D,
        WK::KeyE => KeyCode::E,
        WK::KeyF => KeyCode::F,
        WK::KeyG => KeyCode::G,
        WK::KeyH => KeyCode::H,
        WK::KeyI => KeyCode::I,
        WK::KeyJ => KeyCode::J,
        WK::KeyK => KeyCode::K,
        WK::KeyL => KeyCode::L,
        WK::KeyM => KeyCode::M,
        WK::KeyN => KeyCode::N,
        WK::KeyO => KeyCode::O,
        WK::KeyP => KeyCode::P,
        WK::KeyQ => KeyCode::Q,
        WK::KeyR => KeyCode::R,
        WK::KeyS => KeyCode::S,
        WK::KeyT => KeyCode::T,
        WK::KeyU => KeyCode::U,
        WK::KeyV => KeyCode::V,
        WK::KeyW => KeyCode::W,
        WK::KeyX => KeyCode::X,
        WK::KeyY => KeyCode::Y,
        WK::KeyZ => KeyCode::Z,

        // Digits
        WK::Digit0 => KeyCode::Num0,
        WK::Digit1 => KeyCode::Num1,
        WK::Digit2 => KeyCode::Num2,
        WK::Digit3 => KeyCode::Num3,
        WK::Digit4 => KeyCode::Num4,
        WK::Digit5 => KeyCode::Num5,
        WK::Digit6 => KeyCode::Num6,
        WK::Digit7 => KeyCode::Num7,
        WK::Digit8 => KeyCode::Num8,
        WK::Digit9 => KeyCode::Num9,

        // Function keys
        WK::F1 => KeyCode::F1,
        WK::F2 => KeyCode::F2,
        WK::F3 => KeyCode::F3,
        WK::F4 => KeyCode::F4,
        WK::F5 => KeyCode::F5,
        WK::F6 => KeyCode::F6,
        WK::F7 => KeyCode::F7,
        WK::F8 => KeyCode::F8,
        WK::F9 => KeyCode::F9,
        WK::F10 => KeyCode::F10,
        WK::F11 => KeyCode::F11,
        WK::F12 => KeyCode::F12,

        // Modifiers
        WK::ShiftLeft => KeyCode::LeftShift,
        WK::ShiftRight => KeyCode::RightShift,
        WK::ControlLeft => KeyCode::LeftCtrl,
        WK::ControlRight => KeyCode::RightCtrl,
        WK::AltLeft => KeyCode::LeftAlt,
        WK::AltRight => KeyCode::RightAlt,

        // Navigation
        WK::ArrowUp => KeyCode::Up,
        WK::ArrowDown => KeyCode::Down,
        WK::ArrowLeft => KeyCode::Left,
        WK::ArrowRight => KeyCode::Right,
        WK::Home => KeyCode::Home,
        WK::End => KeyCode::End,
        WK::PageUp => KeyCode::PageUp,
        WK::PageDown => KeyCode::PageDown,

        // Common
        WK::Space => KeyCode::Space,
        WK::Enter => KeyCode::Enter,
        WK::Escape => KeyCode::Escape,
        WK::Tab => KeyCode::Tab,
        WK::Backspace => KeyCode::Backspace,
        WK::Delete => KeyCode::Delete,
        WK::Insert => KeyCode::Insert,

        _ => KeyCode::Unknown,
    }
}

fn map_mouse_button(button: winit::event::MouseButton) -> MouseButton {
    match button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        winit::event::MouseButton::Back => MouseButton::Back,
        winit::event::MouseButton::Forward => MouseButton::Forward,
        winit::event::MouseButton::Other(_) => MouseButton::Left, // fallback
    }
}
