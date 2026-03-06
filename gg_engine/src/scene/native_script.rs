use crate::input::Input;
use crate::scene::{Entity, Scene};
use crate::timestep::Timestep;

/// Trait for attaching behavior to entities via native Rust code.
///
/// Implement this trait to create scripts that respond to entity lifecycle
/// events. Scripts are attached to entities through [`NativeScriptComponent`]
/// and receive their owning entity + scene access on each callback.
///
/// All methods have default empty implementations — override only what you need.
///
/// # Example
///
/// ```ignore
/// struct CameraController { speed: f32 }
///
/// impl Default for CameraController {
///     fn default() -> Self { Self { speed: 5.0 } }
/// }
///
/// impl NativeScript for CameraController {
///     fn on_update(&mut self, entity: Entity, scene: &mut Scene, dt: Timestep, input: &Input) {
///         if let Some(mut t) = scene.get_component_mut::<TransformComponent>(entity) {
///             if input.is_key_pressed(KeyCode::D) {
///                 t.translation.x += self.speed * dt.seconds();
///             }
///         }
///     }
/// }
///
/// // Attach to an entity:
/// scene.add_component(entity, NativeScriptComponent::bind::<CameraController>());
/// ```
pub trait NativeScript: Send + Sync + 'static {
    /// Called once when the script instance is first created (before the first `on_update`).
    fn on_create(&mut self, _entity: Entity, _scene: &mut Scene) {}

    /// Called every frame.
    fn on_update(&mut self, _entity: Entity, _scene: &mut Scene, _dt: Timestep, _input: &Input) {}

    /// Called at the fixed physics rate (1/60 s). Use this for applying forces/impulses.
    fn on_fixed_update(&mut self, _entity: Entity, _scene: &mut Scene, _dt: Timestep, _input: &Input) {}

    /// Called when the script is destroyed.
    fn on_destroy(&mut self, _entity: Entity, _scene: &mut Scene) {}
}
