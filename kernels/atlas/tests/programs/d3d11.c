// DXGI and Direct3D 11 on atlas, for ARM64 and x86-64: no C runtime, COM called through vtables by their slots in
// d3d11.h and dxgi.h. tests/programs/build.sh builds it; run it with a GPU (CARBON_GPU=software or host), or without
// one to check there's no DXGI then.
//
// It checks: CreateDXGIFactory1, EnumAdapters1 and the adapter's DXGI_ADAPTER_DESC1 (the Carbon GPU's vendor ID), its
// output's description; D3D11CreateDeviceAndSwapChain on a window, the device as an IDXGIDevice and its adapter, and
// as an IDXGIDevice2, with WindowFromDC(GetDC(NULL)) the desktop window (what ANGLE checks before using it); the
// swap chain's back buffer and a render target view of it; a clear, then a triangle from HLSL (vkd3d's DXBC: a vertex
// buffer with an input layout, a dynamic constant buffer mapped with WRITE_DISCARD) drawn and presented; the back
// buffer read back through a staging texture, and the window's own pixels with GetPixel; a compute shader writing a
// raw buffer through a UAV, read back the same way; an event query.

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
K HWND __stdcall WindowFromDC(HDC);
K HWND __stdcall GetDesktopWindow(void);
K HRESULT __stdcall CreateDXGIFactory1(const GUID *, void **);
K HRESULT __stdcall D3D11CreateDeviceAndSwapChain(void *, int, HANDLE, UINT, const UINT *, UINT, UINT, const void *, void **, void **, UINT *, void **);

#include "d3d11_shaders.h"

// What MSVC's linker looks for in a program that uses floating point; there's no C runtime to have it (older clangs
// ask for it here too).
int _fltused;

static int failures;

// What the compiler calls to zero the structures here, as there's no C runtime.
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

// A COM method: slot `n` of the object's vtable, called with the object first.
#define M(obj, n, type) ((type)((*(void ***)(obj))[n]))
#define Release(obj) M(obj, 2, UINT (__stdcall *)(void *))(obj)

static const GUID IID_IDXGIFactory1 = {0x770aae78, 0xf26f, 0x4dba, {0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87}};
static const GUID IID_IDXGIDevice = {0x54ec77fa, 0x1377, 0x44e6, {0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c}};
static const GUID IID_IDXGIDevice2 = {0x05008617, 0xfbfd, 0x4051, {0xa7, 0x90, 0x14, 0x48, 0x84, 0xb4, 0xf6, 0xa9}};
static const GUID IID_ID3D11Texture2D = {0x6f15aaf2, 0xd208, 0x4e89, {0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c}};

typedef struct { UINT width, height, mips, array, format, samples, quality, usage, bind, cpu, misc; } TEXTURE2D_DESC;
typedef struct { UINT size, usage, bind, cpu, misc, stride; } BUFFER_DESC;
typedef struct { const void *bytes; UINT pitch, slice; } SUBRESOURCE_DATA;
typedef struct { void *data; UINT row, depth; } MAPPED;
typedef struct { const char *name; UINT index, format, slot, offset, cls, step; } INPUT_ELEMENT;
typedef struct { float x, y, w, h, min, max; } VIEWPORT;
typedef struct { UINT format, dimension, first, count, flags; } UAV_DESC;
typedef struct {
    UINT width, height, num, den, format, scanline, scaling, samples, quality, usage, count;
    HWND window;
    BOOL windowed;
    UINT effect, flags;
} SWAP_CHAIN_DESC;

static LRESULT __stdcall proc(HWND w, UINT m, u64 wp, __INTPTR_TYPE__ lp) { return DefWindowProcW(w, m, wp, lp); }

typedef HRESULT (__stdcall *create_fn)(void *, const void *, const void *, void **);

