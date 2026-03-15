#type compute
#version 450

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform sampler2D u_equirect;
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_face_out;

layout(push_constant) uniform PushConstants {
    int u_face;       // 0-5: +X, -X, +Y, -Y, +Z, -Z
    int u_face_size;  // Output face resolution (e.g. 1024)
} push;

// Cube face direction vectors (Vulkan convention: Y-down in clip space,
// but cubemap sampling uses standard right-handed coordinates with Y-up).
//
// Vulkan's cubemap coordinate system matches OpenGL: +X=right, +Y=up, +Z=toward viewer.
// The Y-flip in Vulkan's clip space does NOT affect cubemap sampling directions.

vec3 cube_dir(int face, vec2 uv) {
    // uv in [0,1]^2, remap to [-1, 1]
    vec2 st = uv * 2.0 - 1.0;
    // Vulkan cubemap: +Y is up, same as OpenGL convention for samplerCube.
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

// Convert a direction vector to equirectangular UV.
vec2 dir_to_equirect(vec3 dir) {
    // atan2(z, x) gives longitude, asin(y) gives latitude.
    float phi = atan(dir.z, dir.x);     // [-pi, pi]
    float theta = asin(clamp(dir.y, -1.0, 1.0)); // [-pi/2, pi/2]

    vec2 uv;
    uv.x = phi / (2.0 * 3.14159265359) + 0.5;   // [0, 1]
    uv.y = -theta / 3.14159265359 + 0.5;         // [0, 1] (flip Y: top=north pole)
    return uv;
}

void main() {
    ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
    if (pixel.x >= push.u_face_size || pixel.y >= push.u_face_size) return;

    vec2 uv = (vec2(pixel) + 0.5) / float(push.u_face_size);
    vec3 dir = cube_dir(push.u_face, uv);
    vec2 equirect_uv = dir_to_equirect(dir);

    vec3 color = texture(u_equirect, equirect_uv).rgb;
    imageStore(u_face_out, pixel, vec4(color, 1.0));
}
