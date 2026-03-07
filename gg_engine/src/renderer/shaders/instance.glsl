#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
    float u_time;
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
// GPU animation parameters (frame_count > 0 activates GPU animation)
layout(location = 12) in float a_anim_start_time;
layout(location = 13) in float a_anim_fps;
layout(location = 14) in float a_anim_start_frame;
layout(location = 15) in float a_anim_frame_count;
layout(location = 16) in float a_anim_columns;
layout(location = 17) in float a_anim_looping;
layout(location = 18) in vec2 a_anim_cell_size;
layout(location = 19) in vec2 a_anim_tex_size;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_tex_coord;
layout(location = 2) out flat float v_tex_index;
layout(location = 3) out flat int v_entity_id;

void main() {
    mat4 model = mat4(a_transform_col0, a_transform_col1, a_transform_col2, a_transform_col3);
    v_color = a_color;
    v_tex_index = a_tex_index;
    v_entity_id = a_entity_id;

    // GPU-computed animation: when frame_count > 0, compute UV coords from
    // animation parameters and u_time instead of using a_uv_min / a_uv_max.
    if (a_anim_frame_count > 0.0) {
        float elapsed = max(camera.u_time - a_anim_start_time, 0.0);
        float raw_frame = floor(elapsed * a_anim_fps);
        float frame_in_clip;
        if (a_anim_looping > 0.5) {
            frame_in_clip = mod(raw_frame, a_anim_frame_count);
        } else {
            frame_in_clip = min(raw_frame, a_anim_frame_count - 1.0);
        }
        float frame = a_anim_start_frame + frame_in_clip;
        float col = mod(frame, a_anim_columns);
        float row = floor(frame / a_anim_columns);
        vec2 cell_uv = a_anim_cell_size / a_anim_tex_size;
        vec2 uv_min = vec2(col, row) * cell_uv;
        vec2 uv_max = uv_min + cell_uv;
        v_tex_coord = uv_min + a_tex_coord * (uv_max - uv_min);
    } else {
        v_tex_coord = (a_uv_min + a_tex_coord * (a_uv_max - a_uv_min)) * a_tiling_factor;
    }

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
