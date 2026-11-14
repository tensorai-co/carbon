// The compatibility profile through Carbon's GL, the way Luanti's Irrlicht draws: fixed function in 2D (an
// orthographic projection, the program's own arrays of positions, BGRA colours and texture coordinates, a texture
// combined by GL_COMBINE's modulate, the alpha test), and a GLSL 1.20 program written with Luanti's own header
// (`gl_ModelViewProjectionMatrix`, `gl_Vertex`, `gl_Color`, `gl_MultiTexCoord0`, `gl_TextureMatrix[0]`) under a
// pushed, translated and popped model-view matrix. Every pixel that matters is read back.
//
// Built and run inside the machine, as gl.c is:
//
//   carbon cp kernels/lambda/tests/programs/gl gl:/root
//   carbon run --name gl -- sh -c 'cc -O2 -o /root/gl/compat /root/gl/compat.c -lGL -lX11 && __GLX_VENDOR_LIBRARY_NAME=carbon /root/gl/compat'
#define GL_GLEXT_PROTOTYPES
#include <GL/gl.h>
#include <GL/glext.h>
#include <GL/glx.h>
#include <X11/Xlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#define N 32

static int checks, failures;

static void pixel(const unsigned char *px, int x, int y, int r, int g, int b, const char *what) {
    const unsigned char *p = px + 4 * (y * N + x);
    checks++;
    if ((abs(p[0] - r) > 2 || abs(p[1] - g) > 2 || abs(p[2] - b) > 2) && failures++ < 20)
        printf("FAIL %s: (%d, %d) is %d,%d,%d, not %d,%d,%d\n", what, x, y, p[0], p[1], p[2], r, g, b);
}

// Luanti's header for the "opengl" driver, then a shader of its kind.
static const char *VS =
    "#version 120\n"
    "#define mWorldViewProj gl_ModelViewProjectionMatrix\n"
    "#define mTexture (gl_TextureMatrix[0])\n"
    "#define inVertexPosition gl_Vertex\n"
    "#define inVertexColor gl_Color\n"
    "#define inTexCoord0 gl_MultiTexCoord0\n"
    "varying vec4 varColor;\n"
    "varying vec2 uv;\n"
    "void main() {\n"
    "    gl_Position = mWorldViewProj * inVertexPosition;\n"
    "    varColor = inVertexColor;\n"
    "    uv = (mTexture * inTexCoord0).st;\n"
    "}\n";
static const char *FS =
    "#version 120\n"
    "uniform sampler2D baseTexture;\n"
    "varying vec4 varColor;\n"
    "varying vec2 uv;\n"
    "void main() { gl_FragColor = texture2D(baseTexture, uv) * varColor; }\n";

