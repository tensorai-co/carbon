// Dialogs and the standard controls, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it,
// with dialog.rc.
//
// It checks: the dialog class as GetClassInfoExW gives it, and a class of the program's own made from it; a dialog
// from a template in memory (DLGTEMPLATE) and from a resource (DLGTEMPLATEEX): its controls, their IDs and text,
// WM_INITDIALOG with its parameter and the focus it gives, DWLP_DLGPROC; SetDlgItemText, GetDlgItemText,
// SetDlgItemInt and GetDlgItemInt; check boxes and radio buttons (CheckDlgButton, CheckRadioButton, BM_CLICK, an
// automatic radio group), WM_COMMAND and EN_CHANGE; DM_GETDEFID; IsDialogMessage's Tab, Enter and Escape, and typing
// into an edit; EnableWindow; a control's procedure called through CallWindowProc; what the controls paint, read back
// with GetPixel, in a colour WM_CTLCOLORSTATIC gave; and modal dialogs, their owner disabled while they're up, ended
// by Enter and by WM_CLOSE.

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

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K HANDLE __stdcall GetModuleHandleW(const WCHAR *);
K int __stdcall GetClassInfoExW(HANDLE, const WCHAR *, WNDCLASSEXW *);
K WORD __stdcall RegisterClassExW(const WNDCLASSEXW *);
K HWND __stdcall CreateWindowExW(DWORD, const WCHAR *, const WCHAR *, DWORD, int, int, int, int, HWND, HANDLE, HANDLE, void *);
K LRESULT __stdcall DefWindowProcW(HWND, UINT, WPARAM, LPARAM);
K int __stdcall DestroyWindow(HWND);
K int __stdcall IsWindow(HWND);
K int __stdcall ShowWindow(HWND, int);
K HWND __stdcall GetParent(HWND);
K HWND __stdcall GetFocus(void);
K HWND __stdcall SetFocus(HWND);
K int __stdcall PeekMessageW(MSG *, HWND, UINT, UINT, UINT);
K int __stdcall TranslateMessage(const MSG *);
K LRESULT __stdcall DispatchMessageW(const MSG *);
K int __stdcall PostMessageW(HWND, UINT, WPARAM, LPARAM);
K LRESULT __stdcall SendMessageW(HWND, UINT, WPARAM, LPARAM);
K __INTPTR_TYPE__ __stdcall GetWindowLongPtrW(HWND, int);
K LRESULT __stdcall CallWindowProcW(WNDPROC, HWND, UINT, WPARAM, LPARAM);
K HDC __stdcall GetDC(HWND);
K int __stdcall ReleaseDC(HWND, HDC);
K DWORD __stdcall GetPixel(HDC, int, int);
K HANDLE __stdcall CreateSolidBrush(DWORD);
K HWND __stdcall CreateDialogIndirectParamW(HANDLE, const void *, HWND, DLGPROC, LPARAM);
K HWND __stdcall CreateDialogParamW(HANDLE, const WCHAR *, HWND, DLGPROC, LPARAM);
K INT_PTR __stdcall DialogBoxIndirectParamW(HANDLE, const void *, HWND, DLGPROC, LPARAM);
K INT_PTR __stdcall DialogBoxParamW(HANDLE, const WCHAR *, HWND, DLGPROC, LPARAM);
K int __stdcall EndDialog(HWND, INT_PTR);
K int __stdcall IsDialogMessageW(HWND, MSG *);
K HWND __stdcall GetDlgItem(HWND, int);
K int __stdcall GetDlgCtrlID(HWND);
K int __stdcall SetDlgItemTextW(HWND, int, const WCHAR *);
K UINT __stdcall GetDlgItemTextW(HWND, int, WCHAR *, int);
K int __stdcall SetDlgItemInt(HWND, int, UINT, int);
K UINT __stdcall GetDlgItemInt(HWND, int, int *, int);
K LRESULT __stdcall SendDlgItemMessageW(HWND, int, UINT, WPARAM, LPARAM);
K int __stdcall CheckDlgButton(HWND, int, UINT);
K UINT __stdcall IsDlgButtonChecked(HWND, int);
K int __stdcall CheckRadioButton(HWND, int, int, int);
K HWND __stdcall GetNextDlgTabItem(HWND, HWND, int);
K int __stdcall MapDialogRect(HWND, RECT *);
K int __stdcall EnableWindow(HWND, int);
K int __stdcall IsWindowEnabled(HWND);

