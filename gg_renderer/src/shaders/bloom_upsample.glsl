#type vertex
#version 450

layout(location = 0) out vec2 v_uv;

void main() {
    vec2 pos = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);
    gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);
    v_uv = pos;
}

#type fragment
#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform sampler2D u_source;

layout(push_constant) uniform PushConstants {
    vec2 texel_size;    // 1.0 / source_resolution
    float filter_radius;
    float _pad;
};

void main() {
    // 3x3 tent filter (9-tap): weights sum to 1.0.
    //   1  2  1
    //   2  4  2  / 16
    //   1  2  1
    float x = texel_size.x * filter_radius;
    float y = texel_size.y * filter_radius;

    vec3 a = texture(u_source, v_uv + vec2(-x, -y)).rgb;
    vec3 b = texture(u_source, v_uv + vec2( 0, -y)).rgb * 2.0;
    vec3 c = texture(u_source, v_uv + vec2( x, -y)).rgb;

    vec3 d = texture(u_source, v_uv + vec2(-x, 0)).rgb * 2.0;
    vec3 e = texture(u_source, v_uv).rgb * 4.0;
    vec3 f = texture(u_source, v_uv + vec2( x, 0)).rgb * 2.0;

    vec3 g = texture(u_source, v_uv + vec2(-x,  y)).rgb;
    vec3 h = texture(u_source, v_uv + vec2( 0,  y)).rgb * 2.0;
    vec3 i = texture(u_source, v_uv + vec2( x,  y)).rgb;

    out_color = vec4((a + b + c + d + e + f + g + h + i) / 16.0, 1.0);
}
