#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

layout(push_constant) uniform PushConstants {
    mat4 u_transform;
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_tex_coord;

layout(location = 0) out vec2 v_tex_coord;

void main() {
    v_tex_coord = a_tex_coord;
    gl_Position = camera.u_view_projection * pc.u_transform * vec4(a_position, 1.0);
}

#type fragment
#version 450

layout(push_constant) uniform PushConstants {
    layout(offset = 64) vec4 u_color;
    layout(offset = 80) float u_tiling_factor;
};

layout(set = 1, binding = 0) uniform sampler2D u_texture;

layout(location = 0) in vec2 v_tex_coord;
layout(location = 0) out vec4 out_color;

void main() {
    out_color = texture(u_texture, v_tex_coord * u_tiling_factor) * u_color;
}