void mainCRTStartup(void) {
    // DXGI on its own: the adapter, and its output.
    void *factory = 0, *adapter = 0, *output = 0;
    HRESULT hr = CreateDXGIFactory1(&IID_IDXGIFactory1, &factory);
    if (hr == (HRESULT)0x887a0004) {
        // A machine without a GPU: no DXGI, as on a Windows without one, and nothing more to check.
        check(!factory, "no GPU: CreateDXGIFactory1 is DXGI_ERROR_UNSUPPORTED (run with CARBON_GPU for the rest)");
        ExitProcess(failures);
    }
    check(hr == 0 && factory, "CreateDXGIFactory1");
    if (!factory) ExitProcess(1);
    hr = M(factory, 12, HRESULT (__stdcall *)(void *, UINT, void **))(factory, 0, &adapter);
    void *none = (void *)1;
    HRESULT second = M(factory, 12, HRESULT (__stdcall *)(void *, UINT, void **))(factory, 1, &none);
    check(hr == 0 && adapter && second == (HRESULT)0x887a0002 && !none, "EnumAdapters1: one adapter, then DXGI_ERROR_NOT_FOUND");
    unsigned char desc[320] = {0};
    hr = M(adapter, 10, HRESULT (__stdcall *)(void *, void *))(adapter, desc);
    UINT vendor = *(UINT *)(desc + 256);
    check(hr == 0 && vendor == 0x1c0de && desc[0] != 0, "the adapter's DXGI_ADAPTER_DESC1: the Carbon GPU's vendor ID and a name");
    hr = M(adapter, 7, HRESULT (__stdcall *)(void *, UINT, void **))(adapter, 0, &output);
    unsigned char odesc[96] = {0};
    if (output) M(output, 7, HRESULT (__stdcall *)(void *, void *))(output, odesc);
    check(hr == 0 && output && *(int *)(odesc + 72) > 0 && *(BOOL *)(odesc + 80), "EnumOutputs: the screen, attached to the desktop");

    // A window, and a device with a swap chain on it.
    static const WCHAR cls[] = {'d', '3', 'd', 0};
    WNDCLASSEXW wc = {sizeof wc, 0, proc, 0, 0, 0, 0, 0, 0, 0, cls, 0};
    RegisterClassExW(&wc);
    int r[4] = {0, 0, 64, 64};
    AdjustWindowRectEx(r, 0x00cf0000, 0, 0);
    HWND window = CreateWindowExW(0, cls, cls, 0x00cf0000, 10, 10, r[2] - r[0], r[3] - r[1], 0, 0, 0, 0);
    ShowWindow(window, 1);
    SWAP_CHAIN_DESC sd = {64, 64, 60, 1, 87, 0, 0, 1, 0, 0x20, 1, window, 1, 0, 0};
    void *swap = 0, *device = 0, *context = 0;
    UINT level = 0;
    UINT levels[] = {0xb000};
    hr = D3D11CreateDeviceAndSwapChain(0, 1, 0, 0, levels, 1, 7, &sd, &swap, &device, &level, &context);
    check(hr == 0 && swap && device && context && level == 0xb000, "D3D11CreateDeviceAndSwapChain at feature level 11_0");
    if (hr) ExitProcess(failures);
    void *dxgi = 0, *parent = 0;
    hr = M(device, 0, HRESULT (__stdcall *)(void *, const GUID *, void **))(device, &IID_IDXGIDevice, &dxgi);
    if (dxgi) M(dxgi, 7, HRESULT (__stdcall *)(void *, void **))(dxgi, &parent);
    unsigned char pdesc[320] = {0};
    if (parent) M(parent, 8, HRESULT (__stdcall *)(void *, void *))(parent, pdesc);
    check(hr == 0 && dxgi != device && parent && *(UINT *)(pdesc + 256) == 0x1c0de, "the device as an IDXGIDevice, and its adapter");
    // What ANGLE asks before it uses a device: DXGI 1.2 (IDXGIDevice2) when the screen's DC's window, the desktop's,
    // isn't the program's own.
    void *dxgi2 = 0;
    hr = M(device, 0, HRESULT (__stdcall *)(void *, const GUID *, void **))(device, &IID_IDXGIDevice2, &dxgi2);
    check(hr == 0 && dxgi2 && WindowFromDC(GetDC(0)) == GetDesktopWindow(), "IDXGIDevice2, and the screen DC's window is the desktop's");

    void *back = 0, *rtv = 0;
    hr = M(swap, 9, HRESULT (__stdcall *)(void *, UINT, const GUID *, void **))(swap, 0, &IID_ID3D11Texture2D, &back);
    TEXTURE2D_DESC bd = {0};
    if (back) M(back, 10, void (__stdcall *)(void *, void *))(back, &bd);
    check(hr == 0 && back && bd.width == 64 && bd.height == 64 && bd.format == 87, "GetBuffer: a 64×64 BGRA texture");
    hr = M(device, 9, HRESULT (__stdcall *)(void *, void *, const void *, void **))(device, back, 0, &rtv);
    check(hr == 0 && rtv, "CreateRenderTargetView of the back buffer");

    // A triangle over the middle, white times the constant buffer's tint, on a blue clear.
    void *vs = 0, *ps = 0, *layout = 0, *vb = 0, *cb = 0;
    hr = M(device, 12, HRESULT (__stdcall *)(void *, const void *, u64, void *, void **))(device, color_vs, sizeof color_vs, 0, &vs);
    HRESULT hr2 = M(device, 15, HRESULT (__stdcall *)(void *, const void *, u64, void *, void **))(device, color_ps, sizeof color_ps, 0, &ps);
    check(hr == 0 && hr2 == 0 && vs && ps, "CreateVertexShader and CreatePixelShader from DXBC");
    INPUT_ELEMENT elements[] = {{"POSITION", 0, 6, 0, 0, 0, 0}, {"COLOR", 0, 28, 0, 12, 0, 0}};
    hr = M(device, 11, HRESULT (__stdcall *)(void *, const void *, UINT, const void *, u64, void **))(device, elements, 2, color_vs, sizeof color_vs, &layout);
    check(hr == 0 && layout, "CreateInputLayout");
    struct { float x, y, z; unsigned color; } tri[3] = {{-0.5f, -0.5f, 0, 0xffffffff}, {0.0f, 0.5f, 0, 0xffffffff}, {0.5f, -0.5f, 0, 0xffffffff}};
    BUFFER_DESC vbd = {sizeof tri, 0, 1, 0, 0, 0};
    SUBRESOURCE_DATA vdata = {tri, 0, 0};
    hr = M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &vbd, &vdata, &vb);
    BUFFER_DESC cbd = {32, 2, 4, 0x10000, 0, 0};
    hr2 = M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &cbd, 0, &cb);
    check(hr == 0 && hr2 == 0 && vb && cb, "CreateBuffer: a vertex buffer with data, a dynamic constant buffer");
    MAPPED m = {0};
    hr = M(context, 14, HRESULT (__stdcall *)(void *, void *, UINT, UINT, UINT, MAPPED *))(context, cb, 0, 4, 0, &m);
    if (m.data) {
        float *f = m.data;
        f[0] = 1; f[1] = 0; f[2] = 0; f[3] = 1; f[4] = 0; f[5] = 0; f[6] = 0; f[7] = 0;
    }
    M(context, 15, void (__stdcall *)(void *, void *, UINT))(context, cb, 0);
    check(hr == 0 && m.data, "Map with WRITE_DISCARD");
    float blue[4] = {0, 0, 1, 1};
    M(context, 50, void (__stdcall *)(void *, void *, const float *))(context, rtv, blue);
    M(context, 33, void (__stdcall *)(void *, UINT, void **, void *))(context, 1, &rtv, 0);
    VIEWPORT vp = {0, 0, 64, 64, 0, 1};
    M(context, 44, void (__stdcall *)(void *, UINT, const VIEWPORT *))(context, 1, &vp);
    M(context, 17, void (__stdcall *)(void *, void *))(context, layout);
    UINT stride = 16, offset = 0;
    M(context, 18, void (__stdcall *)(void *, UINT, UINT, void **, const UINT *, const UINT *))(context, 0, 1, &vb, &stride, &offset);
    M(context, 24, void (__stdcall *)(void *, UINT))(context, 4);
    M(context, 11, void (__stdcall *)(void *, void *, void **, UINT))(context, vs, 0, 0);
    M(context, 7, void (__stdcall *)(void *, UINT, UINT, void **))(context, 0, 1, &cb);
    M(context, 9, void (__stdcall *)(void *, void *, void **, UINT))(context, ps, 0, 0);
    M(context, 13, void (__stdcall *)(void *, UINT, UINT))(context, 3, 0);
    hr = M(swap, 8, HRESULT (__stdcall *)(void *, UINT, UINT))(swap, 0, 0);
    check(hr == 0, "Present");

    // The back buffer through a staging texture: red in the middle, blue in the corner.
    TEXTURE2D_DESC sdesc = {64, 64, 1, 1, 87, 1, 0, 3, 0, 0x20000, 0};
    void *staging = 0;
    hr = M(device, 5, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &sdesc, 0, &staging);
    M(context, 47, void (__stdcall *)(void *, void *, void *))(context, staging, back);
    MAPPED read = {0};
    HRESULT mapped = M(context, 14, HRESULT (__stdcall *)(void *, void *, UINT, UINT, UINT, MAPPED *))(context, staging, 0, 1, 0, &read);
    unsigned middle = 0, corner = 0;
    if (read.data) {
        middle = ((unsigned *)read.data)[32 * (read.row / 4) + 32];
        corner = ((unsigned *)read.data)[0];
    }
    M(context, 15, void (__stdcall *)(void *, void *, UINT))(context, staging, 0);
    check(hr == 0 && mapped == 0 && read.row == 256 && middle == 0xffff0000 && corner == 0xff0000ff, "the triangle and the clear, read back through a staging texture");
    if (middle != 0xffff0000 || corner != 0xff0000ff) {
        out("      middle "); hex(middle); out(", corner "); hex(corner); out("\n");
    }
    HDC dc = GetDC(window);
    DWORD shown = GetPixel(dc, 32, 32), shown_corner = GetPixel(dc, 0, 0);
    check(shown == 0x0000ff && shown_corner == 0xff0000, "Present put them in the window (GetPixel's COLORREFs)");

    // A compute shader into a raw buffer, read back through a staging one.
    void *cs = 0, *buffer = 0, *uav = 0, *readback = 0;
    hr = M(device, 18, HRESULT (__stdcall *)(void *, const void *, u64, void *, void **))(device, flow_cs, sizeof flow_cs, 0, &cs);
    BUFFER_DESC ubd = {512, 0, 0x80, 0, 0x20, 0};
    hr2 = M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &ubd, 0, &buffer);
    UAV_DESC ud = {1, 1, 0, 128, 1};
    HRESULT hr3 = buffer ? M(device, 8, HRESULT (__stdcall *)(void *, void *, const void *, void **))(device, buffer, &ud, &uav) : 1;
    check(hr == 0 && hr2 == 0 && hr3 == 0, "CreateComputeShader, a raw buffer and its UAV");
    M(context, 69, void (__stdcall *)(void *, void *, void **, UINT))(context, cs, 0, 0);
    M(context, 68, void (__stdcall *)(void *, UINT, UINT, void **, const UINT *))(context, 0, 1, &uav, 0);
    M(context, 41, void (__stdcall *)(void *, UINT, UINT, UINT))(context, 2, 1, 1);
    BUFFER_DESC rbd = {512, 3, 0, 0x20000, 0, 0};
    M(device, 3, HRESULT (__stdcall *)(void *, const void *, const void *, void **))(device, &rbd, 0, &readback);
    M(context, 47, void (__stdcall *)(void *, void *, void *))(context, readback, buffer);
    MAPPED rb = {0};
    M(context, 14, HRESULT (__stdcall *)(void *, void *, UINT, UINT, UINT, MAPPED *))(context, readback, 0, 1, 0, &rb);
    unsigned *words = rb.data;
    // flow.hlsl: thread 0 gives 7, thread 1 20, thread 63 0xdead.
    check(words && words[0] == 7 && words[1] == 20 && words[63] == 0xdead, "Dispatch: the compute shader's words");
    M(context, 15, void (__stdcall *)(void *, void *, UINT))(context, readback, 0);

    // An event query: done once it's ended.
    void *query = 0;
    UINT qd[2] = {0, 0};
    hr = M(device, 24, HRESULT (__stdcall *)(void *, const void *, void **))(device, qd, &query);
    BOOL done = 0;
    if (query) {
        M(context, 28, void (__stdcall *)(void *, void *))(context, query);
        hr2 = M(context, 29, HRESULT (__stdcall *)(void *, void *, void *, UINT, UINT))(context, query, &done, sizeof done, 0);
    }
    check(hr == 0 && hr2 == 0 && done, "an event query, done");

    Release(staging);
    Release(rtv);
    Release(back);
    Release(swap);
    Release(context);
    Release(device);
    Release(factory);
    ExitProcess(failures);
}
