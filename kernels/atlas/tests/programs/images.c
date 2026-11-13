// Bitmaps and icons, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it, with images.rc.
//
// Its resources (made by hand, in images.bmp, images.ico and images.cur): a 4 by 2 bitmap, its top row red, green,
// blue and white and its bottom black, grey, yellow and cyan; an icon of two images, 16 by 16 in 32 bits (its left
// half opaque red, its right half transparent by alpha) and 32 by 32 in 16 colours (all blue, its top left 8 by 8
// masked out); and a cursor of the 16 by 16 image.
//
// It checks: LoadBitmapW and GetObjectW; a memory DC with the bitmap selected, read with GetPixel and drawn into;
// BitBlt and StretchBlt between it and a window, both ways, and BLACKNESS; CreateCompatibleBitmap; LoadImageW
// choosing an icon's image by size (and scaling one), and DrawIconEx and DrawIcon blending by alpha and by mask;
// LoadCursorW; a bitmap from a file (LR_LOADFROMFILE), scaled; a resource that isn't there, and a system icon; a static
// showing an icon its template names and a bitmap STM_SETIMAGE gives; and an image list cutting a bitmap into images
// transparent where a colour says, adding an icon, and drawing them.

typedef void *HANDLE;
typedef void *HWND;
typedef void *HDC;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned int UINT;
typedef __UINTPTR_TYPE__ WPARAM;
typedef __INTPTR_TYPE__ LPARAM;
typedef __INTPTR_TYPE__ LRESULT;
typedef __INTPTR_TYPE__ INT_PTR;
typedef struct { int left, top, right, bottom; } RECT;
typedef struct { HWND hwnd; UINT message; WPARAM wParam; LPARAM lParam; DWORD time; int x, y; } MSG;
typedef LRESULT (__stdcall *WNDPROC)(HWND, UINT, WPARAM, LPARAM);
typedef INT_PTR (__stdcall *DLGPROC)(HWND, UINT, WPARAM, LPARAM);
typedef struct {
    UINT cbSize, style;
    WNDPROC lpfnWndProc;
    int cbClsExtra, cbWndExtra;
    HANDLE hInstance, hIcon, hCursor, hbrBackground;
    const WCHAR *lpszMenuName, *lpszClassName;
    HANDLE hIconSm;
} WNDCLASSEXW;
typedef struct { int bmType, bmWidth, bmHeight, bmWidthBytes; WORD bmPlanes, bmBitsPixel; void *bmBits; } BITMAP;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, void *, DWORD, DWORD, HANDLE);
K int __stdcall CloseHandle(HANDLE);
K DWORD __stdcall GetTempPathW(DWORD, WCHAR *);
K WORD __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, WPARAM, LPARAM);
K int __stdcall DestroyWindow(HWND);
K int __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K LRESULT __stdcall SendMessageW(HWND, UINT, WPARAM, LPARAM);
K HDC __stdcall GetDC(HWND);
K int __stdcall ReleaseDC(HWND, HDC);
K int __stdcall FillRect(HDC, const RECT *, HANDLE);
K HWND __stdcall CreateDialogIndirectParamW(HANDLE, const void *, HWND, DLGPROC, LPARAM);
K HWND __stdcall GetDlgItem(HWND, int);
K HANDLE __stdcall LoadBitmapW(HANDLE, const WCHAR *);
K HANDLE __stdcall LoadIconW(HANDLE, const WCHAR *);
K HANDLE __stdcall LoadCursorW(HANDLE, const WCHAR *);
K HANDLE __stdcall LoadImageW(HANDLE, const WCHAR *, UINT, int, int, UINT);
K int __stdcall DrawIconEx(HDC, int, int, HANDLE, int, int, UINT, HANDLE, UINT);
K int __stdcall DrawIcon(HDC, int, int, HANDLE);
K int __stdcall DestroyIcon(HANDLE);
K DWORD __stdcall GetPixel(HDC, int, int);
K HANDLE __stdcall CreateSolidBrush(DWORD);
K HANDLE __stdcall SelectObject(HDC, HANDLE);
K int __stdcall DeleteObject(HANDLE);
K HDC __stdcall CreateCompatibleDC(HDC);
K int __stdcall DeleteDC(HDC);
K HANDLE __stdcall CreateCompatibleBitmap(HDC, int, int);
K int __stdcall GetObjectW(HANDLE, int, void *);
K int __stdcall BitBlt(HDC, int, int, int, int, HDC, int, int, DWORD);
K int __stdcall StretchBlt(HDC, int, int, int, int, HDC, int, int, int, int, DWORD);
K HANDLE __stdcall ImageList_Create(int, int, UINT, int, int);
K int __stdcall ImageList_AddMasked(HANDLE, HANDLE, DWORD);
K int __stdcall ImageList_ReplaceIcon(HANDLE, int, HANDLE);
K int __stdcall ImageList_GetImageCount(HANDLE);
K int __stdcall ImageList_Draw(HANDLE, int, HDC, int, int, UINT);
K int __stdcall ImageList_Destroy(HANDLE);

