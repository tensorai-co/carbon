// vkcube's vertex shader, as Vulkan-Tools writes it: the cube's 36 vertices and texture coordinates come from the
// uniform buffer, indexed by gl_VertexIndex, and the MVP matrix places them.
#version 400
#extension GL_ARB_separate_shader_objects : enable
#extension GL_ARB_shading_language_420pack : enable
layout(std140, binding = 0) uniform buf {
    mat4 MVP;
    vec4 position[12 * 3];
    vec4 attr[12 * 3];
} ubuf;

layout(location = 0) out vec4 texcoord;
layout(location = 1) out vec3 frag_pos;

void main() {
    texcoord = ubuf.attr[gl_VertexIndex];
    gl_Position = ubuf.MVP * ubuf.position[gl_VertexIndex];
    frag_pos = gl_Position.xyz;
}
