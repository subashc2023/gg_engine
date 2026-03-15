/// Cursor grab and visibility mode.
///
/// Controls how the OS cursor behaves and whether a software cursor is rendered.
///
/// # Modes
///
/// | Mode | OS cursor | Grab | Software cursor | Use case |
/// |------|-----------|------|-----------------|----------|
/// | `Normal` | Visible | None | No | Editor UI, menus |
/// | `Confined` | Hidden | Confined | Yes (arrow or custom) | RTS, strategy, in-game UI |
/// | `Locked` | Hidden | Locked | No | FPS camera, flight sim |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorMode {
    /// OS cursor visible, no grab. Default.
    #[default]
    Normal,
    /// OS cursor hidden, software cursor rendered, confined to window bounds.
    /// Mouse position tracked normally via `Input::mouse_position()`.
    Confined,
    /// OS cursor hidden and locked in place. Raw deltas only via `Input::mouse_delta()`.
    /// No software cursor. Use for first-person camera look.
    Locked,
}