#define WS_POPUP 0x80000000u
#define WS_CHILD 0x40000000u
#define WS_VISIBLE 0x10000000u
#define WS_CAPTION 0x00c00000u
#define DS_SETFONT 0x40
#define SRCCOPY 0x00cc0020
#define BLACKNESS 0x00000042
#define IMAGE_BITMAP 0
#define IMAGE_ICON 1
#define LR_LOADFROMFILE 0x10
#define STM_SETIMAGE 0x172
#define RES(n) ((const WCHAR *)(__UINTPTR_TYPE__)(n))
#define RGB(r, g, b) ((DWORD)(r) | (DWORD)(g) << 8 | (DWORD)(b) << 16)
#define RED RGB(255, 0, 0)
#define BLUE RGB(0, 0, 255)
#define CYAN RGB(0, 255, 255)
#define YELLOW RGB(255, 255, 0)
#define WHITE RGB(255, 255, 255)
#define FACE RGB(240, 240, 240)

static HANDLE out;
static int failures;
static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { DWORD n; WriteFile(out, s, len(s), &n, 0); }
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static WCHAR wbuf[4][300];
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }

static void pump(void) {
    MSG msg;
    for (int i = 0; i < 200 && PeekMessageW(&msg, 0, 0, 0, 1); i++) DispatchMessageW(&msg);
}

// images.bmp, as a file to load: 4 by 2, 24 bits.
static const unsigned char bmp_file[78] = {
    'B', 'M', 78, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0,
    40, 0, 0, 0, 4, 0, 0, 0, 2, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 128, 128, 128, 0, 255, 255, 255, 255, 0,
    0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255,
};

static __attribute__((aligned(4))) WORD tmpl[128];
static int tn;
static void w16(unsigned v) { tmpl[tn++] = (WORD)v; }
static void w32(DWORD v) { w16(v & 0xffff); w16(v >> 16); }
static void wstr(const char *s) { while (*s) w16((unsigned char)*s++); w16(0); }
static INT_PTR __stdcall dlg_proc(HWND h, UINT m, WPARAM w, LPARAM l) { (void)h; (void)w; (void)l; return m == 0x110; }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("bitmaps and icons on atlas, arm64\n");
#elif defined(__i386__)
    say("bitmaps and icons on atlas, x86 (WoW64)\n");
#else
    say("bitmaps and icons on atlas, x86-64\n");
