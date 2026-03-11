#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_light_vp;
    mat4 u_model;
} push;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;

void main() {
    gl_Position = push.u_light_vp * push.u_model * vec4(a_position, 1.0);
    // Shadow pancaking: clamp to the near plane so geometry behind the
    // light frustum doesn't waste depth buffer precision.
    gl_Position.z = max(gl_Position.z, 0.0);
}

#type fragment
#version 450

void main() {
    // Depth is written automatically. No color output needed.
}
