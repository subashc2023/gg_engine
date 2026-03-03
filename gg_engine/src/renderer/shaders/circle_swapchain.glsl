#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
} camera;

layout(location = 0) in vec3 a_world_position;
layout(location = 1) in vec3 a_local_position;
layout(location = 2) in vec4 a_color;
layout(location = 3) in float a_thickness;
layout(location = 4) in float a_fade;
layout(location = 5) in int a_entity_id;

layout(location = 0) out vec3 v_local_position;
layout(location = 1) out vec4 v_color;
layout(location = 2) out float v_thickness;
layout(location = 3) out float v_fade;

void main() {
    v_local_position = a_local_position;
    v_color = a_color;
    v_thickness = a_thickness;
    v_fade = a_fade;
    gl_Position = camera.u_view_projection * vec4(a_world_position, 1.0);
}

#type fragment
#version 450

layout(location = 0) in vec3 v_local_position;
layout(location = 1) in vec4 v_color;
layout(location = 2) in float v_thickness;
layout(location = 3) in float v_fade;

layout(location = 0) out vec4 out_color;

void main() {
    float distance = 1.0 - length(v_local_position.xy);
    float circle = smoothstep(0.0, v_fade, distance);
    circle *= smoothstep(v_thickness + v_fade, v_thickness, distance);

    if (circle <= 0.0)
        discard;

    out_color = v_color;
    out_color.a *= circle;
}
