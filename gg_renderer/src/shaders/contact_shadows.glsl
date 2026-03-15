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

layout(set = 0, binding = 0) uniform sampler2D u_depth;
layout(set = 1, binding = 0) uniform sampler2D u_normal;

layout(push_constant) uniform PushConstants {
    mat4 inv_view_projection;
    mat4 view_projection;
    vec4 light_direction;       // xyz = direction toward light
    float max_distance;         // world-space ray length
    float thickness;            // world-space occluder thickness
    float intensity;            // shadow strength [0,1]
    int step_count;             // ray march steps
    float near_plane;
    float far_plane;
    int debug_mode;             // 0=normal, 1=linear depth, 2=raw shadow, 3=precision
    float _pad1;
};

// Interleaved gradient noise (Jimenez 2014).
float ign(vec2 pos) {
    return fract(52.9829189 * fract(0.06711056 * pos.x + 0.00583715 * pos.y));
}

// Linearize reverse-Z depth: NDC 1 = near, NDC 0 = far.
float linearize(float d) {
    return near_plane * far_plane / (near_plane + d * (far_plane - near_plane));
}

vec3 reconstruct_world(vec2 uv, float depth) {
    vec2 ndc = uv * 2.0 - 1.0;
    vec4 hom = inv_view_projection * vec4(ndc, depth, 1.0);
    return hom.xyz / hom.w;
}

// Compute how many D32 float ULPs span the given thickness in NDC at this depth.
// With reverse-Z, the ULP at NDC value x is approximately |x| * 2^-23.
float precision_ulps(float ndc_depth, float linear_depth) {
    float gradient = near_plane * far_plane
                   / ((far_plane - near_plane) * linear_depth * linear_depth);
    float thickness_ndc = thickness * gradient;
    // ULP scales with NDC value for IEEE-754 float (more precision near 0).
    float ndc_ulp = max(abs(ndc_depth) * 1.192e-7, 1.4e-45);
    return thickness_ndc / ndc_ulp;
}

void main() {
    float pixel_depth_ndc = texture(u_depth, v_uv).r;

    // Sky — fully lit. Reverse-Z: sky/clear = 0.
    if (pixel_depth_ndc <= 0.0001) {
        out_color = vec4(1.0);
        return;
    }

    float pixel_lin = linearize(pixel_depth_ndc);

    // --- Debug modes (early out) ---
    if (debug_mode == 1) {
        // Linearized depth (grayscale, normalized to [0, far/5]).
        float viz = pixel_lin / (far_plane * 0.2);
        out_color = vec4(vec3(viz), 1.0);
        return;
    }
    if (debug_mode == 3) {
        // Precision: how many ULPs the thickness spans at this depth.
        // Green = plenty of precision, red = unreliable, black = impossible.
        float ulps = precision_ulps(pixel_depth_ndc, pixel_lin);
        float g = smoothstep(0.0, 30.0, ulps);   // green when > 30 ULPs
        float r = 1.0 - smoothstep(5.0, 20.0, ulps); // red when < 20 ULPs
        out_color = vec4(r, g, 0.0, 1.0);
        return;
    }

    // No normal = 2D sprite — fully lit.
    vec4 ns = texture(u_normal, v_uv);
    if (ns.a < 0.5) {
        out_color = vec4(1.0);
        return;
    }

    vec3 N = normalize(ns.xyz);
    vec3 L = normalize(light_direction.xyz);
    float NdotL = dot(N, L);

    // Attenuate contact shadows on surfaces facing away from light.
    float normal_fade = smoothstep(-0.1, 0.3, NdotL);
    if (normal_fade <= 0.0) {
        out_color = vec4(1.0);
        return;
    }

    vec3 world_pos = reconstruct_world(v_uv, pixel_depth_ndc);

    // Bias the starting position along the surface normal to prevent
    // self-intersection. Scale quadratically with depth to match VP
    // round-trip error growth with distance.
    float normal_bias = max(0.005, pixel_lin * pixel_lin * 3e-5 + pixel_lin * 0.001);
    vec3 biased_pos = world_pos + N * normal_bias;

    // Project ray endpoints into clip space for perspective-correct marching.
    vec4 start_clip = view_projection * vec4(biased_pos, 1.0);
    vec4 end_clip   = view_projection * vec4(biased_pos + L * max_distance, 1.0);

    if (start_clip.w <= 0.0 || end_clip.w <= 0.0) {
        out_color = vec4(1.0);
        return;
    }

    // Per-pixel jitter to break up stairstepping.
    float jitter = ign(gl_FragCoord.xy);

    vec4 step_clip = (end_clip - start_clip) / float(step_count);
    vec4 ray_clip = start_clip + step_clip * (1.0 + jitter);

    // NDC-space comparison: avoids linearization amplification entirely.
    // With reverse-Z, closer objects have HIGHER NDC values.
    // When the ray goes behind a surface (occlusion), buf_ndc > ray_ndc.z.
    float occlusion = 0.0;

    for (int i = 0; i < step_count; i++) {
        vec3 ray_ndc = ray_clip.xyz / ray_clip.w;
        vec2 ray_uv = ray_ndc.xy * 0.5 + 0.5;

        if (ray_uv.x < 0.0 || ray_uv.x > 1.0 || ray_uv.y < 0.0 || ray_uv.y > 1.0) break;

        float buf_ndc = texture(u_depth, ray_uv).r;
        if (buf_ndc > 0.0001) { // not sky
            // Reverse-Z: surface closer to camera = higher NDC.
            // Positive diff means ray went behind the surface.
            float diff_ndc = buf_ndc - ray_ndc.z;

            // Convert world-space thickness to NDC at this depth.
            float ray_lin = linearize(ray_ndc.z);
            float gradient = near_plane * far_plane
                           / ((far_plane - near_plane) * ray_lin * ray_lin);
            float thickness_ndc = thickness * gradient;

            // ULP-scaled epsilon: 16 ULPs at any depth.
            // With reverse-Z, ULPs are proportional to NDC value (tiny near far plane).
            float eps = max(abs(ray_ndc.z) * 1.192e-7 * 16.0, 1e-10);

            if (diff_ndc > eps && diff_ndc < thickness_ndc) {
                float t = float(i + 1) / float(step_count);
                occlusion = max(occlusion, 1.0 - t);
            }
        }

        ray_clip += step_clip;
    }

    // Debug mode 2: raw ray march result (no fades applied).
    if (debug_mode == 2) {
        float raw = 1.0 - occlusion * intensity;
        out_color = vec4(vec3(raw), 1.0);
        return;
    }

    // Precision fade: based on NDC headroom between epsilon and thickness.
    float pixel_gradient = near_plane * far_plane
                         / ((far_plane - near_plane) * pixel_lin * pixel_lin);
    float pixel_thickness_ndc = thickness * pixel_gradient;
    float pixel_eps = max(abs(pixel_depth_ndc) * 1.192e-7 * 16.0, 1e-10);
    float headroom = (pixel_thickness_ndc - pixel_eps) / pixel_eps;
    float precision_fade = smoothstep(0.0, 3.0, headroom);

    occlusion *= normal_fade * precision_fade;

    float shadow = 1.0 - occlusion * intensity;
    shadow = max(shadow, 0.05);
    out_color = vec4(shadow, shadow, shadow, 1.0);
}
