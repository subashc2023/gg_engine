#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
    float u_time;
} camera;

layout(push_constant) uniform PushConstants {
    mat4 u_model;
    int u_entity_id;
} push;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out vec3 v_world_position;
#ifdef OFFSCREEN
layout(location = 4) out flat int v_entity_id;
#endif

void main() {
    vec4 world_pos = push.u_model * vec4(a_position, 1.0);
    v_world_position = world_pos.xyz;
    v_normal = mat3(push.u_model) * a_normal;
    v_uv = a_uv;
    v_color = a_color;
#ifdef OFFSCREEN
    v_entity_id = push.u_entity_id;
#endif
    gl_Position = camera.u_view_projection * world_pos;
}

#type fragment
#version 450

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec2 v_uv;
layout(location = 3) in vec3 v_world_position;
#ifdef OFFSCREEN
layout(location = 4) in flat int v_entity_id;
#endif

layout(location = 0) out vec4 out_color;
#ifdef OFFSCREEN
layout(location = 1) out int out_entity_id;
#endif

void main() {
    // Basic hemisphere lighting for visual depth.
    vec3 n = normalize(v_normal);
    float ndotl = dot(n, normalize(vec3(0.3, 1.0, 0.5))) * 0.5 + 0.5;
    out_color = vec4(v_color.rgb * ndotl, v_color.a);
#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
#endif
}
