use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ash::vk::{self, Handle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowAttributes};

use crate::cursor::{self, CursorMode, SoftwareCursor};
use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
use crate::input::Input;
use crate::layer::LayerStack;
use crate::profiling::ProfileTimer;
use crate::renderer::{
    ClearValues, DrawContext, Framebuffer, GpuAllocator, OrthographicCamera, PresentMode, Renderer,
    Swapchain, VulkanContext, MAX_FRAMES_IN_FLIGHT, MAX_VIEWPORTS,
};
use crate::timestep::Timestep;
use glam::Mat4;

// ---------------------------------------------------------------------------
// WindowConfig
// ---------------------------------------------------------------------------

pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub decorations: bool,
    /// Optional window position (x, y). When `Some`, the window is placed at
    /// the given screen coordinates; when `None`, the OS chooses the position.
    pub position: Option<(i32, i32)>,
    /// Whether the window should start maximized.
    pub maximized: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "GGEngine".into(),
            width: 1280,
            height: 720,
            decorations: true,
            position: None,
            maximized: false,
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
    fn on_attach(&mut self, _renderer: &mut Renderer) {}

    fn on_event(&mut self, event: &Event, _input: &Input) {
        log::trace!("{event}");
    }

    fn on_update(&mut self, _dt: Timestep, _input: &Input) {}

    /// Called after `on_update` and `on_egui`, right before GPU command recording.
    /// Use this to recompute camera transforms with the latest input state,
    /// minimizing input-to-display latency for directly-controlled objects
    /// (e.g. camera, crosshair) that should bypass physics interpolation.
    fn on_late_update(&mut self, _dt: Timestep, _input: &Input) {}

    /// Called after `on_egui` but before viewport framebuffer handles are
    /// captured for GPU command recording. Use this for operations that
    /// recreate framebuffers (e.g. MSAA sample count changes) so the new
    /// handles are used in the current frame's command buffer.
    fn on_pre_render(&mut self, _renderer: &mut Renderer) {}

    /// Submit draw calls. Called each frame between `begin_scene` / `end_scene`.
    fn on_render(&mut self, _renderer: &mut Renderer) {}

    /// Called before any render pass begins to render shadow depth maps.
    /// Override this if your application uses 3D scenes with shadow-casting
    /// directional lights. The `cmd_buf` is the active command buffer
    /// (before the main render pass is started).
    fn on_render_shadows(
        &mut self,
        _renderer: &mut Renderer,
        _cmd_buf: vk::CommandBuffer,
        _current_frame: usize,
    ) {
    }

    /// Build immediate-mode UI each frame. Called inside `egui::Context::run`.
    /// The `window` handle is provided for custom title bar controls
    /// (drag, minimize, maximize, close).
    fn on_egui(&mut self, _ctx: &egui::Context, _window: &Window) {}

    /// Whether the application has requested an exit (e.g. custom close button).
    /// Polled each frame after `on_egui`. Default: `false`.
    fn should_exit(&self) -> bool {
        false
    }

    /// Called when the window close button is pressed. Return `true` to allow
    /// the close, `false` to cancel it (e.g. to show a "save changes?" dialog).
    /// Default: `true` (always allow close).
    fn on_close_requested(&mut self) -> bool {
        true
    }

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
    /// Defaults to Mailbox for lowest latency (falls back to Immediate, then Fifo).
    fn present_mode(&self) -> PresentMode {
        PresentMode::Mailbox
    }

    /// Whether egui should block events from reaching the engine.
    /// When `true` (default), events consumed by egui (e.g. scroll over an
    /// egui panel) are not dispatched to the application. Override this in
    /// editors to let viewport events pass through.
    fn block_events(&self) -> bool {
        true
    }

    /// Returns the input action map for this application, if any.
    /// Called once after `on_attach` to configure action-based input queries.
    fn input_action_map(&self) -> Option<crate::input_action::InputActionMap> {
        None
    }

    /// Returns global gamepad dead zone configuration.
    /// Polled each frame; changes are applied to the input system.
    fn dead_zones(&self) -> Option<[f32; crate::events::gamepad::GamepadAxis::COUNT]> {
        None
    }

    /// Cursor grab and visibility mode. Polled each frame; changes are applied
    /// immediately to the window.
    ///
    /// - [`CursorMode::Normal`] — OS cursor visible, no grab (default).
    /// - [`CursorMode::Confined`] — OS cursor hidden, software cursor rendered,
    ///   mouse confined to window. Use for games with a visible custom cursor.
    /// - [`CursorMode::Locked`] — OS cursor hidden and locked in place, raw
    ///   deltas only. Use for FPS camera / mouse look.
    fn cursor_mode(&self) -> CursorMode {
        CursorMode::Normal
    }

    /// Custom software cursor appearance for [`CursorMode::Confined`] mode.
    ///
    /// Return `Some` to replace the default arrow cursor with a custom texture.
    /// The texture must be registered via [`egui_user_textures()`](Self::egui_user_textures).
    /// Return `None` (default) to use the built-in arrow cursor.
    fn software_cursor(&self) -> Option<SoftwareCursor> {
        None
    }

    /// Requested window resize (physical pixels). Polled each frame; when
    /// `Some((w, h))`, the engine calls `window.request_inner_size()`.
    /// Consumed after reading (use a `Cell` or similar one-shot pattern).
    fn requested_window_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// Requested fullscreen mode change. Polled each frame.
    /// `Some(mode)` requests a transition; `None` means no change.
    /// The engine runner handles winit conversion, including video mode
    /// enumeration for [`FullscreenMode::Exclusive`].
    fn requested_fullscreen(&self) -> Option<crate::scene::FullscreenMode> {
        None
    }

    /// Return the scene framebuffer if this app renders to an offscreen target.
    /// When `Some`, the engine uses a dual-pass flow: offscreen scene pass + swapchain egui pass.
    fn scene_framebuffer(&self) -> Option<&Framebuffer> {
        None
    }

    /// Mutable access to the scene framebuffer (for resize, egui registration).
    fn scene_framebuffer_mut(&mut self) -> Option<&mut Framebuffer> {
        None
    }

    /// Desired viewport size for the scene framebuffer. Return `Some((w, h))`
    /// to trigger a resize when the egui panel size changes.
    fn desired_viewport_size(&self) -> Option<(u32, u32)> {
        None
    }

    // ----- Multi-viewport support -----

    /// Number of scene viewports (offscreen framebuffers) to render.
    /// Default: 1 if `scene_framebuffer()` returns `Some`, else 0.
    /// Override this (along with `viewport_framebuffer` etc.) for multiple viewports.
    fn viewport_count(&self) -> usize {
        if self.scene_framebuffer().is_some() {
            1
        } else {
            0
        }
    }

    /// Access framebuffer by viewport index.
    /// Default: delegates to `scene_framebuffer()` for index 0.
    fn viewport_framebuffer(&self, index: usize) -> Option<&Framebuffer> {
        if index == 0 {
            self.scene_framebuffer()
        } else {
            None
        }
    }

    /// Mutable access to framebuffer by viewport index.
    /// Default: delegates to `scene_framebuffer_mut()` for index 0.
    fn viewport_framebuffer_mut(&mut self, index: usize) -> Option<&mut Framebuffer> {
        if index == 0 {
            self.scene_framebuffer_mut()
        } else {
            None
        }
    }

    /// Desired size for viewport at the given index.
    /// Default: delegates to `desired_viewport_size()` for index 0.
    fn viewport_desired_size(&self, index: usize) -> Option<(u32, u32)> {
        if index == 0 {
            self.desired_viewport_size()
        } else {
            None
        }
    }

    /// Render callback for a specific viewport. Called once per viewport
    /// between `begin_scene`/`end_scene`. Default: calls `on_render` for all.
    fn on_render_viewport(&mut self, renderer: &mut Renderer, _viewport_index: usize) {
        self.on_render(renderer);
    }

    /// Return opaque texture handles (from [`Texture2D::egui_handle`]) that
    /// should be registered as egui user textures for UI rendering (e.g. tile
    /// palette previews). Called each frame; the engine registers new ones
    /// and unregisters stale ones. The resulting `egui::TextureId` mapping is
    /// delivered via [`receive_egui_user_textures`].
    fn egui_user_textures(&self) -> Vec<u64> {
        Vec::new()
    }

    /// Receive the mapping from opaque texture handle → egui TextureId
    /// for textures registered via [`egui_user_textures`].
    fn receive_egui_user_textures(&mut self, _map: &HashMap<u64, egui::TextureId>) {}

    /// Called when the GPU device is lost and rendering can no longer continue.
    /// Override to perform emergency saves before the application exits.
    fn on_device_lost(&mut self) {}
}

