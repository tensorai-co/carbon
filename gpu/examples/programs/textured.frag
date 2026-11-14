// The texture at the interpolated coordinate, its level of detail from the coordinate's derivatives; and the
// derivatives themselves, scaled into the second attachment, with the fragment's own position.
#version 450
layout(set = 0, binding = 0) uniform texture2D image;
layout(set = 0, binding = 1) uniform sampler smp;
layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 color;
layout(location = 1) out vec4 info;

void main() {
    color = texture(sampler2D(image, smp), frag_uv);
    info = vec4(dFdx(frag_uv.x) * 256.0, dFdy(frag_uv.y) * 256.0, gl_FragCoord.x / 256.0, gl_FragCoord.y / 256.0);
}
