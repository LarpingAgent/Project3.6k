#version 450

layout(location = 0) out vec2 screen_uv;

void main() {
    const vec2 positions[3] = vec2[3](
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0)
    );
    vec2 position = positions[gl_VertexIndex];
    screen_uv = position * 0.5 + 0.5;
    gl_Position = vec4(position, 1.0, 1.0);
}
