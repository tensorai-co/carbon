// comctl32's controls, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it.
//
// It checks: a progress bar in a dialog (its range, position, delta and step, the range read back, clamping, its bar's
// colour), and what it painted, read back with GetPixel; a list view in report view (columns, items inserted where
// asked, their text in each column and their lParam, LVM_GETITEM, selection with LVM_SETITEMSTATE and the
// LVN_ITEMCHANGED its parent hears, single selection, the arrow keys, LVM_GETNEXTITEM, a column sized to its text,
// LVM_ENSUREVISIBLE scrolling, deleting items), its image lists, and its procedure called through CallWindowProc; and
// image lists themselves (made, added to, counted, sized, removed from, destroyed).

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
typedef struct { UINT mask; int iItem, iSubItem; UINT state, stateMask; WCHAR *pszText; int cchTextMax, iImage; LPARAM lParam; int iIndent; } LVITEMW;
typedef struct { UINT mask; int fmt, cx; WCHAR *pszText; int cchTextMax, iSubItem; } LVCOLUMNW;
typedef struct { HWND hwndFrom; __UINTPTR_TYPE__ idFrom; UINT code; } NMHDR;
typedef struct { NMHDR hdr; int iItem, iSubItem; UINT uNewState, uOldState, uChanged; int x, y; LPARAM lParam; } NMLISTVIEW;
typedef struct { int low, high; } PBRANGE;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K int __stdcall GetClassInfoExW(HANDLE, const WCHAR *, WNDCLASSEXW *);
K int __stdcall DestroyWindow(HWND);
K int __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K int __stdcall IsDialogMessageW(HWND, MSG *);
K int __stdcall TranslateMessage(const MSG *);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K LRESULT __stdcall SendMessageW(HWND, UINT, WPARAM, LPARAM);
K LRESULT __stdcall CallWindowProcW(WNDPROC, HWND, UINT, WPARAM, LPARAM);
K HDC __stdcall GetDC(HWND);
K int __stdcall ReleaseDC(HWND, HDC);
K DWORD __stdcall GetPixel(HDC, int, int);
K int __stdcall GetClientRect(HWND, RECT *);
K HWND __stdcall SetFocus(HWND);
K HWND __stdcall CreateDialogIndirectParamW(HANDLE, const void *, HWND, DLGPROC, LPARAM);
K HWND __stdcall GetDlgItem(HWND, int);
K void __stdcall InitCommonControls(void);
K HANDLE __stdcall ImageList_Create(int, int, UINT, int, int);
K int __stdcall ImageList_Destroy(HANDLE);
K int __stdcall ImageList_AddMasked(HANDLE, HANDLE, DWORD);
K int __stdcall ImageList_ReplaceIcon(HANDLE, int, HANDLE);
K int __stdcall ImageList_GetImageCount(HANDLE);
K int __stdcall ImageList_GetIconSize(HANDLE, int *, int *);
K int __stdcall ImageList_Remove(HANDLE, int);
K HANDLE __stdcall CreateCompatibleBitmap(HDC, int, int);

