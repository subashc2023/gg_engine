# Engine Core

The core engine lives in `gg_engine/` and provides the foundation that both `gg_editor` and `gg_sandbox` depend on.

## Smart Pointer Type Aliases

`lib.rs` defines two type aliases used throughout the engine:

```rust
pub type Ref<T> = Arc<T>;   // shared-ownership (rendering resources, shaders, pipelines)
pub type Scope<T> = Box<T>; // owning pointer (single owner)
```

Both are exported in the prelude.

## Application Trait

**File:** `gg_engine/src/application.rs`

Client apps implement the `Application` trait and launch via `run::<T>()`. The `run` function handles engine initialization (logging, version banner), creates a `LayerStack`, passes it to `T::new(&mut LayerStack)`, then enters the winit event loop.

### Overridable Methods

| Method | Purpose |
|--------|---------|
| `window_config()` | Customize window title and size |
| `on_attach(&mut self, &Renderer)` | Called once after renderer init; create shaders, buffers, pipelines |
| `on_event(&Event, &Input)` | Handle input events |
| `on_update(Timestep, &Input)` | Per-frame logic (input, camera, game logic) |
| `on_render(&mut self, &mut Renderer)` | Submit draw calls (between `begin_scene`/`end_scene`) |
| `on_egui(&mut self, &egui::Context)` | Build immediate-mode UI each frame |
| `clear_color() -> [f32; 4]` | Clear color for the render pass (polled each frame) |
| `camera() -> Option<&OrthographicCamera>` | Override the default camera (`None` uses aspect-ratio default) |
| `present_mode() -> PresentMode` | Request a present mode dynamically (engine recreates swapchain on change) |
| `scene_framebuffer() / scene_framebuffer_mut()` | Return `Option<&Framebuffer>`; when `Some`, enables dual-pass rendering |
| `desired_viewport_size() -> Option<(u32, u32)>` | Signal a framebuffer resize |
| `block_events() -> bool` | When `true` (default), input events blocked from reaching the app |
| `should_exit() -> bool` | When `true`, exits the event loop |
| `on_device_lost()` | Called on GPU device loss; opportunity for emergency save |
| `viewport_count() -> usize` | Number of offscreen viewports (default 0) |
| `viewport_framebuffer(index)` | Return `Option<&Framebuffer>` for viewport at index |
| `viewport_framebuffer_mut(index)` | Mutable access to viewport framebuffer |
| `viewport_desired_size(index)` | Signal a viewport framebuffer resize |
| `on_render_viewport(renderer, index)` | Per-viewport render callback |
| `egui_user_textures() -> Vec<u64>` | Register framebuffer textures for egui display |
| `receive_egui_user_textures(map)` | Receive egui TextureId mappings for registered textures |

### Lifecycle

```
run::<T>()
  ├── Initialize logging, version banner
  ├── Create LayerStack
  ├── T::new(&mut LayerStack)
  ├── Enter winit event loop
  │     ├── on_attach(&Renderer)         [once, after renderer init]
  │     └── Per frame:
  │           ├── on_event / on_update   [layers first, then app]
  │           ├── on_render              [between begin_scene/end_scene]
  │           └── on_egui               [egui pass]
  └── Drop (GPU wait idle, resource teardown)
```

### Examples

- `gg_sandbox/src/main.rs` and `gg_sandbox/src/sandbox2d.rs` — complete sandbox example
- `gg_editor/src/main.rs` — full editor with ECS, framebuffers, and docking UI

## Layer System

**File:** `gg_engine/src/layer.rs`

The `Layer` trait and `LayerStack` provide a stack-based processing model.

### Layer Trait

```rust
trait Layer {
    fn on_attach(&mut self);
    fn on_detach(&mut self);
    fn on_update(&mut self, dt: Timestep, input: &Input);
    fn on_event(&mut self, event: &Event, input: &Input) -> bool;
}
```

### LayerStack

- Two zones: **normal layers** and **overlays**
- Updates iterate bottom-to-top
- Events dispatch top-to-bottom
- A layer returning `true` from `on_event` stops propagation
- Unhandled events fall through to `Application::on_event`

## Input

**File:** `gg_engine/src/input.rs`

`Input` provides keyboard and mouse polling state, updated from winit events before dispatch. Passed to all callbacks via `&Input`. Exported in prelude.

