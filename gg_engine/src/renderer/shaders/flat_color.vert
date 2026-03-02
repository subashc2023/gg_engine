#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_view_projection;
} pc;

layout(location = 0) in vec3 a_position;

void main() {
    gl_Position = pc.u_view_projection * vec4(a_position, 1.0);
}
