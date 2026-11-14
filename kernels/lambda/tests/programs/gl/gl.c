// OpenGL through Carbon's own GL vendor library, on the machine's GPU: GLVND loads libGLX_carbon for the screen, a
// framebuffer config is chosen, a window and a GLX window made on it, and a context made current. Then what GL does
// so far is checked pixel for pixel: the first bind's viewport and scissor, clears of the whole window, scissored,
// and through a colour mask, read back with glReadPixels as RGBA, BGRA and packed RGB rows; the gets of state, and
// errors set and cleared. A last frame (blue left, green right) is swapped into the window, and left up for a while
// when a number of seconds is given, for `carbon display screenshot`.
//
// It's a real program, linked with the image's GLVND, so it's built inside the machine:
//
//   carbon run --name gl --gpu host debian:13 -- apt-get install -y gcc libc6-dev libgl-dev libx11-dev
//   carbon cp kernels/lambda/tests/programs/gl gl:/root
//   carbon run --name gl -- sh -c 'cc -O2 -o /root/gl/gl /root/gl/gl.c -lGL -lX11 && __GLX_VENDOR_LIBRARY_NAME=carbon /root/gl/gl'
//   carbon run --name gl --gpu software -- env __GLX_VENDOR_LIBRARY_NAME=carbon /root/gl/gl
#include <GL/gl.h>
#include <GL/glx.h>
#include <X11/Xlib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define W 64
#define H 32

static int checks, failures;

static void check(int ok, const char *what, long got, long want) {
    checks++;
    if (!ok && failures++ < 20) printf("FAIL %s: %ld, not %ld\n", what, got, want);
}

static void check_eq(const char *what, long got, long want) { check(got == want, what, got, want); }

// Every pixel of a w×h RGBA read is `want`; the first that isn't is said.
static void check_pixels(const char *what, const unsigned char *px, int n, const unsigned char want[4]) {
    for (int i = 0; i < n; i++) {
        if (memcmp(px + 4 * i, want, 4) != 0) {
            check(0, what, i, -1);
            printf("     pixel %d is %u,%u,%u,%u\n", i, px[4 * i], px[4 * i + 1], px[4 * i + 2], px[4 * i + 3]);
            return;
        }
    }
    check(1, what, 0, 0);
}