// ---------------------------------------------------------------------------
// EngineRunner (internal winit bridge)
// ---------------------------------------------------------------------------

/// Background clear color for the editor chrome (swapchain pass in dual-pass mode).
const EDITOR_CHROME_CLEAR: [f32; 4] = [0.06, 0.06, 0.06, 1.0];

enum FrameResult {
    Ok,
    RecreateSwapchain,
    DeviceLost,
}

/// How long to wait after the last resize event before applying the resize.
/// This prevents repeated swapchain recreations when dragging between monitors
/// with different DPI scaling.
const RESIZE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(100);

struct EngineRunner<T: Application> {
    app: T,
    layers: LayerStack,
    input: Input,
    window_config: WindowConfig,
    current_present_mode: PresentMode,
    current_cursor_mode: CursorMode,
    /// 1x1 transparent cursor image used in Confined mode. Replaces the OS cursor
    /// icon so `CursorMoved` events keep firing (unlike `set_cursor_visible(false)`
    /// which silences them on Windows).
    transparent_cursor: Option<winit::window::CustomCursor>,
    /// Software cursor position in logical pixels, driven by OS `CursorMoved` events.
    software_cursor_pos: (f64, f64),
    /// True when the OS cursor is inside the window's client area.
    cursor_in_window: bool,
    default_camera: OrthographicCamera,
    last_frame_time: Instant,
    /// Exponential moving average of frame dt for smooth camera/movement.
    smoothed_dt: f32,
    minimized: bool,

    /// Pending resize: `(width, height, timestamp_of_last_resize_event)`.
    /// Applied once no new resize arrives within [`RESIZE_DEBOUNCE`].
    pending_resize: Option<(u32, u32, Instant)>,

    // egui state — dropped before Vulkan resources.
    egui_ctx: egui::Context,
    egui_winit_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_ash_renderer::Renderer>,
    /// App-registered user textures: opaque handle → egui TextureId.
    user_textures: HashMap<u64, egui::TextureId>,

    // Renderer abstraction — dropped before swapchain/device.
    renderer: Option<Renderer>,

    // Swapchain — dropped before VulkanContext.
    swapchain: Option<Swapchain>,

    // GPU memory sub-allocator — dropped after renderer/swapchain but before VulkanContext.
    allocator: Option<Arc<Mutex<GpuAllocator>>>,

    // Vulkan context must be dropped before window (surface references native handle).
    vulkan_context: Option<VulkanContext>,
    window: Option<Arc<Window>>,

    // Gamepad backend - gilrs polls OS gamepad events and feeds them to Input.
    #[cfg(feature = "gamepad")]
    gilrs: Option<gilrs::Gilrs>,
}