#endif
    HANDLE instance = GetModuleHandleW(0);
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    wc.lpfnWndProc = DefWindowProcW;
    wc.hInstance = instance;
    wc.hbrBackground = (HANDLE)16; // COLOR_BTNFACE + 1
    wc.lpszClassName = W(0, "AtlasImages");
    RegisterClassExW(&wc);
    HWND wnd = CreateWindowExW(0, W(0, "AtlasImages"), W(1, "images"), WS_POPUP | WS_VISIBLE, 50, 50, 240, 160, 0, 0, instance, 0);
    pump();
    HDC dc = GetDC(wnd);

    // ---- A bitmap, and a memory DC ----
    HANDLE bitmap = LoadBitmapW(instance, RES(1));
    BITMAP bm = {0};
    check(bitmap && GetObjectW(bitmap, sizeof bm, &bm) == (int)sizeof bm && bm.bmWidth == 4 && bm.bmHeight == 2, "LoadBitmapW, and GetObjectW's size");
    HDC mem = CreateCompatibleDC(dc);
    HANDLE before = SelectObject(mem, bitmap);
    check(mem && before, "CreateCompatibleDC, and the bitmap selected into it");
    check(GetPixel(mem, 0, 0) == RED && GetPixel(mem, 2, 0) == BLUE && GetPixel(mem, 3, 1) == CYAN && GetPixel(mem, 1, 1) == RGB(128, 128, 128),
          "its pixels, top row first from a bottom-up DIB");
    BitBlt(dc, 10, 10, 4, 2, mem, 0, 0, SRCCOPY);
    check(GetPixel(dc, 10, 10) == RED && GetPixel(dc, 13, 11) == CYAN && GetPixel(dc, 14, 10) == FACE, "BitBlt into a window");
    StretchBlt(dc, 20, 10, 8, 4, mem, 0, 0, 4, 2, SRCCOPY);
    check(GetPixel(dc, 21, 11) == RED && GetPixel(dc, 27, 13) == CYAN && GetPixel(dc, 24, 12) == YELLOW, "StretchBlt, twice the size");
    RECT r = {0, 0, 1, 1};
    HANDLE blue = CreateSolidBrush(BLUE);
    FillRect(mem, &r, blue);
    check(GetPixel(mem, 0, 0) == BLUE && GetPixel(mem, 1, 0) == RGB(0, 255, 0), "FillRect into the bitmap");
    HANDLE copy = CreateCompatibleBitmap(dc, 4, 2);
    SelectObject(mem, copy);
    BitBlt(mem, 0, 0, 4, 2, dc, 10, 10, SRCCOPY);
    check(GetPixel(mem, 0, 0) == RED && GetPixel(mem, 3, 1) == CYAN, "CreateCompatibleBitmap, and BitBlt from a window into it");
    BitBlt(dc, 40, 10, 3, 3, 0, 0, 0, BLACKNESS);
    check(GetPixel(dc, 41, 11) == 0 && GetPixel(dc, 43, 11) == FACE, "BLACKNESS");
    SelectObject(mem, before);
    check(DeleteObject(copy) && DeleteDC(mem), "DeleteObject and DeleteDC");

    // ---- Icons ----
    HANDLE small = LoadImageW(instance, RES(2), IMAGE_ICON, 16, 16, 0);
    check(small && DrawIconEx(dc, 60, 10, small, 16, 16, 0, 0, 3), "LoadImageW: an icon at 16 by 16, and DrawIconEx");
    check(GetPixel(dc, 61, 11) == RED && GetPixel(dc, 70, 11) == FACE, "the 32-bit image, blended by its alpha");
    HANDLE large = LoadIconW(instance, RES(2));
    check(large && DrawIcon(dc, 100, 10, large), "LoadIconW: its 32 by 32 image, and DrawIcon");
    check(GetPixel(dc, 101, 11) == FACE && GetPixel(dc, 120, 30) == BLUE, "the 16-colour image, by its mask");
    HANDLE big = LoadImageW(instance, RES(2), IMAGE_ICON, 48, 48, 0);
    DrawIconEx(dc, 140, 10, big, 0, 0, 0, 0, 3);
    // Its masked 8 by 8 corner is 12 by 12 at 48.
    check(GetPixel(dc, 140 + 10, 10 + 10) == FACE && GetPixel(dc, 140 + 14, 10 + 14) == BLUE, "a size it hasn't, scaled from the nearest");
    check(LoadCursorW(instance, RES(3)) != 0, "LoadCursorW: a cursor");
    check(LoadBitmapW(instance, RES(99)) == 0 && LoadIconW(instance, RES(99)) == 0, "what isn't there: NULL");
    check(LoadIconW(0, RES(32512)) != 0, "a system icon");

    // ---- A bitmap from a file ----
    WCHAR path[300];
    GetTempPathW(260, path);
    int n = 0;
    while (path[n]) n++;
    const char *name = "atlas.bmp";
    for (int i = 0; name[i]; i++) path[n++] = (WCHAR)name[i];
    path[n] = 0;
    HANDLE f = CreateFileW(path, 0x40000000, 0, 0, 2, 0x80, 0);
    DWORD wrote = 0;
    WriteFile(f, bmp_file, sizeof bmp_file, &wrote, 0);
    CloseHandle(f);
    HANDLE loaded = LoadImageW(0, path, IMAGE_BITMAP, 8, 4, LR_LOADFROMFILE);
    BITMAP lb = {0};
    GetObjectW(loaded, sizeof lb, &lb);
    mem = CreateCompatibleDC(dc);
    SelectObject(mem, loaded);
    check(loaded && lb.bmWidth == 8 && lb.bmHeight == 4 && GetPixel(mem, 7, 3) == CYAN && GetPixel(mem, 1, 1) == RED, "LoadImageW: LR_LOADFROMFILE, scaled to 8 by 4");
    DeleteDC(mem);

    // ---- Statics with pictures ----
    tn = 0;
    w32(WS_POPUP | WS_CAPTION | WS_VISIBLE | DS_SETFONT); w32(0); w16(2); w16(10); w16(10); w16(120); w16(60);
    w16(0); w16(0); wstr("Pictures"); w16(8); wstr("MS Shell Dlg");
    // An icon static naming icon 2, and a bitmap static.
    w32(WS_CHILD | WS_VISIBLE | 3); w32(0); w16(4); w16(4); w16(20); w16(20); w16(100);
    w16(0xffff); w16(0x82); w16(0xffff); w16(2); w16(0);
    if (tn & 1) w16(0);
    w32(WS_CHILD | WS_VISIBLE | 0xe); w32(0); w16(40); w16(4); w16(20); w16(20); w16(101);
    w16(0xffff); w16(0x82); w16(0); w16(0);
    HWND dlg = CreateDialogIndirectParamW(instance, tmpl, 0, dlg_proc, 0);
    HWND icon_static = GetDlgItem(dlg, 100), bitmap_static = GetDlgItem(dlg, 101);
    check(SendMessageW(bitmap_static, STM_SETIMAGE, IMAGE_BITMAP, (LPARAM)bitmap) == 0, "STM_SETIMAGE: no picture before");
    pump();
    HDC sdc = GetDC(icon_static);
    DWORD masked = GetPixel(sdc, 2, 2), shown = GetPixel(sdc, 20, 20);
    ReleaseDC(icon_static, sdc);
    sdc = GetDC(bitmap_static);
    DWORD corner = GetPixel(sdc, 0, 0), past = GetPixel(sdc, 10, 10);
    ReleaseDC(bitmap_static, sdc);
    check(masked == FACE && shown == BLUE, "SS_ICON: the icon its template names");
    check(corner == BLUE && past == FACE, "SS_BITMAP: the bitmap STM_SETIMAGE gave");
    DestroyWindow(dlg);

    // ---- Image lists ----
    HANDLE list = ImageList_Create(2, 2, 0x18 | 1, 0, 4);
    check(ImageList_AddMasked(list, bitmap, BLUE) == 0 && ImageList_GetImageCount(list) == 2, "ImageList_AddMasked: the bitmap cut into two");
    ImageList_Draw(list, 1, dc, 200, 10, 0);
    check(GetPixel(dc, 200, 10) == FACE && GetPixel(dc, 201, 10) == WHITE && GetPixel(dc, 200, 11) == YELLOW, "ImageList_Draw: transparent where it was blue");
    check(ImageList_ReplaceIcon(list, -1, small) == 2, "ImageList_ReplaceIcon: an icon added");
    ImageList_Draw(list, 2, dc, 210, 10, 0);
    check(GetPixel(dc, 210, 10) == RED, "and drawn, at the list's size");
    ImageList_Destroy(list);

    check(DestroyIcon(small) && DestroyIcon(large) && DeleteObject(bitmap), "DestroyIcon and DeleteObject");
    ReleaseDC(wnd, dc);
    DestroyWindow(wnd);
    ExitProcess(failures);
}
