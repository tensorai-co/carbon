// OpenGL draws through Carbon's own GL, on the machine's GPU: shaders compiled by gpu's GLSL compiler (one that
// doesn't compile, saying where), a program linked and its locations asked, a vertex array with a vertex buffer and an
// element buffer, a 2×2 texture, uniforms (a matrix and a tint), and a textured quad drawn indexed, read back pixel by
// pixel; then the same quad blended at half its alpha over itself, and the frame swapped into the window (left up for
// a while when a number of seconds is given, for `carbon display screenshot`).
//
// Built and run inside the machine, as gl.c is:
//
//   carbon cp kernels/lambda/tests/programs/gl gl:/root
//   carbon run --name gl -- sh -c 'cc -O2 -o /root/gl/draw /root/gl/draw.c -lGL -lX11 && __GLX_VENDOR_LIBRARY_NAME=carbon /root/gl/draw'
//   carbon run --name gl --gpu software -- env __GLX_VENDOR_LIBRARY_NAME=carbon /root/gl/draw
#define GL_GLEXT_PROTOTYPES
#include <GL/gl.h>
#include <GL/glext.h>
#include <GL/glx.h>
#include <X11/Xlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define N 64

static int checks, failures;

static void check(int ok, const char *what, long got, long want) {
    checks++;
    if (!ok && failures++ < 20) printf("FAIL %s: %ld, not %ld\n", what, got, want);
}

static void check_eq(const char *what, long got, long want) { check(got == want, what, got, want); }

static const char *VS =
    "#version 330 core\n"
    "layout(location = 0) in vec2 pos;\n"
    "in vec2 uv;\n"
    "uniform mat4 transform;\n"
    "out vec2 v_uv;\n"
    "void main() {\n"
    "    v_uv = uv;\n"
    "    gl_Position = transform * vec4(pos, 0.0, 1.0);\n"
    "}\n";

static const char *FS =
    "#version 330 core\n"
    "in vec2 v_uv;\n"
    "uniform sampler2D tex;\n"
    "uniform vec4 tint;\n"
    "out vec4 color;\n"
    "void main() {\n"
    "    color = texture(tex, v_uv) * tint;\n"
    "}\n";

static GLuint shader(GLenum kind, const char *source, int ok) {
    GLuint s = glCreateShader(kind);
    glShaderSource(s, 1, &source, NULL);
    glCompileShader(s);
    GLint status = 0;
    glGetShaderiv(s, GL_COMPILE_STATUS, &status);
    check_eq(ok ? "a shader compiles" : "a broken shader doesn't", status, ok);
    if (status != ok) {
        char log[512];
        glGetShaderInfoLog(s, sizeof log, NULL, log);
        printf("%s", log);
    }
    return s;
}

// The pixel at (x, y) is (r, g, b) within one step.
static void pixel(const unsigned char *px, int x, int y, int r, int g, int b, const char *what) {
    const unsigned char *p = px + 4 * (y * N + x);
    int ok = abs(p[0] - r) <= 1 && abs(p[1] - g) <= 1 && abs(p[2] - b) <= 1;
    check(ok, what, p[0] << 16 | p[1] << 8 | p[2], r << 16 | g << 8 | b);
}

