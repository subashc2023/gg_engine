#type compute
#version 450

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(set = 0, binding = 0, rg16f) writeonly uniform image2D u_brdf_lut;

layout(push_constant) uniform PushConstants {
    int u_size; // Output texture size (e.g. 512)
} push;

const float PI = 3.14159265359;

// ---------------------------------------------------------------------------
// Split-sum BRDF integration (Epic Games approach)
// ---------------------------------------------------------------------------

float radical_inverse_vdc(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

vec2 hammersley(uint i, uint N) {
    return vec2(float(i) / float(N), radical_inverse_vdc(i));
}

vec3 importance_sample_ggx(vec2 xi, vec3 N, float roughness) {
    float a = roughness * roughness;
    float phi = 2.0 * PI * xi.x;
    float cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    float sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    // From tangent-space half-vector to world-space.
    vec3 H = vec3(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);

    // Build tangent frame (N is always (0,0,1) for the integration).
    vec3 up = abs(N.z) < 0.999 ? vec3(0.0, 0.0, 1.0) : vec3(1.0, 0.0, 0.0);
    vec3 T = normalize(cross(up, N));
    vec3 B = cross(N, T);

    return normalize(T * H.x + B * H.y + N * H.z);
}

// Smith's geometry function (GGX, Schlick approximation).
float geometry_schlick_ggx(float NdotV, float roughness) {
    float a = roughness;
    float k = (a * a) / 2.0;
    return NdotV / (NdotV * (1.0 - k) + k);
}

float geometry_smith(vec3 N, vec3 V, vec3 L, float roughness) {
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    return geometry_schlick_ggx(NdotV, roughness) * geometry_schlick_ggx(NdotL, roughness);
}

// Integrate the split-sum BRDF: returns (F0_scale, F0_bias).
vec2 integrate_brdf(float NdotV, float roughness) {
    vec3 V;
    V.x = sqrt(1.0 - NdotV * NdotV);
    V.y = 0.0;
    V.z = NdotV;

    float A = 0.0; // F0 scale
    float B = 0.0; // F0 bias

    vec3 N = vec3(0.0, 0.0, 1.0);

    const uint SAMPLE_COUNT = 1024u;
    for (uint i = 0u; i < SAMPLE_COUNT; i++) {
        vec2 xi = hammersley(i, SAMPLE_COUNT);
        vec3 H = importance_sample_ggx(xi, N, roughness);
        vec3 L = normalize(2.0 * dot(V, H) * H - V);

        float NdotL = max(L.z, 0.0);
        float NdotH = max(H.z, 0.0);
        float VdotH = max(dot(V, H), 0.0);

        if (NdotL > 0.0) {
            float G = geometry_smith(N, V, L, roughness);
            float G_vis = (G * VdotH) / max(NdotH * NdotV, 0.001);
            float Fc = pow(1.0 - VdotH, 5.0);

            A += (1.0 - Fc) * G_vis;
            B += Fc * G_vis;
        }
    }

    return vec2(A, B) / float(SAMPLE_COUNT);
}

void main() {
    ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
    if (pixel.x >= push.u_size || pixel.y >= push.u_size) return;

    // X = NdotV, Y = roughness (both in [0, 1]).
    float NdotV = (float(pixel.x) + 0.5) / float(push.u_size);
    float roughness = (float(pixel.y) + 0.5) / float(push.u_size);

    // Avoid degenerate cases.
    NdotV = max(NdotV, 0.001);
    roughness = max(roughness, 0.001);

    vec2 brdf = integrate_brdf(NdotV, roughness);
    imageStore(u_brdf_lut, pixel, vec4(brdf, 0.0, 0.0));
}
