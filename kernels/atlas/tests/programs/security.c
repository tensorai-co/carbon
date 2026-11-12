// Security on atlas, for ARM64 and x86-64: no C runtime. tests/programs/build.sh builds it.
//
// - the process's token: its user, an elevated administrator's groups, high integrity, the sizes it asks for first,
//   and the pseudo-handle; OpenThreadToken with nothing to impersonate;
// - privileges: enabled, with the previous state; one the token doesn't have (ERROR_NOT_ALL_ASSIGNED); PrivilegeCheck;
//   their names both ways; a child's copy of the token, changed apart from this one's;
// - DuplicateTokenEx's impersonation token: CheckTokenMembership, and its integrity lowered (not raised);
// - SIDs: made, compared, copied, well known, as strings both ways, and as account names both ways;
// - ACLs: ACEs added, read, counted and deleted, and an ACL that's full;
// - security descriptors: absolute, made self-relative, their parts in either form, and SDDL both ways, with a NULL
//   DACL and a mandatory label;
// - descriptors kept: a named event's, from its SECURITY_ATTRIBUTES (generic rights mapped, the owner filled in),
//   opening it as that allows, GetSecurityInfo's pointers and SetSecurityInfo; a file's and a key's, by name and by
//   handle; the SACL needing SeSecurityPrivilege; files and keys with nothing kept giving the defaults;
// - AccessCheck and MapGenericMask;
// - impersonation: ImpersonateSelf, OpenThreadToken, the thread's token lowered to low integrity (no writing up to a
//   medium object), RevertToSelf, SetThreadToken, and ImpersonateNamedPipeClient.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WORD;
typedef unsigned short WCHAR;
typedef unsigned char BYTE;
typedef unsigned long long u64;
typedef void *PSID;

typedef struct { DWORD low; int high; } LUID;
typedef struct { LUID luid; DWORD attributes; } LUID_AND_ATTRIBUTES;
typedef struct { DWORD count; LUID_AND_ATTRIBUTES privileges[2]; } TOKEN_PRIVILEGES;
typedef struct { PSID sid; DWORD attributes; } SID_AND_ATTRIBUTES;
typedef struct { DWORD count; SID_AND_ATTRIBUTES groups[1]; } TOKEN_GROUPS;
typedef struct { BYTE value[6]; } SID_IDENTIFIER_AUTHORITY;
typedef struct { BYTE type, flags; WORD size; DWORD mask; DWORD sid_start; } ACE;
typedef struct { DWORD count, in_use, free; } ACL_SIZE_INFORMATION;
typedef struct { LUID token_id, authentication_id; long long expiration; DWORD type, level, charged, available, groups,
                 privileges; LUID modified; } TOKEN_STATISTICS;
typedef struct {
    DWORD cb;
    WCHAR *reserved, *desktop, *title;
    DWORD x, y, x_size, y_size, x_chars, y_chars, fill, flags;
    unsigned short show, reserved2_size;
    void *reserved2;
    HANDLE std_input, std_output, std_error;
} STARTUPINFOW;
typedef struct { HANDLE process, thread; DWORD pid, tid; } PROCESS_INFORMATION;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K void __stdcall SetLastError(DWORD);
K HANDLE __stdcall GetCurrentProcess(void);
K HANDLE __stdcall GetCurrentThread(void);
K int __stdcall CloseHandle(HANDLE);
K void *__stdcall LocalFree(void *);
K int __stdcall CreateProcessW(const WCHAR *, WCHAR *, void *, void *, int, DWORD, void *, const WCHAR *, STARTUPINFOW *,
                               PROCESS_INFORMATION *);
K int __stdcall TerminateProcess(HANDLE, unsigned);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K DWORD __stdcall GetModuleFileNameW(HANDLE, WCHAR *, DWORD);

typedef struct { DWORD length; void *descriptor; int inherit; } SECURITY_ATTRIBUTES;
typedef struct { DWORD read, write, execute, all; } GENERIC_MAPPING;
K HANDLE __stdcall CreateEventW(SECURITY_ATTRIBUTES *, int, int, const WCHAR *);
K HANDLE __stdcall OpenEventW(DWORD, int, const WCHAR *);
K HANDLE __stdcall CreateFileW(const WCHAR *, DWORD, DWORD, SECURITY_ATTRIBUTES *, DWORD, DWORD, HANDLE);
K int __stdcall DeleteFileW(const WCHAR *);
K HANDLE __stdcall CreateNamedPipeW(const WCHAR *, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, void *);
K int __stdcall ConnectNamedPipe(HANDLE, void *);

