#type compute
#version 450

layout(local_size_x = 256) in;

// ---------------------------------------------------------------------------
// Particle state (read-write) — must match GpuParticle in Rust (#[repr(C)])
// ---------------------------------------------------------------------------

struct GpuParticle {
    vec2 position;         // offset 0
    vec2 velocity;         // offset 8
    float rotation;        // offset 16
    float rotation_speed;  // offset 20
    float size_begin;      // offset 24
    float size_end;        // offset 28
    vec4 color_begin;      // offset 32
    vec4 color_end;        // offset 48
    float lifetime;        // offset 64
    float life_remaining;  // offset 68
    uint is_active;        // offset 72
    uint _pad;             // offset 76
};
// total: 80 bytes, struct alignment 16

// ---------------------------------------------------------------------------
// Instance output — must match SpriteInstanceData in Rust (#[repr(C)])
// ---------------------------------------------------------------------------

struct InstanceData {
    vec4 transform_col0;
    vec4 transform_col1;
    vec4 transform_col2;
    vec4 transform_col3;
    vec4 color;
    vec2 uv_min;
    vec2 uv_max;
    float tex_index;
    float tiling_factor;
    int entity_id;
    // GPU animation parameters (unused by particles — zeroed)
    float anim_start_time;
    float anim_fps;
    float anim_start_frame;
    float anim_frame_count;
    float anim_columns;
    float anim_looping;
    vec2 anim_cell_size;
    vec2 anim_tex_size;
};
// total: 144 bytes, struct alignment 16

// ---------------------------------------------------------------------------
// Descriptor bindings
// ---------------------------------------------------------------------------

layout(std430, set = 0, binding = 0) buffer ParticleState {
    GpuParticle particles[];
};

layout(std430, set = 0, binding = 1) writeonly buffer InstanceOutput {
    InstanceData instances[];
};

layout(std430, set = 0, binding = 2) buffer IndirectCommand {
    uint indexCount;
    uint instanceCount;
    uint firstIndex;
    int vertexOffset;
    uint firstInstance;
};

// ---------------------------------------------------------------------------
// Push constants
// ---------------------------------------------------------------------------

layout(push_constant) uniform PushConstants {
    float dt;
    uint max_particles;
};

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

void main() {
    uint idx = gl_GlobalInvocationID.x;
    if (idx >= max_particles) return;

    GpuParticle p = particles[idx];
    if (p.is_active == 0u) return;

    // --- Simulate ---
    p.life_remaining -= dt;
    if (p.life_remaining <= 0.0) {
        p.is_active = 0u;
        particles[idx] = p;
        return;
    }

    // Velocity damping — particles slow down over time.
    p.velocity *= 1.0 - 2.0 * dt;
    p.position += p.velocity * dt;
    p.rotation += p.rotation_speed * dt;

    particles[idx] = p;

    // --- Output instance data for alive particles ---
    float life = p.life_remaining / p.lifetime;
    vec4 color = mix(p.color_end, p.color_begin, life);
    float size = mix(p.size_end, p.size_begin, life);

    if (size <= 0.0) return;

    // Newer particles (life~1) get more negative z (closer to camera in LH).
    float z = -0.1 - life * 0.05;
    float c = cos(p.rotation);
    float s = sin(p.rotation);

    uint slot = atomicAdd(instanceCount, 1u);

    instances[slot].transform_col0 = vec4( c * size, s * size, 0.0, 0.0);
    instances[slot].transform_col1 = vec4(-s * size, c * size, 0.0, 0.0);
    instances[slot].transform_col2 = vec4(0.0, 0.0, 1.0, 0.0);
    instances[slot].transform_col3 = vec4(p.position.x, p.position.y, z, 1.0);
    instances[slot].color = color;
    instances[slot].uv_min = vec2(0.0);
    instances[slot].uv_max = vec2(1.0);
    instances[slot].tex_index = 0.0;
    instances[slot].tiling_factor = 1.0;
    instances[slot].entity_id = -1;
    instances[slot].anim_start_time = 0.0;
    instances[slot].anim_fps = 0.0;
    instances[slot].anim_start_frame = 0.0;
    instances[slot].anim_frame_count = 0.0;
    instances[slot].anim_columns = 0.0;
    instances[slot].anim_looping = 0.0;
    instances[slot].anim_cell_size = vec2(0.0);
    instances[slot].anim_tex_size = vec2(0.0);
}
