#type compute
#version 450

layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform samplerCube u_environment;
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_face_out;

layout(push_constant) uniform PushConstants {
    int u_face;       // 0-5: +X, -X, +Y, -Y, +Z, -Z
    int u_face_size;  // Output face resolution (e.g. 32)
} push;

const float PI = 3.14159265359;

// Clamp extreme HDR values before hemisphere integration.
// Without this, a few samples that hit the sun (65504 nits) dominate the
// integral, creating visible per-texel brightness steps in the 32x32 cubemap.
const float MAX_RADIANCE = 64.0;

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
    vec3 normal = cube_dir(push.u_face, uv);

    // Build tangent frame from the normal.
    vec3 up = abs(normal.y) < 0.999 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 right = normalize(cross(up, normal));
    up = cross(normal, right);

    // Hemisphere convolution: integrate incoming radiance weighted by cos(theta).
    // Uniform sampling with fixed sample count for deterministic, artifact-free results.
    vec3 irradiance = vec3(0.0);
    float total_weight = 0.0;

    const float SAMPLE_DELTA = 0.025; // ~2500 samples per texel
    for (float phi = 0.0; phi < 2.0 * PI; phi += SAMPLE_DELTA) {
        for (float theta = 0.0; theta < 0.5 * PI; theta += SAMPLE_DELTA) {
            // Spherical to cartesian (tangent space).
            float sin_theta = sin(theta);
            float cos_theta = cos(theta);
            vec3 tangent_sample = vec3(
                sin_theta * cos(phi),
                sin_theta * sin(phi),
                cos_theta
            );

            // Tangent space to world space.
            vec3 sample_dir = tangent_sample.x * right
                            + tangent_sample.y * up
                            + tangent_sample.z * normal;

            vec3 sample_color = min(texture(u_environment, sample_dir).rgb, vec3(MAX_RADIANCE));
            irradiance += sample_color * cos_theta * sin_theta;
            total_weight += 1.0;
        }
    }

    irradiance = PI * irradiance / total_weight;
    imageStore(u_face_out, pixel, vec4(irradiance, 1.0));
}
