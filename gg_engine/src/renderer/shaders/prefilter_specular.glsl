#type compute
#version 450

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform samplerCube u_environment;
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_face_out;

layout(push_constant) uniform PushConstants {
    int u_face;         // 0-5
    int u_face_size;    // Output face resolution at this mip level
    float u_roughness;  // 0.0 = mirror, 1.0 = fully rough
    int u_sample_count; // Number of importance samples (e.g. 1024)
} push;

const float PI = 3.14159265359;

// Clamp extreme HDR radiance to suppress firefly artifacts in the prefiltered
// result.  The sun disk in typical HDR environments can exceed 60,000 nits,
// which creates visible bright-texel patterns at low–mid roughness because a
// few importance samples land on the sun while their neighbours don't.
// Clamping to a sane maximum still preserves ≈99% of the usable dynamic range
// while eliminating the discontinuity.
const float MAX_RADIANCE = 64.0;

// ---------------------------------------------------------------------------
// GGX (Trowbridge-Reitz) importance sampling
// ---------------------------------------------------------------------------

// Van der Corput radical inverse (base 2).
float radical_inverse_vdc(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10; // 1 / 0x100000000
}

vec2 hammersley(uint i, uint N) {
    return vec2(float(i) / float(N), radical_inverse_vdc(i));
}

// GGX importance sampling: returns a half-vector in tangent space.
vec3 importance_sample_ggx(vec2 xi, float roughness) {
    float a = roughness * roughness;
    float phi = 2.0 * PI * xi.x;
    float cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    float sin_theta = sqrt(1.0 - cos_theta * cos_theta);

    return vec3(
        cos(phi) * sin_theta,
        sin(phi) * sin_theta,
        cos_theta
    );
}

vec3 cube_dir(int face, vec2 uv) {
    vec2 st = uv * 2.0 - 1.0;
    switch (face) {
        case 0: return normalize(vec3( 1.0, -st.y, -st.x)); // +X
        case 1: return normalize(vec3(-1.0, -st.y,  st.x)); // -X
        case 2: return normalize(vec3( st.x,  1.0,  st.y)); // +Y
        case 3: return normalize(vec3( st.x, -1.0, -st.y)); // -Y
        case 4: return normalize(vec3( st.x, -st.y,  1.0)); // +Z
        case 5: return normalize(vec3(-st.x, -st.y, -1.0)); // -Z
    }
    return vec3(0.0);
}

void main() {
    ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
    if (pixel.x >= push.u_face_size || pixel.y >= push.u_face_size) return;

    vec2 uv = (vec2(pixel) + 0.5) / float(push.u_face_size);
    vec3 N = cube_dir(push.u_face, uv);
    // Assume view direction = normal (isotropic BRDF approximation, standard for IBL prefilter).
    vec3 R = N;
    vec3 V = R;

    // Build tangent frame.
    vec3 up = abs(N.y) < 0.999 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 T = normalize(cross(up, N));
    vec3 B = cross(N, T);

    // Per-texel hash rotation: decorrelates the Hammersley sampling pattern
    // between adjacent texels, breaking up the coherent face-aligned structure
    // that otherwise creates visible concentric rings on smooth reflective
    // surfaces.  The hash is deterministic so the prefilter is still stable.
    float texel_hash = fract(sin(dot(vec2(pixel) + 0.1 * float(push.u_face),
                                     vec2(12.9898, 78.233))) * 43758.5453);

    vec3 prefiltered_color = vec3(0.0);
    float total_weight = 0.0;

    float roughness = max(push.u_roughness, 0.001); // Avoid division by zero at roughness=0

    uint sample_count = uint(push.u_sample_count);
    for (uint i = 0u; i < sample_count; i++) {
        vec2 xi = hammersley(i, sample_count);
        // Rotate the azimuthal angle by the per-texel hash to break pattern coherence.
        xi.x = fract(xi.x + texel_hash);
        vec3 H_tangent = importance_sample_ggx(xi, roughness);

        // Tangent to world.
        vec3 H = H_tangent.x * T + H_tangent.y * B + H_tangent.z * N;
        vec3 L = normalize(2.0 * dot(V, H) * H - V);

        float NdotL = max(dot(N, L), 0.0);
        if (NdotL > 0.0) {
            // Mip-level bias: compute the GGX PDF solid angle per sample, then
            // select a mip level that covers at least that solid angle.  This
            // forces off-peak samples to read from blurrier mips, suppressing
            // firefly artifacts from the concentrated HDR sun.
            float NdotH = max(dot(N, H), 0.0);
            float HdotV = max(dot(H, V), 0.0);

            // GGX NDF — must match the distribution used for importance sampling.
            // alpha = roughness², alpha² = roughness⁴ (Disney parameterization).
            float a  = roughness * roughness;
            float a2 = a * a;
            float NdotH2 = NdotH * NdotH;
            // Numerically stable form: NdotH²·a² + sin²θ avoids catastrophic
            // cancellation that the equivalent (NdotH²·(a²-1)+1) suffers when
            // a² << 1 (low roughness).  Without this, float32 computes
            // denom = 0 for roughness < ~0.24, zeroing the NDF and inflating
            // the mip-level bias so the prefilter reads from the blurriest
            // source mip instead of the sharpest.
            float denom = NdotH2 * a2 + (1.0 - NdotH2);
            float D = a2 / (PI * denom * denom);

            float pdf = D * NdotH / (4.0 * HdotV + 0.0001);

            float resolution = float(textureSize(u_environment, 0).x);
            float sa_texel = 4.0 * PI / (6.0 * resolution * resolution);
            float sa_sample = 1.0 / (float(sample_count) * pdf + 0.0001);
            float mip_level = 0.5 * log2(sa_sample / sa_texel) + 1.0;

            vec3 sample_color = min(textureLod(u_environment, L, mip_level).rgb, vec3(MAX_RADIANCE));
            prefiltered_color += sample_color * NdotL;
            total_weight += NdotL;
        }
    }

    prefiltered_color /= max(total_weight, 0.001);
    imageStore(u_face_out, pixel, vec4(prefiltered_color, 1.0));
}
