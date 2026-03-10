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
    ivec4 counts;          // x = num_point_lights, y = has_directional, z = has_shadow, w = unused

    // Cascaded shadow mapping (2 cascades)
    mat4 shadow_light_vp[2];    // per-cascade light-space VP matrices
    vec4 cascade_split_depth;   // x = split depth (NDC [0,1]), y-w = unused
} lighting;

// Shadow map (set 4) — 2-layer depth comparison sampler array.
layout(set = 4, binding = 0) uniform sampler2DArrayShadow u_shadow_map;

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
#endif

// Interleaved gradient noise (Jimenez 2014) — deterministic per-pixel hash.
float interleavedGradientNoise(vec2 pos) {
    return fract(52.9829189 * fract(0.06711056 * pos.x + 0.00583715 * pos.y));
}

// Calculate shadow factor for directional light (1.0 = fully lit, 0.0 = fully shadowed).
// Uses 2-cascade CSM with per-pixel rotated PCF.
float calculate_shadow(vec3 world_pos, vec3 normal) {
    if (lighting.counts.z == 0) return 1.0; // No shadow mapping active

    // Select cascade based on fragment depth in NDC.
    vec4 clip_pos = camera.u_view_projection * vec4(world_pos, 1.0);
    float depth_ndc = clip_pos.z / clip_pos.w; // [0,1] in Vulkan
    int cascade = (depth_ndc > lighting.cascade_split_depth.x) ? 1 : 0;

    // Offset along surface normal to reduce shadow acne on curved geometry.
    // Scale bias by surface angle to light — grazing angles need more offset.
    // Near cascade (0) has higher texel density so needs less bias.
    vec3 light_dir = normalize(-lighting.dir_direction.xyz);
    float cos_theta = clamp(dot(normal, light_dir), 0.0, 1.0);
    float bias_scale = (cascade == 0) ? 0.5 : 1.0;
    float normal_bias = mix(0.01, 0.0005, cos_theta) * bias_scale;
    vec3 biased_pos = world_pos + normal * normal_bias;
    vec4 light_space_pos = lighting.shadow_light_vp[cascade] * vec4(biased_pos, 1.0);
    vec3 proj_coords = light_space_pos.xyz / light_space_pos.w;

    // Vulkan NDC: x,y in [-1, 1], z in [0, 1].
    proj_coords.xy = proj_coords.xy * 0.5 + 0.5;

    // Smooth fade at shadow map edges to avoid hard rectangular cutoff.
    float fade_margin = 0.10;
    float fade = smoothstep(0.0, fade_margin, proj_coords.x)
               * smoothstep(1.0, 1.0 - fade_margin, proj_coords.x)
               * smoothstep(0.0, fade_margin, proj_coords.y)
               * smoothstep(1.0, 1.0 - fade_margin, proj_coords.y);

    // Outside shadow map frustum = not in shadow.
    if (fade <= 0.0 || proj_coords.z > 1.0 || proj_coords.z < 0.0) {
        return 1.0;
    }

    // Vogel disk PCF — golden-angle spiral with per-pixel rotation.
    // Adaptive radius ensures ≥16 screen pixels of penumbra at any zoom.
    const int SHADOW_SAMPLES = 32;
    const float GOLDEN_ANGLE = 2.399963; // pi * (3 - sqrt(5))
    float noise = interleavedGradientNoise(gl_FragCoord.xy);
    float shadow = 0.0;
    vec2 texel_size = 1.0 / textureSize(u_shadow_map, 0).xy;
    // screen_scale = texels-per-screen-pixel (via shadow UV derivatives).
    // radius = 8/screen_scale ensures 2*radius/screen_scale ≥ 16 screen pixels.
    float screen_scale = length(fwidth(proj_coords.xy)) / length(texel_size);
    float radius = clamp(8.0 / max(screen_scale, 0.5), 3.0, 16.0);
    for (int i = 0; i < SHADOW_SAMPLES; i++) {
        float r = sqrt((float(i) + 0.5) / float(SHADOW_SAMPLES));
        float theta = float(i) * GOLDEN_ANGLE + noise * 6.283185;
        vec2 offset = vec2(cos(theta), sin(theta)) * r * radius * texel_size;
        shadow += texture(u_shadow_map, vec4(proj_coords.xy + offset, float(cascade), proj_coords.z));
    }
    shadow /= float(SHADOW_SAMPLES);

    // Apply edge fade. Blend toward fully-lit at shadow map boundaries.
    shadow = mix(1.0, shadow, fade);

    // Minimum shadow prevents pitch-black shadows (some scattered light always reaches).
    shadow = max(shadow, 0.08);

    return shadow;
}

// Blinn-Phong specular with roughness-based shininess.
vec3 blinn_phong(vec3 light_dir, vec3 light_color, float light_intensity,
                 vec3 normal, vec3 view_dir, vec3 albedo) {
    // Diffuse
    float ndotl = max(dot(normal, light_dir), 0.0);
    vec3 diffuse = albedo * light_color * light_intensity * ndotl;

    // Specular (Blinn-Phong) — only when surface faces the light.
    vec3 specular = vec3(0.0);
    if (ndotl > 0.0) {
        vec3 half_dir = normalize(light_dir + view_dir);
        float shininess = max(2.0 / (push.u_roughness * push.u_roughness + 0.001) - 2.0, 1.0);
        float spec = pow(max(dot(normal, half_dir), 0.0), shininess);
        // Metallic surfaces reflect the albedo color; dielectrics reflect white.
        vec3 spec_color = mix(vec3(0.04), albedo, push.u_metallic);
        specular = spec_color * light_color * light_intensity * spec;
    }

    return diffuse + specular;
}

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
            // Smooth quadratic attenuation: (1 - (d/r)^2)^2
            float ratio = dist / radius;
            float atten = max(1.0 - ratio * ratio, 0.0);
            atten = atten * atten;

            result += blinn_phong(light_dir, light_color, intensity * atten, n, view_dir, albedo);
        }
    }

    // Emissive contribution.
    result += push.u_emissive_color.rgb * push.u_emissive_strength;

    out_color = vec4(result, v_color.a * push.u_albedo_color.a * tex_color.a);

#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
#endif
}