#define WM_NOTIFY 0x4e
#define WM_INITDIALOG 0x110
#define WM_KEYDOWN 0x100
#define WS_POPUP 0x80000000u
#define WS_CHILD 0x40000000u
#define WS_VISIBLE 0x10000000u
#define WS_CAPTION 0x00c00000u
#define WS_BORDER 0x00800000u
#define WS_TABSTOP 0x00010000u
#define DS_SETFONT 0x40
#define PBM_SETRANGE32 0x406
#define PBM_GETRANGE 0x407
#define PBM_SETPOS 0x402
#define PBM_DELTAPOS 0x403
#define PBM_SETSTEP 0x404
#define PBM_STEPIT 0x405
#define PBM_GETPOS 0x408
#define PBM_SETBARCOLOR 0x409
#define LVM_SETIMAGELIST 0x1003
#define LVM_GETITEMCOUNT 0x1004
#define LVM_DELETEITEM 0x1008
#define LVM_DELETEALLITEMS 0x1009
#define LVM_GETNEXTITEM 0x100c
#define LVM_ENSUREVISIBLE 0x1013
#define LVM_GETCOLUMNWIDTH 0x101d
#define LVM_SETCOLUMNWIDTH 0x101e
#define LVM_GETTOPINDEX 0x1027
#define LVM_GETCOUNTPERPAGE 0x1028
#define LVM_SETITEMSTATE 0x102b
#define LVM_GETSELECTEDCOUNT 0x1032
#define LVM_SETEXTENDEDLISTVIEWSTYLE 0x1036
#define LVM_GETITEMW 0x104b
#define LVM_INSERTITEMW 0x104d
#define LVM_INSERTCOLUMNW 0x1061
#define LVM_GETITEMTEXTW 0x1073
#define LVM_SETITEMTEXTW 0x1074
#define LVIF_TEXT 1
#define LVIF_PARAM 4
#define LVIF_STATE 8
#define LVIS_FOCUSED 1
#define LVIS_SELECTED 2
#define LVNI_SELECTED 2
#define LVCF_WIDTH 2
#define LVCF_TEXT 4
#define LVN_ITEMCHANGED (-101)
#define VK_DOWN 0x28
#define RGB(r, g, b) ((DWORD)(r) | (DWORD)(g) << 8 | (DWORD)(b) << 16)

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
static WCHAR wbuf[4][64];
static WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }

static __attribute__((aligned(4))) WORD tmpl[256];
static int tn;
static void w16(unsigned v) { tmpl[tn++] = (WORD)v; }
static void w32(DWORD v) { w16(v & 0xffff); w16(v >> 16); }
static void wstr(const char *s) { while (*s) w16((unsigned char)*s++); w16(0); }
static void item(DWORD style, int x, int y, int cx, int cy, int id, const char *cls) {
    if (tn & 1) w16(0);
    w32(style | WS_CHILD | WS_VISIBLE); w32(0); w16(x); w16(y); w16(cx); w16(cy); w16(id);
    wstr(cls); w16(0); w16(0);
}

static HWND dlg;
static int changed, last_item, selected_now;
static INT_PTR __stdcall dlg_proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    (void)h; (void)w;
    if (m == WM_INITDIALOG) return 1;
    if (m == WM_NOTIFY) {
        NMLISTVIEW *n = (NMLISTVIEW *)l;
        if ((int)n->hdr.code == LVN_ITEMCHANGED && n->hdr.idFrom == 200) {
            changed++;
            last_item = n->iItem;
            selected_now = (n->uNewState & LVIS_SELECTED) != 0;
        }
        return 1;
    }
    return 0;
}