#define WM_CLOSE 0x10
#define WM_KEYDOWN 0x100
#define WM_CHAR 0x102
#define WM_INITDIALOG 0x110
#define WM_COMMAND 0x111
#define WM_CTLCOLORSTATIC 0x138
#define BM_GETCHECK 0xf0
#define BM_SETCHECK 0xf1
#define BM_CLICK 0xf5
#define EM_SETSEL 0xb1
#define DM_GETDEFID 0x400
#define WS_POPUP 0x80000000u
#define WS_CHILD 0x40000000u
#define WS_VISIBLE 0x10000000u
#define WS_CAPTION 0x00c00000u
#define WS_SYSMENU 0x00080000u
#define WS_BORDER 0x00800000u
#define WS_GROUP 0x00020000u
#define WS_TABSTOP 0x00010000u
#define WS_OVERLAPPEDWINDOW 0x00cf0000u
#define DS_MODALFRAME 0x80
#define DS_SETFONT 0x40
#define VK_TAB 0x09
#define VK_RETURN 0x0d
#define VK_ESCAPE 0x1b
#define IDOK 1
#define IDCANCEL 2
#define RGB(r, g, b) ((DWORD)(r) | (DWORD)(g) << 8 | (DWORD)(b) << 16)
#define DWLP_DLGPROC ((int)sizeof(LRESULT))

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
static const WCHAR *W(int i, const char *s) { int n = 0; for (; s[n]; n++) wbuf[i][n] = (WCHAR)s[n]; wbuf[i][n] = 0; return wbuf[i]; }
static int weq(const WCHAR *a, const char *b) { while (*b && *a == (WCHAR)*b) { a++; b++; } return *a == 0 && *b == 0; }

// ---- A template in memory ----

static __attribute__((aligned(4))) WORD tmpl[512];
static int tn;
static void w16(unsigned v) { tmpl[tn++] = (WORD)v; }
static void w32(DWORD v) { w16(v & 0xffff); w16(v >> 16); }
static void wstr(const char *s) { while (*s) w16((unsigned char)*s++); w16(0); }
static void begin(DWORD style, int count, int cx, int cy, const char *cls, const char *title) {
    tn = 0;
    w32(style); w32(0); w16(count); w16(10); w16(10); w16(cx); w16(cy);
    w16(0);
    if (cls) wstr(cls); else w16(0);
    wstr(title);
    w16(8); wstr("MS Shell Dlg");
}
static void item(DWORD style, int x, int y, int cx, int cy, int id, int atom, const char *title) {
    if (tn & 1) w16(0);
    w32(style | WS_CHILD | WS_VISIBLE); w32(0); w16(x); w16(y); w16(cx); w16(cy); w16(id);
    w16(0xffff); w16(atom); wstr(title); w16(0);
}
static void controls(void) {
    item(0, 7, 7, 40, 10, 100, 0x82, "Name:");
    item(WS_BORDER | WS_TABSTOP | 0x80, 50, 6, 100, 12, 101, 0x81, "");
    item(3 | WS_TABSTOP, 7, 24, 80, 10, 102, 0x80, "&Remember");
    item(9 | WS_TABSTOP | WS_GROUP, 7, 38, 60, 10, 103, 0x80, "One");
    item(9, 7, 50, 60, 10, 104, 0x80, "Two");
    item(1 | WS_TABSTOP | WS_GROUP, 90, 96, 50, 14, IDOK, 0x80, "OK");
    item(WS_TABSTOP, 145, 96, 50, 14, IDCANCEL, 0x80, "Cancel");
}

