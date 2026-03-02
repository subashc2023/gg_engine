#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_view_projection;
    mat4 u_transform;
} pc;

layout(location = 0) in vec3 a_position;

void main() {
    gl_Position = pc.u_view_projection * pc.u_transform * vec4(a_position, 1.0);
}

#type fragment
#version 450

layout(push_constant) uniform PushConstants {
    layout(offset = 128) vec4 u_color;
};

layout(location = 0) out vec4 out_color;

void main() {
    out_color = u_color;
}
