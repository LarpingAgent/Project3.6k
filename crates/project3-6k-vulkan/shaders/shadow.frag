#version 450

layout(location = 0) in vec2 texture_uv;
layout(location = 1) in vec4 vertex_color;
layout(set = 1, binding = 0) uniform sampler2D base_color_map;

layout(push_constant) uniform MaterialPushConstants {
    mat4 model;
    vec4 base_color_factor;
    vec4 material_params;
    vec4 emissive_factor;
    vec4 material_flags;
} material;

void main() {
    if (material.material_flags.y > 0.5) {
        float alpha = texture(base_color_map, texture_uv).a
            * material.base_color_factor.a
            * vertex_color.a;
        if (alpha < material.material_flags.x) {
            discard;
        }
    }
}