static void pump(void) {
    MSG msg;
    for (int i = 0; i < 200 && PeekMessageW(&msg, 0, 0, 0, 1); i++) {
        if (!IsDialogMessageW(dlg, &msg)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

static LRESULT insert(HWND list, int at, const char *text, LPARAM param) {
    LVITEMW it = {0};
    it.mask = LVIF_TEXT | LVIF_PARAM;
    it.iItem = at;
    it.pszText = W(3, text);
    it.lParam = param;
    return SendMessageW(list, LVM_INSERTITEMW, 0, (LPARAM)&it);
}

static void select_item(HWND list, int at) {
    LVITEMW it = {0};
    it.state = it.stateMask = LVIS_SELECTED | LVIS_FOCUSED;
    SendMessageW(list, LVM_SETITEMSTATE, (WPARAM)at, (LPARAM)&it);
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("comctl32's controls on atlas, arm64\n");
#elif defined(__i386__)
    say("comctl32's controls on atlas, x86 (WoW64)\n");
#else
    say("comctl32's controls on atlas, x86-64\n");
#endif
    InitCommonControls();
    HANDLE instance = GetModuleHandleW(0);
    WCHAR text[64];

    // A dialog with a progress bar and a report list view, single selection.
    tn = 0;
    w32(WS_POPUP | WS_CAPTION | WS_VISIBLE | DS_SETFONT); w32(0); w16(2); w16(10); w16(10); w16(220); w16(150);
    w16(0); w16(0); wstr("Progress"); w16(8); wstr("MS Shell Dlg");
    item(1, 10, 10, 200, 10, 100, "msctls_progress32");
    item(WS_BORDER | WS_TABSTOP | 0x1 | 0x4, 10, 30, 200, 100, 200, "SysListView32");
    dlg = CreateDialogIndirectParamW(instance, tmpl, 0, dlg_proc, 0);
    HWND bar = GetDlgItem(dlg, 100), list = GetDlgItem(dlg, 200);
    check(dlg && bar && list, "a dialog with a progress bar and a list view, by their class names");

    // ---- The progress bar ----
    check(SendMessageW(bar, PBM_SETRANGE32, 0, 200) == (100 << 16), "PBM_SETRANGE32: the range before, 0 to 100");
    check(SendMessageW(bar, PBM_SETPOS, 50, 0) == 0 && SendMessageW(bar, PBM_GETPOS, 0, 0) == 50, "PBM_SETPOS and PBM_GETPOS");
    check(SendMessageW(bar, PBM_DELTAPOS, 10, 0) == 50 && SendMessageW(bar, PBM_GETPOS, 0, 0) == 60, "PBM_DELTAPOS");
    SendMessageW(bar, PBM_SETSTEP, 20, 0);
    check(SendMessageW(bar, PBM_STEPIT, 0, 0) == 60 && SendMessageW(bar, PBM_GETPOS, 0, 0) == 80, "PBM_SETSTEP and PBM_STEPIT");
    PBRANGE range = {0};
    check(SendMessageW(bar, PBM_GETRANGE, 1, (LPARAM)&range) == 0 && range.low == 0 && range.high == 200, "PBM_GETRANGE");
    SendMessageW(bar, PBM_SETPOS, 1000, 0);
    check(SendMessageW(bar, PBM_GETPOS, 0, 0) == 200, "a position past the range is its top");
    check(SendMessageW(bar, PBM_SETBARCOLOR, 0, RGB(255, 0, 0)) == (LRESULT)0xff000000, "PBM_SETBARCOLOR: CLR_DEFAULT before");
    SendMessageW(bar, PBM_SETPOS, 100, 0);
    pump();
    RECT r;
    GetClientRect(bar, &r);
    HDC dc = GetDC(bar);
    DWORD done = GetPixel(dc, r.right / 4, r.bottom / 2), left = GetPixel(dc, r.right * 3 / 4, r.bottom / 2);
    ReleaseDC(bar, dc);
    check(done == RGB(255, 0, 0) && left == RGB(230, 230, 230), "painted: half filled in its bar's colour, the rest its track");

    // ---- The list view ----
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    check(GetClassInfoExW(0, W(0, "SysListView32"), &wc) && wc.lpfnWndProc, "GetClassInfoExW: the list view's class");
    LVCOLUMNW col = {0};
    col.mask = LVCF_TEXT | LVCF_WIDTH;
    col.cx = 100;
    col.pszText = W(0, "Name");
    check(SendMessageW(list, LVM_INSERTCOLUMNW, 0, (LPARAM)&col) == 0, "LVM_INSERTCOLUMNW: the first");
    col.pszText = W(0, "Size");
    check(SendMessageW(list, LVM_INSERTCOLUMNW, 1, (LPARAM)&col) == 1, "and a second");
    SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, 0x20, 0x20);
    check(insert(list, 0, "two", 2) == 0 && insert(list, 0, "one", 1) == 0 && insert(list, 99, "three", 3) == 2, "LVM_INSERTITEMW, where it's asked, or at the end");
    check(SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 3, "LVM_GETITEMCOUNT");
    LVITEMW it = {0};
    it.iSubItem = 1;
    it.pszText = W(1, "12 KB");
    check(SendMessageW(list, LVM_SETITEMTEXTW, 1, (LPARAM)&it), "LVM_SETITEMTEXTW: a second column's text");
    it.pszText = text;
    it.cchTextMax = 64;
    it.iSubItem = 1;
    check(SendMessageW(list, LVM_GETITEMTEXTW, 1, (LPARAM)&it) == 5 && weq(text, "12 KB"), "LVM_GETITEMTEXTW: and back");
    it.iSubItem = 0;
    check(SendMessageW(list, LVM_GETITEMTEXTW, 2, (LPARAM)&it) == 5 && weq(text, "three"), "an item's own text");
    LVITEMW got = {0};
    got.mask = LVIF_PARAM | LVIF_TEXT;
    got.iItem = 1;
    got.pszText = text;
    got.cchTextMax = 64;
    check(SendMessageW(list, LVM_GETITEMW, 0, (LPARAM)&got) && got.lParam == 2 && weq(text, "two"), "LVM_GETITEMW: its lParam and text");

    select_item(list, 1);
    check(changed >= 1 && last_item == 1 && selected_now, "LVM_SETITEMSTATE: selected, and LVN_ITEMCHANGED to its parent");
    check(SendMessageW(list, LVM_GETNEXTITEM, (WPARAM)-1, LVNI_SELECTED) == 1 && SendMessageW(list, LVM_GETSELECTEDCOUNT, 0, 0) == 1, "LVM_GETNEXTITEM and LVM_GETSELECTEDCOUNT");
    select_item(list, 2);
    check(SendMessageW(list, LVM_GETNEXTITEM, (WPARAM)-1, LVNI_SELECTED) == 2 && SendMessageW(list, LVM_GETSELECTEDCOUNT, 0, 0) == 1, "single selection: one at a time");
    SetFocus(list);
    select_item(list, 0);
    SendMessageW(list, WM_KEYDOWN, VK_DOWN, 1);
    check(SendMessageW(list, LVM_GETNEXTITEM, (WPARAM)-1, LVNI_SELECTED) == 1, "the down arrow: the next");
    SendMessageW(list, LVM_SETCOLUMNWIDTH, 0, (LPARAM)-1);
    check(SendMessageW(list, LVM_GETCOLUMNWIDTH, 0, 0) == 5 * 6 + 12, "LVSCW_AUTOSIZE: its widest text's width");
    WNDPROC proc = wc.lpfnWndProc;
    check(CallWindowProcW(proc, list, LVM_GETITEMCOUNT, 0, 0) == 3, "its procedure, through CallWindowProcW");
    check(SendMessageW(list, LVM_DELETEITEM, 0, 0) && SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 2, "LVM_DELETEITEM");
    for (int i = 0; i < 50; i++) insert(list, 1000, "line", i);
    int last = (int)SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) - 1;
    SendMessageW(list, LVM_ENSUREVISIBLE, (WPARAM)last, 0);
    int top = (int)SendMessageW(list, LVM_GETTOPINDEX, 0, 0), page = (int)SendMessageW(list, LVM_GETCOUNTPERPAGE, 0, 0);
    check(top > 0 && page > 0 && last >= top && last < top + page, "LVM_ENSUREVISIBLE scrolls the last into view");
    pump();
    GetClientRect(list, &r);
    dc = GetDC(list);
    DWORD background = GetPixel(dc, r.right - 3, r.bottom - 3);
    ReleaseDC(list, dc);
    check(background == RGB(255, 255, 255), "painted on its background");

    // ---- Image lists ----
    HANDLE images = ImageList_Create(16, 16, 0x20, 4, 4);
    int cx = 0, cy = 0;
    HANDLE strip = CreateCompatibleBitmap(0, 32, 16);
    check(images != 0 && ImageList_AddMasked(images, strip, 0) == 0 && ImageList_GetImageCount(images) == 2, "ImageList_Create, and ImageList_AddMasked cutting a bitmap into two");
    check(ImageList_ReplaceIcon(images, -1, (HANDLE)1) == 2 && ImageList_GetImageCount(images) == 3, "ImageList_ReplaceIcon adding one, and the count");
    check(ImageList_GetIconSize(images, &cx, &cy) && cx == 16 && cy == 16, "ImageList_GetIconSize");
    check(ImageList_Remove(images, 0) && ImageList_GetImageCount(images) == 2, "ImageList_Remove");
    check(SendMessageW(list, LVM_SETIMAGELIST, 1, (LPARAM)images) == 0 && SendMessageW(list, LVM_SETIMAGELIST, 1, 0) == (LRESULT)images, "LVM_SETIMAGELIST: the list before");
    check(ImageList_Destroy(images) && !ImageList_Destroy(images), "ImageList_Destroy, once");

    check(SendMessageW(list, LVM_DELETEALLITEMS, 0, 0) && SendMessageW(list, LVM_GETITEMCOUNT, 0, 0) == 0, "LVM_DELETEALLITEMS");
    DestroyWindow(dlg);
    ExitProcess(failures);
}
