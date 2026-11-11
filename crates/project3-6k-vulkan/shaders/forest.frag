#version 450

layout(location = 0) in vec3 vertex_color;
layout(location = 1) in vec2 texture_uv;
layout(set = 0, binding = 1) uniform sampler2D forest_texture;
layout(location = 0) out vec4 out_color;

void main() {
    vec3 texel = texture(forest_texture, texture_uv).rgb;
    out_color = vec4(vertex_color * texel, 1.0);
}
