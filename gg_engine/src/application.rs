use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowAttributes};

use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
use crate::input::Input;
use crate::layer::LayerStack;
use crate::renderer::{
    DrawContext, OrthographicCamera, PresentMode, Renderer, Swapchain, VulkanContext,
};
use crate::timestep::Timestep;

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
    fn new(layers: &mut LayerStack) -> Self
    where
        Self: Sized;

    fn window_config(&self) -> WindowConfig {
        WindowConfig::default()
    }

    /// Called once after the renderer is initialized. Use this to create
    /// rendering resources (shaders, buffers, vertex arrays, pipelines).
    fn on_attach(&mut self, _renderer: &Renderer) {}

    fn on_event(&mut self, event: &Event, _input: &Input) {
        log::trace!("{event}");
    }

    fn on_update(&mut self, _dt: Timestep, _input: &Input) {}

    /// Submit draw calls. Called each frame between `begin_scene` / `end_scene`.
    fn on_render(&self, _renderer: &Renderer) {}

    /// Build immediate-mode UI each frame. Called inside `egui::Context::run`.
    fn on_egui(&mut self, _ctx: &egui::Context) {}

    /// Clear color for the render pass. Polled each frame.
    fn clear_color(&self) -> [f32; 4] {
        [0.01, 0.01, 0.01, 1.0]
    }

    /// Orthographic camera used for rendering. Return `Some` to override
    /// the engine's default aspect-ratio camera.
    fn camera(&self) -> Option<&OrthographicCamera> {
        None
    }

    /// Desired present mode. Polled each frame; changes trigger swapchain recreation.
    fn present_mode(&self) -> PresentMode {
        PresentMode::Fifo
    }
}

// ---------------------------------------------------------------------------
// EngineRunner (internal winit bridge)
// ---------------------------------------------------------------------------

enum FrameResult {
    Ok,
    RecreateSwapchain,
}

struct EngineRunner<T: Application> {
    app: T,
    layers: LayerStack,
    input: Input,
    window_config: WindowConfig,
    current_present_mode: PresentMode,
    default_camera: OrthographicCamera,
    last_frame_time: Instant,

    // egui state — dropped before Vulkan resources.
    egui_ctx: egui::Context,
    egui_winit_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_ash_renderer::Renderer>,

    // Renderer abstraction — dropped before swapchain/device.
    renderer: Option<Renderer>,

    // Swapchain — dropped before VulkanContext.
    swapchain: Option<Swapchain>,

    // Vulkan context must be dropped before window (surface references native handle).
    vulkan_context: Option<VulkanContext>,
    window: Option<Arc<Window>>,
}

impl<T: Application> Drop for EngineRunner<T> {
    fn drop(&mut self) {
        // Wait for all GPU work to finish before Rust drops the Vulkan resources.
        if let Some(vk_ctx) = &self.vulkan_context {
            unsafe {
                let _ = vk_ctx.device().device_wait_idle();
            }
        }
    }
}