impl<T: Application> Drop for EngineRunner<T> {
    fn drop(&mut self) {
        // End the runtime profiling session before resource teardown.
        crate::profiling::end_session();

        // Wait for all GPU work to finish before Rust drops the Vulkan resources.
        if let Some(vk_ctx) = &self.vulkan_context {
            unsafe {
                let _ = vk_ctx.device().device_wait_idle();
            }
        }

        // Unregister all viewport framebuffers' egui textures before the egui renderer drops.
        for i in 0..self.app.viewport_count() {
            if let Some(fb) = self.app.viewport_framebuffer(i) {
                if let Some(tex_id) = fb.egui_texture_id() {
                    if let Some(egui_renderer) = &mut self.egui_renderer {
                        egui_renderer.remove_user_texture(tex_id);
                    }
                }
            }
        }

        // Unregister app user textures.
        if let Some(egui_renderer) = &mut self.egui_renderer {
            for (_, tex_id) in self.user_textures.drain() {
                egui_renderer.remove_user_texture(tex_id);
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
        let mut attrs = WindowAttributes::default()
            .with_title(&self.window_config.title)
            .with_inner_size(size)
            .with_decorations(self.window_config.decorations)
            .with_maximized(self.window_config.maximized);

        // Validate saved position against available monitors — if the window
        // wouldn't be at least partially visible, fall back to OS placement.
        let validated_position = self.window_config.position.filter(|&(x, y)| {
            let min_visible = 100i32;
            let w = self.window_config.width as i32;
            let h = self.window_config.height as i32;
            event_loop.available_monitors().any(|monitor| {
                let pos = monitor.position();
                let mon_size = monitor.size();
                let mx = pos.x;
                let my = pos.y;
                let mw = mon_size.width as i32;
                let mh = mon_size.height as i32;
                // Check that at least min_visible pixels overlap on both axes
                let overlap_x = (x + w).min(mx + mw) - x.max(mx);
                let overlap_y = (y + h).min(my + mh) - y.max(my);
                overlap_x >= min_visible && overlap_y >= min_visible
            })
        });
        if let Some((x, y)) = validated_position {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }

        match event_loop.create_window(attrs) {
            Ok(window) => {
                log::info!(target: "gg_engine", "Window created: \"{}\" ({}x{})",
                    self.window_config.title, self.window_config.width, self.window_config.height);
                let window = Arc::new(window);

                // Create a 1x1 transparent cursor for Confined mode.
                if let Ok(source) =
                    winit::window::CustomCursor::from_rgba(vec![0u8; 4], 1, 1, 0, 0)
                {
                    self.transparent_cursor =
                        Some(event_loop.create_custom_cursor(source));
                }

                // Initialize the job thread pool for parallel ECS work.
                crate::jobs::init();

                // Initialize Vulkan immediately after window creation.
                match VulkanContext::new(&window) {
                    Ok(ctx) => {
                        // Create GPU memory sub-allocator.
                        let gpu_alloc = match GpuAllocator::new(
                            ctx.instance(),
                            ctx.device(),
                            ctx.physical_device(),
                        ) {
                            Ok(a) => a,
                            Err(e) => {
                                log::error!(target: "gg_engine", "GPU allocator init failed: {e}");
                                self.vulkan_context = Some(ctx);
                                self.window = Some(window);
                                return;
                            }
                        };
                        let allocator = Arc::new(Mutex::new(gpu_alloc));

                        // Create swapchain.
                        match Swapchain::new(
                            &ctx,
                            self.window_config.width,
                            self.window_config.height,
                            self.current_present_mode,
                            &allocator,
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
                                // sRGB framebuffer flag tells egui-ash-renderer to output
                                // linear values (no manual gamma). This is correct for both
                                // sRGB framebuffers (hardware does linear→sRGB) and scRGB
                                // HDR (compositor expects linear values directly).
                                let is_srgb = matches!(
                                    sc.format().format,
                                    vk::Format::B8G8R8A8_SRGB
                                        | vk::Format::R8G8B8A8_SRGB
                                        | vk::Format::A8B8G8R8_SRGB_PACK32
                                ) || sc.is_hdr();

                                match egui_ash_renderer::Renderer::with_default_allocator(
                                    ctx.instance(),
                                    ctx.physical_device(),
                                    ctx.device().clone(),
                                    sc.render_pass(),
                                    egui_ash_renderer::Options {
                                        in_flight_frames: MAX_FRAMES_IN_FLIGHT,
                                        srgb_framebuffer: is_srgb,
                                        ..Default::default()
                                    },
                                ) {
                                    Ok(egui_rend) => {
                                        match Renderer::new(
                                            &ctx,
                                            &allocator,
                                            sc.render_pass(),
                                            sc.command_pool(),
                                            sc.format().format,
                                            sc.depth_format(),
                                        ) {
                                            Ok(renderer) => {
                                                self.renderer = Some(renderer);
                                            }
                                            Err(e) => {
                                                log::error!(target: "gg_engine", "Renderer init failed: {e}");
                                                self.swapchain = Some(sc);
                                                self.allocator = Some(allocator);
                                                self.vulkan_context = Some(ctx);
                                                self.window = Some(window);
                                                return;
                                            }
                                        }
                                        self.egui_winit_state = Some(egui_winit_state);
                                        self.egui_renderer = Some(egui_rend);
                                        self.swapchain = Some(sc);
                                        self.allocator = Some(allocator);
                                        self.vulkan_context = Some(ctx);
                                        log::info!(target: "gg_engine", "Egui initialized");

                                        // Initialize built-in 2D renderer resources.
                                        if let Some(renderer) = &mut self.renderer {
                                            if let Err(e) = renderer.init_2d() {
                                                log::error!(target: "gg_engine", "2D renderer init failed: {e}");
                                            }
                                        }

                                        // Initialize GPU timestamp profiler.
                                        if let (Some(renderer), Some(vk_ctx)) =
                                            (&mut self.renderer, &self.vulkan_context)
                                        {
                                            renderer
                                                .init_gpu_profiler(vk_ctx.timestamp_period_ns());
                                        }

                                        // Notify the application that the renderer is ready.
                                        if let Some(renderer) = &mut self.renderer {
                                            self.app.on_attach(renderer);
                                        }

                                        // Set up input action map from the application (if any).
                                        if let Some(action_map) = self.app.input_action_map() {
                                            self.input.set_action_map(action_map);
                                        }

                                        // Apply initial cursor mode so the transparent cursor is
                                        // set from the very first frame.
                                        let initial_cursor = self.app.cursor_mode();
                                        if initial_cursor != CursorMode::Normal {
                                            apply_cursor_mode(
                                                &window,
                                                initial_cursor,
                                                self.transparent_cursor.as_ref(),
                                            );
                                            self.current_cursor_mode = initial_cursor;
                                        }

                                        // If any viewport has a multi-attachment framebuffer, create
                                        // an offscreen batch pipeline compatible with its render pass.
                                        // All viewports share the same attachment formats, so we only
                                        // need to create the pipeline once from the first one found.
                                        for vi in 0..self.app.viewport_count() {
                                            if let Some(fb) = self.app.viewport_framebuffer(vi) {
                                                if fb.color_attachment_count() > 1 {
                                                    if let Some(renderer) = &mut self.renderer {
                                                        if let Err(e) = renderer
                                                            .create_offscreen_batch_pipeline(
                                                                fb.render_pass(),
                                                                fb.color_attachment_count() as u32,
                                                                fb.sample_count(),
                                                            )
                                                        {
                                                            log::error!(target: "gg_engine", "Offscreen pipeline init failed: {e}");
                                                        }
                                                    }
                                                    break; // All viewports share the same pipeline
                                                }
                                            }
                                        }

                                        // Startup is complete — close the startup profile.
                                        // Runtime profiling is on-demand: call
                                        // begin_session("Runtime", "gg_profile_runtime.json")
                                        // from the application to capture a trace for gg_tools.
                                        crate::profiling::end_session();
                                    }
                                    Err(e) => {
                                        log::error!(target: "gg_engine", "Egui renderer init failed: {e}");
                                        // Still store vulkan context and swapchain so they drop cleanly.
                                        self.swapchain = Some(sc);
                                        self.allocator = Some(allocator);
                                        self.vulkan_context = Some(ctx);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!(target: "gg_engine", "Swapchain creation failed: {e}");
                                self.allocator = Some(allocator);
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

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            // Accumulate raw deltas for Input::mouse_delta() — used by Locked
            // mode (FPS camera look) and any game code that wants raw motion.
            self.input.accumulate_mouse_delta(dx, dy);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        // Update input polling state from raw winit event BEFORE egui
        // consumes it. This ensures the Input struct always tracks the true
        // physical state of keys/mouse/scroll, regardless of whether egui
        // panels have focus. Game systems (input actions, Lua scripts) need
        // accurate polling state even when egui is active.
        match &event {
            winit::event::WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
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
                // Always sync software cursor from OS cursor position (logical
                // pixels). In Confined mode the OS cursor is transparent but
                // still tracked, so CursorMoved events give OS-accelerated,
                // DPI-correct positions.
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor())
                    .unwrap_or(1.0);
                self.software_cursor_pos = (position.x / scale, position.y / scale);
            }
            winit::event::WindowEvent::CursorEntered { .. } => {
                self.cursor_in_window = true;
            }
            winit::event::WindowEvent::CursorLeft { .. } => {
                self.cursor_in_window = false;
            }
            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                if let Some(btn) = map_mouse_button(*button) {
                    match state {
                        ElementState::Pressed => self.input.press_mouse_button(btn),
                        ElementState::Released => self.input.release_mouse_button(btn),
                    }
                }
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x as f64, *y as f64),
                    MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                };
                self.input.accumulate_scroll_delta(dx, dy);
            }
            winit::event::WindowEvent::Focused(focused) => {
                if !focused {
                    // Clear all pressed keys/buttons on focus loss to prevent
                    // "stuck" keys when the user Alt+Tabs away while holding keys.
                    self.input.clear_all();
                    // Release cursor grab on focus loss (only Locked mode uses
                    // a grab — Confined mode has no grab). Re-applied on gain.
                    if self.current_cursor_mode == CursorMode::Locked {
                        if let Some(window) = &self.window {
                            apply_cursor_mode(
                                window,
                                CursorMode::Normal,
                                self.transparent_cursor.as_ref(),
                            );
                        }
                    }
                } else {
                    // Re-apply cursor mode on focus gain (restores Locked grab
                    // or Confined transparent cursor).
                    if self.current_cursor_mode != CursorMode::Normal {
                        if let Some(window) = &self.window {
                            apply_cursor_mode(
                                window,
                                self.current_cursor_mode,
                                self.transparent_cursor.as_ref(),
                            );
                        }
                    }
                }
                // Dispatch focus event to application.
                let focus_event = Event::Window(WindowEvent::Focused(*focused));
                if !self.layers.dispatch_event(&focus_event, &self.input) {
                    self.app.on_event(&focus_event, &self.input);
                }
            }
            _ => {}
        }

        // Forward raw winit event to egui-winit. If egui consumed the event
        // and the application wants blocking, stop here (don't dispatch engine
        // events). Input polling above has already recorded the raw state.
        if let (Some(state), Some(window)) = (&mut self.egui_winit_state, &self.window) {
            let response = state.on_window_event(window, &event);
            if response.consumed && self.app.block_events() {
                return;
            }
        }

        // Handle resize: debounce to avoid repeated swapchain recreations when
        // dragging between monitors with different DPI scaling.
        if let winit::event::WindowEvent::Resized(size) = &event {
            // Borderless windows report non-zero size when minimized (e.g. 199x34
            // on Windows). Check `is_minimized()` in addition to zero-size.
            let is_minimized = self
                .window
                .as_ref()
                .and_then(|w| w.is_minimized())
                .unwrap_or(false);
            if size.width == 0 || size.height == 0 || is_minimized {
                self.minimized = true;
                self.pending_resize = None;
            } else {
                self.minimized = false;
                self.pending_resize = Some((size.width, size.height, Instant::now()));
            }
        }

        // Map to engine event(s) and dispatch through layer stack.
        // Resize events are deferred (debounced) and dispatched from about_to_wait.
        let (primary, secondary) = map_window_event(&event);
        for engine_event in primary.into_iter().chain(secondary) {
            if matches!(engine_event, Event::Window(WindowEvent::Resize { .. })) {
                continue;
            }
            if !self.layers.dispatch_event(&engine_event, &self.input) {
                self.app.on_event(&engine_event, &self.input);
            }

            if matches!(engine_event, Event::Window(WindowEvent::Close))
                && self.app.on_close_requested()
            {
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _run_loop = ProfileTimer::new("Run loop");
        let now = Instant::now();
        let raw_dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;

        // Clamp raw dt to [0.0001, 0.1] (10 FPS floor) to prevent teleportation
        // on extreme spikes (window drag, OS stalls, debugger breaks).
        let clamped = raw_dt.clamp(0.0001, 0.1);

        // Exponential moving average smoothing to reduce frame-to-frame jitter
        // in dt-dependent systems (camera fly movement, animation, physics interp).
        self.smoothed_dt = self.smoothed_dt * 0.8 + clamped * 0.2;
        let dt = Timestep::from_seconds(self.smoothed_dt);

        // Apply debounced resize once events have settled.
        if let Some((w, h, stamp)) = self.pending_resize {
            if now.duration_since(stamp) >= RESIZE_DEBOUNCE {
                self.pending_resize = None;
                if let (Some(vk_ctx), Some(sc)) = (&self.vulkan_context, &mut self.swapchain) {
                    if let Err(e) = sc.recreate(vk_ctx, w, h, None) {
                        log::error!(target: "gg_engine", "Swapchain recreate failed: {e}");
                        return;
                    }
                    if let Some(renderer) = &mut self.renderer {
                        renderer.update_render_pass(sc.render_pass());
                    }
                    if let Some(egui_renderer) = &mut self.egui_renderer {
                        if let Err(e) = egui_renderer.set_render_pass(sc.render_pass()) {
                            log::error!(target: "gg_engine", "Failed to update egui render pass: {e:?}");
                        }
                    }
                }
                let aspect = w as f32 / h as f32;
                self.default_camera
                    .set_projection(-aspect, aspect, -1.0, 1.0);

                // Dispatch the resize event so the app/layers can react.
                let resize_event = Event::Window(WindowEvent::Resize {
                    width: w,
                    height: h,
                });
                if !self.layers.dispatch_event(&resize_event, &self.input) {
                    self.app.on_event(&resize_event, &self.input);
                }
            }
        }

        if !self.minimized {
            // Sync input action map from the application. This supports
            // hot-reload when the editor loads a project after startup.
            // Only clone when the action count changes to avoid per-frame allocation.
            if let Some(action_map) = self.app.input_action_map() {
                if !self.input.has_action_map()
                    || self.input.action_count() != action_map.actions.len()
                {
                    self.input.set_action_map(action_map);
                }
            }

            // Sync dead zones from the application (project config / Lua overrides).
            if let Some(dz) = self.app.dead_zones() {
                self.input.set_global_dead_zones(dz);
            }

            // Poll gamepad events from gilrs and feed them to Input.
            #[cfg(feature = "gamepad")]
            self.poll_gamepads();

            // Evaluate input action bindings before update callbacks.
            self.input.update_actions();

            {
                let _timer = ProfileTimer::new("LayerStack::on_update");
                self.layers.update_all(dt, &self.input);
            }
            self.app.on_update(dt, &self.input);

            // Apply cursor mode changes (after on_update so app logic takes effect this frame).
            let desired_cursor = self.app.cursor_mode();
            if desired_cursor != self.current_cursor_mode {
                if let Some(window) = &self.window {
                    apply_cursor_mode(
                        window,
                        desired_cursor,
                        self.transparent_cursor.as_ref(),
                    );
                }
                self.current_cursor_mode = desired_cursor;
            }

            // Apply window resize requests from scripts.
            if let Some((w, h)) = self.app.requested_window_size() {
                if let Some(window) = &self.window {
                    let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(w, h));
                }
            }

            // Apply fullscreen mode changes from scripts.
            if let Some(fs_mode) = self.app.requested_fullscreen() {
                if let Some(window) = &self.window {
                    use crate::scene::FullscreenMode;
                    let winit_mode = match fs_mode {
                        FullscreenMode::Windowed => None,
                        FullscreenMode::Borderless => {
                            Some(winit::window::Fullscreen::Borderless(None))
                        }
                        FullscreenMode::Exclusive => {
                            // Find the best video mode matching the current window size.
                            let video_mode = window.current_monitor().and_then(|monitor| {
                                let target = window.inner_size();
                                let mut best: Option<winit::monitor::VideoModeHandle> = None;
                                for mode in monitor.video_modes() {
                                    let s = mode.size();
                                    if s.width == target.width && s.height == target.height {
                                        best = Some(match best {
                                            Some(prev)
                                                if mode.refresh_rate_millihertz()
                                                    > prev.refresh_rate_millihertz() =>
                                            {
                                                mode
                                            }
                                            Some(prev) => prev,
                                            None => mode,
                                        });
                                    }
                                }
                                best
                            });
                            match video_mode {
                                Some(mode) => Some(winit::window::Fullscreen::Exclusive(mode)),
                                None => {
                                    log::warn!(
                                        "No matching video mode for exclusive fullscreen; \
                                         falling back to borderless"
                                    );
                                    Some(winit::window::Fullscreen::Borderless(None))
                                }
                            }
                        }
                    };
                    window.set_fullscreen(winit_mode);
                }
            }
        }

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

            // Skip rendering when minimized or window is too small (borderless
            // windows report non-zero sizes like 199x34 when minimized on Windows).
            let extent = swapchain.extent();
            if extent.width < 100 || extent.height < 100 {
                break 'render;
            }

            // Gather input and run egui.
            let raw_input = egui_state.take_egui_input(window);

            // Split borrows so the application can build egui UI.
            let egui_ctx = &self.egui_ctx;
            let app = &mut self.app;
            let window_ref = Arc::clone(window);
            // Capture cursor state before the closure borrows `app` mutably.
            let cursor_mode = self.current_cursor_mode;
            let cursor_pos = self.software_cursor_pos;
            let cursor_visible = self.cursor_in_window;
            let custom_cursor = app.software_cursor();
            let full_output = egui_ctx.run(raw_input, |ctx| {
                let _timer = ProfileTimer::new("Application::on_egui");
                app.on_egui(ctx, &window_ref);

                // Draw software cursor on top of everything in Confined mode,
                // but only when the OS cursor is inside the window.
                if cursor_mode == CursorMode::Confined && cursor_visible {
                    if let Some(ref sc) = custom_cursor {
                        cursor::draw_custom_cursor(ctx, cursor_pos, sc);
                    } else {
                        cursor::draw_default_cursor(ctx, cursor_pos);
                    }
                }
            });

            // Check if the application requested an exit (e.g. custom close button).
            if self.app.should_exit() {
                event_loop.exit();
            }

            // Handle platform output (cursor, clipboard, etc).
            egui_state.handle_platform_output(window, full_output.platform_output);

            // egui's handle_platform_output sets window.set_cursor(CursorIcon::Default)
            // on its first frame (internal state transition), which overrides our
            // transparent cursor. Re-apply every frame in Confined mode to ensure
            // the software cursor is the only one visible.
            if self.current_cursor_mode == CursorMode::Confined {
                if let Some(tc) = &self.transparent_cursor {
                    window.set_cursor(tc.clone());
                }
            }

            // Tessellate.
            let primitives = {
                let _t = ProfileTimer::new("egui::tessellate");
                egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point)
            };

            // Upload textures.
            if !full_output.textures_delta.set.is_empty() {
                let _t = ProfileTimer::new("egui::set_textures");
                egui_renderer
                    .set_textures(
                        vk_ctx.graphics_queue(),
                        swapchain.command_pool(),
                        &full_output.textures_delta.set,
                    )
                    .expect("Failed to set egui textures");
            }

            // Pre-render callback: runs after on_egui (UI flags set) but before
            // viewport framebuffer handles are captured for command recording.
            // Used for MSAA changes that recreate framebuffers.
            self.app.on_pre_render(renderer);

            // If a viewport framebuffer's render pass changed (e.g. MSAA recreation),
            // recreate the offscreen batch pipeline to match.
            for vi in 0..self.app.viewport_count() {
                if let Some(fb) = self.app.viewport_framebuffer(vi) {
                    if fb.color_attachment_count() > 1 {
                        let need_update = renderer
                            .offscreen_render_pass()
                            .is_none_or(|rp| rp != fb.render_pass());
                        if need_update {
                            if let Err(e) = renderer.create_offscreen_batch_pipeline(
                                fb.render_pass(),
                                fb.color_attachment_count() as u32,
                                fb.sample_count(),
                            ) {
                                log::error!(target: "gg_engine", "Offscreen pipeline recreate failed: {e}");
                            }
                        }
                        break;
                    }
                }
            }

            // Egui texture registration for all viewport framebuffers (first frame / after resize / after MSAA recreation).
            for vi in 0..self.app.viewport_count() {
                if let Some(fb) = self.app.viewport_framebuffer_mut(vi) {
                    if fb.egui_texture_id().is_none() {
                        let tex_id = egui_renderer.add_user_texture(fb.descriptor_set());
                        fb.set_egui_texture_id(tex_id);
                    }
                }
            }

            // Register/unregister app-provided user textures with egui.
            // Stale textures are collected but NOT removed yet — the
            // tessellated primitives from this frame's on_egui may still
            // reference them.  Removal is deferred to after render_frame.
            let stale_user_textures: Vec<(u64, egui::TextureId)>;
            {
                let wanted: Vec<u64> = self.app.egui_user_textures();
                let wanted_set: std::collections::HashSet<u64> = wanted.iter().copied().collect();

                // Collect stale registrations (deferred removal).
                stale_user_textures = self
                    .user_textures
                    .iter()
                    .filter(|(h, _)| !wanted_set.contains(h))
                    .map(|(h, id)| (*h, *id))
                    .collect();

                // Register new ones (handle is a raw vk::DescriptorSet).
                for h in wanted {
                    self.user_textures.entry(h).or_insert_with(|| {
                        let ds = vk::DescriptorSet::from_raw(h);
                        egui_renderer.add_user_texture(ds)
                    });
                }

                // Provide the mapping to the app.
                self.app.receive_egui_user_textures(&self.user_textures);
            }

            // Resize viewport framebuffers if their panel sizes changed.
            for vi in 0..self.app.viewport_count() {
                if let Some((w, h)) = self.app.viewport_desired_size(vi) {
                    if let Some(fb) = self.app.viewport_framebuffer_mut(vi) {
                        if w > 0 && h > 0 && (fb.width() != w || fb.height() != h) {
                            log::debug!(target: "gg_engine",
                                "Framebuffer[{vi}] resize: {}x{} -> {}x{}",
                                fb.width(), fb.height(), w, h
                            );
                            unsafe {
                                let _ = vk_ctx.device().device_wait_idle();
                            }
                            if let Err(e) = fb.resize(w, h) {
                                log::error!(target: "gg_engine", "Framebuffer[{vi}] resize failed: {e}");
                            } else if vi == 0 {
                                // Resize post-processing pipeline to match.
                                let color_view = fb.color_image_view();
                                let depth_view = fb.depth_image_view();
                                let msaa_depth_view = fb.msaa_depth_image_view();
                                let normal_view = fb.normal_image_view();
                                if let Err(e) = renderer.resize_postprocess(
                                    color_view,
                                    depth_view,
                                    msaa_depth_view,
                                    normal_view,
                                    w,
                                    h,
                                ) {
                                    log::error!(target: "gg_engine", "Post-process resize failed: {e}");
                                }
                            }
                        }
                    }
                }
            }

            // Poll clear color from application.
            renderer.set_clear_color(self.app.clear_color());

            // Late input sampling: let the app recompute camera/view with
            // the latest input state, right before we read the VP matrix.
            self.app.on_late_update(dt, &self.input);

            // Copy the VP matrix before the mutable borrow for render_frame.
            let camera_vp = *self
                .app
                .camera()
                .unwrap_or(&self.default_camera)
                .view_projection_matrix();

            // Extract offscreen framebuffer info for ALL viewports (all Copy values)
            // so the immutable borrow on self.app drops before render_frame
            // takes &mut self.app.
            let viewport_count = self.app.viewport_count().min(MAX_VIEWPORTS);
            let mut viewport_infos: Vec<(SceneFbInfo, ClearValues)> =
                Vec::with_capacity(viewport_count);
            for vi in 0..viewport_count {
                if let Some(fb) = self.app.viewport_framebuffer_mut(vi) {
                    let pending = fb.take_pending_readback();
                    let readback_image = pending
                        .as_ref()
                        .map(|(idx, _, _)| fb.color_attachment_image(*idx));
                    let clear_vals = fb.clear_values(renderer.clear_color());
                    let info = SceneFbInfo {
                        render_pass: fb.render_pass(),
                        vk_framebuffer: fb.vk_framebuffer(),
                        width: fb.width(),
                        height: fb.height(),
                        color_image: fb.color_image(),
                        depth_image: fb.depth_image(),
                        pending_readback: pending,
                        readback_image,
                        readback_buffer: fb.readback_buffer(),
                    };
                    viewport_infos.push((info, clear_vals));
                }
            }

            // Render frame.
            let frame_result = render_frame(
                vk_ctx,
                swapchain,
                renderer,
                &camera_vp,
                &mut self.app,
                egui_renderer,
                &primitives,
                full_output.pixels_per_point,
                &viewport_infos,
                dt.seconds(),
            );

            // Handle fatal GPU device lost.
            if matches!(frame_result, FrameResult::DeviceLost) {
                log::error!(target: "gg_engine",
                    "GPU device lost — the application cannot continue rendering. \
                     Save your work and restart.");
                self.app.on_device_lost();
                event_loop.exit();
                return;
            }

            // Advance to the next frame's sync primitives.
            swapchain.advance_frame();

            // Now that rendering is complete, remove stale user textures
            // that were deferred from earlier in this frame.
            for (h, tex_id) in stale_user_textures {
                self.user_textures.remove(&h);
                egui_renderer.remove_user_texture(tex_id);
            }

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
                    if let Err(e) = swapchain.recreate(vk_ctx, size.width, size.height, mode_arg) {
                        log::error!(target: "gg_engine", "Swapchain recreate failed: {e}");
                        return;
                    }
                    renderer.update_render_pass(swapchain.render_pass());
                    if let Err(e) = egui_renderer.set_render_pass(swapchain.render_pass()) {
                        log::error!(target: "gg_engine", "Failed to update egui render pass: {e:?}");
                    }
                    self.current_present_mode = desired;
                }
            }
        }

        // Snapshot input state so next frame can detect just-pressed transitions.
        self.input.end_frame();

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// Frame rendering
// ---------------------------------------------------------------------------

/// Extracted framebuffer data (all Copy) so we don't hold a borrow on the app.
#[derive(Clone, Copy)]
struct SceneFbInfo {
    render_pass: vk::RenderPass,
    vk_framebuffer: vk::Framebuffer,
    width: u32,
    height: u32,
    color_image: vk::Image,
    /// Raw depth `vk::Image` (either 1x or MSAA) for pipeline barriers.
    depth_image: Option<vk::Image>,
    // Pixel readback.
    pending_readback: Option<(usize, i32, i32)>,
    readback_image: Option<vk::Image>,
    readback_buffer: vk::Buffer,
}

#[allow(clippy::too_many_arguments)]
fn render_frame<T: Application>(
    vk_ctx: &VulkanContext,
    swapchain: &mut Swapchain,
    renderer: &mut Renderer,
    camera_vp: &Mat4,
    app: &mut T,
    egui_renderer: &mut egui_ash_renderer::Renderer,
    primitives: &[egui::ClippedPrimitive],
    pixels_per_point: f32,
    viewport_infos: &[(SceneFbInfo, ClearValues)],
    dt_seconds: f32,
) -> FrameResult {
    let _total = ProfileTimer::new("render_frame");
    let device = vk_ctx.device();
    let sc_extent = swapchain.extent();

    // Wait for this frame-slot's fence (still signaled from the previous use).
    {
        let _t = ProfileTimer::new("render_frame::wait_fence");
        if let Err(e) =
            unsafe { device.wait_for_fences(&[swapchain.in_flight_fence()], true, u64::MAX) }
        {
            log::error!("Failed to wait for fence: {e}");
            return FrameResult::DeviceLost;
        }
    }

    // Acquire next image — do NOT reset the fence yet.
    // If acquire fails with OUT_OF_DATE, the fence stays signaled so the
    // next frame's wait_for_fences won't deadlock.
    let acquire_result = {
        let _t = ProfileTimer::new("render_frame::acquire_image");
        unsafe {
            swapchain.swapchain_loader().acquire_next_image(
                swapchain.swapchain(),
                u64::MAX,
                swapchain.image_available_semaphore(),
                vk::Fence::null(),
            )
        }
    };

    let (image_index, acquire_suboptimal) = match acquire_result {
        Ok((idx, suboptimal)) => (idx, suboptimal),
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR) => {
            return FrameResult::RecreateSwapchain
        }
        Err(vk::Result::ERROR_DEVICE_LOST) => {
            log::error!("GPU device lost during image acquire");
            return FrameResult::DeviceLost;
        }
        Err(e) => {
            log::error!("Failed to acquire swapchain image: {e}");
            return FrameResult::RecreateSwapchain;
        }
    };

    // Acquire succeeded — now it's safe to reset the fence.
    if let Err(e) = unsafe { device.reset_fences(&[swapchain.in_flight_fence()]) } {
        log::error!("Failed to reset fence: {e}");
        return FrameResult::DeviceLost;
    }

    // Read pixel results from the staging buffer for this frame slot
    // (data written 2 frames ago, now safe to read after fence wait).
    for vi in 0..viewport_infos.len() {
        if let Some(fb) = app.viewport_framebuffer_mut(vi) {
            fb.read_pixel_result(swapchain.current_frame());
        }
    }

    let cmd_buf = swapchain.command_buffer(swapchain.current_frame());
    let current_frame = swapchain.current_frame();

    let _record = ProfileTimer::new("render_frame::record_commands");

    unsafe {
        device
            .reset_command_buffer(cmd_buf, vk::CommandBufferResetFlags::empty())
            .expect("Failed to reset command buffer");
        device
            .begin_command_buffer(cmd_buf, &vk::CommandBufferBeginInfo::default())
            .expect("Failed to begin command buffer");
    }

    // GPU profiler: read back previous results and reset for new recording.
    // Must be called AFTER begin_command_buffer (cmd_reset_query_pool requires an active buffer).
    if let Some(profiler) = renderer.gpu_profiler_mut() {
        profiler.begin_frame(cmd_buf, current_frame);
    }

    // GPU timestamp: frame start.
    if let Some(profiler) = renderer.gpu_profiler_mut() {
        profiler.timestamp(cmd_buf, current_frame, "Particles");
    }

    // Dispatch GPU particle compute (before any render pass).
    renderer.dispatch_particle_compute(cmd_buf, current_frame, dt_seconds);

    // GPU timestamp: after particles.
    if let Some(profiler) = renderer.gpu_profiler_mut() {
        profiler.timestamp(cmd_buf, current_frame, "Shadows");
    }

    // Reset the bone palette write offset for this frame (before shadows and scene).
    renderer.begin_bone_frame(current_frame);

    // Shadow pass (before any render pass — uses its own depth-only render pass).
    app.on_render_shadows(renderer, cmd_buf, current_frame);

    // GPU timestamp: after shadows, before scene.
    if let Some(profiler) = renderer.gpu_profiler_mut() {
        profiler.timestamp(cmd_buf, current_frame, "Scene");
    }

    if !viewport_infos.is_empty() {
        // --- Multi-viewport dual-pass path: N offscreen scene passes + swapchain egui ---

        for (vi, (fb, clear_values)) in viewport_infos.iter().enumerate() {
            let fb_extent = vk::Extent2D {
                width: fb.width,
                height: fb.height,
            };

            // Offscreen render pass for viewport `vi`.
            let offscreen_rp_info = vk::RenderPassBeginInfo::default()
                .render_pass(fb.render_pass)
                .framebuffer(fb.vk_framebuffer)
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: fb_extent,
                })
                .clear_values(clear_values);

            renderer.use_offscreen_pipeline(true);

            unsafe {
                device.cmd_begin_render_pass(
                    cmd_buf,
                    &offscreen_rp_info,
                    vk::SubpassContents::INLINE,
                );
            }

            renderer.begin_scene(
                None, // VP set by on_render_viewport via set_view_projection
                DrawContext {
                    cmd_buf,
                    extent: fb_extent,
                    current_frame: swapchain.current_frame(),
                    viewport_index: vi,
                },
            );
            app.on_render_viewport(renderer, vi);
            renderer.end_scene();

            renderer.use_offscreen_pipeline(false);

            unsafe {
                device.cmd_end_render_pass(cmd_buf);

                // Pipeline barriers: ensure offscreen color AND depth writes
                // are visible as shader reads for post-processing / egui.
                let color_barrier = vk::ImageMemoryBarrier::default()
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(fb.color_image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);

                let mut barriers = vec![color_barrier];

                // Depth barrier: flush depth writes so the post-processing
                // contact shadow pass (or MSAA depth resolve) can read them.
                if let Some(depth_img) = fb.depth_image {
                    let depth_barrier = vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
                        .new_layout(vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(depth_img)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::DEPTH,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .src_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ);
                    barriers.push(depth_barrier);
                }

                device.cmd_pipeline_barrier(
                    cmd_buf,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &barriers,
                );

                // Pixel readback: copy 1x1 region from the target attachment
                // to the staging buffer for CPU readback next frame.
                if let (Some((_, x, y)), Some(readback_image)) =
                    (fb.pending_readback, fb.readback_image)
                {
                    // Barrier: SHADER_READ_ONLY → TRANSFER_SRC.
                    let pre_barrier = vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(readback_image)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .src_access_mask(vk::AccessFlags::SHADER_READ)
                        .dst_access_mask(vk::AccessFlags::TRANSFER_READ);

                    device.cmd_pipeline_barrier(
                        cmd_buf,
                        vk::PipelineStageFlags::FRAGMENT_SHADER,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[pre_barrier],
                    );

                    // Copy 1x1 pixel.
                    let region = vk::BufferImageCopy {
                        buffer_offset: (swapchain.current_frame() * std::mem::size_of::<i32>())
                            as u64,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: vk::ImageSubresourceLayers {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            mip_level: 0,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        image_offset: vk::Offset3D { x, y, z: 0 },
                        image_extent: vk::Extent3D {
                            width: 1,
                            height: 1,
                            depth: 1,
                        },
                    };

                    device.cmd_copy_image_to_buffer(
                        cmd_buf,
                        readback_image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        fb.readback_buffer,
                        &[region],
                    );

                    // Barrier: TRANSFER_SRC → SHADER_READ_ONLY.
                    let post_barrier = vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .image(readback_image)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                        .dst_access_mask(vk::AccessFlags::SHADER_READ);

                    device.cmd_pipeline_barrier(
                        cmd_buf,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::FRAGMENT_SHADER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[post_barrier],
                    );
                }
            }
        }

        // GPU timestamp: after scene, before post-processing.
        if let Some(profiler) = renderer.gpu_profiler_mut() {
            profiler.timestamp(cmd_buf, current_frame, "PostProcess");
        }

        // Post-processing: bloom + tone mapping + color grading.
        if let Some(pp) = renderer.postprocess() {
            if pp.enabled {
                pp.execute(cmd_buf);
            }
        }

        // GPU timestamp: after post-processing, before egui.
        if let Some(profiler) = renderer.gpu_profiler_mut() {
            profiler.timestamp(cmd_buf, current_frame, "Egui");
        }

        // Swapchain render pass (egui only, dark background).
        let egui_clear = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: EDITOR_CHROME_CLEAR,
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 0.0, // Reverse-Z: far plane = 0.
                    stencil: 0,
                },
            },
        ];

        let swapchain_rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(swapchain.render_pass())
            .framebuffer(swapchain.framebuffer(image_index as usize))
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc_extent,
            })
            .clear_values(&egui_clear);

        unsafe {
            device.cmd_begin_render_pass(cmd_buf, &swapchain_rp_info, vk::SubpassContents::INLINE);
        }

        egui_renderer
            .cmd_draw(cmd_buf, sc_extent, pixels_per_point, primitives)
            .expect("Failed to draw egui");

        unsafe {
            device.cmd_end_render_pass(cmd_buf);
        }

        // GPU timestamp: frame end.
        if let Some(profiler) = renderer.gpu_profiler_mut() {
            profiler.timestamp(cmd_buf, current_frame, "End");
        }
    } else {
        // --- Single-pass path (backward compatible) ---

        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: renderer.clear_color(),
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 0.0, // Reverse-Z: far plane = 0.
                    stencil: 0,
                },
            },
        ];

        let render_pass_info = vk::RenderPassBeginInfo::default()
            .render_pass(swapchain.render_pass())
            .framebuffer(swapchain.framebuffer(image_index as usize))
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: sc_extent,
            })
            .clear_values(&clear_values);

        unsafe {
            device.cmd_begin_render_pass(cmd_buf, &render_pass_info, vk::SubpassContents::INLINE);
        }

        renderer.begin_scene(
            Some(camera_vp),
            DrawContext {
                cmd_buf,
                extent: sc_extent,
                current_frame: swapchain.current_frame(),
                viewport_index: 0,
            },
        );
        app.on_render(renderer);
        renderer.end_scene();

        egui_renderer
            .cmd_draw(cmd_buf, sc_extent, pixels_per_point, primitives)
            .expect("Failed to draw egui");

        unsafe {
            device.cmd_end_render_pass(cmd_buf);
        }

        // GPU timestamp: frame end.
        if let Some(profiler) = renderer.gpu_profiler_mut() {
            profiler.timestamp(cmd_buf, current_frame, "End");
        }
    }

    // End command buffer.
    unsafe {
        device
            .end_command_buffer(cmd_buf)
            .expect("Failed to end command buffer");
    }

    // Explicitly drop the record timer before submit.
    drop(_record);

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

    {
        let _t = ProfileTimer::new("render_frame::queue_submit");
        if let Err(e) = unsafe {
            device.queue_submit(
                vk_ctx.graphics_queue(),
                &[submit_info],
                swapchain.in_flight_fence(),
            )
        } {
            log::error!("Failed to submit draw command buffer: {e}");
            return FrameResult::DeviceLost;
        }
    }

    // Present.
    let swapchains = [swapchain.swapchain()];
    let image_indices = [image_index];
    let present_info = vk::PresentInfoKHR::default()
        .wait_semaphores(&signal_semaphores)
        .swapchains(&swapchains)
        .image_indices(&image_indices);

    let present_result = {
        let _t = ProfileTimer::new("render_frame::queue_present");
        unsafe {
            swapchain
                .swapchain_loader()
                .queue_present(vk_ctx.graphics_queue(), &present_info)
        }
    };

    match present_result {
        Ok(false) if !acquire_suboptimal => FrameResult::Ok,
        Ok(_) => FrameResult::RecreateSwapchain,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR) => {
            FrameResult::RecreateSwapchain
        }
        Err(vk::Result::ERROR_DEVICE_LOST) => {
            log::error!("GPU device lost during present");
            FrameResult::DeviceLost
        }
        Err(e) => {
            log::error!("Failed to present: {e}");
            FrameResult::RecreateSwapchain
        }
    }
}