int main(int argc, char **argv) {
    int hold = argc > 1 ? atoi(argv[1]) : 0;
    Display *dpy = XOpenDisplay(NULL);
    if (!dpy) {
        printf("FAIL no display\n");
        return 1;
    }
    int screen = DefaultScreen(dpy);
    int attrs[] = {GLX_RENDER_TYPE, GLX_RGBA_BIT, GLX_DRAWABLE_TYPE, GLX_WINDOW_BIT, GLX_RED_SIZE, 8, GLX_GREEN_SIZE, 8,
                   GLX_BLUE_SIZE, 8, GLX_DEPTH_SIZE, 24, GLX_DOUBLEBUFFER, True, None};
    int n = 0;
    GLXFBConfig *configs = glXChooseFBConfig(dpy, screen, attrs, &n);
    check(n > 0, "framebuffer configs chosen", n, 1);
    if (n == 0) return 1;
    int depth = 0, doubled = 0;
    glXGetFBConfigAttrib(dpy, configs[0], GLX_DEPTH_SIZE, &depth);
    glXGetFBConfigAttrib(dpy, configs[0], GLX_DOUBLEBUFFER, &doubled);
    check(depth >= 24 && doubled, "the chosen config's depth and double buffer", depth, 24);
    XVisualInfo *vi = glXGetVisualFromFBConfig(dpy, configs[0]);
    check(vi != NULL, "the config's visual", 0, 0);
    if (!vi) return 1;
    Window root = RootWindow(dpy, screen);
    XSetWindowAttributes swa = {0};
    swa.colormap = XCreateColormap(dpy, root, vi->visual, AllocNone);
    Window win = XCreateWindow(dpy, root, 0, 0, W, H, 0, vi->depth, InputOutput, vi->visual, CWColormap, &swa);
    XStoreName(dpy, win, "carbon gl");
    XMapWindow(dpy, win);
    GLXWindow glxwin = glXCreateWindow(dpy, configs[0], win, NULL);
    GLXContext ctx = glXCreateNewContext(dpy, configs[0], GLX_RGBA_TYPE, NULL, True);
    check(ctx != NULL, "a context", 0, 0);
    if (!ctx) return 1;
    check_eq("made current", glXMakeContextCurrent(dpy, glxwin, glxwin, ctx), True);
    check_eq("direct", glXIsDirect(dpy, ctx), True);

    const char *vendor = (const char *)glGetString(GL_VENDOR);
    check(vendor && strcmp(vendor, "Carbon") == 0, "GL_VENDOR is Carbon's (set __GLX_VENDOR_LIBRARY_NAME=carbon)", 0, 0);
    printf("%s / %s / %s\n", vendor, glGetString(GL_RENDERER), glGetString(GL_VERSION));

    // The first bind: the viewport and scissor are the window's.
    GLint box[4];
    glGetIntegerv(GL_VIEWPORT, box);
    check(box[0] == 0 && box[1] == 0 && box[2] == W && box[3] == H, "the first viewport's width", box[2], W);
    glGetIntegerv(GL_SCISSOR_BOX, box);
    check_eq("the first scissor's height", box[3], H);
    GLint bits;
    glGetIntegerv(GL_RED_BITS, &bits);
    check_eq("red bits", bits, 8);
    glGetIntegerv(GL_DEPTH_BITS, &bits);
    check_eq("depth bits", bits, 24);
    check_eq("no error yet", glGetError(), GL_NO_ERROR);

    // A whole clear, read back.
    static unsigned char px[W * H * 4];
    const unsigned char red[4] = {255, 0, 0, 255}, yellow[4] = {255, 255, 0, 255}, blue[4] = {0, 0, 255, 255};
    glClearColor(1, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check_pixels("a whole clear, red", px, W * H, red);
    GLfloat clear[4];
    glGetFloatv(GL_COLOR_CLEAR_VALUE, clear);
    check(clear[0] == 1.0f && clear[1] == 0.0f, "the clear colour read back", (long)(clear[0] * 100), 100);

    // Scissored, and only green written: a yellow rectangle inside red.
    glEnable(GL_SCISSOR_TEST);
    GLboolean on = 0;
    glGetBooleanv(GL_SCISSOR_TEST, &on);
    check_eq("the scissor test is on", on, 1);
    check_eq("glIsEnabled says so", glIsEnabled(GL_SCISSOR_TEST), 1);
    glScissor(8, 4, 16, 8);
    glColorMask(GL_FALSE, GL_TRUE, GL_FALSE, GL_FALSE);
    glClearColor(0, 1, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
    glReadPixels(8, 4, 16, 8, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check_pixels("inside the scissor, red and green", px, 16 * 8, yellow);
    glReadPixels(0, 0, 8, 4, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check_pixels("outside it, red still", px, 8 * 4, red);
    glReadPixels(24, 12, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check_pixels("just past its corner, red still", px, 1, red);

    // BGRA, and packed RGB with skipped pixels and no row padding.
    glReadPixels(8, 4, 1, 1, GL_BGRA, GL_UNSIGNED_BYTE, px);
    const unsigned char bgra_yellow[4] = {0, 255, 255, 255};
    check_pixels("BGRA", px, 1, bgra_yellow);
    memset(px, 7, sizeof px);
    glPixelStorei(GL_PACK_ALIGNMENT, 1);
    glPixelStorei(GL_PACK_SKIP_PIXELS, 1);
    glReadPixels(7, 4, 3, 1, GL_RGB, GL_UNSIGNED_BYTE, px);
    const unsigned char rgb[13] = {7, 7, 7, 255, 0, 0, 255, 255, 0, 255, 255, 0, 7};
    check(memcmp(px, rgb, sizeof rgb) == 0, "packed RGB after a skipped pixel", px[3], 255);
    glPixelStorei(GL_PACK_SKIP_PIXELS, 0);
    glPixelStorei(GL_PACK_ALIGNMENT, 4);
    GLint alignment = 0;
    glGetIntegerv(GL_PACK_ALIGNMENT, &alignment);
    check_eq("the pack alignment read back", alignment, 4);

    // Errors: kept until read, then cleared.
    glEnable(0x1234);
    glClear(0x8);
    check_eq("an unknown capability is GL_INVALID_ENUM", glGetError(), GL_INVALID_ENUM);
    check_eq("read once, it's gone", glGetError(), GL_NO_ERROR);
    glViewport(0, 0, -1, 1);
    check_eq("a negative viewport is GL_INVALID_VALUE", glGetError(), GL_INVALID_VALUE);
    glPixelStorei(GL_PACK_ALIGNMENT, 3);
    check_eq("an alignment of 3 is GL_INVALID_VALUE", glGetError(), GL_INVALID_VALUE);
    check(glGetString(0x1234) == NULL, "an unknown string is NULL", 0, 0);
    check_eq("and GL_INVALID_ENUM", glGetError(), GL_INVALID_ENUM);

    // The last frame: blue on the left half, green on the right, swapped into the window.
    glScissor(0, 0, W / 2, H);
    glClearColor(0, 0, 1, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glScissor(W / 2, 0, W / 2, H);
    glClearColor(0, 1, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT);
    glDisable(GL_SCISSOR_TEST);
    glReadPixels(0, 0, W / 2, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    check_pixels("the frame's left half", px, W / 2 * H, blue);
    glXSwapBuffers(dpy, glxwin);
    check_eq("no error at the end", glGetError(), GL_NO_ERROR);
    printf("%d checks, %d failures\n", checks, failures);
    if (hold) sleep(hold);

    glXMakeContextCurrent(dpy, None, None, NULL);
    glXDestroyContext(dpy, ctx);
    glXDestroyWindow(dpy, glxwin);
    XFree(vi);
    XFree(configs);
    XCloseDisplay(dpy);
    return failures != 0;
}
