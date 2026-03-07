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
#ifdef OFFSCREEN
layout(location = 3) out flat int v_entity_id;
#endif

void main() {
    v_color = a_color;
    v_tex_coord = a_tex_coord;
    v_tex_index = a_tex_index;
#ifdef OFFSCREEN
    v_entity_id = a_entity_id;
#endif
    gl_Position = camera.u_view_projection * vec4(a_position, 1.0);
}

#type fragment
#version 450
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 1, binding = 0) uniform sampler2D u_textures[];

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_tex_coord;
layout(location = 2) in flat float v_tex_index;
#ifdef OFFSCREEN
layout(location = 3) in flat int v_entity_id;
#endif

layout(location = 0) out vec4 out_color;
#ifdef OFFSCREEN
layout(location = 1) out int out_entity_id;
#endif

void main() {
    int index = clamp(int(v_tex_index), 0, 4095);
    vec4 tex_color = texture(u_textures[nonuniformEXT(index)], v_tex_coord) * v_color;
    if (tex_color.a < 0.01)
        discard;
    out_color = tex_color;
#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
#endif
}
