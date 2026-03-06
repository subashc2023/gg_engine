use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ash::vk::{self, Handle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowAttributes};

use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
use crate::input::Input;
use crate::layer::LayerStack;
use crate::profiling::ProfileTimer;
use crate::renderer::{
    ClearValues, DrawContext, Framebuffer, GpuAllocator, OrthographicCamera, PresentMode, Renderer,
    Swapchain, VulkanContext, MAX_FRAMES_IN_FLIGHT,
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
    fn on_late_update(&mut self, _input: &Input) {}

    /// Submit draw calls. Called each frame between `begin_scene` / `end_scene`.
    fn on_render(&mut self, _renderer: &mut Renderer) {}

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
    default_camera: OrthographicCamera,
    last_frame_time: Instant,
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

        // Unregister scene framebuffer's egui texture before the egui renderer drops.
        if let Some(fb) = self.app.scene_framebuffer() {
            if let Some(tex_id) = fb.egui_texture_id() {
                if let Some(egui_renderer) = &mut self.egui_renderer {
                    egui_renderer.remove_user_texture(tex_id);
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
        if let Some((x, y)) = self.window_config.position {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }

        match event_loop.create_window(attrs) {
            Ok(window) => {
                log::info!(target: "gg_engine", "Window created: \"{}\" ({}x{})",
                    self.window_config.title, self.window_config.width, self.window_config.height);
                let window = Arc::new(window);

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

                                        // Notify the application that the renderer is ready.
                                        if let Some(renderer) = &mut self.renderer {
                                            self.app.on_attach(renderer);
                                        }

                                        // If the app has a multi-attachment framebuffer, create
                                        // an offscreen batch pipeline compatible with its render pass.
                                        if let Some(fb) = self.app.scene_framebuffer() {
                                            if fb.color_attachment_count() > 1 {
                                                if let Some(renderer) = &mut self.renderer {
                                                    if let Err(e) = renderer.create_offscreen_batch_pipeline(
                                                        fb.render_pass(),
                                                        fb.color_attachment_count() as u32,
                                                    ) {
                                                        log::error!(target: "gg_engine", "Offscreen pipeline init failed: {e}");
                                                    }
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

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        // Forward raw winit event to egui-winit first.
        if let (Some(state), Some(window)) = (&mut self.egui_winit_state, &self.window) {
            let response = state.on_window_event(window, &event);
            // Only let egui block events from reaching the engine when the
            // application says so. Editors override `block_events()` to let
            // viewport-targeted events (e.g. scroll zoom) pass through.
            if response.consumed && self.app.block_events() {
                return;
            }
        }

        // Update input polling state from raw winit event.
        // This happens before engine event mapping so that key presses
        // producing Typed events are still tracked by the polling system.
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
            }
            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                if let Some(btn) = map_mouse_button(*button) {
                    match state {
                        ElementState::Pressed => self.input.press_mouse_button(btn),
                        ElementState::Released => self.input.release_mouse_button(btn),
                    }
                }
            }
            winit::event::WindowEvent::Focused(false) => {
                // Clear all pressed keys/buttons on focus loss to prevent
                // "stuck" keys when the user Alt+Tabs away while holding keys.
                self.input.clear_all();
            }
            _ => {}
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
        let dt = Timestep::from_seconds(now.duration_since(self.last_frame_time).as_secs_f32());
        self.last_frame_time = now;

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
            {
                let _timer = ProfileTimer::new("LayerStack::on_update");
                self.layers.update_all(dt, &self.input);
            }
            self.app.on_update(dt, &self.input);
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
            let full_output = egui_ctx.run(raw_input, |ctx| {
                let _timer = ProfileTimer::new("Application::on_egui");
                app.on_egui(ctx, &window_ref);
            });

            // Check if the application requested an exit (e.g. custom close button).
            if self.app.should_exit() {
                event_loop.exit();
            }

            // Handle platform output (cursor, clipboard, etc).
            egui_state.handle_platform_output(window, full_output.platform_output);

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

            // Egui texture registration for scene framebuffer (first frame).
            if let Some(fb) = self.app.scene_framebuffer_mut() {
                if fb.egui_texture_id().is_none() {
                    let tex_id = egui_renderer.add_user_texture(fb.descriptor_set());
                    fb.set_egui_texture_id(tex_id);
                }
            }

            // Register/unregister app-provided user textures with egui.
            {
                let wanted: Vec<u64> = self.app.egui_user_textures();
                let wanted_set: std::collections::HashSet<u64> =
                    wanted.iter().copied().collect();

                // Remove stale registrations.
                let stale: Vec<u64> = self
                    .user_textures
                    .keys()
                    .filter(|h| !wanted_set.contains(h))
                    .copied()
                    .collect();
                for h in stale {
                    if let Some(tex_id) = self.user_textures.remove(&h) {
                        egui_renderer.remove_user_texture(tex_id);
                    }
                }

                // Register new ones (handle is a raw vk::DescriptorSet).
                for h in wanted {
                    self.user_textures
                        .entry(h)
                        .or_insert_with(|| {
                            let ds = vk::DescriptorSet::from_raw(h);
                            egui_renderer.add_user_texture(ds)
                        });
                }

                // Provide the mapping to the app.
                self.app.receive_egui_user_textures(&self.user_textures);
            }

            // Resize scene framebuffer if the viewport size changed.
            if let Some((w, h)) = self.app.desired_viewport_size() {
                if let Some(fb) = self.app.scene_framebuffer_mut() {
                    if w > 0 && h > 0 && (fb.width() != w || fb.height() != h) {
                        log::debug!(target: "gg_engine",
                            "Framebuffer resize: {}x{} -> {}x{}",
                            fb.width(), fb.height(), w, h
                        );
                        unsafe {
                            let _ = vk_ctx.device().device_wait_idle();
                        }
                        if let Err(e) = fb.resize(w, h) {
                            log::error!(target: "gg_engine", "Framebuffer resize failed: {e}");
                        }
                    }
                }
            }

            // Poll clear color from application.
            renderer.set_clear_color(self.app.clear_color());

            // Late input sampling: let the app recompute camera/view with
            // the latest input state, right before we read the VP matrix.
            self.app.on_late_update(&self.input);

            // Copy the VP matrix before the mutable borrow for render_frame.
            let camera_vp = *self
                .app
                .camera()
                .unwrap_or(&self.default_camera)
                .view_projection_matrix();

            // Extract offscreen framebuffer info (all Copy values) so
            // the immutable borrow on self.app drops before render_frame
            // takes &mut self.app.
            // Uses scene_framebuffer_mut() because take_pending_readback is &mut.
            let (scene_fb_info, scene_clear_values) =
                if let Some(fb) = self.app.scene_framebuffer_mut() {
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
                        pending_readback: pending,
                        readback_image,
                        readback_buffer: fb.readback_buffer(),
                    };
                    (Some(info), clear_vals)
                } else {
                    (None, ClearValues::default())
                };

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
                scene_fb_info,
                &scene_clear_values,
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
    scene_fb: Option<SceneFbInfo>,
    scene_clear_values: &[vk::ClearValue],
    dt_seconds: f32,
) -> FrameResult {
    let _total = ProfileTimer::new("render_frame");
    let device = vk_ctx.device();
    let sc_extent = swapchain.extent();

    // Wait for this frame-slot's fence (still signaled from the previous use).
    {
        let _t = ProfileTimer::new("render_frame::wait_fence");
        if let Err(e) = unsafe {
            device.wait_for_fences(&[swapchain.in_flight_fence()], true, u64::MAX)
        } {
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

    // Read pixel result from the staging buffer for this frame slot
    // (data written 2 frames ago, now safe to read after fence wait).
    if scene_fb.is_some() {
        if let Some(fb) = app.scene_framebuffer_mut() {
            fb.read_pixel_result(swapchain.current_frame());
        }
    }

    let cmd_buf = swapchain.command_buffer(swapchain.current_frame());

    let _record = ProfileTimer::new("render_frame::record_commands");

    unsafe {
        device
            .reset_command_buffer(cmd_buf, vk::CommandBufferResetFlags::empty())
            .expect("Failed to reset command buffer");
        device
            .begin_command_buffer(cmd_buf, &vk::CommandBufferBeginInfo::default())
            .expect("Failed to begin command buffer");
    }

    // Dispatch GPU particle compute (before any render pass).
    renderer.dispatch_particle_compute(cmd_buf, swapchain.current_frame(), dt_seconds);

    if let Some(fb) = scene_fb {
        // --- Dual-pass path: offscreen scene + swapchain egui ---

        let fb_extent = vk::Extent2D {
            width: fb.width,
            height: fb.height,
        };

        // 1. Offscreen render pass (scene draws).
        let offscreen_rp_info = vk::RenderPassBeginInfo::default()
            .render_pass(fb.render_pass)
            .framebuffer(fb.vk_framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: fb_extent,
            })
            .clear_values(scene_clear_values);

        // Switch to offscreen batch pipeline (multi-attachment compatible).
        renderer.use_offscreen_pipeline(true);

        unsafe {
            device.cmd_begin_render_pass(cmd_buf, &offscreen_rp_info, vk::SubpassContents::INLINE);
        }

        renderer.begin_scene(
            camera_vp,
            DrawContext {
                cmd_buf,
                extent: fb_extent,
                current_frame: swapchain.current_frame(),
            },
        );
        app.on_render(renderer);
        renderer.end_scene();

        // Switch back to normal pipeline for subsequent passes.
        renderer.use_offscreen_pipeline(false);

        unsafe {
            device.cmd_end_render_pass(cmd_buf);

            // Pipeline barrier: ensure offscreen color write is visible
            // as a shader read when egui samples the texture in the
            // swapchain render pass. This replaces the exit subpass
            // dependency to keep the dependency count at 1 (matching
            // the swapchain render pass for pipeline compatibility).
            let barrier = vk::ImageMemoryBarrier::default()
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

            device.cmd_pipeline_barrier(
                cmd_buf,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );

            // Pixel readback: copy 1×1 region from the target attachment
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

                // Copy 1×1 pixel.
                let region = vk::BufferImageCopy {
                    buffer_offset: (swapchain.current_frame() * std::mem::size_of::<i32>()) as u64,
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

        // 2. Swapchain render pass (egui only, dark background).
        let egui_clear = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: EDITOR_CHROME_CLEAR,
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 1.0,
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
                    depth: 1.0,
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
            camera_vp,
            DrawContext {
                cmd_buf,
                extent: sc_extent,
                current_frame: swapchain.current_frame(),
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
// run()
// ---------------------------------------------------------------------------

pub fn run<T: Application>() {
    crate::log_init();
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
        default_camera,
        last_frame_time: Instant::now(),
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
