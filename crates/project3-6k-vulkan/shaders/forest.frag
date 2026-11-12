#version 450

const float PI = 3.14159265359;

layout(location = 0) in vec3 world_position;
layout(location = 1) in vec3 world_normal;
layout(location = 2) in vec4 world_tangent;
layout(location = 3) in vec2 texture_uv;
layout(location = 4) in vec4 vertex_color;

layout(set = 0, binding = 0) uniform SceneUniform {
    mat4 view_projection;
    vec4 camera_position;
    vec4 sun_direction;
    vec4 sun_color;
    vec4 ambient_color;
} scene;

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

    vec3 direct = evaluate_sun(normal, view, light, base.rgb, metallic, roughness);
    float hemi = 0.45 + 0.55 * max(normal.y, 0.0);
    vec3 ambient = scene.ambient_color.rgb * hemi * base.rgb * ao;
    vec3 emissive = texture(emissive_map, texture_uv).rgb * material.emissive_factor.rgb;
    vec3 hdr_color = direct + ambient + emissive;

    // Temporary display transform for the current LDR swapchain. The next renderer stage moves
    // this transform to a full-resolution FP16 HDR target and a dedicated post-process pass.
    vec3 mapped = hdr_color / (hdr_color + vec3(1.0));
    out_color = vec4(mapped, base.a);
}
