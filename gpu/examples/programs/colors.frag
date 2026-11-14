// The vertices' colour, smooth or flat as a push of the attachment's location says: location 0 gets the smooth one,
// location 1 the flat one; fragments whose smooth red is above 0.9 are discarded.
#version 450
layout(location = 0) in vec4 smooth_color;
layout(location = 1) flat in vec4 flat_color;
layout(location = 0) out vec4 out_smooth;
layout(location = 1) out vec4 out_flat;

void main() {
    if (smooth_color.r > 0.9) {
        discard;
    }
    out_smooth = smooth_color;
    out_flat = flat_color;
}