// ---------------------------------------------------------------------------
// Gamepad integration (gilrs)
// ---------------------------------------------------------------------------

#[cfg(feature = "gamepad")]
fn map_gilrs_button(button: gilrs::Button) -> Option<crate::events::gamepad::GamepadButton> {
    use crate::events::gamepad::GamepadButton;
    match button {
        gilrs::Button::South => Some(GamepadButton::South),
        gilrs::Button::East => Some(GamepadButton::East),
        gilrs::Button::West => Some(GamepadButton::West),
        gilrs::Button::North => Some(GamepadButton::North),
        gilrs::Button::LeftTrigger => Some(GamepadButton::LeftBumper),
        gilrs::Button::LeftTrigger2 => Some(GamepadButton::LeftTrigger),
        gilrs::Button::RightTrigger => Some(GamepadButton::RightBumper),
        gilrs::Button::RightTrigger2 => Some(GamepadButton::RightTrigger),
        gilrs::Button::Select => Some(GamepadButton::Select),
        gilrs::Button::Start => Some(GamepadButton::Start),
        gilrs::Button::Mode => Some(GamepadButton::Guide),
        gilrs::Button::LeftThumb => Some(GamepadButton::LeftStick),
        gilrs::Button::RightThumb => Some(GamepadButton::RightStick),
        gilrs::Button::DPadUp => Some(GamepadButton::DPadUp),
        gilrs::Button::DPadDown => Some(GamepadButton::DPadDown),
        gilrs::Button::DPadLeft => Some(GamepadButton::DPadLeft),
        gilrs::Button::DPadRight => Some(GamepadButton::DPadRight),
        _ => None,
    }
}

