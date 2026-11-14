// A Direct3D 11 cube on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it; run it with a
// GPU (CARBON_GPU=software or host), or without one to check there's no Direct3D then.
//
// Three frames of a textured cube on a flip-model swap chain: each maps its constant buffer with WRITE_DISCARD for a
// new matrix, clears colour and depth (D24_UNORM_S8_UINT), and draws 36 indices (16-bit) with a 2×2 texture through
// a shader resource view and a point sampler, culling nothing, so the depth test is what hides the far faces. The
// last frame looks straight at the near face, which covers the middle half of the window: after Present, its four
// texels are its quadrants (GetPixel) and the clear colour is around them. Then ResizeBuffers and a new back buffer.

typedef __UINTPTR_TYPE__ u64;
typedef void *HANDLE;
typedef void *HWND;
typedef void *HDC;
typedef unsigned int DWORD;
typedef unsigned int UINT;
typedef int BOOL;
typedef unsigned short WCHAR;
typedef long HRESULT;
typedef __INTPTR_TYPE__ LRESULT;
typedef LRESULT (__stdcall *WNDPROC)(HWND, UINT, u64, __INTPTR_TYPE__);
typedef struct { UINT size, style; WNDPROC proc; int cls, wnd; HANDLE instance, icon, cursor, background; const WCHAR *menu, *name; HANDLE small; } WNDCLASSEXW;
typedef struct { unsigned int a; unsigned short b, c; unsigned char d[8]; } GUID;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K unsigned short __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, u64, __INTPTR_TYPE__);
K BOOL __stdcall ShowWindow(HWND, int);
K BOOL __stdcall AdjustWindowRectEx(int *, DWORD, BOOL, DWORD);
K HDC __stdcall GetDC(HWND);
K DWORD __stdcall GetPixel(HDC, int, int);
K HRESULT __stdcall D3D11CreateDeviceAndSwapChain(void *, int, HANDLE, UINT, const UINT *, UINT, UINT, const void *, void **, void **, UINT *, void **);

#include "d3d11_shaders.h"

static int failures;

// What MSVC's linker looks for in a program that uses floating point; there's no C runtime to have it.
int _fltused;

void *memset(void *at, int value, __SIZE_TYPE__ n) {
    volatile unsigned char *p = at;
    while (n--) *p++ = (unsigned char)value;
    return at;
}

static void out(const char *s) {
    DWORD n = 0, w;
    while (s[n]) n++;
    WriteFile(GetStdHandle((DWORD)-11), s, n, &w, 0);
}

static void hex(unsigned v) {
    char b[11] = "0x";
    for (int i = 0; i < 8; i++) b[2 + i] = "0123456789abcdef"[(v >> (28 - 4 * i)) & 15];
    b[10] = 0;
    out(b);
}

static void check(int ok, const char *what) {
    out(ok ? "ok    " : "FAIL  ");
    out(what);
    out("\n");
    if (!ok) failures++;
}

#define M(obj, n, type) ((type)((*(void ***)(obj))[n]))
#define Release(obj) M(obj, 2, UINT (__stdcall *)(void *))(obj)

static const GUID IID_ID3D11Texture2D = {0x6f15aaf2, 0xd208, 0x4e89, {0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c}};

typedef struct { UINT width, height, mips, array, format, samples, quality, usage, bind, cpu, misc; } TEXTURE2D_DESC;
typedef struct { UINT size, usage, bind, cpu, misc, stride; } BUFFER_DESC;
typedef struct { const void *bytes; UINT pitch, slice; } SUBRESOURCE_DATA;
typedef struct { void *data; UINT row, depth; } MAPPED;
typedef struct { const char *name; UINT index, format, slot, offset, cls, step; } INPUT_ELEMENT;
typedef struct { float x, y, w, h, min, max; } VIEWPORT;
typedef struct { UINT filter, u, v, w; float bias; UINT aniso, cmp; float border[4], min, max; } SAMPLER_DESC;
typedef struct { UINT fill, cull; BOOL ccw; int bias; float clamp, slope; BOOL clip, scissor, ms, aa; } RASTERIZER_DESC;
typedef struct {
    UINT width, height, num, den, format, scanline, scaling, samples, quality, usage, count;
    HWND window;
    BOOL windowed;
    UINT effect, flags;
} SWAP_CHAIN_DESC;