impl<T: Application> ApplicationHandler for EngineRunner<T> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let size =
            winit::dpi::LogicalSize::new(self.window_config.width, self.window_config.height);
        let attrs = WindowAttributes::default()
            .with_title(&self.window_config.title)
            .with_inner_size(size);

        match event_loop.create_window(attrs) {
            Ok(window) => {
                log::info!(target: "gg_engine", "Window created: \"{}\" ({}x{})",
                    self.window_config.title, self.window_config.width, self.window_config.height);
                let window = Arc::new(window);

                // Initialize Vulkan immediately after window creation.
                match VulkanContext::new(&window) {
                    Ok(ctx) => {
                        // Create swapchain.
                        match Swapchain::new(
                            &ctx,
                            self.window_config.width,
                            self.window_config.height,
                            self.current_present_mode,
                        ) {
                            Ok(sc) => {
                                // Initialize egui-winit state.
                                let egui_winit_state = egui_winit::State::new(
                                    self.egui_ctx.clone(),
                                    egui::ViewportId::ROOT,
                                    &window,
                                    None,
                                    None,
                                    None,
                                );

                                // Initialize egui Vulkan renderer.
                                let is_srgb = matches!(
                                    sc.format().format,
                                    vk::Format::B8G8R8A8_SRGB
                                        | vk::Format::R8G8B8A8_SRGB
                                        | vk::Format::A8B8G8R8_SRGB_PACK32
                                );

                                match egui_ash_renderer::Renderer::with_default_allocator(
                                    ctx.instance(),
                                    ctx.physical_device(),
                                    ctx.device().clone(),
                                    sc.render_pass(),
                                    egui_ash_renderer::Options {
                                        in_flight_frames: 2,
                                        srgb_framebuffer: is_srgb,
                                        ..Default::default()
                                    },
                                ) {
                                    Ok(egui_rend) => {
                                        self.renderer =
                                            Some(Renderer::new(&ctx, sc.render_pass()));
                                        self.egui_winit_state = Some(egui_winit_state);
                                        self.egui_renderer = Some(egui_rend);
                                        self.swapchain = Some(sc);
                                        self.vulkan_context = Some(ctx);
                                        log::info!(target: "gg_engine", "Egui initialized");

                                        // Notify the application that the renderer is ready.
                                        if let Some(renderer) = &self.renderer {
                                            self.app.on_attach(renderer);
                                        }
                                    }
                                    Err(e) => {
                                        log::error!(target: "gg_engine", "Egui renderer init failed: {e}");
                                        // Still store vulkan context and swapchain so they drop cleanly.
                                        self.swapchain = Some(sc);
                                        self.vulkan_context = Some(ctx);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!(target: "gg_engine", "Swapchain creation failed: {e}");
                                self.vulkan_context = Some(ctx);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!(target: "gg_engine", "Vulkan initialization failed: {e}");
                        event_loop.exit();
                        return;
                    }
                }

                self.window = Some(window);
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
        // Forward raw winit event to egui-winit first.
        if let (Some(state), Some(window)) = (&mut self.egui_winit_state, &self.window) {
            let response = state.on_window_event(window, &event);
            if response.consumed {
                return;
            }
        }

        // Update input polling state from raw winit event.
        // This happens before engine event mapping so that key presses
        // producing Typed events are still tracked by the polling system.
        match &event {
            winit::event::WindowEvent::KeyboardInput { event: key_event, .. } => {
                if let PhysicalKey::Code(code) = key_event.physical_key {
                    let key_code = map_key_code(code);
                    match key_event.state {
                        ElementState::Pressed => self.input.press_key(key_code),
                        ElementState::Released => self.input.release_key(key_code),
                    }
                }
            }
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                self.input.set_mouse_position(position.x, position.y);
            }
            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                let btn = map_mouse_button(*button);
                match state {
                    ElementState::Pressed => self.input.press_mouse_button(btn),
                    ElementState::Released => self.input.release_mouse_button(btn),
                }
            }
            _ => {}
        }

        // Handle resize for swapchain recreation and camera projection update.
        if let winit::event::WindowEvent::Resized(size) = &event {
            if size.width > 0 && size.height > 0 {
                if let (Some(vk_ctx), Some(sc)) =
                    (&self.vulkan_context, &mut self.swapchain)
                {
                    sc.recreate(vk_ctx, size.width, size.height, None);
                    if let Some(renderer) = &mut self.renderer {
                        renderer.update_render_pass(sc.render_pass());
                    }
                }
                let aspect = size.width as f32 / size.height as f32;
                self.default_camera
                    .set_projection(-aspect, aspect, -1.0, 1.0);
            }
        }

        // Map to engine event(s) and dispatch through layer stack.
        let (primary, secondary) = map_window_event(&event);
        for engine_event in primary.into_iter().chain(secondary) {
            if !self.layers.dispatch_event(&engine_event, &self.input) {
                self.app.on_event(&engine_event, &self.input);
            }

            if matches!(engine_event, Event::Window(WindowEvent::Close)) {
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = Timestep::from_seconds(now.duration_since(self.last_frame_time).as_secs_f32());
        self.last_frame_time = now;

        self.layers.update_all(dt, &self.input);
        self.app.on_update(dt, &self.input);

        // Render egui frame — requires all graphics resources.
        'render: {
            let Some(window) = self.window.as_ref() else {
                break 'render;
            };
            let Some(vk_ctx) = self.vulkan_context.as_ref() else {
                break 'render;
            };
            let Some(swapchain) = self.swapchain.as_mut() else {
                break 'render;
            };
            let Some(egui_state) = self.egui_winit_state.as_mut() else {
                break 'render;
            };
            let Some(egui_renderer) = self.egui_renderer.as_mut() else {
                break 'render;
            };
            let Some(renderer) = self.renderer.as_mut() else {
                break 'render;
            };

            // Skip rendering when minimized.
            let extent = swapchain.extent();
            if extent.width == 0 || extent.height == 0 {
                break 'render;
            }

            // Gather input and run egui.
            let raw_input = egui_state.take_egui_input(window);

            // Split borrows so the application can build egui UI.
            let egui_ctx = &self.egui_ctx;
            let app = &mut self.app;
            let full_output = egui_ctx.run(raw_input, |ctx| {
                app.on_egui(ctx);
            });

            // Handle platform output (cursor, clipboard, etc).
            egui_state.handle_platform_output(window, full_output.platform_output);

            // Tessellate.
            let primitives = egui_ctx
                .tessellate(full_output.shapes, full_output.pixels_per_point);

            // Upload textures.
            if !full_output.textures_delta.set.is_empty() {
                egui_renderer
                    .set_textures(
                        vk_ctx.graphics_queue(),
                        swapchain.command_pool(),
                        &full_output.textures_delta.set,
                    )
                    .expect("Failed to set egui textures");
            }

            // Poll clear color from application.
            renderer.set_clear_color(self.app.clear_color());

            // Use app-provided camera or fall back to the default.
            let camera = self.app.camera().unwrap_or(&self.default_camera);

            // Render frame.
            let frame_result = render_frame(
                vk_ctx,
                swapchain,
                renderer,
                camera,
                &self.app,
                egui_renderer,
                &primitives,
                full_output.pixels_per_point,
            );

            // Advance to the next frame's sync primitives.
            swapchain.advance_frame();

            // Free textures that are no longer needed.
            if !full_output.textures_delta.free.is_empty() {
                egui_renderer
                    .free_textures(&full_output.textures_delta.free)
                    .expect("Failed to free egui textures");
            }

            // Check for present mode change.
            let desired = self.app.present_mode();
            let mode_changed = desired != self.current_present_mode;

            // Recreate swapchain if needed (out-of-date / suboptimal / present mode change).
            if matches!(frame_result, FrameResult::RecreateSwapchain) || mode_changed {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    let mode_arg = if mode_changed { Some(desired) } else { None };
                    swapchain.recreate(vk_ctx, size.width, size.height, mode_arg);
                    renderer.update_render_pass(swapchain.render_pass());
                    self.current_present_mode = desired;
                }
            }
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// Frame rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_frame<T: Application>(
    vk_ctx: &VulkanContext,
    swapchain: &mut Swapchain,
    renderer: &mut Renderer,
    camera: &OrthographicCamera,
    app: &T,
    egui_renderer: &mut egui_ash_renderer::Renderer,
    primitives: &[egui::ClippedPrimitive],
    pixels_per_point: f32,
) -> FrameResult {
    let device = vk_ctx.device();
    let extent = swapchain.extent();

    // Wait for this frame-slot's fence (still signaled from the previous use).
    unsafe {
        device
            .wait_for_fences(&[swapchain.in_flight_fence()], true, u64::MAX)
            .expect("Failed to wait for fence");
    }

    // Acquire next image — do NOT reset the fence yet.
    // If acquire fails with OUT_OF_DATE, the fence stays signaled so the
    // next frame's wait_for_fences won't deadlock.
    let acquire_result = unsafe {
        swapchain.swapchain_loader().acquire_next_image(
            swapchain.swapchain(),
            u64::MAX,
            swapchain.image_available_semaphore(),
            vk::Fence::null(),
        )
    };

    let (image_index, acquire_suboptimal) = match acquire_result {
        Ok((idx, suboptimal)) => (idx, suboptimal),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return FrameResult::RecreateSwapchain,
        Err(e) => panic!("Failed to acquire swapchain image: {e}"),
    };

    // Acquire succeeded — now it's safe to reset the fence.
    unsafe {
        device
            .reset_fences(&[swapchain.in_flight_fence()])
            .expect("Failed to reset fence");
    }

    let cmd_buf = swapchain.command_buffer(swapchain.current_frame());

    // Record command buffer.
    unsafe {
        device
            .reset_command_buffer(cmd_buf, vk::CommandBufferResetFlags::empty())
            .expect("Failed to reset command buffer");
        device
            .begin_command_buffer(cmd_buf, &vk::CommandBufferBeginInfo::default())
            .expect("Failed to begin command buffer");

        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: renderer.clear_color(),
            },
        }];

        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(swapchain.render_pass())
            .framebuffer(swapchain.framebuffer(image_index as usize))
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .clear_values(&clear_values);

        device.cmd_begin_render_pass(cmd_buf, &render_pass_info, vk::SubpassContents::INLINE);
    }

    // Begin scene — sets camera VP matrix + viewport/scissor via the Renderer.
    renderer.begin_scene(camera, DrawContext { cmd_buf, extent });

    // Application draw calls.
    app.on_render(renderer);

    // End scene.
    renderer.end_scene();

    // Draw egui.
    egui_renderer
        .cmd_draw(cmd_buf, extent, pixels_per_point, primitives)
        .expect("Failed to draw egui");

    // End render pass and command buffer.
    unsafe {
        device.cmd_end_render_pass(cmd_buf);
        device
            .end_command_buffer(cmd_buf)
            .expect("Failed to end command buffer");
    }

    // Submit.
    let wait_semaphores = [swapchain.image_available_semaphore()];
    let signal_semaphores = [swapchain.render_finished_semaphore(image_index)];
    let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
    let command_buffers = [cmd_buf];

    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphores)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&command_buffers)
        .signal_semaphores(&signal_semaphores);

    unsafe {
        device
            .queue_submit(
                vk_ctx.graphics_queue(),
                &[submit_info],
                swapchain.in_flight_fence(),
            )
            .expect("Failed to submit draw command buffer");
    }

    // Present.
    let swapchains = [swapchain.swapchain()];
    let image_indices = [image_index];
    let present_info = vk::PresentInfoKHR::default()
        .wait_semaphores(&signal_semaphores)
        .swapchains(&swapchains)
        .image_indices(&image_indices);

    let present_result = unsafe {
        swapchain
            .swapchain_loader()
            .queue_present(vk_ctx.graphics_queue(), &present_info)
    };

    match present_result {
        Ok(false) if !acquire_suboptimal => FrameResult::Ok,
        Ok(_) => FrameResult::RecreateSwapchain,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => FrameResult::RecreateSwapchain,
        Err(e) => panic!("Failed to present: {e}"),
    }
}

