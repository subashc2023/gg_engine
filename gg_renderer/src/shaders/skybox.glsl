#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_vp_rotation;  // projection * mat4(mat3(view)) — rotation only, no translation
    float u_exposure;
    float u_rotation_y;  // radians
} push;

layout(location = 0) in vec3 a_position;
layout(location = 0) out vec3 v_world_dir;

void main() {
    v_world_dir = a_position;
    vec4 clip_pos = push.u_vp_rotation * vec4(a_position, 1.0);
    // Force depth to 0.0 for reverse-Z far plane (so skybox is behind everything).
    gl_Position = vec4(clip_pos.xy, 0.0, clip_pos.w);
}

#type fragment
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_vp_rotation;
    float u_exposure;
    float u_rotation_y;
} push;

// Environment cubemap is at set 1 (lighting descriptor set), binding 4.
layout(set = 1, binding = 4) uniform samplerCube u_environment_map;

layout(location = 0) in vec3 v_world_dir;

layout(location = 0) out vec4 out_color;
#ifdef OFFSCREEN
layout(location = 1) out int out_entity_id;
layout(location = 2) out vec4 out_normal;
#endif

void main() {
    // Apply Y-axis rotation to the sampling direction.
    float c = cos(push.u_rotation_y);
    float s = sin(push.u_rotation_y);
    vec3 dir = normalize(v_world_dir);
    vec3 rotated_dir = vec3(
        c * dir.x + s * dir.z,
        dir.y,
        -s * dir.x + c * dir.z
    );

    vec3 color = texture(u_environment_map, rotated_dir).rgb * push.u_exposure;

#ifdef OFFSCREEN
    // OFFSCREEN: output linear HDR — post-processing pipeline handles tonemapping.
    out_color = vec4(color, 1.0);
    out_entity_id = -1;
    out_normal = vec4(0.0, 0.0, 0.0, 0.0);
#else
    // Direct-to-swapchain: ACES tonemapping.
    const float a = 2.51;
    const float b = 0.03;
    const float c2 = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    vec3 mapped = clamp((color * (a * color + b)) / (color * (c2 * color + d) + e), 0.0, 1.0);
    out_color = vec4(mapped, 1.0);
#endif
}
