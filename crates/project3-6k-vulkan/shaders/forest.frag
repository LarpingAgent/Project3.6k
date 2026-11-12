#version 450

const float PI = 3.14159265359;

layout(location = 0) in vec3 world_position;
layout(location = 1) in vec3 world_normal;
layout(location = 2) in vec4 world_tangent;
layout(location = 3) in vec2 texture_uv;
layout(location = 4) in vec4 vertex_color;

layout(set = 0, binding = 0) uniform SceneUniform {
    mat4 view_projection;
    mat4 sun_view_projection;
    mat4 inverse_view_projection;
    vec4 camera_position;
    vec4 sun_direction;
    vec4 sun_color;
    vec4 ambient_color;
    vec4 irradiance_sh[9];
} scene;
layout(set = 0, binding = 1) uniform sampler2DShadow directional_shadow_map;
layout(set = 0, binding = 3) uniform sampler2D prefiltered_environment;
layout(set = 0, binding = 4) uniform sampler2D environment_brdf_lut;

layout(set = 1, binding = 0) uniform sampler2D base_color_map;
layout(set = 1, binding = 1) uniform sampler2D normal_map;
layout(set = 1, binding = 2) uniform sampler2D metallic_roughness_map;
layout(set = 1, binding = 3) uniform sampler2D occlusion_map;
layout(set = 1, binding = 4) uniform sampler2D emissive_map;

layout(push_constant) uniform MaterialPushConstants {
    mat4 model;
    vec4 base_color_factor;
    vec4 material_params;
    vec4 emissive_factor;
    vec4 material_flags;
} material;

layout(location = 0) out vec4 out_color;

float distribution_ggx(float n_dot_h, float roughness) {
    float alpha = roughness * roughness;
    float alpha_squared = alpha * alpha;
    float denominator = n_dot_h * n_dot_h * (alpha_squared - 1.0) + 1.0;
    return alpha_squared / max(PI * denominator * denominator, 0.000001);
}

float geometry_schlick_ggx(float n_dot_v, float roughness) {
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;
    return n_dot_v / max(n_dot_v * (1.0 - k) + k, 0.000001);
}

vec3 fresnel_schlick(float cosine, vec3 f0) {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cosine, 0.0, 1.0), 5.0);
}

vec2 environment_uv(vec3 direction) {
    vec3 d = normalize(direction);
    return vec2(atan(d.z, d.x) / (2.0 * PI) + 0.5,
        acos(clamp(d.y, -1.0, 1.0)) / PI);
}

vec3 evaluate_diffuse_irradiance(vec3 normal) {
    vec3 n = normalize(normal);
    float basis[9] = float[9](
        0.282095,
        0.488603 * n.y,
        0.488603 * n.z,
        0.488603 * n.x,
        1.092548 * n.x * n.y,
        1.092548 * n.y * n.z,
        0.315392 * (3.0 * n.z * n.z - 1.0),
        1.092548 * n.x * n.z,
        0.546274 * (n.x * n.x - n.y * n.y)
    );
    vec3 irradiance = vec3(0.0);
    for (int index = 0; index < 9; ++index) {
        irradiance += scene.irradiance_sh[index].rgb * basis[index];
    }
    return max(irradiance, vec3(0.0));
}

float sun_visibility(vec3 position, vec3 normal, vec3 light) {
    vec4 shadow_clip = scene.sun_view_projection * vec4(position + normal * 0.015, 1.0);
    if (shadow_clip.w <= 0.0) {
        return 1.0;
    }
    vec3 shadow_position = shadow_clip.xyz / shadow_clip.w;
    vec2 uv = shadow_position.xy * 0.5 + 0.5;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0
        || shadow_position.z < 0.0 || shadow_position.z > 1.0) {
        return 1.0;
    }

    float bias = max(0.00035 * (1.0 - dot(normal, light)), 0.00008);
    vec2 texel = 1.0 / vec2(textureSize(directional_shadow_map, 0));
    float reference_depth = shadow_position.z - bias;
    float visibility =
        texture(directional_shadow_map, vec3(uv + texel * vec2(-0.5, -0.5), reference_depth))
        + texture(directional_shadow_map, vec3(uv + texel * vec2(0.5, -0.5), reference_depth))
        + texture(directional_shadow_map, vec3(uv + texel * vec2(-0.5, 0.5), reference_depth))
        + texture(directional_shadow_map, vec3(uv + texel * vec2(0.5, 0.5), reference_depth));
    return visibility * 0.25;
}