// ---------------------------------------------------------------------------
// run()
// ---------------------------------------------------------------------------

pub fn run<T: Application>() {
    crate::log_init();
    log::info!(target: "gg_engine", "Engine v{}", crate::engine_version());

    let mut layers = LayerStack::new();
    let app = T::new(&mut layers);
    let window_config = app.window_config();
    let current_present_mode = app.present_mode();

    // Default camera sized to the window's aspect ratio.
    let aspect = window_config.width as f32 / window_config.height as f32;
    let default_camera = OrthographicCamera::new(-aspect, aspect, -1.0, 1.0);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut runner = EngineRunner {
        app,
        layers,
        input: Input::new(),
        window_config,
        current_present_mode,
        default_camera,
        last_frame_time: Instant::now(),
        egui_ctx: egui::Context::default(),
        egui_winit_state: None,
        egui_renderer: None,
        renderer: None,
        swapchain: None,
        vulkan_context: None,
        window: None,
    };

    event_loop.run_app(&mut runner).expect("event loop error");

    log::info!(target: "gg_engine", "Shutting down");
}

// ---------------------------------------------------------------------------
// Event mapping: winit → GGEngine
// ---------------------------------------------------------------------------

/// Map a winit window event to one or two engine events.
///
/// Returns `(primary, secondary)`. For keyboard presses that produce printable
/// text the primary event is the `Pressed` key event and the secondary is the
/// `Typed(char)` event. Previously only the `Typed` event was returned,
/// swallowing the `Pressed` event.
fn map_window_event(event: &winit::event::WindowEvent) -> (Option<Event>, Option<Event>) {
    match event {
        winit::event::WindowEvent::CloseRequested => {
            (Some(Event::Window(WindowEvent::Close)), None)
        }

        winit::event::WindowEvent::Resized(size) => (
            Some(Event::Window(WindowEvent::Resize {
                width: size.width,
                height: size.height,
            })),
            None,
        ),

        winit::event::WindowEvent::KeyboardInput { event, .. } => {
            let PhysicalKey::Code(code) = event.physical_key else {
                return (None, None);
            };
            let key_code = map_key_code(code);

            let primary = match event.state {
                ElementState::Pressed => Some(Event::Key(KeyEvent::Pressed {
                    key_code,
                    repeat: event.repeat,
                })),
                ElementState::Released => Some(Event::Key(KeyEvent::Released { key_code })),
            };

            // Emit an additional Typed event for printable characters on
            // non-repeat presses.
            let secondary = if event.state == ElementState::Pressed && !event.repeat {
                event
                    .text
                    .as_ref()
                    .and_then(|t| t.chars().find(|c| !c.is_control()))
                    .map(|c| Event::Key(KeyEvent::Typed(c)))
            } else {
                None
            };

            (primary, secondary)
        }

        winit::event::WindowEvent::CursorMoved { position, .. } => (
            Some(Event::Mouse(MouseEvent::Moved {
                x: position.x,
                y: position.y,
            })),
            None,
        ),

        winit::event::WindowEvent::MouseWheel { delta, .. } => {
            let (x_offset, y_offset) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x as f64, *y as f64),
                MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
            };
            (
                Some(Event::Mouse(MouseEvent::Scrolled { x_offset, y_offset })),
                None,
            )
        }

        winit::event::WindowEvent::MouseInput { state, button, .. } => {
            let btn = map_mouse_button(*button);
            (
                match state {
                    ElementState::Pressed => Some(Event::Mouse(MouseEvent::ButtonPressed(btn))),
                    ElementState::Released => {
                        Some(Event::Mouse(MouseEvent::ButtonReleased(btn)))
                    }
                },
                None,
            )
        }

        _ => (None, None),
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
