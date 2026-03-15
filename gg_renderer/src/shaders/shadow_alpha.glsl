#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_light_vp;
    mat4 u_model;
    float u_alpha_cutoff;
    int u_tex_index;
} push;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;

layout(location = 0) out vec2 v_uv;

void main() {
    gl_Position = push.u_light_vp * push.u_model * vec4(a_position, 1.0);
    // Shadow pancaking: clamp to the near plane so geometry behind the
    // light frustum doesn't waste depth buffer precision.
    gl_Position.z = max(gl_Position.z, 0.0);
    v_uv = a_uv;
}

#type fragment
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_light_vp;
    mat4 u_model;
    float u_alpha_cutoff;
    int u_tex_index;
} push;

// Bindless texture array — set 0 in this pipeline (only DS layout bound).
layout(set = 0, binding = 0) uniform sampler2D u_textures[4096];

layout(location = 0) in vec2 v_uv;

void main() {
    if (push.u_tex_index >= 0) {
        float alpha = texture(u_textures[push.u_tex_index], v_uv).a;
        if (alpha < push.u_alpha_cutoff) {
            discard;
        }
    }
    // Depth is written automatically.
}
