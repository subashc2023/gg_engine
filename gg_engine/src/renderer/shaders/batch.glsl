#type vertex
#version 450

layout(push_constant) uniform PushConstants {
    mat4 u_view_projection;
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec4 a_color;
layout(location = 2) in vec2 a_tex_coord;
layout(location = 3) in float a_tex_index;
layout(location = 4) in int a_entity_id;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_tex_coord;
layout(location = 2) out flat float v_tex_index;
layout(location = 3) out flat int v_entity_id;

void main() {
    v_color = a_color;
    v_tex_coord = a_tex_coord;
    v_tex_index = a_tex_index;
    v_entity_id = a_entity_id;
    gl_Position = pc.u_view_projection * vec4(a_position, 1.0);
}

#type fragment
#version 450
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 0, binding = 0) uniform sampler2D u_textures[];

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_tex_coord;
layout(location = 2) in flat float v_tex_index;
layout(location = 3) in flat int v_entity_id;

layout(location = 0) out vec4 out_color;
layout(location = 1) out int out_entity_id;

void main() {
    int index = int(v_tex_index);
    out_color = texture(u_textures[nonuniformEXT(index)], v_tex_coord) * v_color;
    out_entity_id = v_entity_id;
}
