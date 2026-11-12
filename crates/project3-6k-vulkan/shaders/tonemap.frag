#version 450

layout(location = 0) in vec2 texture_uv;
layout(set = 0, binding = 0) uniform sampler2D hdr_color;
layout(push_constant) uniform ToneMapParameters {
    float exposure;
    uint output_is_srgb;
} parameters;
layout(location = 0) out vec4 out_color;

vec3 aces_fitted(vec3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((color * (a * color + b)) / (color * (c * color + d) + e), 0.0, 1.0);
}

void main() {
    vec3 hdr = max(texture(hdr_color, texture_uv).rgb, vec3(0.0));
    vec3 mapped = aces_fitted(hdr * exp2(parameters.exposure));
    if (parameters.output_is_srgb == 0u) {
        mapped = pow(mapped, vec3(1.0 / 2.2));
    }
    out_color = vec4(mapped, 1.0);
}