// ---- The modeless dialog's procedure ----

static HWND dlg;
static LPARAM init_param;
static WPARAM init_focus;
static int changes, clicked_one, oks, cancels;
static HANDLE blue;

static INT_PTR __stdcall dlg_proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_INITDIALOG:
        init_param = l;
        init_focus = w;
        return 1;
    case WM_COMMAND:
        if ((w & 0xffff) == 101 && (w >> 16) == 0x300) changes++;
        if ((w & 0xffff) == 103 && (w >> 16) == 0) clicked_one++;
        if ((w & 0xffff) == IDOK) oks++;
        if ((w & 0xffff) == IDCANCEL) cancels++;
        return 1;
    case WM_CTLCOLORSTATIC:
        if ((HWND)l == GetDlgItem(h, 100)) return (INT_PTR)blue;
        return 0;
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

// ---- Modal dialogs ----

static HWND owner;
static int owner_disabled, modal_inits;
static LPARAM modal_param;

static INT_PTR __stdcall modal_proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
    case WM_INITDIALOG:
        modal_inits++;
        modal_param = l;
        owner_disabled = !IsWindowEnabled(owner);
        // Enter in its edit (the first), or the window closed.
        if (l == 7) PostMessageW(GetDlgItem(h, 101), WM_KEYDOWN, VK_RETURN, 1);
        else PostMessageW(h, WM_CLOSE, 0, 0);
        return 1;
    case WM_COMMAND:
        if ((w & 0xffff) == IDOK) { EndDialog(h, 99); return 1; }
        if ((w & 0xffff) == IDCANCEL) { EndDialog(h, IDCANCEL); return 1; }
        return 0;
    }
    return 0;
}

static INT_PTR __stdcall resource_proc(HWND h, UINT m, WPARAM w, LPARAM l) {
    (void)h; (void)w; (void)l;
    return m == WM_INITDIALOG;
}

static LRESULT __stdcall owner_proc(HWND h, UINT m, WPARAM w, LPARAM l) { return DefWindowProcW(h, m, w, l); }

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("dialogs and controls on atlas, arm64\n");
#elif defined(__i386__)
    say("dialogs and controls on atlas, x86 (WoW64)\n");
#else
    say("dialogs and controls on atlas, x86-64\n");
