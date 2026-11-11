/*
 * winkvm.sys on its own, before exec uses it: tiny ring-0 guests in long mode, straight against the device.
 *
 *   build\Release\smoke.exe          (as Administrator, with winkvm running)
 *
 * Each check prints ok or FAILED, and the exit code is how many failed.
 */
#include <windows.h>
#include <winioctl.h>
#include <stdio.h>
#include <string.h>

#include "winkvm.h"

#define RAM_BASE   0x100000000ULL
#define RAM_SIZE   (2ULL << 20)
/* Guest physical layout, from RAM_BASE: tables, then the GDT/IDT/TSS page, then code, then the stack's top. */
#define PML4       0x1000
#define PDPT       0x2000
#define PD         0x3000
#define SYSTEM     0x4000
#define CODE       0x10000
#define STACK_TOP  0x100000

static HANDLE device;
static int failures;

static void check(const char* what, BOOL ok)
{
    printf("%-52s %s\n", what, ok ? "ok" : "FAILED");
    failures += !ok;
}

/* Overlapped, each call waiting for its own result: on a synchronous handle the I/O manager runs one call at a time, so a
   CANCEL from another thread would wait behind the RUN it's meant to stop. */
static BOOL call(DWORD code, void* in, DWORD inLength, void* out, DWORD outLength)
{
    DWORD returned = 0;
    OVERLAPPED overlapped = { 0 };
    overlapped.hEvent = CreateEventW(NULL, TRUE, FALSE, NULL);
    BOOL ok = DeviceIoControl(device, code, in, inLength, out, outLength, &returned, &overlapped);
    if (!ok && GetLastError() == ERROR_IO_PENDING) {
        ok = GetOverlappedResult(device, &overlapped, &returned, TRUE);
    }
    CloseHandle(overlapped.hEvent);
    return ok;
}

static WINKVM_SEGMENT segment(UINT16 selector, UINT8 type, UINT8 flags)
{
    WINKVM_SEGMENT s = { 0, 0xFFFFFFFF, selector, type, flags };
    return s;
}

typedef struct GUEST {
    WINKVM_VCPU id;
    UINT8* ram;
} GUEST;

