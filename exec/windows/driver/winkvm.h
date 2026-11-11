/*
 * winkvm.sys: Carbon's own VT-x driver, and the whole of its interface.
 *
 * Its job is to be the smallest hypervisor exec's vm backend needs on Windows, shaped like KVM: one guest RAM region in
 * EPT, vCPUs on host threads, registers in and out, and RUN until the guest halts. Everything else (the guest's tables,
 * the shim, what a stop means) is exec's and shared with KVM.
 *
 * The driver trusts nothing from user mode: every id, address, size, segment and control register value is checked
 * before it reaches a VMCS, and guest RAM is pinned for as long as the VM lives.
 *
 * Device: \\.\WinKvm. Every call is METHOD_BUFFERED.
 */
#ifndef WINKVM_H
#define WINKVM_H

#define WINKVM_API_VERSION 0x0200
#define WINKVM_DEVICE_TYPE 0x8000

/* Unsigned: the device type is above 0x8000, so CTL_CODE alone gives a negative int. */
#define WINKVM_CTL(n) ((unsigned long)CTL_CODE(WINKVM_DEVICE_TYPE, (n), METHOD_BUFFERED, FILE_ANY_ACCESS))

/* in: none                 out: WINKVM_CAPS */
#define IOCTL_WINKVM_CAPS        WINKVM_CTL(0x800)
/* in: none                 out: WINKVM_VCPU (vm set, vcpu 0) */
#define IOCTL_WINKVM_CREATE_VM   WINKVM_CTL(0x801)
/* in: WINKVM_VCPU (vm)     out: none */
#define IOCTL_WINKVM_DESTROY_VM  WINKVM_CTL(0x802)
/* in: WINKVM_MEMORY        out: none */
#define IOCTL_WINKVM_SET_MEMORY  WINKVM_CTL(0x803)
/* in: WINKVM_VCPU (vm)     out: WINKVM_VCPU (vm, vcpu). The vCPU belongs to the calling thread. */
#define IOCTL_WINKVM_CREATE_VCPU WINKVM_CTL(0x804)
/* in: WINKVM_SYSTEM        out: none */
#define IOCTL_WINKVM_SET_SYSTEM  WINKVM_CTL(0x805)
/* in: WINKVM_VCPU          out: WINKVM_FP */
#define IOCTL_WINKVM_GET_FP      WINKVM_CTL(0x806)
/* in: WINKVM_FP            out: none */
#define IOCTL_WINKVM_SET_FP      WINKVM_CTL(0x807)
/* in: WINKVM_RUN           out: WINKVM_RUN. Registers go in, run until an exit, and come back. */
#define IOCTL_WINKVM_RUN         WINKVM_CTL(0x808)
/* in: WINKVM_VCPU          out: none. Any thread: makes RUN return WINKVM_EXIT_CANCELED as soon as it can. */
#define IOCTL_WINKVM_CANCEL      WINKVM_CTL(0x809)

#define WINKVM_EXIT_HLT          1 /* the guest halted */
#define WINKVM_EXIT_CANCELED     2 /* IOCTL_WINKVM_CANCEL, or a host interrupt the driver gave back */
#define WINKVM_EXIT_FAILED_ENTRY 3 /* VMLAUNCH/VMRESUME failed; qualification is the VM-instruction error */
#define WINKVM_EXIT_OTHER        4 /* any other exit; qualification is the basic exit reason */

#pragma pack(push, 8)

typedef struct WINKVM_CAPS {
    UINT32 Version;
    UINT32 Vmx;            /* VT-x is there and on */
    UINT32 Ept;
    UINT32 MaxVcpus;
} WINKVM_CAPS;

typedef struct WINKVM_VCPU {
    UINT32 Vm;
    UINT32 Vcpu;
} WINKVM_VCPU;

typedef struct WINKVM_MEMORY {
    UINT32 Vm;
    UINT32 Reserved;
    UINT64 GuestPhys;      /* page-aligned */
    UINT64 Size;           /* page-aligned */
    UINT64 UserAddress;    /* page-aligned, committed, pinned by the driver */
} WINKVM_MEMORY;

/* A segment, as VMX describes it. Flags: bit 0 present, bits 1-2 DPL, bit 3 D/B, bit 4 S, bit 5 L, bit 6 G. */
typedef struct WINKVM_SEGMENT {
    UINT64 Base;
    UINT32 Limit;
    UINT16 Selector;
    UINT8  Type;
    UINT8  Flags;
} WINKVM_SEGMENT;

typedef struct WINKVM_MSR {
    UINT32 Index;          /* only STAR, LSTAR, CSTAR, SFMASK, FS_BASE, GS_BASE, KERNEL_GS_BASE */
    UINT32 Reserved;
    UINT64 Value;
} WINKVM_MSR;

typedef struct WINKVM_SYSTEM {
    UINT32 Vm;
    UINT32 Vcpu;
    UINT64 Cr0;
    UINT64 Cr4;
    UINT64 Efer;
    UINT64 Xcr0;
    UINT64 GdtBase;
    UINT64 IdtBase;
    UINT16 GdtLimit;
    UINT16 IdtLimit;
    UINT32 MsrCount;
    WINKVM_SEGMENT Tr;
    WINKVM_MSR Msrs[8];
} WINKVM_SYSTEM;

/* General registers in x86's encoding order: rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8-r15. */
typedef struct WINKVM_REGS {
    UINT64 Gpr[16];
    UINT64 Rip;
    UINT64 Rflags;
    UINT64 Cr2;
    UINT64 Cr3;
    WINKVM_SEGMENT Cs, Ss, Ds, Es, Fs, Gs;
} WINKVM_REGS;

typedef struct WINKVM_FP {
    UINT32 Vm;
    UINT32 Vcpu;
    UINT8  Fxsave[512];    /* FXSAVE's layout */
} WINKVM_FP;

typedef struct WINKVM_RUN {
    UINT32 Vm;
    UINT32 Vcpu;
    UINT32 ExitReason;     /* out: WINKVM_EXIT_* */
    UINT32 Reserved;
    UINT64 Qualification;  /* out */
    WINKVM_REGS Regs;      /* in and out */
} WINKVM_RUN;

#pragma pack(pop)

#endif
