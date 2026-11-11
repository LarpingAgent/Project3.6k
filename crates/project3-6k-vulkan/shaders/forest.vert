#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_color;
layout(location = 2) in vec2 in_uv;

layout(set = 0, binding = 0) uniform CameraUniform {
    mat4 view_projection;
} camera;

layout(location = 0) out vec3 vertex_color;
layout(location = 1) out vec2 texture_uv;

void main() {
    gl_Position = camera.view_projection * vec4(in_position, 1.0);
    vertex_color = in_color;
    texture_uv = in_uv;
}
