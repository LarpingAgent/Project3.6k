#version 450

layout(location = 0) in vec2 in_position;
layout(location = 1) in vec3 in_color;

layout(location = 0) out vec3 vertex_color;

void main() {
    // Flip Y for the positive-height Vulkan viewport used by the renderer.
    gl_Position = vec4(in_position.x, -in_position.y, 0.0, 1.0);
    vertex_color = in_color;
}
