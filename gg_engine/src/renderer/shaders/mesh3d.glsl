#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
    float u_time;
} camera;

layout(push_constant) uniform PushConstants {
    mat4 u_model;
    int u_entity_id;
    float u_metallic;
    float u_roughness;
    float u_emissive_strength;
    vec4 u_albedo_color;
    vec4 u_emissive_color;
    int u_albedo_tex_index;
} push;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_color;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec3 v_normal;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out vec3 v_world_position;
#ifdef OFFSCREEN
layout(location = 4) out flat int v_entity_id;
#endif

void main() {
    vec4 world_pos = push.u_model * vec4(a_position, 1.0);
    v_world_position = world_pos.xyz;
    // Transform normal to world space using the inverse-transpose of the
    // upper-left 3x3 model matrix. This handles non-uniform scale correctly.
    v_normal = transpose(inverse(mat3(push.u_model))) * a_normal;
    v_uv = a_uv;
    v_color = a_color;
#ifdef OFFSCREEN
    v_entity_id = push.u_entity_id;
#endif
    gl_Position = camera.u_view_projection * world_pos;
}

#type fragment
#version 450

#define MAX_POINT_LIGHTS 16
#define NUM_CASCADES 4

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
    float u_time;
} camera;

// Material properties passed via push constants (per-draw).
layout(push_constant) uniform PushConstants {
    mat4 u_model;
    int u_entity_id;
    float u_metallic;
    float u_roughness;
    float u_emissive_strength;
    vec4 u_albedo_color;
    vec4 u_emissive_color;
    int u_albedo_tex_index;
} push;

// Bindless texture array (set 1) — shared with 2D renderer.
layout(set = 1, binding = 0) uniform sampler2D u_textures[4096];

// Lighting UBO (set 3) — scene lights + shadow data.
layout(set = 3, binding = 0) uniform LightingUBO {
    // Directional light
    vec4 dir_direction;   // xyz = direction, w = unused
    vec4 dir_color;       // xyz = color, w = intensity

    // Point lights
    vec4 point_positions[MAX_POINT_LIGHTS]; // xyz = position, w = radius
    vec4 point_colors[MAX_POINT_LIGHTS];    // xyz = color, w = intensity

    // Scene-wide
    vec4 ambient_color;    // xyz = color, w = intensity
    vec4 camera_position;  // xyz = eye position, w = unused
    ivec4 counts;          // x = num_point_lights, y = has_directional, z = has_shadow, w = csm_debug

    // Cascaded shadow mapping (4 cascades)
    mat4 shadow_light_vp[NUM_CASCADES]; // per-cascade light-space VP matrices
    vec4 cascade_split_depth;           // xyz = 3 split depths (NDC), w = shadow_distance
} lighting;

// Shadow map (set 4):
//   binding 0 = comparison sampler (sampler2DArrayShadow) for PCF
//   binding 1 = non-comparison sampler (sampler2DArray) for PCSS blocker search
layout(set = 4, binding = 0) uniform sampler2DArrayShadow u_shadow_map;
layout(set = 4, binding = 1) uniform sampler2DArray u_shadow_map_raw;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec3 v_normal;
layout(location = 2) in vec2 v_uv;
layout(location = 3) in vec3 v_world_position;
#ifdef OFFSCREEN
layout(location = 4) in flat int v_entity_id;
#endif

layout(location = 0) out vec4 out_color;
#ifdef OFFSCREEN
layout(location = 1) out int out_entity_id;
layout(location = 2) out vec4 out_normal;
#endif

// ---------------------------------------------------------------------------
// PCSS — Percentage-Closer Soft Shadows
// ---------------------------------------------------------------------------

const float GOLDEN_ANGLE = 2.399963; // pi * (3 - sqrt(5))
const int BLOCKER_SAMPLES = 16;
const int PCF_SAMPLES = 32;
const float LIGHT_SIZE = 0.04; // World-space light source size — controls penumbra growth rate
const float MIN_PENUMBRA = 1.5; // Minimum PCF radius in texels (sharp contact shadows)
const float MAX_PENUMBRA = 32.0; // Maximum PCF radius in texels (avoids over-blur)

// Interleaved gradient noise (Jimenez 2014) — deterministic per-pixel hash.
float interleavedGradientNoise(vec2 pos) {
    return fract(52.9829189 * fract(0.06711056 * pos.x + 0.00583715 * pos.y));
}

