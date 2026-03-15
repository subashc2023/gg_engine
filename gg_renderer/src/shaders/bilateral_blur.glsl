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
//
// Separable bilateral blur for contact shadow factor.
//
// 9-tap depth-aware Gaussian: preserves edges at depth discontinuities
// while smoothing out ray march stairstepping on continuous surfaces.
// Run twice (H then V) for full 2D blur.
//

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

layout(set = 0, binding = 0) uniform sampler2D u_shadow;
layout(set = 1, binding = 0) uniform sampler2D u_depth;

layout(push_constant) uniform PushConstants {
    vec2 texel_size;    // 1.0 / resolution
    vec2 direction;     // (1,0) for horizontal, (0,1) for vertical
    float near_plane;
    float far_plane;
    float _pad0;
    float _pad1;
};

// Linearize reverse-Z depth: NDC 1 = near, NDC 0 = far.
float linearize(float d) {
    float denom = near_plane + d * (far_plane - near_plane);
    return near_plane * far_plane / max(denom, 0.0001);
}

void main() {
    float center_shadow = texture(u_shadow, v_uv).r;
    float center_depth  = texture(u_depth, v_uv).r;

    // Sky — pass through. Reverse-Z: sky/clear = 0.
    if (center_depth <= 0.0001) {
        out_color = vec4(center_shadow);
        return;
    }

    float center_lin = linearize(center_depth);

    // Gaussian weights for 13-tap kernel (sigma ≈ 4.0), step size 2px.
    // Effective radius = 12px per axis (24px diameter), enough to smooth
    // ray march stairstepping into soft penumbra.
    const int KERNEL_RADIUS = 6;
    const float STEP = 2.0;
    const float weights[7] = float[](0.1592, 0.1504, 0.1268, 0.0955, 0.0643, 0.0387, 0.0208);

    float total_weight = weights[0];
    float total_shadow = center_shadow * weights[0];

    // Depth threshold: 2% of center distance. Samples across depth
    // discontinuities (edges) are rejected to preserve sharp shadow boundaries.
    float depth_threshold = center_lin * 0.02;

    for (int i = 1; i <= KERNEL_RADIUS; i++) {
        float w = weights[i];
        vec2 offset = direction * texel_size * float(i) * STEP;

        for (int s = -1; s <= 1; s += 2) {
            vec2 sample_uv = v_uv + offset * float(s);

            float sample_shadow = texture(u_shadow, sample_uv).r;
            float sample_depth  = texture(u_depth, sample_uv).r;
            float sample_lin    = linearize(sample_depth);

            // Edge-stopping weight: smooth falloff at depth discontinuities.
            float depth_diff = abs(center_lin - sample_lin);
            float edge_weight = 1.0 - smoothstep(0.0, depth_threshold, depth_diff);

            float final_weight = w * edge_weight;
            total_shadow += sample_shadow * final_weight;
            total_weight += final_weight;
        }
    }

    out_color = vec4(total_shadow / total_weight);
}
