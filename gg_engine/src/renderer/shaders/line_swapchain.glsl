#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec4 a_color;
layout(location = 2) in int a_entity_id;

layout(location = 0) out vec4 v_color;

void main() {
    v_color = a_color;
    gl_Position = camera.u_view_projection * vec4(a_position, 1.0);
}

#type fragment
#version 450

layout(location = 0) in vec4 v_color;

layout(location = 0) out vec4 out_color;

void main() {
    out_color = v_color;
}