// PCSS blocker search: find average depth of occluders within the search region.
// Returns -1.0 if no blockers found (fully lit).
float findBlockerDepth(vec2 uv, int cascade, float receiver_depth, float search_radius_texels) {
    float noise = interleavedGradientNoise(gl_FragCoord.xy);
    vec2 texel_size = 1.0 / textureSize(u_shadow_map_raw, 0).xy;

    float blocker_sum = 0.0;
    int blocker_count = 0;

    for (int i = 0; i < BLOCKER_SAMPLES; i++) {
        float r = sqrt((float(i) + 0.5) / float(BLOCKER_SAMPLES));
        float theta = float(i) * GOLDEN_ANGLE + noise * 6.283185;
        vec2 offset = vec2(cos(theta), sin(theta)) * r * search_radius_texels * texel_size;
        vec2 sample_uv = clamp(uv + offset, vec2(0.0), vec2(1.0));

        float depth = texture(u_shadow_map_raw, vec3(sample_uv, float(cascade))).r;
        if (depth < receiver_depth) {
            blocker_sum += depth;
            blocker_count++;
        }
    }

    if (blocker_count == 0) return -1.0;
    return blocker_sum / float(blocker_count);
}

// Sample shadow from a single cascade using PCSS.
// Returns vec2(shadow, coverage).
vec2 sample_cascade_shadow(vec3 world_pos, vec3 normal, int cascade) {
    // Receiver-side bias: push the sample point to prevent self-shadowing.
    vec3 light_dir = normalize(-lighting.dir_direction.xyz);
    float cos_theta = clamp(dot(normal, light_dir), 0.0, 1.0);
    float normal_bias = mix(0.02, 0.002, cos_theta);
    float light_bias = mix(0.001, 0.01, cos_theta);
    vec3 biased_pos = world_pos + normal * normal_bias + light_dir * light_bias;
    vec4 light_space_pos = lighting.shadow_light_vp[cascade] * vec4(biased_pos, 1.0);
    vec3 proj_coords = light_space_pos.xyz / light_space_pos.w;

    // Vulkan NDC: x,y in [-1, 1], z in [0, 1].
    proj_coords.xy = proj_coords.xy * 0.5 + 0.5;

    // Smooth coverage falloff at shadow map edges.
    float fade_margin = 0.10;
    float coverage = smoothstep(0.0, fade_margin, proj_coords.x)
                   * smoothstep(1.0, 1.0 - fade_margin, proj_coords.x)
                   * smoothstep(0.0, fade_margin, proj_coords.y)
                   * smoothstep(1.0, 1.0 - fade_margin, proj_coords.y);

    if (coverage <= 0.0 || proj_coords.z > 1.0 || proj_coords.z < 0.0) {
        return vec2(1.0, 0.0);
    }

    // Compute texels-per-world for this cascade (derivative-based).
    vec2 texel_size = 1.0 / textureSize(u_shadow_map_raw, 0).xy;
    float world_per_pixel = length(fwidth(v_world_position));
    float texels_per_pixel = length(fwidth(proj_coords.xy)) / length(texel_size);
    float texels_per_world = texels_per_pixel / max(world_per_pixel, 0.0001);

    // PCSS Step 1: Blocker search.
    // Search radius in texels — proportional to light size in shadow map space.
    float search_radius = clamp(LIGHT_SIZE * texels_per_world, 4.0, 32.0);
    float blocker_depth = findBlockerDepth(proj_coords.xy, cascade, proj_coords.z, search_radius);

    float pcf_radius;
    if (blocker_depth < 0.0) {
        // No blockers found — fully lit.
        return vec2(1.0, coverage);
    } else {
        // PCSS Step 2: Estimate penumbra from blocker distance.
        // penumbra ∝ light_size × (receiver - blocker) / blocker
        float penumbra_ratio = (proj_coords.z - blocker_depth) / max(blocker_depth, 0.0001);
        float penumbra_world = LIGHT_SIZE * penumbra_ratio;
        pcf_radius = clamp(penumbra_world * texels_per_world, MIN_PENUMBRA, MAX_PENUMBRA);
    }

    // PCSS Step 3: Variable-radius PCF using Vogel disk.
    float noise = interleavedGradientNoise(gl_FragCoord.xy);
    float shadow = 0.0;
    for (int i = 0; i < PCF_SAMPLES; i++) {
        float r = sqrt((float(i) + 0.5) / float(PCF_SAMPLES));
        float theta = float(i) * GOLDEN_ANGLE + noise * 6.283185;
        vec2 offset = vec2(cos(theta), sin(theta)) * r * pcf_radius * texel_size;
        vec2 sample_uv = clamp(proj_coords.xy + offset, vec2(0.0), vec2(1.0));
        shadow += texture(u_shadow_map, vec4(sample_uv, float(cascade), proj_coords.z));
    }
    shadow /= float(PCF_SAMPLES);

    // Minimum shadow prevents pitch-black (some scattered light always reaches).
    shadow = max(shadow, 0.08);

    return vec2(shadow, coverage);
}