int main(int argc, char **argv) {
    int hold = argc > 1 ? atoi(argv[1]) : 0;
    Display *dpy = XOpenDisplay(NULL);
    if (!dpy) return 1;
    int attrs[] = {GLX_RENDER_TYPE, GLX_RGBA_BIT, GLX_DRAWABLE_TYPE, GLX_WINDOW_BIT, GLX_DOUBLEBUFFER, True, GLX_DEPTH_SIZE, 24, None};
    int n = 0;
    GLXFBConfig *configs = glXChooseFBConfig(dpy, DefaultScreen(dpy), attrs, &n);
    if (n == 0) return 1;
    XVisualInfo *vi = glXGetVisualFromFBConfig(dpy, configs[0]);
    Window root = RootWindow(dpy, DefaultScreen(dpy));
    XSetWindowAttributes swa = {0};
    swa.colormap = XCreateColormap(dpy, root, vi->visual, AllocNone);
    Window win = XCreateWindow(dpy, root, 0, 0, N, N, 0, vi->depth, InputOutput, vi->visual, CWColormap, &swa);
    XStoreName(dpy, win, "carbon gl draw");
    XMapWindow(dpy, win);
    GLXContext ctx = glXCreateNewContext(dpy, configs[0], GLX_RGBA_TYPE, NULL, True);
    glXMakeContextCurrent(dpy, win, win, ctx);
    printf("%s / %s / GLSL %s\n", glGetString(GL_VENDOR), glGetString(GL_VERSION), glGetString(GL_SHADING_LANGUAGE_VERSION));

    // A shader that doesn't compile says where.
    GLuint broken = shader(GL_FRAGMENT_SHADER, "#version 330 core\nout vec4 c;\nvoid main() {\n  c = vec4(nothing);\n}\n", 0);
    char log[256] = {0};
    GLint length = 0;
    glGetShaderiv(broken, GL_INFO_LOG_LENGTH, &length);
    glGetShaderInfoLog(broken, sizeof log, NULL, log);
    check(strstr(log, "0:4:") != NULL && length > 0, "the broken shader's log names line 4", length, 1);
    GLuint bad = glCreateProgram();
    glAttachShader(bad, broken);
    glLinkProgram(bad);
    GLint linked = 1;
    glGetProgramiv(bad, GL_LINK_STATUS, &linked);
    check_eq("a program of it doesn't link", linked, 0);
    glDeleteProgram(bad);
    glDeleteShader(broken);

    GLuint vs = shader(GL_VERTEX_SHADER, VS, 1), fs = shader(GL_FRAGMENT_SHADER, FS, 1);
    GLuint program = glCreateProgram();
    glAttachShader(program, vs);
    glAttachShader(program, fs);
    glBindAttribLocation(program, 3, "uv");
    glLinkProgram(program);
    glGetProgramiv(program, GL_LINK_STATUS, &linked);
    check_eq("the program links", linked, 1);
    if (!linked) {
        glGetProgramInfoLog(program, sizeof log, NULL, log);
        printf("%s", log);
        return 1;
    }
    check_eq("pos is at its layout's location", glGetAttribLocation(program, "pos"), 0);
    check_eq("uv is where it was bound", glGetAttribLocation(program, "uv"), 3);
    GLint transform = glGetUniformLocation(program, "transform"), tint = glGetUniformLocation(program, "tint"), tex = glGetUniformLocation(program, "tex");
    check(transform >= 0 && tint >= 0 && tex >= 0, "the uniforms have locations", transform, 0);
    check_eq("an unknown uniform has none", glGetUniformLocation(program, "missing"), -1);
    GLint active = 0;
    glGetProgramiv(program, GL_ACTIVE_UNIFORMS, &active);
    check_eq("three active uniforms", active, 3);
    glUseProgram(program);

    // The quad: a vertex buffer of positions and coordinates, and an element buffer of its two triangles.
    float vertices[] = {-1, -1, 0, 0, 1, -1, 1, 0, 1, 1, 1, 1, -1, 1, 0, 1};
    unsigned short indices[] = {0, 1, 2, 0, 2, 3};
    GLuint vao, buffers[2];
    glGenVertexArrays(1, &vao);
    glBindVertexArray(vao);
    glGenBuffers(2, buffers);
    glBindBuffer(GL_ARRAY_BUFFER, buffers[0]);
    glBufferData(GL_ARRAY_BUFFER, sizeof vertices, vertices, GL_STATIC_DRAW);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, buffers[1]);
    glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof indices, indices, GL_STATIC_DRAW);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 16, (void *)0);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(3, 2, GL_FLOAT, GL_FALSE, 16, (void *)8);
    glEnableVertexAttribArray(3);

    // The texture: red, green / blue, white (rows lowest first).
    unsigned char texels[] = {255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255};
    GLuint texture;
    glGenTextures(1, &texture);
    glActiveTexture(GL_TEXTURE1);
    glBindTexture(GL_TEXTURE_2D, texture);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glUniform1i(tex, 1);

    // Half the window's size, in its middle, tinted half-strength green.
    float m[16] = {0.5f, 0, 0, 0, 0, 0.5f, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1};
    glUniformMatrix4fv(transform, 1, GL_FALSE, m);
    glUniform4f(tint, 1.0f, 0.5f, 1.0f, 1.0f);
    glClearColor(0.1f, 0.2f, 0.3f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, 0);
    check_eq("no error after drawing", glGetError(), GL_NO_ERROR);
    static unsigned char px[N * N * 4];
    glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px);
    pixel(px, 2, 2, 26, 51, 77, "outside the quad, the clear colour");
    pixel(px, 20, 20, 255, 0, 0, "the lower left texel");
    pixel(px, 40, 20, 0, 128, 0, "the lower right, tinted");
    pixel(px, 20, 40, 0, 0, 255, "the upper left");
    pixel(px, 40, 40, 255, 128, 255, "the upper right, tinted");

    // Again with blending: the quad at half alpha over what's there.
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glUniform4f(tint, 0.0f, 0.0f, 0.0f, 0.5f);
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, 0);
    glDisable(GL_BLEND);
    glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px);
    pixel(px, 20, 20, 128, 0, 0, "the lower left texel, half black over itself");
    pixel(px, 2, 2, 26, 51, 77, "outside, still the clear colour");

    // The first frame again for the window, swapped.
    glUniform4f(tint, 1.0f, 1.0f, 1.0f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, 0);
    glXSwapBuffers(dpy, win);
    check_eq("no error at the end", glGetError(), GL_NO_ERROR);
    printf("%d checks, %d failures\n", checks, failures);
    if (hold) sleep(hold);
    glXMakeContextCurrent(dpy, None, None, NULL);
    glXDestroyContext(dpy, ctx);
    XCloseDisplay(dpy);
    return failures != 0;
}