int main(int argc, char **argv) {
    int hold = argc > 1 ? atoi(argv[1]) : 0;
    Display *dpy = XOpenDisplay(NULL);
    int attrs[] = {GLX_RGBA, GLX_DOUBLEBUFFER, GLX_DEPTH_SIZE, 24, None};
    XVisualInfo *vi = glXChooseVisual(dpy, DefaultScreen(dpy), attrs);
    if (!vi) return 1;
    Window root = RootWindow(dpy, DefaultScreen(dpy));
    XSetWindowAttributes swa = {0};
    swa.colormap = XCreateColormap(dpy, root, vi->visual, AllocNone);
    Window win = XCreateWindow(dpy, root, 0, 0, N, N, 0, vi->depth, InputOutput, vi->visual, CWColormap, &swa);
    XStoreName(dpy, win, "carbon gl compat");
    XMapWindow(dpy, win);
    GLXContext ctx = glXCreateContext(dpy, vi, NULL, True);
    glXMakeCurrent(dpy, win, ctx);
    printf("%s\n", glGetString(GL_EXTENSIONS));

    // 2D, as Irrlicht sets it up: pixels, y up.
    glMatrixMode(GL_PROJECTION);
    glLoadIdentity();
    glOrtho(0, N, 0, N, -1, 1);
    glMatrixMode(GL_MODELVIEW);
    glLoadIdentity();
    glClearColor(0, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);

    // A white texture with a transparent quarter (its lower right texel).
    unsigned char texels[] = {255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 255, 255, 255};
    GLuint tex;
    glGenTextures(1, &tex);
    glBindTexture(GL_TEXTURE_2D, tex);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glEnable(GL_TEXTURE_2D);
    glTexEnvi(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_COMBINE);
    glTexEnvi(GL_TEXTURE_ENV, GL_COMBINE_RGB, GL_MODULATE);
    glTexEnvi(GL_TEXTURE_ENV, GL_SOURCE0_RGB, GL_TEXTURE);
    glTexEnvi(GL_TEXTURE_ENV, GL_SOURCE1_RGB, GL_PRIMARY_COLOR);
    glTexEnvi(GL_TEXTURE_ENV, GL_COMBINE_ALPHA, GL_MODULATE);
    glTexEnvi(GL_TEXTURE_ENV, GL_SOURCE0_ALPHA, GL_TEXTURE);
    glTexEnvi(GL_TEXTURE_ENV, GL_SOURCE1_ALPHA, GL_PRIMARY_COLOR);
    glEnable(GL_ALPHA_TEST);
    glAlphaFunc(GL_GREATER, 0.5f);

    // The left half: a quad whose colours are BGRA bytes (blue in memory first), orange-ish everywhere.
    float pos[] = {0, 0, 16, 0, 16, 32, 0, 32};
    float uv[] = {0, 0, 1, 0, 1, 1, 0, 1};
    unsigned char bgra[16];
    for (int i = 0; i < 4; i++) bgra[4 * i] = 0, bgra[4 * i + 1] = 128, bgra[4 * i + 2] = 255, bgra[4 * i + 3] = 255;
    glEnableClientState(GL_VERTEX_ARRAY);
    glVertexPointer(2, GL_FLOAT, 0, pos);
    glEnableClientState(GL_COLOR_ARRAY);
    glColorPointer(GL_BGRA, GL_UNSIGNED_BYTE, 0, bgra);
    glClientActiveTexture(GL_TEXTURE0);
    glEnableClientState(GL_TEXTURE_COORD_ARRAY);
    glTexCoordPointer(2, GL_FLOAT, 0, uv);
    glDrawArrays(GL_TRIANGLE_FAN, 0, 4);
    glDisableClientState(GL_COLOR_ARRAY);
    glDisable(GL_ALPHA_TEST);
    glDisable(GL_TEXTURE_2D);

    // The right half: Luanti's kind of shader, moved there by a pushed model-view matrix, tinted green.
    GLuint vs = glCreateShader(GL_VERTEX_SHADER), fs = glCreateShader(GL_FRAGMENT_SHADER);
    glShaderSource(vs, 1, &VS, NULL);
    glShaderSource(fs, 1, &FS, NULL);
    glCompileShader(vs);
    glCompileShader(fs);
    GLuint program = glCreateProgram();
    glAttachShader(program, vs);
    glAttachShader(program, fs);
    glLinkProgram(program);
    GLint linked = 0;
    glGetProgramiv(program, GL_LINK_STATUS, &linked);
    checks++;
    if (!linked) {
        char log[512];
        glGetProgramInfoLog(program, sizeof log, NULL, log);
        printf("FAIL the GLSL 1.20 program: %s\n", log);
        failures++;
    }
    glUseProgram(program);
    glUniform1i(glGetUniformLocation(program, "baseTexture"), 0);
    glPushMatrix();
    glTranslatef(16, 0, 0);
    glColor4f(0, 1, 0, 1);
    glEnable(GL_TEXTURE_2D);
    glDrawArrays(GL_TRIANGLE_FAN, 0, 4);
    glPopMatrix();
    glUseProgram(0);
    GLfloat mv[16];
    glGetFloatv(GL_MODELVIEW_MATRIX, mv);
    checks++;
    if (mv[12] != 0.0f && failures++ < 20) printf("FAIL the model-view matrix popped: %f\n", mv[12]);

    // glRectf in the current colour, with a colour array left on (as Irrlicht leaves it): immediate mode ignores it.
    glEnableClientState(GL_COLOR_ARRAY);
    glColor4ub(0, 0, 255, 255);
    glRectf(0, 28, 4, 32);
    glDisableClientState(GL_COLOR_ARRAY);

    static unsigned char px[N * N * 4];
    glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px);
    pixel(px, 4, 4, 255, 128, 0, "fixed function: texel times its colour");
    pixel(px, 12, 4, 0, 0, 0, "fixed function: the transparent texel, alpha tested away");
    pixel(px, 12, 20, 255, 128, 0, "fixed function: the upper right texel");
    pixel(px, 20, 4, 0, 255, 0, "GLSL 1.20: moved right, tinted green");
    pixel(px, 28, 4, 0, 255, 0, "GLSL 1.20: no alpha test (the texel is transparent)");
    pixel(px, 28, 28, 0, 255, 0, "GLSL 1.20: its top right");
    pixel(px, 2, 30, 0, 0, 255, "glRectf: the current colour, not the array's");
    pixel(px, 6, 30, 255, 128, 0, "glRectf: only its rectangle");
    checks++;
    if (glGetError() != GL_NO_ERROR && failures++ < 20) printf("FAIL an error\n");
    glXSwapBuffers(dpy, win);
    printf("%d checks, %d failures\n", checks, failures);
    if (hold) sleep(hold);
    return failures != 0;
}