// Calculate shadow factor for directional light (1.0 = fully lit, 0.0 = fully shadowed).
// Uses 4-cascade CSM with PCSS and coverage-weighted blending at cascade boundaries.
float calculate_shadow(vec3 world_pos, vec3 normal) {
    if (lighting.counts.z == 0) return 1.0;

    // Fragment depth in NDC. Reverse-Z: near→1, far→0.
    vec4 clip_pos = camera.u_view_projection * vec4(world_pos, 1.0);
    float depth_ndc = clip_pos.z / clip_pos.w;

    // 3 split depths for 4 cascades (reverse-Z: split[0] > split[1] > split[2]).
    float splits[3] = float[3](
        lighting.cascade_split_depth.x,
        lighting.cascade_split_depth.y,
        lighting.cascade_split_depth.z
    );

    // Select cascade based on depth: cascade 0 is nearest.
    int cascade = NUM_CASCADES - 1;
    for (int i = 0; i < NUM_CASCADES - 1; i++) {
        if (depth_ndc > splits[i]) {
            cascade = i;
            break;
        }
    }

    // Blend factor with the next cascade for smooth transitions.
    float blend = 0.0;
    if (cascade < NUM_CASCADES - 1) {
        float blend_width = splits[cascade] * 0.15;
        blend = smoothstep(splits[cascade] + blend_width, splits[cascade] - blend_width, depth_ndc);
    }

    // Shadow distance fade-out.
    float shadow_distance = lighting.cascade_split_depth.w;
    float frag_dist = length(world_pos - lighting.camera_position.xyz);
    float distance_fade = 1.0 - smoothstep(shadow_distance * 0.85, shadow_distance, frag_dist);
    if (distance_fade <= 0.0) return 1.0;

    // Early-out for no blend zone.
    if (blend < 0.01) {
        vec2 sc = sample_cascade_shadow(world_pos, normal, cascade);
        float shadow = mix(1.0, sc.x, sc.y);
        return mix(1.0, shadow, distance_fade);
    }

    // Blend zone: coverage-weighted blend between current and next cascade.
    vec2 sc0 = sample_cascade_shadow(world_pos, normal, cascade);
    vec2 sc1 = sample_cascade_shadow(world_pos, normal, cascade + 1);

    float w0 = (1.0 - blend) * sc0.y;
    float w1 = blend * sc1.y;
    float total = w0 + w1;

    if (total <= 0.0) return 1.0;
    float shadow = (sc0.x * w0 + sc1.x * w1) / total;
    return mix(1.0, shadow, distance_fade);
}

// ---------------------------------------------------------------------------
// Blinn-Phong lighting
// ---------------------------------------------------------------------------

