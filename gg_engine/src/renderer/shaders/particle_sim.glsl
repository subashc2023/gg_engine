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
// All fields are float/int (alignment 4) to avoid std430 padding mismatches
// with the Rust #[repr(C)] layout. Using vec2/vec4 would introduce alignment
// gaps (vec2 align 8, vec4 align 16) that don't exist in the Rust struct.

struct InstanceData {
    float tc0_x, tc0_y, tc0_z, tc0_w;   // transform col 0
    float tc1_x, tc1_y, tc1_z, tc1_w;   // transform col 1
    float tc2_x, tc2_y, tc2_z, tc2_w;   // transform col 2
    float tc3_x, tc3_y, tc3_z, tc3_w;   // transform col 3
    float cr, cg, cb, ca;                // color
    float uv_min_x, uv_min_y;           // uv_min
    float uv_max_x, uv_max_y;           // uv_max
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
    float anim_cell_w, anim_cell_h;     // anim_cell_size
    float anim_tex_w, anim_tex_h;       // anim_tex_size
};
// total: 148 bytes (37 floats/ints × 4), struct alignment 4

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

    // Velocity damping — exponential decay (frame-rate independent, safe for large dt).
    p.velocity *= exp(-2.0 * dt);
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

    InstanceData inst;
    inst.tc0_x =  c * size; inst.tc0_y = s * size; inst.tc0_z = 0.0; inst.tc0_w = 0.0;
    inst.tc1_x = -s * size; inst.tc1_y = c * size; inst.tc1_z = 0.0; inst.tc1_w = 0.0;
    inst.tc2_x = 0.0; inst.tc2_y = 0.0; inst.tc2_z = 1.0; inst.tc2_w = 0.0;
    inst.tc3_x = p.position.x; inst.tc3_y = p.position.y; inst.tc3_z = z; inst.tc3_w = 1.0;
    inst.cr = color.r; inst.cg = color.g; inst.cb = color.b; inst.ca = color.a;
    inst.uv_min_x = 0.0; inst.uv_min_y = 0.0;
    inst.uv_max_x = 1.0; inst.uv_max_y = 1.0;
    inst.tex_index = 0.0;
    inst.tiling_factor = 1.0;
    inst.entity_id = -1;
    inst.anim_start_time = 0.0;
    inst.anim_fps = 0.0;
    inst.anim_start_frame = 0.0;
    inst.anim_frame_count = 0.0;
    inst.anim_columns = 0.0;
    inst.anim_looping = 0.0;
    inst.anim_cell_w = 0.0; inst.anim_cell_h = 0.0;
    inst.anim_tex_w = 0.0; inst.anim_tex_h = 0.0;
    instances[slot] = inst;
}
