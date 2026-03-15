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

layout(set = 0, binding = 0) uniform sampler2D u_scene;
layout(set = 1, binding = 0) uniform sampler2D u_bloom;
layout(set = 2, binding = 0) uniform sampler2D u_shadow;

layout(push_constant) uniform PushConstants {
    float bloom_intensity;
    float exposure;
    float contrast;
    float saturation;
    int tonemapping_mode;  // 0 = none, 1 = ACES, 2 = Reinhard
    int apply_shadow;      // 0 = no shadow, 1 = multiply, 2 = debug passthrough
    float _pad0;
    float _pad1;
};

// ACES filmic tone mapping (Krzysztof Narkowicz approximation).
vec3 ACES(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

void main() {
    vec3 scene = texture(u_scene, v_uv).rgb;
    vec3 bloom = texture(u_bloom, v_uv).rgb;

    // Contact shadow: 1 = multiply scene by shadow factor, 2 = debug passthrough.
    if (apply_shadow == 2) {
        // Debug: display shadow texture directly (bypasses all post-processing).
        out_color = vec4(texture(u_shadow, v_uv).rgb, 1.0);
        return;
    }
    if (apply_shadow == 1) {
        float shadow = texture(u_shadow, v_uv).r;
        scene *= shadow;
    }

    // Combine scene + bloom.
    vec3 color = scene + bloom * bloom_intensity;

    // Exposure (EV stops).
    color *= exp2(exposure);

    // Tone mapping.
    if (tonemapping_mode == 1) {
        color = ACES(color);
    } else if (tonemapping_mode == 2) {
        color = color / (color + 1.0);  // Reinhard
    }

    // Contrast (pivot at 0.5).
    color = mix(vec3(0.5), color, contrast);

    // Saturation.
    float lum = dot(color, vec3(0.2126, 0.7152, 0.0722));
    color = mix(vec3(lum), color, saturation);

    out_color = vec4(clamp(color, 0.0, 1.0), 1.0);
}
