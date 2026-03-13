#type vertex
#version 450

// Bone matrix palette (set 0 in shadow pipeline — only DS layout bound).
layout(set = 0, binding = 0) readonly buffer BoneMatrices {
    mat4 bones[];
} bone_data;

layout(push_constant) uniform PushConstants {
    mat4 u_light_vp;
    mat4 u_model;
    uint u_bone_offset;
} push;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;
layout(location = 4) in vec4 a_tangent;
layout(location = 5) in ivec4 a_bone_indices;
layout(location = 6) in vec4 a_bone_weights;

void main() {
    // Skeletal skinning.
    uint off = push.u_bone_offset;
    mat4 skin_matrix =
        a_bone_weights.x * bone_data.bones[off + uint(a_bone_indices.x)] +
        a_bone_weights.y * bone_data.bones[off + uint(a_bone_indices.y)] +
        a_bone_weights.z * bone_data.bones[off + uint(a_bone_indices.z)] +
        a_bone_weights.w * bone_data.bones[off + uint(a_bone_indices.w)];

    vec4 skinned_pos = skin_matrix * vec4(a_position, 1.0);

    gl_Position = push.u_light_vp * push.u_model * skinned_pos;
    // Shadow pancaking.
    gl_Position.z = max(gl_Position.z, 0.0);
}

#type fragment
#version 450

void main() {
    // Depth is written automatically.
}