/* A VM with 2 MiB of RAM identity-mapped by one large page, a vCPU in 64-bit ring 0, and `code` at CODE. */
static BOOL make_guest(GUEST* guest, const UINT8* code, size_t length)
{
    WINKVM_VCPU none = { 0 }, vm = { 0 };
    if (!call(IOCTL_WINKVM_CREATE_VM, &none, 0, &vm, sizeof(vm))) {
        return FALSE;
    }
    guest->ram = VirtualAlloc(NULL, RAM_SIZE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    if (!guest->ram) {
        return FALSE;
    }
    WINKVM_MEMORY memory = { vm.Vm, 0, RAM_BASE, RAM_SIZE, (UINT64)(ULONG_PTR)guest->ram };
    if (!call(IOCTL_WINKVM_SET_MEMORY, &memory, sizeof(memory), NULL, 0)) {
        return FALSE;
    }

    /* VA RAM_BASE.. -> the same guest physical addresses, with one 2 MiB page. */
    UINT64* pml4 = (UINT64*)(guest->ram + PML4);
    UINT64* pdpt = (UINT64*)(guest->ram + PDPT);
    UINT64* pd = (UINT64*)(guest->ram + PD);
    pml4[(RAM_BASE >> 39) & 511] = (RAM_BASE + PDPT) | 3;
    pdpt[(RAM_BASE >> 30) & 511] = (RAM_BASE + PD) | 3;
    pd[(RAM_BASE >> 21) & 511] = RAM_BASE | 0x83;
    memcpy(guest->ram + CODE, code, length);

    if (!call(IOCTL_WINKVM_CREATE_VCPU, &vm, sizeof(vm), &guest->id, sizeof(guest->id))) {
        return FALSE;
    }
    WINKVM_SYSTEM system = { 0 };
    system.Vm = guest->id.Vm;
    system.Vcpu = guest->id.Vcpu;
    system.Cr0 = 1 | 1 << 1 | 1 << 4 | 1 << 5 | 1 << 16 | 1ULL << 31;
    system.Cr4 = 1 << 5 | 1 << 9 | 1 << 10 | 1 << 18;
    system.Efer = 1 | 1 << 8 | 1 << 10 | 1 << 11;
    system.GdtBase = RAM_BASE + SYSTEM;
    system.GdtLimit = 0x5F;
    system.IdtBase = RAM_BASE + SYSTEM + 0x100;
    system.IdtLimit = 0xFFF;
    system.Tr = (WINKVM_SEGMENT){ RAM_BASE + SYSTEM + 0x800, 0x67, 0x40, 0xB, 1 };
    return call(IOCTL_WINKVM_SET_SYSTEM, &system, sizeof(system), NULL, 0);
}

static WINKVM_RUN first_run(GUEST* guest)
{
    WINKVM_RUN run = { 0 };
    run.Vm = guest->id.Vm;
    run.Vcpu = guest->id.Vcpu;
    run.Regs.Rip = RAM_BASE + CODE;
    run.Regs.Gpr[4] = RAM_BASE + STACK_TOP;
    run.Regs.Rflags = 2;
    run.Regs.Cr3 = RAM_BASE + PML4;
    run.Regs.Cs = segment(0x10, 0xB, 0x71); /* present, S, L, G */
    run.Regs.Ss = segment(0x18, 0x3, 0x59); /* present, D/B, S, G */
    run.Regs.Ds = run.Regs.Ss;
    run.Regs.Es = run.Regs.Ss;
    run.Regs.Fs = run.Regs.Ss;
    run.Regs.Gs = run.Regs.Ss;
    return run;
}

static BOOL run_guest(WINKVM_RUN* run)
{
    return call(IOCTL_WINKVM_RUN, run, sizeof(*run), run, sizeof(*run));
}

static void destroy(GUEST* guest)
{
    call(IOCTL_WINKVM_DESTROY_VM, &guest->id, sizeof(guest->id), NULL, 0);
    VirtualFree(guest->ram, 0, MEM_RELEASE);
}

static DWORD WINAPI cancel_later(void* id)
{
    Sleep(200);
    call(IOCTL_WINKVM_CANCEL, id, sizeof(WINKVM_VCPU), NULL, 0);
    return 0;
}

/* A check that never comes back ends the test, rather than leaving it spinning. */
static DWORD WINAPI watchdog(void* unused)
{
    (void)unused;
    Sleep(60000);
    printf("a check took over a minute: stopping\n");
    ExitProcess(100);
}

int main(void)
{
    /* Unbuffered, so the last check shown is the last one that finished, even when one hangs. */
    setvbuf(stdout, NULL, _IONBF, 0);
    CloseHandle(CreateThread(NULL, 0, watchdog, NULL, 0, NULL));
    device = CreateFileW(L"\\\\.\\WinKvm", GENERIC_READ | GENERIC_WRITE, 0, NULL, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, NULL);
    if (device == INVALID_HANDLE_VALUE) {
        printf("can't open \\\\.\\WinKvm: error %lu (is winkvm running, and is this Administrator?)\n", GetLastError());
        return 1;
    }

    WINKVM_CAPS caps = { 0 };
    check("CAPS answers", call(IOCTL_WINKVM_CAPS, NULL, 0, &caps, sizeof(caps)));
    check("CAPS: the API version", caps.Version == WINKVM_API_VERSION);
    check("CAPS: VT-x with EPT", caps.Vmx && caps.Ept);
    if (!caps.Vmx) {
        return failures;
    }

    {
        /* mov eax, 42; hlt */
        static const UINT8 code[] = { 0xB8, 0x2A, 0, 0, 0, 0xF4 };
        GUEST guest;
        check("a VM with memory and a vCPU", make_guest(&guest, code, sizeof(code)));
        WINKVM_RUN run = first_run(&guest);
        check("RUN returns", run_guest(&run));
        check("it halts", run.ExitReason == WINKVM_EXIT_HLT);
        check("rax is 42", run.Regs.Gpr[0] == 42);
        check("rip is past the hlt", run.Regs.Rip == RAM_BASE + CODE + sizeof(code));
        destroy(&guest);
    }
    {
        /* mov eax, 1; cpuid; hlt */
        static const UINT8 code[] = { 0xB8, 1, 0, 0, 0, 0x0F, 0xA2, 0xF4 };
        GUEST guest;
        make_guest(&guest, code, sizeof(code));
        WINKVM_RUN run = first_run(&guest);
        run_guest(&run);
        check("CPUID is answered, and it halts after", run.ExitReason == WINKVM_EXIT_HLT);
        check("CPUID says there's no hypervisor", !(run.Regs.Gpr[1] & (1ULL << 31)));
        check("CPUID says there's no VMX", !(run.Regs.Gpr[1] & (1 << 5)));
        destroy(&guest);
    }
    {
        /* jmp $ */
        static const UINT8 code[] = { 0xEB, 0xFE };
        GUEST guest;
        make_guest(&guest, code, sizeof(code));
        WINKVM_RUN run = first_run(&guest);
        HANDLE thread = CreateThread(NULL, 0, cancel_later, &guest.id, 0, NULL);
        DWORD start = GetTickCount();
        run_guest(&run);
        check("a spinning guest is cancelled from another thread", run.ExitReason == WINKVM_EXIT_CANCELED);
        check("...within a second", GetTickCount() - start < 1000);
        check("...and it's still in its loop", run.Regs.Rip == RAM_BASE + CODE);
        WaitForSingleObject(thread, INFINITE);
        CloseHandle(thread);
        destroy(&guest);
    }
    {
        /* out 0x80, al */
        static const UINT8 code[] = { 0xE6, 0x80, 0xF4 };
        GUEST guest;
        make_guest(&guest, code, sizeof(code));
        WINKVM_RUN run = first_run(&guest);
        run_guest(&run);
        check("I/O never reaches the host", run.ExitReason == WINKVM_EXIT_OTHER && (run.Qualification & 0xFFFF) == 30);
        destroy(&guest);
    }
    {
        /* mov ecx, 0xC0000082; wrmsr */
        static const UINT8 code[] = { 0xB9, 0x82, 0, 0, 0xC0, 0x0F, 0x30, 0xF4 };
        GUEST guest;
        make_guest(&guest, code, sizeof(code));
        WINKVM_RUN run = first_run(&guest);
        run_guest(&run);
        check("WRMSR never reaches the host", run.ExitReason == WINKVM_EXIT_OTHER && (run.Qualification & 0xFFFF) == 32);
        destroy(&guest);
    }
    {
        /* mov rax, [0x200000000]: its page tables map it to guest physical 0x200000000, which isn't guest RAM. */
        static const UINT8 code[] = { 0x48, 0xA1, 0, 0, 0, 0, 2, 0, 0, 0, 0xF4 };
        GUEST guest;
        make_guest(&guest, code, sizeof(code));
        UINT64* pdpt = (UINT64*)(guest.ram + PDPT);
        UINT64* outside = (UINT64*)(guest.ram + 0x5000);
        pdpt[(0x200000000ULL >> 30) & 511] = (RAM_BASE + 0x5000) | 3;
        outside[0] = 0x200000000ULL | 0x83;
        WINKVM_RUN run = first_run(&guest);
        run_guest(&run);
        check("memory outside guest RAM is an EPT violation", run.ExitReason == WINKVM_EXIT_OTHER && (run.Qualification & 0xFFFF) == 48);
        destroy(&guest);
    }

    CloseHandle(device);
    printf("%d failed\n", failures);
    return failures;
}
