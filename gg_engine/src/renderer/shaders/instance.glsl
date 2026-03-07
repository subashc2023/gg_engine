// TODO(perf): GPU-computed sprite animation
// To eliminate ALL CPU animation cost for mass entities, add per-instance
// animation parameters and compute UVs in this shader:
//
//   // Additional per-instance inputs:
//   in float a_anim_start_time;
//   in float a_anim_fps;
//   in uint  a_anim_start_frame;
//   in uint  a_anim_frame_count;
//   in uint  a_anim_columns;
//   in vec2  a_anim_cell_size;
//   in vec2  a_anim_tex_size;
//
//   // Add u_time to the camera UBO:
//   layout(set = 0, binding = 0) uniform CameraBuffer {
//       mat4 u_view_projection;
//       float u_time;
//   };
//
//   // In main():
//   if (a_anim_frame_count > 0u) {
//       uint frame_in_clip = uint(floor((u_time - a_anim_start_time) * a_anim_fps))
//                            % a_anim_frame_count;
//       uint frame = a_anim_start_frame + frame_in_clip;
//       uint col = frame % a_anim_columns;
//       uint row = frame / a_anim_columns;
//       vec2 uv_min = vec2(col, row) * a_anim_cell_size / a_anim_tex_size;
//       vec2 uv_max = uv_min + a_anim_cell_size / a_anim_tex_size;
//       v_tex_coord = uv_min + a_tex_coord * (uv_max - uv_min);
//   }
//
// This makes the GPU compute frame selection — zero CPU animation work.
// Only implement if CPU stateless math (InstancedSpriteAnimator) becomes
// a bottleneck at 10K+ animated entities.

#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

// Per-vertex data (binding 0, rate = VERTEX)
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_tex_coord;

// Per-instance data (binding 1, rate = INSTANCE)
layout(location = 2) in vec4 a_transform_col0;
layout(location = 3) in vec4 a_transform_col1;
layout(location = 4) in vec4 a_transform_col2;
layout(location = 5) in vec4 a_transform_col3;
layout(location = 6) in vec4 a_color;
layout(location = 7) in vec2 a_uv_min;
layout(location = 8) in vec2 a_uv_max;
layout(location = 9) in float a_tex_index;
layout(location = 10) in float a_tiling_factor;
layout(location = 11) in int a_entity_id;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_tex_coord;
layout(location = 2) out flat float v_tex_index;
layout(location = 3) out flat int v_entity_id;

void main() {
    mat4 model = mat4(a_transform_col0, a_transform_col1, a_transform_col2, a_transform_col3);
    v_color = a_color;
    v_tex_coord = (a_uv_min + a_tex_coord * (a_uv_max - a_uv_min)) * a_tiling_factor;
    v_tex_index = a_tex_index;
    v_entity_id = a_entity_id;
    gl_Position = camera.u_view_projection * model * vec4(a_position, 1.0);
}

#type fragment
#version 450
#extension GL_EXT_nonuniform_qualifier : require

layout(set = 1, binding = 0) uniform sampler2D u_textures[];

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_tex_coord;
layout(location = 2) in flat float v_tex_index;
layout(location = 3) in flat int v_entity_id;

layout(location = 0) out vec4 out_color;
layout(location = 1) out int out_entity_id;

void main() {
    int index = clamp(int(v_tex_index), 0, 4095);
    vec4 tex_color = texture(u_textures[nonuniformEXT(index)], v_tex_coord) * v_color;
    if (tex_color.a < 0.01)
        discard;
    out_color = tex_color;
    out_entity_id = v_entity_id;
}
