#type vertex
#version 450

layout(set = 0, binding = 0) uniform CameraBuffer {
    mat4 u_view_projection;
    float u_time;
} camera;

layout(push_constant) uniform PushConstants {
    mat4 u_model;
    int u_entity_id;
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
    // Transform normal to world space (using inverse-transpose for non-uniform scale).
    v_normal = mat3(push.u_model) * a_normal;
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

// Material UBO (set 2) — PBR surface properties.
layout(set = 2, binding = 0) uniform MaterialUBO {
    vec4 albedo_color;
    vec3 emissive_color;
    float metallic;
    float roughness;
    float emissive_strength;
    float alpha_cutoff;
    int albedo_tex_index;
    int normal_tex_index;
    float _pad[3];
} material;

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

    // Shadow mapping
    mat4 shadow_light_vp;  // light-space VP matrix
} lighting;

// Shadow map (set 4) — depth comparison sampler.
layout(set = 4, binding = 0) uniform sampler2DShadow u_shadow_map;

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

// Calculate shadow factor for directional light (1.0 = fully lit, 0.0 = fully shadowed).
float calculate_shadow(vec3 world_pos, vec3 normal) {
    if (lighting.counts.z == 0) return 1.0; // No shadow mapping active

    vec4 light_space_pos = lighting.shadow_light_vp * vec4(world_pos, 1.0);
    vec3 proj_coords = light_space_pos.xyz / light_space_pos.w;

    // Vulkan NDC: x,y in [-1, 1], z in [0, 1].
    proj_coords.xy = proj_coords.xy * 0.5 + 0.5;

    // Outside shadow map frustum = not in shadow.
    if (proj_coords.x < 0.0 || proj_coords.x > 1.0 ||
        proj_coords.y < 0.0 || proj_coords.y > 1.0 ||
        proj_coords.z > 1.0) {
        return 1.0;
    }

    // 5x5 PCF (percentage-closer filtering).
    float shadow = 0.0;
    vec2 texel_size = 1.0 / textureSize(u_shadow_map, 0);
    for (int x = -2; x <= 2; ++x) {
        for (int y = -2; y <= 2; ++y) {
            vec2 offset = vec2(x, y) * texel_size;
            shadow += texture(u_shadow_map, vec3(proj_coords.xy + offset, proj_coords.z));
        }
    }
    shadow /= 25.0;

    return shadow;
}

// Blinn-Phong specular with roughness-based shininess.
vec3 blinn_phong(vec3 light_dir, vec3 light_color, float light_intensity,
                 vec3 normal, vec3 view_dir, vec3 albedo) {
    // Diffuse
    float ndotl = max(dot(normal, light_dir), 0.0);
    vec3 diffuse = albedo * light_color * light_intensity * ndotl;

    // Specular (Blinn-Phong)
    vec3 half_dir = normalize(light_dir + view_dir);
    float shininess = max(2.0 / (material.roughness * material.roughness + 0.001) - 2.0, 1.0);
    float spec = pow(max(dot(normal, half_dir), 0.0), shininess);
    // Metallic surfaces reflect the albedo color; dielectrics reflect white.
    vec3 spec_color = mix(vec3(0.04), albedo, material.metallic);
    vec3 specular = spec_color * light_color * light_intensity * spec;

    return diffuse + specular;
}

void main() {
    vec3 n = normalize(v_normal);
    vec3 view_dir = normalize(lighting.camera_position.xyz - v_world_position);
    vec3 albedo = v_color.rgb * material.albedo_color.rgb;

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
    result += material.emissive_color * material.emissive_strength;

    out_color = vec4(result, v_color.a * material.albedo_color.a);

#ifdef OFFSCREEN
    out_entity_id = v_entity_id;
#endif
}