#[cfg(feature = "gamepad")]
fn map_gilrs_axis(axis: gilrs::Axis) -> Option<crate::events::gamepad::GamepadAxis> {
    use crate::events::gamepad::GamepadAxis;
    match axis {
        gilrs::Axis::LeftStickX => Some(GamepadAxis::LeftStickX),
        gilrs::Axis::LeftStickY => Some(GamepadAxis::LeftStickY),
        gilrs::Axis::RightStickX => Some(GamepadAxis::RightStickX),
        gilrs::Axis::RightStickY => Some(GamepadAxis::RightStickY),
        _ => None,
    }
}

#[cfg(feature = "gamepad")]
impl<T: Application> EngineRunner<T> {
    /// Drain all pending gamepad events from gilrs and feed them into [].
    fn poll_gamepads(&mut self) {
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };

        while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
            let gamepad_id: usize = id.into();

            match event {
                gilrs::EventType::Connected => {
                    log::info!(target: "gg_engine", "Gamepad {gamepad_id} connected");
                    self.input.gamepad_connect(gamepad_id);
                }
                gilrs::EventType::Disconnected => {
                    log::info!(target: "gg_engine", "Gamepad {gamepad_id} disconnected");
                    self.input.gamepad_disconnect(gamepad_id);
                }
                gilrs::EventType::ButtonPressed(button, _) => {
                    if let Some(mapped) = map_gilrs_button(button) {
                        self.input.press_gamepad_button(gamepad_id, mapped);
                    }
                }
                gilrs::EventType::ButtonReleased(button, _) => {
                    if let Some(mapped) = map_gilrs_button(button) {
                        self.input.release_gamepad_button(gamepad_id, mapped);
                    }
                }
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    if let Some(mapped) = map_gilrs_axis(axis) {
                        self.input.set_gamepad_axis(gamepad_id, mapped, value);
                    }
                }
                gilrs::EventType::ButtonChanged(button, value, _) => {
                    // Analog trigger values reported as ButtonChanged.
                    // Map trigger buttons to their corresponding axes.
                    use crate::events::gamepad::GamepadAxis;
                    match button {
                        gilrs::Button::LeftTrigger2 => {
                            self.input
                                .set_gamepad_axis(gamepad_id, GamepadAxis::LeftTrigger, value);
                        }
                        gilrs::Button::RightTrigger2 => {
                            self.input
                                .set_gamepad_axis(gamepad_id, GamepadAxis::RightTrigger, value);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// run()
// ---------------------------------------------------------------------------

pub fn run<T: Application>() {
    crate::log_init();

    // Install a panic hook that logs the panic info before the default handler runs.
    // This ensures panics are captured in the engine log even when the terminal
    // output is lost (e.g. windowed applications, crash reports).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<dyn Any>".to_string()
        };
        log::error!(target: "gg_engine", "PANIC at {location}: {payload}");
        default_hook(info);
    }));

