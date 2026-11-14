// Coloured vertices: a position in clip space (x, y, z, w) and a colour, passed on smooth; and the colour again,
// flat, so the provoking vertex is seen.
#version 450
layout(location = 0) in vec4 position;
layout(location = 1) in vec4 color;
layout(location = 0) out vec4 smooth_color;
layout(location = 1) flat out vec4 flat_color;

void main() {
    gl_Position = position;
    smooth_color = color;
    flat_color = color;
}
