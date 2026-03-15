mod application;
pub mod asset;
pub mod cursor;
pub mod error;
pub mod events;
mod input;
pub mod input_action;
pub mod jobs;
mod layer;
mod logging;
mod orthographic_camera_controller;
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
pub use ash;
pub use asset::{
    cook_assets, AssetHandle, AssetMetadata, AssetRegistry, AssetType, BuildManifest,
    EditorAssetManager, FileCategory, ManifestEntry,
};
pub use cursor::{CursorMode, SoftwareCursor};
pub use egui;
pub use error::{EngineError, EngineResult};
pub use events::gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId};
pub use glam;
pub use hecs;
pub use input::Input;
pub use input_action::{ActionType, InputAction, InputActionMap, InputBinding};
pub use layer::{Layer, LayerStack};
pub use log;
pub use logging::{clear_log_buffer, init as log_init, with_log_buffer, LogEntry};
#[cfg(feature = "lua-scripting")]
pub use mlua;
pub use orthographic_camera_controller::OrthographicCameraController;
pub use renderer::ParticleProps;
pub use platform_utils::{error_dialog, FileDialogs};
pub use project::{DeadZoneConfig, Project};
pub use renderer::shaders;
pub use renderer::{
    as_bytes, load_gltf, BlendMode, BufferElement, BufferLayout, CullMode, DepthConfig,
    EditorCamera, Font, Framebuffer, FramebufferSpec, FramebufferTextureFormat,
    FramebufferTextureSpec, GpuProfiler, GpuTimingResult, IndexBuffer, Material, MaterialGpuData,
    MaterialHandle, MaterialLibrary, Mesh, MeshVertex, MsaaSamples, OrthographicCamera, Pipeline,
    PostProcessPipeline, PresentMode, ProjectionType, Renderer, Renderer2DStats, RendererBackend,
    SceneCamera, Shader, ShaderDataType, ShaderLibrary, SubTexture2D, Texture2D,
    TextureSpecification, TonemappingMode, VertexArray, VertexBuffer, WireframeMode,
};
pub use scene::{Aabb2D, Aabb3D, CullingStats, Frustum3D, SpatialGrid, SpatialGrid3D};
pub use scene::{
    AmbientLightComponent, AnimationClip, AnimationControllerComponent, AnimationEvent,
    AnimationTransition, AudioCategory, AudioListenerComponent, AudioSourceComponent,
    BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent, CircleRendererComponent,
    DirectionalLightComponent, Entity, FloatOrdering, FullscreenMode, IdComponent,
    InstancedSpriteAnimator, MeshPrimitive, MeshRendererComponent, MeshSource, NativeScript,
    NativeScriptComponent, ParticleEmitterComponent, PointLightComponent, PrefabInstanceComponent,
    RelationshipComponent,
    RigidBody2DComponent, RigidBody2DType, RigidBodyType, Scene, SceneSerializer,
    SkeletalAnimationComponent, SpriteAnimatorComponent, SpriteRendererComponent, TagComponent,
    TextComponent, TilemapComponent, TransformComponent, TransitionCondition, UIAnchorComponent,
    UIEvent, UIImageComponent, UIInteractableComponent, UIInteractionState, UILayoutAlignment,
    UILayoutComponent, UILayoutDirection, UIRectComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
};
#[cfg(feature = "physics-3d")]
pub use scene::{
    BoxCollider3DComponent, CapsuleCollider3DComponent, RigidBody3DComponent, RigidBody3DType,
    SphereCollider3DComponent,
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
    pub use crate::cursor::{CursorMode, SoftwareCursor};
    pub use crate::error::{EngineError, EngineResult};
    pub use crate::events::gamepad::{GamepadAxis, GamepadButton, GamepadEvent, GamepadId};
    pub use crate::events::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, WindowEvent};
    pub use crate::input::Input;
    pub use crate::input_action::{ActionType, InputAction, InputActionMap, InputBinding};
    pub use crate::layer::{Layer, LayerStack};
    pub use crate::orthographic_camera_controller::OrthographicCameraController;
    pub use crate::renderer::ParticleProps;
    pub use crate::platform_utils::{error_dialog, FileDialogs};
    pub use crate::profiling::{
        begin_session, drain_profile_results, end_session, is_session_active, ProfileResult,
        ProfileTimer,
    };
    pub use crate::project::Project;
    pub use crate::renderer::{
        as_bytes, load_gltf, load_gltf_skinned, BlendMode, BufferElement, BufferLayout, CullMode,
        DepthConfig, EditorCamera, Font, Framebuffer, FramebufferSpec, FramebufferTextureFormat,
        FramebufferTextureSpec, IndexBuffer, LightEnvironment, LightGpuData, Material,
        MaterialGpuData, MaterialHandle, MaterialLibrary, Mesh, MeshVertex, MsaaSamples,
        OrthographicCamera, Pipeline, PresentMode, ProjectionType, Renderer, Renderer2DStats,
        RendererBackend, SceneCamera, Shader, ShaderDataType, ShaderLibrary, SubTexture2D,
        Texture2D, TextureSpecification, TonemappingMode, VertexArray, VertexBuffer, WireframeMode,
        MAX_POINT_LIGHTS,
    };
    pub use crate::scene::{Aabb2D, Aabb3D, Frustum3D, SpatialGrid, SpatialGrid3D};
    pub use crate::scene::{
        AmbientLightComponent, AnimationClip, AnimationControllerComponent, AnimationEvent,
        AnimationTransition, AudioCategory, AudioListenerComponent, AudioSourceComponent,
        BoxCollider2DComponent, CameraComponent, CircleCollider2DComponent,
        CircleRendererComponent, DirectionalLightComponent, Entity, EnvironmentComponent,
        FloatOrdering, FullscreenMode, IdComponent, InstancedSpriteAnimator, MeshPrimitive,
        MeshRendererComponent, MeshSource, NativeScript, NativeScriptComponent,
        ParticleEmitterComponent, PointLightComponent, PrefabInstanceComponent,
        RelationshipComponent,
        RigidBody2DComponent, RigidBody2DType, RigidBodyType, Scene, SceneSerializer,
        SkeletalAnimationComponent, SpriteAnimatorComponent, SpriteRendererComponent,
        TagComponent, TextComponent, TilemapComponent, TransformComponent, TransitionCondition,
        UIAnchorComponent, UIEvent, UIImageComponent, UIInteractableComponent,
        UIInteractionState, UILayoutAlignment, UILayoutComponent,
        UILayoutDirection, UIRectComponent, TILE_FLIP_H, TILE_FLIP_V, TILE_ID_MASK,
    };
    #[cfg(feature = "physics-3d")]
    pub use crate::scene::{
        BoxCollider3DComponent, CapsuleCollider3DComponent, MeshCollider3DComponent,
        RigidBody3DComponent, RigidBody3DType, SphereCollider3DComponent,
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