#endif
    HANDLE instance = GetModuleHandleW(0);
    WCHAR text[64];

    // ---- The dialog class, and one of the program's own from it ----
    WNDCLASSEXW wc = {0};
    wc.cbSize = sizeof wc;
    check(GetClassInfoExW(0, W(0, "#32770"), &wc) != 0 && wc.lpfnWndProc != 0 && wc.cbWndExtra == (sizeof(void *) == 8 ? 48 : 30),
          "GetClassInfoExW: the dialog class, with DLGWINDOWEXTRA");
    wc.lpszClassName = W(1, "AtlasDlg");
    wc.hInstance = instance;
    check(RegisterClassExW(&wc) != 0, "a class of its own from it");
    blue = CreateSolidBrush(RGB(0, 0, 255));

    // ---- A dialog from a template in memory, of that class ----
    begin(WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE | DS_MODALFRAME | DS_SETFONT, 7, 200, 120, "AtlasDlg", "Test dialog");
    controls();
    dlg = CreateDialogIndirectParamW(instance, tmpl, 0, dlg_proc, 1234);
    check(dlg != 0 && init_param == 1234, "CreateDialogIndirectParamW: WM_INITDIALOG with its parameter");
    HWND edit = GetDlgItem(dlg, 101), box = GetDlgItem(dlg, 102), ok_button = GetDlgItem(dlg, IDOK);
    check(edit && GetDlgCtrlID(edit) == 101 && GetParent(edit) == dlg && GetDlgItem(dlg, 999) == 0, "GetDlgItem and GetDlgCtrlID");
    check(init_focus == (WPARAM)edit && GetFocus() == edit, "the first control that takes Tab has the focus");
    check(GetWindowLongPtrW(dlg, DWLP_DLGPROC) == (__INTPTR_TYPE__)dlg_proc, "DWLP_DLGPROC is its procedure");
    GetDlgItemTextW(dlg, IDOK, text, 64);
    check(weq(text, "OK") && GetDlgItemTextW(dlg, 100, text, 64) == 5 && weq(text, "Name:"), "its controls' text, from the template");

    check(SetDlgItemTextW(dlg, 101, W(0, "hello")) && GetDlgItemTextW(dlg, 101, text, 64) == 5 && weq(text, "hello") && changes == 1,
          "SetDlgItemTextW and GetDlgItemTextW, and EN_CHANGE");
    int translated = 0;
    SetDlgItemInt(dlg, 101, (UINT)-42, 1);
    check((int)GetDlgItemInt(dlg, 101, &translated, 1) == -42 && translated, "SetDlgItemInt and GetDlgItemInt, signed");

    CheckDlgButton(dlg, 102, 1);
    check(IsDlgButtonChecked(dlg, 102) == 1, "CheckDlgButton");
    SendDlgItemMessageW(dlg, 102, BM_CLICK, 0, 0);
    check(IsDlgButtonChecked(dlg, 102) == 0, "BM_CLICK unchecks an automatic check box");
    CheckRadioButton(dlg, 103, 104, 104);
    check(IsDlgButtonChecked(dlg, 104) == 1 && IsDlgButtonChecked(dlg, 103) == 0, "CheckRadioButton");
    SendDlgItemMessageW(dlg, 103, BM_CLICK, 0, 0);
    check(IsDlgButtonChecked(dlg, 103) == 1 && IsDlgButtonChecked(dlg, 104) == 0 && clicked_one == 1,
          "an automatic radio button clicked: the others in its group not, and BN_CLICKED");
    WNDPROC button_proc = (WNDPROC)GetWindowLongPtrW(box, -4);
    CallWindowProcW(button_proc, box, BM_SETCHECK, 1, 0);
    check(button_proc != 0 && CallWindowProcW(button_proc, box, BM_GETCHECK, 0, 0) == 1, "a button's procedure, called through CallWindowProcW");
    check(SendMessageW(dlg, DM_GETDEFID, 0, 0) == (0x534b << 16 | IDOK), "DM_GETDEFID: the default push button");

    // ---- The keyboard, through IsDialogMessageW ----
    pump();
    PostMessageW(edit, WM_KEYDOWN, VK_TAB, 1);
    pump();
    check(GetFocus() == box, "Tab: the next control that takes it");
    PostMessageW(box, WM_KEYDOWN, VK_TAB, 1);
    pump();
    check(GetFocus() == GetDlgItem(dlg, 103) && GetNextDlgTabItem(dlg, GetFocus(), 0) == ok_button,
          "and the next, past a radio button that isn't a tab stop");
    SetFocus(edit);
    SendMessageW(edit, EM_SETSEL, 0, -1);
    PostMessageW(edit, WM_CHAR, 'a', 1);
    PostMessageW(edit, WM_CHAR, 'b', 1);
    PostMessageW(edit, WM_CHAR, 'c', 1);
    PostMessageW(edit, WM_CHAR, 8, 1);
    pump();
    GetDlgItemTextW(dlg, 101, text, 64);
    check(weq(text, "ab"), "typing over the selection, and a backspace");
    PostMessageW(edit, WM_KEYDOWN, VK_RETURN, 1);
    pump();
    check(oks == 1, "Enter: the default button's WM_COMMAND");
    PostMessageW(edit, WM_KEYDOWN, VK_ESCAPE, 1);
    pump();
    check(cancels == 1, "Escape: IDCANCEL's");
    check(EnableWindow(ok_button, 0) == 0 && !IsWindowEnabled(ok_button), "EnableWindow: disabled");
    PostMessageW(edit, WM_KEYDOWN, VK_RETURN, 1);
    pump();
    check(oks == 1, "Enter does nothing while the default button's disabled");
    check(EnableWindow(ok_button, 1) != 0 && IsWindowEnabled(ok_button), "and enabled again");
    pump();

    // ---- What it painted ----
    HDC dc = GetDC(dlg);
    RECT ok = {90, 96, 140, 110}, label = {7, 7, 47, 17}, field = {50, 6, 150, 18};
    MapDialogRect(dlg, &ok);
    MapDialogRect(dlg, &label);
    MapDialogRect(dlg, &field);
    DWORD face = GetPixel(dc, 2, 2), button = GetPixel(dc, ok.left + 3, ok.top + 3), behind = GetPixel(dc, label.right - 2, label.bottom - 2),
          inside = GetPixel(dc, field.right - 4, field.bottom - 4);
    ReleaseDC(dlg, dc);
    check(ok.left == 135 && ok.top == 156 && ok.right == 210, "MapDialogRect: dialog units to pixels");
    check(face == RGB(240, 240, 240), "the dialog's face");
    check(button == RGB(225, 225, 225), "a push button's face");
    check(behind == RGB(0, 0, 255), "a static on the brush WM_CTLCOLORSTATIC gave");
    check(inside == RGB(255, 255, 255), "an edit's background");
    DestroyWindow(dlg);
    check(!IsWindow(dlg) && !IsWindow(edit), "DestroyWindow: the dialog and its controls");

    // ---- A dialog from a resource (DLGTEMPLATEEX) ----
    HWND from = CreateDialogParamW(instance, (const WCHAR *)(__UINTPTR_TYPE__)10, 0, resource_proc, 0);
    check(from != 0 && GetDlgItemTextW(from, 200, text, 64) == 13 && weq(text, "Resource text") && GetDlgItem(from, 201) && GetDlgItem(from, IDOK),
          "CreateDialogParamW: a DIALOGEX resource");
    check(SendMessageW(from, DM_GETDEFID, 0, 0) == (0x534b << 16 | IDOK) && GetFocus() == GetDlgItem(from, 201), "its default button, and the focus in its edit");
    DestroyWindow(from);
    check(CreateDialogParamW(instance, (const WCHAR *)(__UINTPTR_TYPE__)11, 0, resource_proc, 0) == 0, "a dialog that isn't there");

    // ---- Modal dialogs ----
    WNDCLASSEXW oc = {0};
    oc.cbSize = sizeof oc;
    oc.lpfnWndProc = owner_proc;
    oc.hInstance = instance;
    oc.lpszClassName = W(0, "AtlasOwner");
    RegisterClassExW(&oc);
    owner = CreateWindowExW(0, W(0, "AtlasOwner"), W(1, "owner"), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 50, 50, 400, 300, 0, 0, instance, 0);
    begin(WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT, 7, 200, 120, 0, "Modal");
    controls();
    INT_PTR r = DialogBoxIndirectParamW(instance, tmpl, owner, modal_proc, 7);
    check(r == 99 && modal_param == 7, "DialogBoxIndirectParamW: Enter ended it, with EndDialog's result");
    check(owner_disabled && IsWindowEnabled(owner), "its owner disabled while it was up, and enabled after");
    r = DialogBoxIndirectParamW(instance, tmpl, owner, modal_proc, 8);
    check(r == IDCANCEL && modal_inits == 2, "WM_CLOSE: IDCANCEL");
    r = DialogBoxParamW(instance, (const WCHAR *)(__UINTPTR_TYPE__)11, owner, modal_proc, 0);
    check(r == -1, "DialogBoxParamW with no such dialog: -1");
    DestroyWindow(owner);

    ExitProcess(failures);
}