    log::info!(target: "gg_engine", "Engine v{}", crate::engine_version());

    crate::profiling::begin_session("Startup", "gg_profile_startup.json");

    let mut layers = LayerStack::new();
    let app = T::new(&mut layers);
    let window_config = app.window_config();
    let current_present_mode = app.present_mode();

    // Default camera sized to the window's aspect ratio.
    let aspect = window_config.width as f32 / window_config.height as f32;
    let default_camera = OrthographicCamera::new(-aspect, aspect, -1.0, 1.0);

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    // NOTE: Startup session stays open — it will be ended inside resumed()
    // after Vulkan init + on_attach, so the startup profile captures
    // everything up to first frame.

    let mut runner = EngineRunner {
        app,
        layers,
        input: Input::new(),
        window_config,
        current_present_mode,
        current_cursor_mode: CursorMode::Normal,
        transparent_cursor: None,
        software_cursor_pos: (0.0, 0.0),
        cursor_in_window: false,
        default_camera,
        last_frame_time: Instant::now(),
        smoothed_dt: 1.0 / 60.0,
        minimized: false,
        pending_resize: None,
        egui_ctx: {
            let ctx = egui::Context::default();
            crate::ui_theme::apply_engine_theme(&ctx);
            ctx
        },
        egui_winit_state: None,
        egui_renderer: None,
        user_textures: HashMap::new(),
        renderer: None,
        swapchain: None,
        allocator: None,
        vulkan_context: None,
        window: None,
        #[cfg(feature = "gamepad")]
        gilrs: gilrs::Gilrs::new().ok(),
    };