static LRESULT __stdcall proc(HWND w, UINT m, u64 wp, __INTPTR_TYPE__ lp) { return DefWindowProcW(w, m, wp, lp); }

typedef struct { float x, y, z, u, v; } VERTEX;

void mainCRTStartup(void) {
    static const WCHAR cls[] = {'c', 'u', 'b', 'e', 0};
    WNDCLASSEXW wc = {sizeof wc, 0, proc, 0, 0, 0, 0, 0, 0, 0, cls, 0};
    RegisterClassExW(&wc);
    int r[4] = {0, 0, 64, 64};
    AdjustWindowRectEx(r, 0x00cf0000, 0, 0);
    HWND window = CreateWindowExW(0, cls, cls, 0x00cf0000, 20, 20, r[2] - r[0], r[3] - r[1], 0, 0, 0, 0);
    ShowWindow(window, 1);
    // A flip-model swap chain of two buffers, its size the window's (0).
    SWAP_CHAIN_DESC sd = {0, 0, 60, 1, 87, 0, 0, 1, 0, 0x20, 2, window, 1, 4, 0};
    void *swap = 0, *device = 0, *context = 0;
    HRESULT hr = D3D11CreateDeviceAndSwapChain(0, 1, 0, 0, 0, 0, 7, &sd, &swap, &device, 0, &context);
    if (hr == (HRESULT)0x887a0004) {
        check(!device && !swap, "no GPU: D3D11CreateDeviceAndSwapChain is DXGI_ERROR_UNSUPPORTED (run with CARBON_GPU for the rest)");
        ExitProcess(failures);
    }
    check(hr == 0 && swap && device && context, "D3D11CreateDeviceAndSwapChain, flip model, sized by the window");
    if (hr) ExitProcess(failures);

    void *back = 0, *rtv = 0, *zbuf = 0, *dsv = 0;
    M(swap, 9, HRESULT (__stdcall *)(void *, UINT, const GUID *, void **))(swap, 0, &IID_ID3D11Texture2D, &back);
    TEXTURE2D_DESC bd = {0};
    M(back, 10, void (__stdcall *)(void *, void *))(back, &bd);
    check(bd.width == 64 && bd.height == 64, "the back buffer is the window's client area's size");
    M(device, 9, HRESULT (__stdcall *)(void *, void *, const void *, void **))(device, back, 0, &rtv);
    TEXTURE2D_DESC zd = {64, 64, 1, 1, 45, 1, 0, 0, 0x40, 0, 0};
    hr = M(device, 5, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &zd, 0, &zbuf);
    HRESULT hr2 = zbuf ? M(device, 10, HRESULT (__stdcall *)(void *, void *, const void *, void **))(device, zbuf, 0, &dsv) : 1;
    check(hr == 0 && hr2 == 0 && dsv, "a D24_UNORM_S8_UINT texture and its depth-stencil view");

    // Red, green, blue and white texels.
    static const unsigned texels[4] = {0xff0000ff, 0xff00ff00, 0xffff0000, 0xffffffff};
    TEXTURE2D_DESC td = {2, 2, 1, 1, 28, 1, 0, 1, 8, 0, 0};
    SUBRESOURCE_DATA tdata = {texels, 8, 0};
    void *tex = 0, *srv = 0, *sampler = 0, *raster = 0;
    hr = M(device, 5, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &td, &tdata, &tex);
    hr2 = tex ? M(device, 7, HRESULT (__stdcall *)(void *, void *, const void *, void **))(device, tex, 0, &srv) : 1;
    SAMPLER_DESC smp = {0, 3, 3, 3, 0, 1, 1, {0, 0, 0, 0}, 0, 1000};
    HRESULT hr3 = M(device, 23, HRESULT (__stdcall *)(void *, const void *, void **))(device, &smp, &sampler);
    RASTERIZER_DESC rd = {3, 1, 0, 0, 0, 0, 1, 0, 0, 0};
    HRESULT hr4 = M(device, 22, HRESULT (__stdcall *)(void *, const void *, void **))(device, &rd, &raster);
    check(hr == 0 && hr2 == 0 && hr3 == 0 && hr4 == 0, "an immutable texture, its view, a point sampler, a rasterizer state culling nothing");

    // The cube: four corners a face, each face's texture coordinates from its top left as it's seen.
    VERTEX v[24];
    unsigned short idx[36];
    static const float faces[6][4][3] = {
        {{-1, 1, -1}, {1, 1, -1}, {1, -1, -1}, {-1, -1, -1}},
        {{1, 1, 1}, {-1, 1, 1}, {-1, -1, 1}, {1, -1, 1}},
        {{-1, 1, 1}, {-1, 1, -1}, {-1, -1, -1}, {-1, -1, 1}},
        {{1, 1, -1}, {1, 1, 1}, {1, -1, 1}, {1, -1, -1}},
        {{-1, 1, 1}, {1, 1, 1}, {1, 1, -1}, {-1, 1, -1}},
        {{-1, -1, -1}, {1, -1, -1}, {1, -1, 1}, {-1, -1, 1}},
    };
    static const float uv[4][2] = {{0, 0}, {1, 0}, {1, 1}, {0, 1}};
    for (int f = 0; f < 6; f++) {
        for (int c = 0; c < 4; c++) {
            VERTEX x = {faces[f][c][0], faces[f][c][1], faces[f][c][2], uv[c][0], uv[c][1]};
            v[f * 4 + c] = x;
        }
        unsigned short q[6] = {0, 1, 2, 0, 2, 3};
        for (int i = 0; i < 6; i++) idx[f * 6 + i] = (unsigned short)(f * 4 + q[i]);
    }
    BUFFER_DESC vbd = {sizeof v, 1, 1, 0, 0, 0}, ibd = {sizeof idx, 1, 2, 0, 0, 0}, cbd = {64, 2, 4, 0x10000, 0, 0};
    SUBRESOURCE_DATA vdata = {v, 0, 0}, idata = {idx, 0, 0};
    void *vb = 0, *ib = 0, *cb = 0, *vs = 0, *ps = 0, *layout = 0;
    M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &vbd, &vdata, &vb);
    M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &ibd, &idata, &ib);
    M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &cbd, 0, &cb);
    M(device, 12, HRESULT (__stdcall *)(void *, const void *, u64, void *, void **))(device, cube_vs, sizeof cube_vs, 0, &vs);
    M(device, 15, HRESULT (__stdcall *)(void *, const void *, u64, void *, void **))(device, cube_ps, sizeof cube_ps, 0, &ps);
    INPUT_ELEMENT elements[] = {{"POSITION", 0, 6, 0, 0, 0, 0}, {"TEXCOORD", 0, 16, 0, 12, 0, 0}};
    M(device, 11, HRESULT (__stdcall *)(void *, const void *, UINT, const void *, u64, void **))(device, elements, 2, cube_vs, sizeof cube_vs, &layout);
    check(vb && ib && cb && vs && ps && layout, "the cube's buffers, shaders and input layout");

    // Three frames: turned a quarter, an eighth, then straight on. Each scales by a half, and takes z from [-1, 1] to
    // [0.25, 0.75].
    static const float c[3] = {0, 0.70710678f, 1}, s[3] = {1, 0.70710678f, 0};
    UINT stride = sizeof(VERTEX), offset = 0;
    VIEWPORT vp = {0, 0, 64, 64, 0, 1};
    float clear[4] = {0.25f, 0.25f, 0.25f, 1};
    int frames = 0;
    for (int f = 0; f < 3; f++) {
        MAPPED m = {0};
        hr = M(context, 14, HRESULT (__stdcall *)(void *, void *, UINT, UINT, UINT, MAPPED *))(context, cb, 0, 4, 0, &m);
        if (hr || !m.data) break;
        float *mvp = m.data;
        float rows[16] = {0.5f * c[f], 0, 0.25f * s[f], 0, 0, 0.5f, 0, 0, -0.5f * s[f], 0, 0.25f * c[f], 0, 0, 0, 0.5f, 1};
        for (int i = 0; i < 16; i++) mvp[i] = rows[i];
        M(context, 15, void (__stdcall *)(void *, void *, UINT))(context, cb, 0);
        M(context, 50, void (__stdcall *)(void *, void *, const float *))(context, rtv, clear);
        M(context, 53, void (__stdcall *)(void *, void *, UINT, float, unsigned char))(context, dsv, 1, 1.0f, 0);
        M(context, 33, void (__stdcall *)(void *, UINT, void **, void *))(context, 1, &rtv, dsv);
        M(context, 44, void (__stdcall *)(void *, UINT, const VIEWPORT *))(context, 1, &vp);
        M(context, 43, void (__stdcall *)(void *, void *))(context, raster);
        M(context, 17, void (__stdcall *)(void *, void *))(context, layout);
        M(context, 18, void (__stdcall *)(void *, UINT, UINT, void **, const UINT *, const UINT *))(context, 0, 1, &vb, &stride, &offset);
        M(context, 19, void (__stdcall *)(void *, void *, UINT, UINT))(context, ib, 57, 0);
        M(context, 24, void (__stdcall *)(void *, UINT))(context, 4);
        M(context, 11, void (__stdcall *)(void *, void *, void **, UINT))(context, vs, 0, 0);
        M(context, 7, void (__stdcall *)(void *, UINT, UINT, void **))(context, 0, 1, &cb);
        M(context, 9, void (__stdcall *)(void *, void *, void **, UINT))(context, ps, 0, 0);
        M(context, 8, void (__stdcall *)(void *, UINT, UINT, void **))(context, 0, 1, &srv);
        M(context, 10, void (__stdcall *)(void *, UINT, UINT, void **))(context, 0, 1, &sampler);
        M(context, 12, void (__stdcall *)(void *, UINT, UINT, int))(context, 36, 0, 0);
        if (M(swap, 8, HRESULT (__stdcall *)(void *, UINT, UINT))(swap, 1, 0) == 0) frames++;
    }
    check(frames == 3, "three frames mapped, drawn and presented");

    // GetPixel's COLORREFs: red 0x0000ff, green 0x00ff00, blue 0xff0000.
    HDC dc = GetDC(window);
    DWORD q[4] = {GetPixel(dc, 24, 24), GetPixel(dc, 40, 24), GetPixel(dc, 24, 40), GetPixel(dc, 40, 40)};
    DWORD around = GetPixel(dc, 4, 4);
    int quadrants = q[0] == 0x0000ff && q[1] == 0x00ff00 && q[2] == 0xff0000 && q[3] == 0xffffff;
    check(quadrants, "the near face's four texels in its quadrants, the far ones hidden by depth");
    if (!quadrants) {
        for (int i = 0; i < 4; i++) { hex(q[i]); out(" "); }
        out("\n");
    }
    check(around == 0x404040, "the clear colour around the cube");

    // A new size: the old back buffer let go first, as Direct3D asks.
    M(context, 33, void (__stdcall *)(void *, UINT, void **, void *))(context, 0, 0, 0);
    Release(rtv);
    Release(back);
    hr = M(swap, 13, HRESULT (__stdcall *)(void *, UINT, UINT, UINT, UINT, UINT))(swap, 2, 32, 16, 0, 0);
    back = 0;
    M(swap, 9, HRESULT (__stdcall *)(void *, UINT, const GUID *, void **))(swap, 0, &IID_ID3D11Texture2D, &back);
    TEXTURE2D_DESC nd = {0};
    if (back) M(back, 10, void (__stdcall *)(void *, void *))(back, &nd);
    check(hr == 0 && back && nd.width == 32 && nd.height == 16 && nd.format == 87, "ResizeBuffers: a 32×16 back buffer, its format kept");

    Release(back);
    Release(swap);
    Release(context);
    Release(device);
    ExitProcess(failures);
}