vec3 blinn_phong(vec3 light_dir, vec3 light_color, float light_intensity,
                 vec3 normal, vec3 view_dir, vec3 albedo) {
    float ndotl = max(dot(normal, light_dir), 0.0);
    vec3 diffuse = albedo * light_color * light_intensity * ndotl;

    vec3 specular = vec3(0.0);
    if (ndotl > 0.0) {
        vec3 half_dir = normalize(light_dir + view_dir);
        float shininess = max(2.0 / (push.u_roughness * push.u_roughness + 0.001) - 2.0, 1.0);
        float spec = pow(max(dot(normal, half_dir), 0.0), shininess);
        vec3 spec_color = mix(vec3(0.04), albedo, push.u_metallic);
        specular = spec_color * light_color * light_intensity * spec;
    }

    return diffuse + specular;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

void main() {
    vec3 n = normalize(v_normal);
    vec3 view_dir = normalize(lighting.camera_position.xyz - v_world_position);

    // Sample albedo texture if assigned, otherwise use white.
    vec4 tex_color = vec4(1.0);
    if (push.u_albedo_tex_index >= 0) {
        tex_color = texture(u_textures[push.u_albedo_tex_index], v_uv);
    }

    vec3 albedo = v_color.rgb * push.u_albedo_color.rgb * tex_color.rgb;

    // Ambient contribution.
    vec3 result = albedo * lighting.ambient_color.rgb * lighting.ambient_color.w;

    // Directional light.
    if (lighting.counts.y > 0) {
        vec3 light_dir = normalize(-lighting.dir_direction.xyz);
        float intensity = lighting.dir_color.w;
        float shadow = calculate_shadow(v_world_position, n);
        result += shadow * blinn_phong(light_dir, lighting.dir_color.rgb, intensity, n, view_dir, albedo);
    }

    // Point lights.
    int num_points = lighting.counts.x;
    for (int i = 0; i < num_points; i++) {
        vec3 light_pos = lighting.point_positions[i].xyz;
        float radius = lighting.point_positions[i].w;
        vec3 light_color = lighting.point_colors[i].rgb;
        float intensity = lighting.point_colors[i].w;

        vec3 to_light = light_pos - v_world_position;
        float dist = length(to_light);

        if (dist < radius) {
            vec3 light_dir = to_light / dist;
            float ratio = dist / radius;
            float atten = max(1.0 - ratio * ratio, 0.0);
            atten = atten * atten;

            result += blinn_phong(light_dir, light_color, intensity * atten, n, view_dir, albedo);
        }
    }

    // Emissive contribution.
    result += push.u_emissive_color.rgb * push.u_emissive_strength;

    // CSM debug visualization (runtime toggle via lighting.counts.w).
    int csm_debug = lighting.counts.w;
    if (csm_debug > 0) {
        vec4 dbg_clip_pos = camera.u_view_projection * vec4(v_world_position, 1.0);
        float dbg_depth = dbg_clip_pos.z / dbg_clip_pos.w;

        float splits[3] = float[3](
            lighting.cascade_split_depth.x,
            lighting.cascade_split_depth.y,
            lighting.cascade_split_depth.z
        );

        int dbg_cascade = NUM_CASCADES - 1;
        for (int i = 0; i < NUM_CASCADES - 1; i++) {
            if (dbg_depth > splits[i]) { dbg_cascade = i; break; }
        }

        vec3 debug_color = vec3(0.0);

        if (csm_debug == 1) {
            vec3 cascade_colors[4] = vec3[4](
                vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0),
                vec3(0.0, 0.0, 1.0), vec3(1.0, 1.0, 0.0)
            );
            debug_color = cascade_colors[dbg_cascade];
        }
        else if (csm_debug == 2) {
            vec2 sc = sample_cascade_shadow(v_world_position, n, 0);
            debug_color = vec3(sc.x);
        }
        else if (csm_debug == 3) {
            vec2 sc = sample_cascade_shadow(v_world_position, n, 1);
            debug_color = vec3(sc.x);
        }
        else if (csm_debug == 4) {
            vec2 sc = sample_cascade_shadow(v_world_position, n, 2);
            debug_color = vec3(sc.x);
        }
        else if (csm_debug == 5) {
            vec2 sc = sample_cascade_shadow(v_world_position, n, 3);
            debug_color = vec3(sc.x);
        }
        else if (csm_debug == 6) {
            float shadow = calculate_shadow(v_world_position, n);
            debug_color = vec3(shadow);
        }
        else if (csm_debug == 7) {
            float shadow_distance = lighting.cascade_split_depth.w;
            float frag_dist = length(v_world_position - lighting.camera_position.xyz);
            float fade = 1.0 - smoothstep(shadow_distance * 0.85, shadow_distance, frag_dist);
            debug_color = vec3(fade, 0.0, 1.0 - fade);
        }

        out_color = vec4(debug_color, 1.0);
    } else {
        out_color = vec4(result, v_color.a * push.u_albedo_color.a * tex_color.a);
    }

#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
    out_normal = vec4(n, 1.0);
#endif
}
