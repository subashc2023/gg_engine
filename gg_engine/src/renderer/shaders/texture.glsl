#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_view_projection;
    mat4 u_transform;
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_tex_coord;

layout(location = 0) out vec2 v_tex_coord;

void main() {
    v_tex_coord = a_tex_coord;
    gl_Position = pc.u_view_projection * pc.u_transform * vec4(a_position, 1.0);
}

#type fragment
#version 450

layout(push_constant) uniform PushConstants {
    layout(offset = 128) vec4 u_color;
};

layout(set = 0, binding = 0) uniform sampler2D u_texture;

layout(location = 0) in vec2 v_tex_coord;
layout(location = 0) out vec4 out_color;

void main() {
    out_color = texture(u_texture, v_tex_coord) * u_color;
}
