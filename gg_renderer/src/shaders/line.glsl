#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec4 a_color;
layout(location = 2) in int a_entity_id;

layout(location = 0) out vec4 v_color;
#ifdef OFFSCREEN
layout(location = 1) out flat int v_entity_id;
#endif

void main() {
    v_color = a_color;
#ifdef OFFSCREEN
    v_entity_id = a_entity_id;
#endif
    gl_Position = camera.u_view_projection * vec4(a_position, 1.0);
}

#type fragment
#version 450

layout(location = 0) in vec4 v_color;
#ifdef OFFSCREEN
layout(location = 1) in flat int v_entity_id;
#endif

layout(location = 0) out vec4 out_color;
#ifdef OFFSCREEN
layout(location = 1) out int out_entity_id;
layout(location = 2) out vec4 out_normal;
#endif

void main() {
    out_color = v_color;
#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
    out_normal = vec4(0.0, 0.0, 1.0, 0.0);
#endif
}
