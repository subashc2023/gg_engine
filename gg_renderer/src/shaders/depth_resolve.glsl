#type vertex
#version 450

void main() {
    vec2 pos = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);
    gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);
}

#type fragment
#version 450

layout(location = 0) out vec4 out_depth;

layout(set = 0, binding = 0) uniform sampler2DMS u_depth_ms;

// Resolve MSAA depth by taking the maximum (closest) sample.
// Reverse-Z: closer = higher NDC value, so max = closest.
void main() {
    ivec2 coord = ivec2(gl_FragCoord.xy);
    int num_samples = textureSamples(u_depth_ms);
    float max_depth = 0.0;
    for (int i = 0; i < num_samples; i++) {
        max_depth = max(max_depth, texelFetch(u_depth_ms, coord, i).r);
    }
    out_depth = vec4(max_depth, 0.0, 0.0, 1.0);
}
