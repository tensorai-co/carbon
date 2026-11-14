// The texture at the interpolated coordinate, at the level of detail a uniform names: for mip levels, the sampler's
// mipmap filter and its range of levels of detail.
#version 450
layout(set = 0, binding = 0) uniform texture2D image;
layout(set = 0, binding = 1) uniform sampler smp;
layout(set = 0, binding = 2) uniform Lod { float lod; };
layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 color;

void main() {
    color = textureLod(sampler2D(image, smp), frag_uv, lod);
}