## Timestep

**File:** `gg_engine/src/timestep.rs`

`Timestep` is a `Copy` wrapper around `f32` (seconds).

```rust
Timestep::from_seconds(0.016)
dt.seconds()  // -> f32
dt.millis()   // -> f32
dt * 2.0      // Supports Mul<f32> in both directions
```

Passed to `on_update` each frame.

## Events

**File:** `gg_engine/src/events/`

The `Event` enum has three variants: `Window`, `Key`, `Mouse`. All are `Copy + PartialEq + Display`.

- `EngineRunner` maps winit events to engine events via `map_window_event()`
- `KeyEvent::Typed(char)` is a separate variant for text input (press only, filters control chars)

## Egui Integration

Egui is integrated at the `EngineRunner` level in `application.rs`, **not** as a Layer. Per-frame flow:

1. Collect raw input via `egui-winit`
2. Run `app.on_egui(ctx)`
3. Tessellate
4. Upload textures
5. Draw inside the Vulkan render pass

If `egui_winit_state.on_window_event()` returns `consumed = true`, the event is swallowed before reaching the engine event system.

`gg_engine` re-exports `egui` and `glam` at the crate root, so client crates use `gg_engine::egui` without declaring their own dependency.

## UI Theme

**File:** `gg_engine/src/ui_theme.rs`

Applies a VS Code Dark+ inspired egui theme automatically during engine init via `apply_engine_theme(ctx)`. Embeds JetBrains Mono Regular + Bold TTF fonts from `gg_engine/assets/fonts/`.

```rust
// Use the bold font in egui
pub const BOLD_FONT: &str = "JetBrainsMono-Bold"; // re-exported in prelude
RichText::new("text").font(FontId::new(14.0, FontFamily::Name(BOLD_FONT.into())))
```

## Particle System

**File:** `gg_engine/src/particle_system.rs`

Pool-based particle emitter with fixed-size allocation.

```rust
let mut particles = ParticleSystem::new(10_000);

// Configure
let props = ParticleProps {
    position: Vec2::ZERO,
    velocity: Vec2::new(0.0, 1.0),
    color_begin: Vec4::new(1.0, 0.5, 0.0, 1.0),
    color_end: Vec4::new(1.0, 1.0, 0.0, 0.0),
    size_begin: 0.5,
    size_end: 0.0,
    lifetime: 2.0,
    ..Default::default()
};

particles.emit(props);
particles.on_update(dt);
particles.on_render(&mut renderer);
```

- Round-robin cycling (overwrites oldest particles when full)
- Velocity damping, rotation, color/size interpolation from life fraction
- Renders at z = -0.1 to -0.15 (in front of z=0 scene)

## Platform Utilities

**File:** `gg_engine/src/platform_utils.rs`

`FileDialogs` — thin wrapper around the `rfd` crate for native OS file dialogs.

```rust
let path = FileDialogs::open_file("Scene files", &["ggscene"]);
let path = FileDialogs::save_file("Scene files", &["ggscene"]);
```

Returns `Option<String>` (`None` if the user cancels). Exported in prelude.

## UUID System

**File:** `gg_engine/src/uuid.rs`

64-bit random UUID (not 128-bit, sufficient for game engine use). Generated via `rand::rng().random::<u64>()`.

```rust
Uuid::new()              // random
Uuid::from_raw(12345u64) // for deserialization
uuid.raw() -> u64        // raw value
```

`IdComponent` wraps `Uuid` and is spawned on every entity automatically.

## Prelude

Client crates should use:

```rust
use gg_engine::prelude::*;
```

Re-exports all engine types, `glam` math types, event types, `log` macros, and more. Built-in shader SPIR-V bytes are available separately via `gg_engine::shaders`.

## EngineRunner Drop Order

`EngineRunner` uses `Option<T>` for components initialized in `resumed()`. The custom `Drop` impl calls `device_wait_idle()` before field drops begin. Fields drop in reverse declaration order so the application (and its rendering resources) outlives the Vulkan infrastructure.

## Egui 0.33 API Notes

Key breaking changes from earlier versions:
- `Rounding` renamed to `CornerRadius` (takes `u8`)
- `Button.rounding()` renamed to `.corner_radius()`
- Use `egui::MenuBar::new().ui()` instead of deprecated `egui::menu::bar()`
