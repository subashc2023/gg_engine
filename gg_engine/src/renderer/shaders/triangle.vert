#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_view_projection;
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec4 a_color;

layout(location = 0) out vec4 v_color;

void main() {
    v_color = a_color;
    gl_Position = pc.u_view_projection * vec4(a_position, 1.0);
}