#define A __declspec(dllimport)
A DWORD __stdcall GetSecurityInfo(HANDLE, DWORD, DWORD, PSID *, PSID *, void **, void **, void **);
A DWORD __stdcall SetSecurityInfo(HANDLE, DWORD, DWORD, PSID, PSID, void *, void *);
A DWORD __stdcall GetNamedSecurityInfoW(const WCHAR *, DWORD, DWORD, PSID *, PSID *, void **, void **, void **);
A DWORD __stdcall SetNamedSecurityInfoW(const WCHAR *, DWORD, DWORD, PSID, PSID, void *, void *);
A int __stdcall GetKernelObjectSecurity(HANDLE, DWORD, void *, DWORD, DWORD *);
A int __stdcall GetFileSecurityW(const WCHAR *, DWORD, void *, DWORD, DWORD *);
A long __stdcall RegGetKeySecurity(HANDLE, DWORD, void *, DWORD *);
A long __stdcall RegSetKeySecurity(HANDLE, DWORD, void *);
A long __stdcall RegCreateKeyExW(HANDLE, const WCHAR *, DWORD, WCHAR *, DWORD, DWORD, void *, HANDLE *, DWORD *);
A long __stdcall RegOpenKeyExW(HANDLE, const WCHAR *, DWORD, DWORD, HANDLE *);
A long __stdcall RegCloseKey(HANDLE);
A long __stdcall RegDeleteKeyW(HANDLE, const WCHAR *);
A int __stdcall AccessCheck(void *, HANDLE, DWORD, GENERIC_MAPPING *, void *, DWORD *, DWORD *, int *);
A void __stdcall MapGenericMask(DWORD *, GENERIC_MAPPING *);
A int __stdcall ImpersonateSelf(DWORD);
A int __stdcall RevertToSelf(void);
A int __stdcall SetThreadToken(HANDLE *, HANDLE);
A int __stdcall ImpersonateNamedPipeClient(HANDLE);
A int __stdcall OpenProcessToken(HANDLE, DWORD, HANDLE *);
A int __stdcall OpenThreadToken(HANDLE, DWORD, int, HANDLE *);
A int __stdcall GetTokenInformation(HANDLE, DWORD, void *, DWORD, DWORD *);
A int __stdcall SetTokenInformation(HANDLE, DWORD, void *, DWORD);
A int __stdcall AdjustTokenPrivileges(HANDLE, int, TOKEN_PRIVILEGES *, DWORD, TOKEN_PRIVILEGES *, DWORD *);
A int __stdcall PrivilegeCheck(HANDLE, void *, int *);
A int __stdcall DuplicateTokenEx(HANDLE, DWORD, void *, DWORD, DWORD, HANDLE *);
A int __stdcall CheckTokenMembership(HANDLE, PSID, int *);
A int __stdcall LookupPrivilegeValueW(const WCHAR *, const WCHAR *, LUID *);
A int __stdcall LookupPrivilegeNameW(const WCHAR *, LUID *, WCHAR *, DWORD *);
A int __stdcall AllocateAndInitializeSid(SID_IDENTIFIER_AUTHORITY *, BYTE, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD, DWORD,
                                         DWORD, PSID *);
A void *__stdcall FreeSid(PSID);
A DWORD *__stdcall GetSidSubAuthority(PSID, DWORD);
A BYTE *__stdcall GetSidSubAuthorityCount(PSID);
A DWORD __stdcall GetLengthSid(PSID);
A int __stdcall IsValidSid(PSID);
A int __stdcall EqualSid(PSID, PSID);
A int __stdcall CopySid(DWORD, PSID, PSID);
A int __stdcall CreateWellKnownSid(DWORD, PSID, PSID, DWORD *);
A int __stdcall IsWellKnownSid(PSID, DWORD);
A int __stdcall ConvertSidToStringSidW(PSID, WCHAR **);
A int __stdcall ConvertStringSidToSidW(const WCHAR *, PSID *);
A int __stdcall LookupAccountSidW(const WCHAR *, PSID, WCHAR *, DWORD *, WCHAR *, DWORD *, DWORD *);
A int __stdcall LookupAccountNameW(const WCHAR *, const WCHAR *, PSID, DWORD *, WCHAR *, DWORD *, DWORD *);
A int __stdcall InitializeSecurityDescriptor(void *, DWORD);
A int __stdcall IsValidSecurityDescriptor(void *);
A DWORD __stdcall GetSecurityDescriptorLength(void *);
A int __stdcall GetSecurityDescriptorControl(void *, WORD *, DWORD *);
A int __stdcall SetSecurityDescriptorOwner(void *, PSID, int);
A int __stdcall SetSecurityDescriptorDacl(void *, int, void *, int);
A int __stdcall GetSecurityDescriptorOwner(void *, PSID *, int *);
A int __stdcall GetSecurityDescriptorDacl(void *, int *, void **, int *);
A int __stdcall MakeSelfRelativeSD(void *, void *, DWORD *);
A int __stdcall InitializeAcl(void *, DWORD, DWORD);
A int __stdcall IsValidAcl(void *);
A int __stdcall AddAccessAllowedAce(void *, DWORD, DWORD, PSID);
A int __stdcall AddAccessDeniedAceEx(void *, DWORD, DWORD, DWORD, PSID);
A int __stdcall GetAce(void *, DWORD, void **);
A int __stdcall DeleteAce(void *, DWORD);
A int __stdcall GetAclInformation(void *, void *, DWORD, DWORD);
A int __stdcall ConvertStringSecurityDescriptorToSecurityDescriptorW(const WCHAR *, DWORD, void **, DWORD *);
A int __stdcall ConvertSecurityDescriptorToStringSecurityDescriptorW(void *, DWORD, DWORD, WCHAR **, DWORD *);

