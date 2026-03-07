mod application;
pub mod asset;
pub mod events;
mod input;
mod layer;
mod logging;
mod orthographic_camera_controller;
pub mod particle_system;
pub mod platform_utils;
pub mod profiling;
pub mod project;
pub mod renderer;
pub mod scene;
mod timestep;
pub mod ui_theme;
pub mod uuid;

/// Shared-ownership smart pointer for rendering resources.
/// Wraps `Arc<T>` for thread-safe reference counting.
pub type Ref<T> = std::sync::Arc<T>;

/// Owning smart pointer (heap-allocated, single owner).
pub type Scope<T> = Box<T>;

pub use application::{run, Application, WindowConfig};
pub use asset::{AssetHandle, AssetMetadata, AssetRegistry, AssetType, EditorAssetManager};
pub use egui;
pub use glam;
pub use hecs;
pub use input::Input;
pub use layer::{Layer, LayerStack};
pub use log;
pub use logging::{clear_log_buffer, init as log_init, with_log_buffer, LogEntry};
#[cfg(feature = "lua-scripting")]
pub use mlua;
pub use orthographic_camera_controller::OrthographicCameraController;
pub use particle_system::{ParticleProps, ParticleSystem};
pub use platform_utils::{error_dialog, FileDialogs};
pub use project::Project;
pub use renderer::shaders;
pub use renderer::{
    as_bytes, BufferElement, BufferLayout, EditorCamera, Font, Framebuffer, FramebufferSpec,
    FramebufferTextureFormat, FramebufferTextureSpec, IndexBuffer, OrthographicCamera, Pipeline,
    PresentMode, ProjectionType, Renderer, Renderer2DStats, RendererBackend, SceneCamera, Shader,
    ShaderDataType, ShaderLibrary, SubTexture2D, Texture2D, TextureSpecification, VertexArray,
    VertexBuffer,
};
pub use scene::{
    AnimationClip, AnimationControllerComponent, AnimationTransition, AudioListenerComponent,
    AudioSourceComponent, BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent,
    CircleRendererComponent, Entity, FloatOrdering, IdComponent, InstancedSpriteAnimator,
    NativeScript, NativeScriptComponent, ParticleEmitterComponent, RelationshipComponent,
    RigidBody2DComponent, RigidBody2DType, Scene, SceneSerializer, SpriteAnimatorComponent,
    SpriteRendererComponent, TagComponent, TextComponent, TilemapComponent, TransformComponent,
    TransitionCondition, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
#[cfg(feature = "lua-scripting")]
pub use scene::{LuaScriptComponent, ScriptEngine, ScriptFieldValue};
pub use timestep::Timestep;
pub use uuid::Uuid;
pub use winit;

pub fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Convenience re-exports for client applications.
pub mod prelude {
    pub use crate::asset::{AssetHandle, AssetType, EditorAssetManager};
    pub use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
    pub use crate::input::Input;
    pub use crate::layer::{Layer, LayerStack};
    pub use crate::orthographic_camera_controller::OrthographicCameraController;
    pub use crate::particle_system::{ParticleProps, ParticleSystem};
    pub use crate::platform_utils::{error_dialog, FileDialogs};
    pub use crate::profiling::{
        begin_session, drain_profile_results, end_session, is_session_active, ProfileResult,
        ProfileTimer,
    };
    pub use crate::project::Project;
    pub use crate::renderer::{
        as_bytes, BufferElement, BufferLayout, EditorCamera, Font, Framebuffer, FramebufferSpec,
        FramebufferTextureFormat, FramebufferTextureSpec, IndexBuffer, OrthographicCamera,
        Pipeline, PresentMode, ProjectionType, Renderer, Renderer2DStats, RendererBackend,
        SceneCamera, Shader, ShaderDataType, ShaderLibrary, SubTexture2D, Texture2D, VertexArray,
        VertexBuffer,
    };
    pub use crate::scene::{
        AnimationClip, AnimationControllerComponent, AnimationTransition,
        AudioListenerComponent, AudioSourceComponent, BoxCollider2DComponent, CameraComponent,
        CircleCollider2DComponent, CircleRendererComponent, Entity, FloatOrdering, IdComponent,
        InstancedSpriteAnimator, NativeScript, NativeScriptComponent, ParticleEmitterComponent,
        RelationshipComponent, RigidBody2DComponent, RigidBody2DType, Scene, SceneSerializer,
        SpriteAnimatorComponent, SpriteRendererComponent, TagComponent, TextComponent,
        TilemapComponent, TransformComponent, TransitionCondition, TILE_FLIP_H, TILE_FLIP_V,
        TILE_ID_MASK,
    };
    #[cfg(feature = "lua-scripting")]
    pub use crate::scene::{LuaScriptComponent, ScriptEngine, ScriptFieldValue};
    pub use crate::timestep::Timestep;
    pub use crate::ui_theme::BOLD_FONT;
    pub use crate::uuid::Uuid;
    pub use crate::{profile_scope, run, Application, Ref, Scope, WindowConfig};
    pub use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
    pub use log::{debug, error, info, trace, warn};
    pub use winit::window::Window;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_exists() {
        assert!(!engine_version().is_empty());
    }
}
