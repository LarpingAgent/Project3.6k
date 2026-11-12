#version 450

const float PI = 3.14159265359;

layout(location = 0) in vec2 screen_uv;
layout(location = 0) out vec4 out_color;

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
layout(set = 0, binding = 2) uniform sampler2D environment_map;

void main() {
    vec2 ndc = screen_uv * 2.0 - 1.0;
    vec4 far_point = scene.inverse_view_projection * vec4(ndc, 1.0, 1.0);
    vec3 world_point = far_point.xyz / max(far_point.w, 0.000001);
    vec3 view_direction = normalize(world_point - scene.camera_position.xyz);
    vec2 environment_uv = vec2(
        atan(view_direction.z, view_direction.x) / (2.0 * PI) + 0.5,
        acos(clamp(view_direction.y, -1.0, 1.0)) / PI
    );
    out_color = vec4(texture(environment_map, environment_uv).rgb, 1.0);
}