enum { TOKEN_QUERY = 8, TOKEN_ADJUST_PRIVILEGES = 0x20, TOKEN_ALL_ACCESS = 0xf01ff, TokenUser = 1, TokenGroups = 2,
       TokenPrivileges = 3, TokenOwner = 4, TokenType = 8, TokenImpersonationLevel = 9, TokenStatistics = 10,
       TokenElevationType = 18, TokenElevation = 20, TokenIntegrityLevel = 25, SE_PRIVILEGE_ENABLED = 2,
       ERROR_INSUFFICIENT_BUFFER = 122, ERROR_NO_TOKEN = 1008, ERROR_NOT_ALL_ASSIGNED = 1300,
       ERROR_NO_IMPERSONATION_TOKEN = 1309, ERROR_PRIVILEGE_NOT_HELD = 1314, ERROR_NONE_MAPPED = 1332,
       ERROR_INVALID_SID = 1337, ERROR_INVALID_SECURITY_DESCR = 1338, ERROR_ALLOTTED_SPACE_EXCEEDED = 1344,
       WinBuiltinAdministratorsSid = 26, WinLocalSystemSid = 22, WinWorldSid = 1, WinLowLabelSid = 66,
       WinHighLabelSid = 68, SE_DACL_PRESENT = 4, SE_SELF_RELATIVE = 0x8000 };

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
static int wstarts(const WCHAR *a, const char *b) { if (!a) return 0; while (*b) if (*a++ != (WCHAR)*b++) return 0; return 1; }
static int wcontains(const WCHAR *a, const char *b) { for (; a && *a; a++) if (wstarts(a, b)) return 1; return 0; }
static int wsame(const WCHAR *a, const char *b) { if (!a) return 0; while (*b) if (*a++ != (WCHAR)*b++) return 0; return *a == 0; }
static WCHAR wbuf[4][256];
static const WCHAR *W(int i, const char *s) { int n = 0; while (s[n]) { wbuf[i][n] = (WCHAR)s[n]; n++; } wbuf[i][n] = 0; return wbuf[i]; }
static int has_privilege(HANDLE token, DWORD luid, DWORD *attributes) {
    static unsigned char b[1024];
    DWORD got;
    if (!GetTokenInformation(token, TokenPrivileges, b, sizeof b, &got)) return 0;
    TOKEN_PRIVILEGES *p = (TOKEN_PRIVILEGES *)b;
    LUID_AND_ATTRIBUTES *each = (LUID_AND_ATTRIBUTES *)(b + 4);
    for (DWORD i = 0; i < p->count; i++) if (each[i].luid.low == luid) { *attributes = each[i].attributes; return 1; }
    return 0;
}
static DWORD integrity(HANDLE token) {
    static unsigned char b[64];
    DWORD got;
    if (!GetTokenInformation(token, TokenIntegrityLevel, b, sizeof b, &got)) return 0;
    PSID sid = ((SID_AND_ATTRIBUTES *)b)->sid;
    return *GetSidSubAuthority(sid, *GetSidSubAuthorityCount(sid) - 1);
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    static unsigned char info[4096];
    DWORD got = 0;

    // The process's token.
    HANDLE token = 0;
    check(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES, &token) && token, "OpenProcessToken");
    SetLastError(0);
    check(!GetTokenInformation(token, TokenUser, info, 8, &got) && GetLastError() == ERROR_INSUFFICIENT_BUFFER && got == 16 + 28,
          "GetTokenInformation: TokenUser's size, first");
    check(GetTokenInformation(token, TokenUser, info, got, &got), "  then the user");
    static unsigned char user[68];
    CopySid(sizeof user, user, ((SID_AND_ATTRIBUTES *)info)->sid);
    WCHAR *text = 0;
    check(ConvertSidToStringSidW(user, &text) && wsame(text, "S-1-5-21-0-0-0-1000"), "  S-1-5-21-0-0-0-1000");
    LocalFree(text);
    WCHAR name[64], domain[64];
    DWORD name_len = 64, domain_len = 64, use = 0;
    check(LookupAccountSidW(0, user, name, &name_len, domain, &domain_len, &use) && wsame(name, "carbon") && wsame(domain, "CARBON") &&
          name_len == 6 && use == 1,
          "  LookupAccountSidW: CARBON\\carbon, a user");
    DWORD value = 0;
    check(GetTokenInformation(token, TokenElevation, &value, 4, &got) && value == 1, "elevated");
    check(GetTokenInformation(token, TokenElevationType, &value, 4, &got) && value == 1, "  with no split token");
    check(GetTokenInformation(token, TokenType, &value, 4, &got) && value == 1, "  a primary token");
    check(integrity(token) == 0x3000, "  at high integrity");
    check(integrity((HANDLE)(long long)-4) == 0x3000, "  and so says GetCurrentProcessToken()'s pseudo-handle");

    unsigned char admins[16], system[16], world[16], low[16], high[16];
    DWORD size = 16;
    CreateWellKnownSid(WinBuiltinAdministratorsSid, 0, admins, &size);
    size = 16;
    CreateWellKnownSid(WinLocalSystemSid, 0, system, &size);
    size = 16;
    CreateWellKnownSid(WinWorldSid, 0, world, &size);
    size = 16;
    CreateWellKnownSid(WinLowLabelSid, 0, low, &size);
    size = 16;
    CreateWellKnownSid(WinHighLabelSid, 0, high, &size);
    check(GetTokenInformation(token, TokenGroups, info, sizeof info, &got), "TokenGroups");
    TOKEN_GROUPS *groups = (TOKEN_GROUPS *)info;
    int admin_owner = 0, world_found = 0;
    for (DWORD i = 0; i < groups->count; i++) {
        if (EqualSid(groups->groups[i].sid, admins)) admin_owner = groups->groups[i].attributes == 0xf;
        if (EqualSid(groups->groups[i].sid, world)) world_found = 1;
    }
    check(admin_owner && world_found, "  Administrators, enabled and an owner, and Everyone");
    check(GetTokenInformation(token, TokenOwner, info, sizeof info, &got) && EqualSid(*(PSID *)info, admins), "TokenOwner: Administrators");
    int member = 0;
    check(CheckTokenMembership(0, admins, &member) && member, "CheckTokenMembership: in Administrators");
    check(CheckTokenMembership(0, system, &member) && !member, "  not SYSTEM");
    SetLastError(0);
    check(!CheckTokenMembership(token, admins, &member) && GetLastError() == ERROR_NO_IMPERSONATION_TOKEN, "  not with a primary token");
    HANDLE thread_token = (HANDLE)1;
    SetLastError(0);
    check(!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 0, &thread_token) && GetLastError() == ERROR_NO_TOKEN && !thread_token,
          "OpenThreadToken: ERROR_NO_TOKEN, as nothing impersonates");

    // Privileges.
    LUID debug, tcb, notify;
    check(LookupPrivilegeValueW(0, W(0, "SeDebugPrivilege"), &debug) && debug.low == 20 && debug.high == 0,
          "LookupPrivilegeValueW: SeDebugPrivilege");
    LookupPrivilegeValueW(0, W(0, "SeTcbPrivilege"), &tcb);
    notify.low = 23;
    notify.high = 0;
    DWORD name_size = 64;
    check(LookupPrivilegeNameW(0, &notify, name, &name_size) && wsame(name, "SeChangeNotifyPrivilege") && name_size == 23,
          "LookupPrivilegeNameW: SeChangeNotifyPrivilege");
    DWORD attributes = 99;
    check(has_privilege(token, 20, &attributes) && attributes == 0, "SeDebugPrivilege: held, not enabled");
    check(has_privilege(token, 23, &attributes) && attributes == 3, "SeChangeNotifyPrivilege: enabled by default");
    TOKEN_PRIVILEGES want = {1, {{debug, SE_PRIVILEGE_ENABLED}}}, previous;
    DWORD previous_size = 0;
    SetLastError(99);
    check(AdjustTokenPrivileges(token, 0, &want, sizeof previous, &previous, &previous_size) && GetLastError() == 0 &&
          previous.count == 1 && previous.privileges[0].luid.low == 20 && previous.privileges[0].attributes == 0,
          "AdjustTokenPrivileges: enabled, with what it was");
    check(has_privilege(token, 20, &attributes) && attributes == SE_PRIVILEGE_ENABLED, "  and the token says so");
    struct { DWORD count, control; LUID_AND_ATTRIBUTES privileges[1]; } set = {1, 1, {{debug, 0}}};
    int held = 0;
    check(PrivilegeCheck(token, &set, &held) && held && (set.privileges[0].attributes & 0x80000000), "PrivilegeCheck: enabled, and marked used");
    TOKEN_PRIVILEGES missing = {1, {{tcb, SE_PRIVILEGE_ENABLED}}};
    SetLastError(0);
    check(AdjustTokenPrivileges(token, 0, &missing, 0, 0, 0) && GetLastError() == ERROR_NOT_ALL_ASSIGNED,
          "  one it doesn't have: TRUE, and ERROR_NOT_ALL_ASSIGNED");

    // A child's token: a copy of this one's.
    WCHAR self[260];
    GetModuleFileNameW(0, self, 260);
    STARTUPINFOW si;
    for (unsigned i = 0; i < sizeof si; i++) ((char *)&si)[i] = 0;
    si.cb = sizeof si;
    PROCESS_INFORMATION pi;
    HANDLE child_token = 0;
    int started = CreateProcessW(self, 0, 0, 0, 0, 4 /* CREATE_SUSPENDED */, 0, 0, &si, &pi);
    check(started && OpenProcessToken(pi.process, TOKEN_ALL_ACCESS, &child_token), "a child's token");
    TOKEN_STATISTICS mine, its;
    GetTokenInformation(token, TokenStatistics, &mine, sizeof mine, &got);
    GetTokenInformation(child_token, TokenStatistics, &its, sizeof its, &got);
    check(got == sizeof its && mine.token_id.low != its.token_id.low && mine.authentication_id.low == its.authentication_id.low,
          "  its own token ID, in the same logon session");
    check(has_privilege(child_token, 20, &attributes) && attributes == SE_PRIVILEGE_ENABLED, "  with SeDebugPrivilege enabled, as this one's was");
    want.privileges[0].attributes = 0;
    AdjustTokenPrivileges(token, 0, &want, 0, 0, 0);
    check(has_privilege(token, 20, &attributes) && attributes == 0 && has_privilege(child_token, 20, &attributes) && attributes == SE_PRIVILEGE_ENABLED,
          "  disabled here, still enabled there");
    CloseHandle(child_token);
    TerminateProcess(pi.process, 0);
    WaitForSingleObject(pi.process, 10000);

    // An impersonation token.
    HANDLE copy = 0;
    check(DuplicateTokenEx(token, TOKEN_ALL_ACCESS, 0, 2 /* SecurityImpersonation */, 2 /* TokenImpersonation */, &copy) && copy,
          "DuplicateTokenEx: an impersonation token");
    check(GetTokenInformation(copy, TokenType, &value, 4, &got) && value == 2 && GetTokenInformation(copy, TokenImpersonationLevel, &value, 4, &got) && value == 2,
          "  its type and level");
    check(CheckTokenMembership(copy, admins, &member) && member, "  CheckTokenMembership with it: in Administrators");
    SID_AND_ATTRIBUTES label = {low, 0x20};
    check(SetTokenInformation(copy, TokenIntegrityLevel, &label, sizeof label) && integrity(copy) == 0x1000 && integrity(token) == 0x3000,
          "  its integrity lowered, and not this one's");
    label.sid = high;
    SetLastError(0);
    check(!SetTokenInformation(copy, TokenIntegrityLevel, &label, sizeof label) && GetLastError() == ERROR_PRIVILEGE_NOT_HELD && integrity(copy) == 0x1000,
          "  and not raised again");
    CloseHandle(copy);

    // SIDs.
    SID_IDENTIFIER_AUTHORITY nt = {{0, 0, 0, 0, 0, 5}};
    PSID made = 0;
    check(AllocateAndInitializeSid(&nt, 2, 32, 544, 0, 0, 0, 0, 0, 0, &made) && IsValidSid(made) && GetLengthSid(made) == 16,
          "AllocateAndInitializeSid: S-1-5-32-544, 16 bytes");
    check(EqualSid(made, admins) && IsWellKnownSid(made, WinBuiltinAdministratorsSid) && !IsWellKnownSid(made, WinLocalSystemSid),
          "  equal to CreateWellKnownSid's Administrators, and well known as it");
    check(*GetSidSubAuthorityCount(made) == 2 && *GetSidSubAuthority(made, 1) == 544, "  its subauthorities");
    unsigned char copied[16];
    SetLastError(0);
    check(!CopySid(8, copied, made) && GetLastError() == ERROR_INSUFFICIENT_BUFFER && CopySid(16, copied, made) && EqualSid(copied, made),
          "CopySid: too small, then copied");
    check(FreeSid(made) == 0, "FreeSid");
    PSID from_string = 0;
    check(ConvertStringSidToSidW(W(0, "BA"), &from_string) && EqualSid(from_string, admins), "ConvertStringSidToSidW: BA, an alias");
    LocalFree(from_string);
    check(ConvertStringSidToSidW(W(0, "S-1-16-4096"), &from_string) && EqualSid(from_string, low), "  S-1-16-4096");
    LocalFree(from_string);
    SetLastError(0);
    check(!ConvertStringSidToSidW(W(0, "S-1-5-x"), &from_string) && GetLastError() == ERROR_INVALID_SID, "  and not S-1-5-x");
    unsigned char found[64];
    size = sizeof found;
    domain_len = 64;
    check(LookupAccountNameW(0, W(0, "Administrators"), found, &size, domain, &domain_len, &use) && EqualSid(found, admins) &&
          wsame(domain, "BUILTIN") && use == 4 && size == 16,
          "LookupAccountNameW: Administrators, BUILTIN's alias");
    size = sizeof found;
    domain_len = 64;
    check(LookupAccountNameW(0, W(0, "carbon\\CARBON"), found, &size, domain, &domain_len, &use) && EqualSid(found, user) && use == 1,
          "  DOMAIN\\name, in any case");
    size = sizeof found;
    domain_len = 64;
    SetLastError(0);
    check(!LookupAccountNameW(0, W(0, "nobody"), found, &size, domain, &domain_len, &use) && GetLastError() == ERROR_NONE_MAPPED,
          "  nobody: ERROR_NONE_MAPPED");

    // ACLs.
    static unsigned char acl[1024], small[16];
    check(InitializeAcl(acl, sizeof acl, 2) && IsValidAcl(acl), "InitializeAcl");
    check(AddAccessAllowedAce(acl, 2, 0x80000000, world) && AddAccessDeniedAceEx(acl, 2, 3, 0x1f01ff, admins), "  an allowed ACE and a denied one");
    ACL_SIZE_INFORMATION sizes;
    check(GetAclInformation(acl, &sizes, sizeof sizes, 2) && sizes.count == 2 && sizes.in_use == 8 + 20 + 24 && sizes.free == sizeof acl - sizes.in_use,
          "  GetAclInformation: two, and the bytes they take");
    ACE *ace = 0;
    check(GetAce(acl, 1, (void **)&ace) && ace->type == 1 && ace->flags == 3 && ace->mask == 0x1f01ff && EqualSid(&ace->sid_start, admins),
          "  GetAce: the denied one, where it is");
    InitializeAcl(small, sizeof small, 2);
    SetLastError(0);
    check(!AddAccessAllowedAce(small, 2, 1, world) && GetLastError() == ERROR_ALLOTTED_SPACE_EXCEEDED, "  an ACL that's full: ERROR_ALLOTTED_SPACE_EXCEEDED");

    // Security descriptors.
    static unsigned char sd[64], relative[2048];
    WORD control = 0;
    DWORD revision = 0;
    check(InitializeSecurityDescriptor(sd, 1) && SetSecurityDescriptorOwner(sd, admins, 0) && SetSecurityDescriptorDacl(sd, 1, acl, 0) &&
          IsValidSecurityDescriptor(sd),
          "an absolute descriptor: owner and DACL");
    check(GetSecurityDescriptorControl(sd, &control, &revision) && control == SE_DACL_PRESENT && revision == 1, "  its control: the DACL's there");
    WCHAR *sddl = 0;
    DWORD sddl_len = 0;
    check(ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, 1, 1 | 4, &sddl, &sddl_len) && wsame(sddl, "O:BAD:(A;;GR;;;WD)(D;OICI;FA;;;BA)") &&
          sddl_len == 35,
          "  as SDDL: O:BAD:(A;;GR;;;WD)(D;OICI;FA;;;BA)");
    LocalFree(sddl);
    DWORD relative_size = 8;
    SetLastError(0);
    check(!MakeSelfRelativeSD(sd, relative, &relative_size) && GetLastError() == ERROR_INSUFFICIENT_BUFFER && relative_size == 20 + sizeof acl + 16,
          "MakeSelfRelativeSD: the size it needs, the ACL at its full size");
    check(MakeSelfRelativeSD(sd, relative, &relative_size) && GetSecurityDescriptorLength(relative) == relative_size, "  then self-relative");
    int present = 0, defaulted = 1;
    void *dacl = 0;
    PSID owner = 0;
    check(GetSecurityDescriptorDacl(relative, &present, &dacl, &defaulted) && present && !defaulted &&
          (unsigned char *)dacl > relative && (unsigned char *)dacl < relative + relative_size && GetAce(dacl, 0, (void **)&ace) && ace->type == 0,
          "  its DACL, inside it");
    check(GetSecurityDescriptorOwner(relative, &owner, &defaulted) && EqualSid(owner, admins), "  its owner");
    SetLastError(0);
    check(!SetSecurityDescriptorDacl(relative, 1, acl, 0) && GetLastError() == ERROR_INVALID_SECURITY_DESCR, "  which can't be set in place");
    check(DeleteAce(acl, 0) && GetAclInformation(acl, &sizes, sizeof sizes, 2) && sizes.count == 1, "DeleteAce");

    const char *full = "O:SYG:SYD:PAI(A;OICI;FA;;;SY)(A;OICIID;0x1200a9;;;BU)S:(ML;;NW;;;LW)";
    void *parsed = 0;
    DWORD parsed_size = 0;
    check(ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, full), 1, &parsed, &parsed_size) && IsValidSecurityDescriptor(parsed) &&
          GetSecurityDescriptorLength(parsed) == parsed_size,
          "SDDL to a descriptor: self-relative, its length");
    check(GetSecurityDescriptorControl(parsed, &control, &revision) && (control & SE_SELF_RELATIVE) && (control & 0x1400) == 0x1400,
          "  protected and auto-inherited");
    check(ConvertSecurityDescriptorToStringSecurityDescriptorW(parsed, 1, 0x1f, &sddl, 0) && wsame(sddl, full), "  and back, the same");
    LocalFree(sddl);
    check(ConvertSecurityDescriptorToStringSecurityDescriptorW(parsed, 1, 0x10, &sddl, 0) && wsame(sddl, "S:(ML;;NW;;;LW)"), "  just its label");
    LocalFree(sddl);
    LocalFree(parsed);
    check(ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:NO_ACCESS_CONTROL"), 1, &parsed, 0) &&
          GetSecurityDescriptorDacl(parsed, &present, &dacl, &defaulted) && present && dacl == 0,
          "  a NULL DACL: present, and none");
    LocalFree(parsed);
    check(!ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;FA;;;nobody)"), 1, &parsed, 0), "  an account that isn't one: refused");

    // Descriptors kept on a kernel object.
    enum { SE_FILE_OBJECT = 1, SE_REGISTRY_KEY = 4, SE_KERNEL_OBJECT = 6, EVENT_MODIFY_STATE = 2, SYNCHRONIZE = 0x100000,
           ERROR_ACCESS_DENIED = 5, ERROR_GENERIC_NOT_MAPPED = 1360 };
    void *guard = 0;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(D;;0x2;;;WD)(A;;GA;;;WD)"), 1, &guard, 0);
    SECURITY_ATTRIBUTES sa = {sizeof sa, guard, 0};
    HANDLE guarded = CreateEventW(&sa, 1, 0, W(2, "atlas-guarded"));
    LocalFree(guard);
    SetLastError(0);
    check(guarded && !OpenEventW(EVENT_MODIFY_STATE, 0, W(2, "atlas-guarded")) && GetLastError() == ERROR_ACCESS_DENIED,
          "an event made with a descriptor: opened to set it, denied");
    HANDLE waiter = OpenEventW(SYNCHRONIZE, 0, W(2, "atlas-guarded"));
    check(waiter != 0, "  opened to wait on it, allowed");
    CloseHandle(waiter);
    void *kept = 0;
    PSID kept_owner = 0;
    void *kept_dacl = 0;
    check(GetSecurityInfo(guarded, SE_KERNEL_OBJECT, 1 | 4, &kept_owner, 0, &kept_dacl, 0, &kept) == 0 && EqualSid(kept_owner, admins) &&
          GetAce(kept_dacl, 1, (void **)&ace) && ace->mask == 0x1f0003,
          "GetSecurityInfo: the owner filled in, GA mapped to the event's rights, pointers into it");
    check(ConvertSecurityDescriptorToStringSecurityDescriptorW(kept, 1, 1 | 4, &sddl, 0) && wsame(sddl, "O:BAD:(D;;DC;;;WD)(A;;0x1f0003;;;WD)"),
          "  as SDDL: O:BAD:(D;;DC;;;WD)(A;;0x1f0003;;;WD)");
    LocalFree(sddl);
    LocalFree(kept);
    void *open_dacl = 0;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;GA;;;WD)"), 1, &parsed, 0);
    GetSecurityDescriptorDacl(parsed, &present, &open_dacl, &defaulted);
    check(SetSecurityInfo(guarded, SE_KERNEL_OBJECT, 4, 0, 0, open_dacl, 0) == 0, "SetSecurityInfo: a DACL that allows it");
    HANDLE setter = OpenEventW(EVENT_MODIFY_STATE, 0, W(2, "atlas-guarded"));
    check(setter != 0, "  and now it opens to set it");
    CloseHandle(setter);
    LocalFree(parsed);
    check(GetSecurityInfo(guarded, SE_KERNEL_OBJECT, 8, 0, 0, 0, 0, &kept) == ERROR_PRIVILEGE_NOT_HELD, "  its SACL: SeSecurityPrivilege, not enabled");
    LUID security_luid;
    LookupPrivilegeValueW(0, W(0, "SeSecurityPrivilege"), &security_luid);
    TOKEN_PRIVILEGES auditing = {1, {{security_luid, SE_PRIVILEGE_ENABLED}}};
    AdjustTokenPrivileges(token, 0, &auditing, 0, 0, 0);
    check(GetSecurityInfo(guarded, SE_KERNEL_OBJECT, 8, 0, 0, 0, 0, &kept) == 0, "  enabled: it may");
    LocalFree(kept);
    unsigned char raw[256];
    DWORD needed = 0;
    SetLastError(0);
    check(!GetKernelObjectSecurity(guarded, 4, raw, 8, &needed) && GetLastError() == ERROR_INSUFFICIENT_BUFFER && needed > 20 &&
          GetKernelObjectSecurity(guarded, 4, raw, needed, &needed) && IsValidSecurityDescriptor(raw),
          "GetKernelObjectSecurity: the size, then self-relative");

    // AccessCheck.
    GENERIC_MAPPING mapping = {1, 2, 4, 7};
    DWORD mask = 0x80000000;
    MapGenericMask(&mask, &mapping);
    check(mask == 1, "MapGenericMask: GENERIC_READ as 1");
    void *checked = 0;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "O:BAG:BAD:(A;;0x1;;;WD)(D;;0x2;;;BA)"), 1, &checked, 0);
    HANDLE client = 0;
    DuplicateTokenEx(token, TOKEN_ALL_ACCESS, 0, 2, 2, &client);
    unsigned char privileges[20];
    DWORD privileges_size = sizeof privileges, granted = 0;
    int allowed = 0;
    check(AccessCheck(checked, client, 1, &mapping, privileges, &privileges_size, &granted, &allowed) && allowed && granted == 1,
          "AccessCheck: allowed to everyone, granted");
    SetLastError(0);
    check(AccessCheck(checked, client, 2, &mapping, privileges, &privileges_size, &granted, &allowed) && !allowed && granted == 0 &&
          GetLastError() == ERROR_ACCESS_DENIED,
          "  denied to Administrators: TRUE, but not allowed");
    check(AccessCheck(checked, client, 0x02000000, &mapping, privileges, &privileges_size, &granted, &allowed) && allowed &&
          granted == (1 | 0x20000 | 0x40000),
          "  MAXIMUM_ALLOWED: what's allowed, and the owner's READ_CONTROL and WRITE_DAC");
    SetLastError(0);
    check(!AccessCheck(checked, client, 0x80000000, &mapping, privileges, &privileges_size, &granted, &allowed) &&
          GetLastError() == ERROR_GENERIC_NOT_MAPPED,
          "  generic rights not mapped: refused");
    SetLastError(0);
    check(!AccessCheck(checked, token, 1, &mapping, privileges, &privileges_size, &granted, &allowed) && GetLastError() == ERROR_NO_IMPERSONATION_TOKEN,
          "  a primary token: refused");
    LocalFree(checked);

    // Impersonation, at low integrity.
    void *medium = 0;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;GA;;;WD)"), 1, &medium, 0);
    SECURITY_ATTRIBUTES medium_sa = {sizeof medium_sa, medium, 0};
    HANDLE everyones = CreateEventW(&medium_sa, 1, 0, W(2, "atlas-everyones"));
    LocalFree(medium);
    check(ImpersonateSelf(2), "ImpersonateSelf");
    HANDLE mine_now = 0;
    check(OpenThreadToken(GetCurrentThread(), TOKEN_ALL_ACCESS, 1, &mine_now) && GetTokenInformation(mine_now, TokenType, &value, 4, &got) && value == 2,
          "  OpenThreadToken: its impersonation token");
    check(CheckTokenMembership(0, admins, &member) && member, "  CheckTokenMembership(NULL) through it: Administrators");
    SID_AND_ATTRIBUTES lowered = {low, 0x20};
    check(SetTokenInformation(mine_now, TokenIntegrityLevel, &lowered, sizeof lowered) && integrity((HANDLE)(long long)-6) == 0x1000 &&
          integrity((HANDLE)(long long)-4) == 0x3000,
          "  lowered to low integrity: the thread's effective token, not the process's");
    SetLastError(0);
    check(!OpenEventW(EVENT_MODIFY_STATE, 0, W(2, "atlas-everyones")) && GetLastError() == ERROR_ACCESS_DENIED,
          "  low can't write up to a medium object, though everyone may");
    waiter = OpenEventW(SYNCHRONIZE, 0, W(2, "atlas-everyones"));
    check(waiter != 0, "  but may wait on it");
    CloseHandle(waiter);
    check(RevertToSelf() && integrity((HANDLE)(long long)-6) == 0x3000, "RevertToSelf: high again");
    setter = OpenEventW(EVENT_MODIFY_STATE, 0, W(2, "atlas-everyones"));
    check(setter != 0, "  and it may write it");
    CloseHandle(setter);
    SetLastError(0);
    check(!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &thread_token) && GetLastError() == ERROR_NO_TOKEN, "  no thread token again");
    check(SetThreadToken(0, mine_now) && integrity((HANDLE)(long long)-6) == 0x1000 && SetThreadToken(0, 0) && integrity((HANDLE)(long long)-6) == 0x3000,
          "SetThreadToken: on, then off");
    SetLastError(0);
    check(!SetThreadToken(0, token) && GetLastError() == 1349, "  not a primary token: ERROR_BAD_TOKEN_TYPE");
    CloseHandle(mine_now);
    CloseHandle(everyones);
    CloseHandle(guarded);
    CloseHandle(client);
    HANDLE server = CreateNamedPipeW(W(2, "\\\\.\\pipe\\atlas-impersonate"), 3, 0, 1, 512, 512, 0, 0);
    HANDLE pipe_client = CreateFileW(W(2, "\\\\.\\pipe\\atlas-impersonate"), 0xc0000000, 0, 0, 3, 0, 0);
    ConnectNamedPipe(server, 0);
    check(server != (HANDLE)(long long)-1 && pipe_client != (HANDLE)(long long)-1 && ImpersonateNamedPipeClient(server) &&
          integrity((HANDLE)(long long)-6) == 0x3000 && OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &thread_token),
          "ImpersonateNamedPipeClient: the client's token");
    CloseHandle(thread_token);
    RevertToSelf();
    CloseHandle(pipe_client);
    CloseHandle(server);

    // A file's descriptor, by handle and by name.
    const WCHAR *path = W(3, "C:\\Windows\\Temp\\atlas-secured.txt");
    DeleteFileW(path);
    void *read_only = 0;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;FR;;;WD)"), 1, &read_only, 0);
    SECURITY_ATTRIBUTES file_sa = {sizeof file_sa, read_only, 0};
    HANDLE file = CreateFileW(path, 0xc0000000, 0, &file_sa, 1 /* CREATE_NEW */, 0, 0);
    LocalFree(read_only);
    check(file != (HANDLE)(long long)-1, "a new file with a descriptor: made, with the access asked for");
    CloseHandle(file);
    SetLastError(0);
    file = CreateFileW(path, 0x40000000, 0, 0, 3 /* OPEN_EXISTING */, 0, 0);
    check(file == (HANDLE)(long long)-1 && GetLastError() == ERROR_ACCESS_DENIED, "  opened to write: denied");
    file = CreateFileW(path, 0x80000000, 0, 0, 3, 0, 0);
    check(file != (HANDLE)(long long)-1, "  to read: allowed");
    check(GetSecurityInfo(file, SE_FILE_OBJECT, 4, 0, 0, 0, 0, &kept) == 0 && ConvertSecurityDescriptorToStringSecurityDescriptorW(kept, 1, 4, &sddl, 0) &&
          wsame(sddl, "D:(A;;FR;;;WD)"),
          "  GetSecurityInfo by its handle: D:(A;;FR;;;WD)");
    LocalFree(sddl);
    LocalFree(kept);
    CloseHandle(file);
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;FA;;;WD)"), 1, &parsed, 0);
    GetSecurityDescriptorDacl(parsed, &present, &open_dacl, &defaulted);
    check(SetNamedSecurityInfoW((WCHAR *)path, SE_FILE_OBJECT, 4, 0, 0, open_dacl, 0) == 0, "SetNamedSecurityInfoW: everyone may write it");
    LocalFree(parsed);
    file = CreateFileW(path, 0x40000000, 0, 0, 3, 0, 0);
    check(file != (HANDLE)(long long)-1, "  and now it opens to write");
    CloseHandle(file);
    check(GetNamedSecurityInfoW((WCHAR *)path, SE_FILE_OBJECT, 4, 0, 0, &kept_dacl, 0, &kept) == 0 && GetAce(kept_dacl, 0, (void **)&ace) &&
          ace->mask == 0x1f01ff,
          "  GetNamedSecurityInfoW: its DACL");
    LocalFree(kept);
    DeleteFileW(path);
    needed = 0;
    check(GetFileSecurityW(W(3, "C:\\Windows"), 1 | 4, raw, sizeof raw, &needed) &&
          ConvertSecurityDescriptorToStringSecurityDescriptorW(raw, 1, 1 | 4, &sddl, 0) && wstarts(sddl, "O:BAD:") && wcontains(sddl, "(A;ID;FA;;;BA)"),
          "GetFileSecurityW: a folder with nothing kept, the default");
    LocalFree(sddl);
    check(GetNamedSecurityInfoW(W(3, "C:\\nowhere\\at-all"), SE_FILE_OBJECT, 4, 0, 0, 0, 0, &kept) == 3 ||
          GetNamedSecurityInfoW(W(3, "C:\\nowhere\\at-all"), SE_FILE_OBJECT, 4, 0, 0, 0, 0, &kept) == 2,
          "  a path that isn't there: not found");

    // A key's.
    HANDLE key = 0, reopened = 0;
    DWORD disposition = 0;
    const WCHAR *subkey = W(3, "Software\\atlas-security");
    RegCreateKeyExW((HANDLE)(long long)(int)0x80000001, subkey, 0, 0, 0, 0xf003f, 0, &key, &disposition);
    ConvertStringSecurityDescriptorToSecurityDescriptorW(W(1, "D:(A;;KR;;;WD)"), 1, &parsed, 0);
    check(key && RegSetKeySecurity(key, 4, parsed) == 0, "RegSetKeySecurity: read-only for everyone");
    LocalFree(parsed);
    check(RegOpenKeyExW((HANDLE)(long long)(int)0x80000001, subkey, 0, 2 /* KEY_SET_VALUE */, &reopened) == ERROR_ACCESS_DENIED,
          "  opened to set a value: denied");
    check(RegOpenKeyExW((HANDLE)(long long)(int)0x80000001, subkey, 0, 0x20019 /* KEY_READ */, &reopened) == 0, "  to read: allowed");
    RegCloseKey(reopened);
    DWORD key_size = 8;
    check(RegGetKeySecurity(key, 4, raw, &key_size) == ERROR_INSUFFICIENT_BUFFER && key_size > 20 &&
          RegGetKeySecurity(key, 4, raw, &key_size) == 0 && ConvertSecurityDescriptorToStringSecurityDescriptorW(raw, 1, 4, &sddl, 0) &&
          wsame(sddl, "D:(A;;KR;;;WD)"),
          "RegGetKeySecurity: the size, then D:(A;;KR;;;WD)");
    LocalFree(sddl);
    check(GetNamedSecurityInfoW(W(0, "CURRENT_USER\\Software\\atlas-security"), SE_REGISTRY_KEY, 4, 0, 0, 0, 0, &kept) == 0 &&
          ConvertSecurityDescriptorToStringSecurityDescriptorW(kept, 1, 4, &sddl, 0) && wsame(sddl, "D:(A;;KR;;;WD)"),
          "  and by name, CURRENT_USER\\…");
    LocalFree(sddl);
    LocalFree(kept);
    RegCloseKey(key);
    RegDeleteKeyW((HANDLE)(long long)(int)0x80000001, subkey);

    CloseHandle(token);
    say(failures ? "some checks failed\n" : "all passed\n");
    ExitProcess(failures);
}