vec3 evaluate_sun(vec3 normal, vec3 view, vec3 light, vec3 albedo, float metallic, float roughness) {
    vec3 half_vector = normalize(view + light);
    float n_dot_l = max(dot(normal, light), 0.0);
    float n_dot_v = max(dot(normal, view), 0.001);
    float n_dot_h = max(dot(normal, half_vector), 0.0);
    float h_dot_v = max(dot(half_vector, view), 0.0);

    vec3 f0 = mix(vec3(0.04), albedo, metallic);
    vec3 fresnel = fresnel_schlick(h_dot_v, f0);
    float distribution = distribution_ggx(n_dot_h, roughness);
    float geometry = geometry_schlick_ggx(n_dot_v, roughness)
        * geometry_schlick_ggx(n_dot_l, roughness);
    vec3 specular = (distribution * geometry * fresnel)
        / max(4.0 * n_dot_v * max(n_dot_l, 0.001), 0.001);
    vec3 diffuse_weight = (vec3(1.0) - fresnel) * (1.0 - metallic);
    return (diffuse_weight * albedo / PI + specular) * scene.sun_color.rgb * n_dot_l;
}

void main() {
    vec4 base_sample = texture(base_color_map, texture_uv);
    vec4 base = base_sample * material.base_color_factor * vertex_color;
    if (material.material_flags.y > 0.5 && base.a < material.material_flags.x) {
        discard;
    }

    vec3 geometric_normal = normalize(world_normal);
    vec3 tangent = normalize(world_tangent.xyz - geometric_normal * dot(geometric_normal, world_tangent.xyz));
    vec3 bitangent = normalize(cross(geometric_normal, tangent)) * world_tangent.w;
    vec3 tangent_normal = texture(normal_map, texture_uv).xyz * 2.0 - 1.0;
    tangent_normal.xy *= material.material_params.z;
    vec3 normal = normalize(mat3(tangent, bitangent, geometric_normal) * tangent_normal);
    if (material.material_flags.z > 0.5 && !gl_FrontFacing) {
        normal = -normal;
    }

    vec3 mr = texture(metallic_roughness_map, texture_uv).rgb;
    float metallic = clamp(material.material_params.x * mr.b, 0.0, 1.0);
    float roughness = clamp(material.material_params.y * mr.g, 0.045, 1.0);
    float ao_sample = texture(occlusion_map, texture_uv).r;
    float ao = mix(1.0, ao_sample, material.material_params.w);
    vec3 view = normalize(scene.camera_position.xyz - world_position);
    vec3 light = normalize(scene.sun_direction.xyz);

    float visibility = sun_visibility(world_position, normal, light);
    vec3 direct = evaluate_sun(normal, view, light, base.rgb, metallic, roughness) * visibility;
    float n_dot_v = max(dot(normal, view), 0.001);
    vec3 f0 = mix(vec3(0.04), base.rgb, metallic);
    vec3 environment_fresnel = fresnel_schlick(n_dot_v, f0);
    vec3 diffuse_irradiance = evaluate_diffuse_irradiance(normal) * 0.55;
    if (material.material_flags.y > 0.5) {
        diffuse_irradiance += evaluate_diffuse_irradiance(-normal) * 0.35;
    }
    vec3 reflected = reflect(-view, normal);
    float max_specular_mip = float(textureQueryLevels(prefiltered_environment) - 1);
    vec3 prefiltered_radiance = textureLod(
        prefiltered_environment,
        environment_uv(reflected),
        roughness * max_specular_mip
    ).rgb;
    vec2 brdf = texture(environment_brdf_lut, vec2(n_dot_v, roughness)).rg;
    vec3 specular_ibl = prefiltered_radiance * (f0 * brdf.x + brdf.y);
    vec3 diffuse_weight = (vec3(1.0) - environment_fresnel) * (1.0 - metallic);
    float indirect_ao = mix(0.55, 1.0, ao);
    vec3 ambient = (diffuse_weight * base.rgb * diffuse_irradiance
        + specular_ibl) * indirect_ao;
    ambient += scene.ambient_color.rgb * base.rgb * ao * 0.15;
    vec3 emissive = texture(emissive_map, texture_uv).rgb * material.emissive_factor.rgb;
    vec3 hdr_color = direct + ambient + emissive;
    float camera_distance = length(scene.camera_position.xyz - world_position);
    float fog = 1.0 - exp(-camera_distance * camera_distance * 0.00055);
    hdr_color = mix(hdr_color, vec3(0.05, 0.08, 0.07), fog);
    out_color = vec4(max(hdr_color, vec3(0.0)), base.a);
}
