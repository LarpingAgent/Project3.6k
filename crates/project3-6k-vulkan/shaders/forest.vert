#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec4 in_tangent;
layout(location = 3) in vec2 in_uv;
layout(location = 4) in vec4 in_color;

layout(set = 0, binding = 0) uniform SceneUniform {
    mat4 view_projection;
    mat4 sun_view_projection;
    vec4 camera_position;
    vec4 sun_direction;
    vec4 sun_color;
    vec4 ambient_color;
    vec4 irradiance_sh[9];
} scene;

layout(push_constant) uniform MaterialPushConstants {
    mat4 model;
    vec4 base_color_factor;
    vec4 material_params;
    vec4 emissive_factor;
    vec4 material_flags;
} material;

layout(location = 0) out vec3 world_position;
layout(location = 1) out vec3 world_normal;
layout(location = 2) out vec4 world_tangent;
layout(location = 3) out vec2 texture_uv;
layout(location = 4) out vec4 vertex_color;

void main() {
    vec4 world = material.model * vec4(in_position, 1.0);
    mat3 normal_matrix = transpose(inverse(mat3(material.model)));
    world_position = world.xyz;
    world_normal = normalize(normal_matrix * in_normal);
    world_tangent = vec4(normalize(mat3(material.model) * in_tangent.xyz), in_tangent.w);
    texture_uv = in_uv;
    vertex_color = in_color;
    gl_Position = scene.view_projection * world;
}