    event_loop.run_app(&mut runner).expect("event loop error");

    // Runtime session is ended by EngineRunner::Drop, shutdown session wraps
    // the drop itself.
    log::info!(target: "gg_engine", "Shutting down");
    crate::profiling::begin_session("Shutdown", "gg_profile_shutdown.json");
    drop(runner);
    crate::profiling::end_session();
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
            if let Some(btn) = map_mouse_button(*button) {
                (
                    match state {
                        ElementState::Pressed => Some(Event::Mouse(MouseEvent::ButtonPressed(btn))),
                        ElementState::Released => {
                            Some(Event::Mouse(MouseEvent::ButtonReleased(btn)))
                        }
                    },
                    None,
                )
            } else {
                (None, None)
            }
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

fn map_mouse_button(button: winit::event::MouseButton) -> Option<MouseButton> {
    match button {
        winit::event::MouseButton::Left => Some(MouseButton::Left),
        winit::event::MouseButton::Right => Some(MouseButton::Right),
        winit::event::MouseButton::Middle => Some(MouseButton::Middle),
        winit::event::MouseButton::Back => Some(MouseButton::Back),
        winit::event::MouseButton::Forward => Some(MouseButton::Forward),
        winit::event::MouseButton::Other(_) => None, // ignore unknown buttons
    }
}

// ---------------------------------------------------------------------------
// Cursor mode
// ---------------------------------------------------------------------------

/// Apply a [`CursorMode`] to the winit window (cursor icon + grab).
///
/// In `Confined` mode the OS cursor is replaced with a transparent image (no grab)
/// so `CursorMoved` events keep firing. The cursor is free to leave the window —
/// standard windowed behaviour matching how shipped games handle software cursors.
fn apply_cursor_mode(
    window: &Window,
    mode: CursorMode,
    transparent: Option<&winit::window::CustomCursor>,
) {
    use winit::window::CursorGrabMode;
    match mode {
        CursorMode::Normal => {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            window.set_cursor(winit::window::CursorIcon::Default);
        }
        CursorMode::Confined => {
            // No OS-level grab — cursor is free to leave the window, which is
            // standard windowed behaviour. The software cursor provides the
            // visual; CursorEntered/CursorLeft track whether to draw it.
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            if let Some(tc) = transparent {
                window.set_cursor(tc.clone());
            } else {
                window.set_cursor_visible(false);
            }
        }
        CursorMode::Locked => {
            // Lock cursor (best for raw deltas); fall back to Confined.
            if window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
                let _ = window.set_cursor_grab(CursorGrabMode::Confined);
            }
            window.set_cursor_visible(false);
        }
    }
}
