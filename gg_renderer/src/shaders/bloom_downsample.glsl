#type vertex
#version 450

layout(location = 0) out vec2 v_uv;

void main() {
    // Fullscreen triangle: 3 vertices cover the entire screen.
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
    vec2 texel_size;  // 1.0 / source_resolution
    float threshold;
    int first_pass;   // 1 = extract bright pixels, 0 = plain downsample
};

void main() {
    // 4-tap bilinear downsample: sample at half-texel offsets to leverage
    // hardware bilinear filtering for an effective 4x4 box filter.
    vec2 hs = texel_size * 0.5;
    vec3 a = texture(u_source, v_uv + vec2(-hs.x, -hs.y)).rgb;
    vec3 b = texture(u_source, v_uv + vec2( hs.x, -hs.y)).rgb;
    vec3 c = texture(u_source, v_uv + vec2(-hs.x,  hs.y)).rgb;
    vec3 d = texture(u_source, v_uv + vec2( hs.x,  hs.y)).rgb;

    vec3 color;
    if (first_pass != 0) {
        // Karis average (Jimenez 2014): weight each sample by 1/(1+luma)
        // so sub-pixel specular spikes can't dominate the downsample and
        // cause bloom to flicker as highlights shift between texels.
        float wa = 1.0 / (1.0 + max(a.r, max(a.g, a.b)));
        float wb = 1.0 / (1.0 + max(b.r, max(b.g, b.b)));
        float wc = 1.0 / (1.0 + max(c.r, max(c.g, c.b)));
        float wd = 1.0 / (1.0 + max(d.r, max(d.g, d.b)));
        color = (a * wa + b * wb + c * wc + d * wd) / (wa + wb + wc + wd);

        // Brightness threshold: only pass through values above the bloom threshold.
        float brightness = max(color.r, max(color.g, color.b));
        float contribution = max(0.0, brightness - threshold) / max(brightness, 0.001);
        color *= contribution;
    } else {
        color = (a + b + c + d) * 0.25;
    }

    out_color = vec4(color, 1.0);
}
