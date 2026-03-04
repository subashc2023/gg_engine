#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec4 a_color;
layout(location = 2) in vec2 a_tex_coord;
layout(location = 3) in float a_tex_index;
layout(location = 4) in int a_entity_id;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_tex_coord;
layout(location = 2) out flat float v_tex_index;

void main() {
    v_color = a_color;
    v_tex_coord = a_tex_coord;
    v_tex_index = a_tex_index;
    gl_Position = camera.u_view_projection * vec4(a_position, 1.0);
}

#type fragment
#version 450
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 1, binding = 0) uniform sampler2D u_textures[];

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_tex_coord;
layout(location = 2) in flat float v_tex_index;

layout(location = 0) out vec4 out_color;

float median(float r, float g, float b) {
    return max(min(r, g), min(max(r, g), b));
}

void main() {
    int index = int(v_tex_index);
    vec3 msdf = texture(u_textures[nonuniformEXT(index)], v_tex_coord).rgb;
    float dist = median(msdf.r, msdf.g, msdf.b);

    // MSDF rendering: 0.5 = edge, >0.5 = inside, <0.5 = outside.
    float smoothing = fwidth(dist) * 0.5;
    float opacity = smoothstep(0.5 - smoothing, 0.5 + smoothing, dist);

    if (opacity <= 0.01)
        discard;

    out_color = vec4(v_color.rgb, v_color.a * opacity);
}
