#version 450

layout(location = 0) in vec3 vertex_color;
layout(location = 1) in vec2 texture_uv;
layout(location = 2) in vec3 world_normal;
layout(set = 0, binding = 1) uniform sampler2D forest_texture;
layout(location = 0) out vec4 out_color;

void main() {
    vec3 texel = texture(forest_texture, texture_uv).rgb;
    vec3 light_direction = normalize(vec3(-0.45, 0.75, 0.55));
    float ambient = 0.26;
    float diffuse = max(dot(normalize(world_normal), light_direction), 0.0);
    float illumination = ambient + (1.0 - ambient) * diffuse;
    out_color = vec4(vertex_color * texel * illumination, 1.0);
}
