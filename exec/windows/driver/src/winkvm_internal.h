/*
 * winkvm.sys's insides: its VMs and vCPUs, VMX's numbers, and what each file provides.
 */
#pragma once

// ntifs.h is ntddk.h and more: PsIsThreadTerminating is in it.
#include <ntifs.h>
#include <wdmsec.h>
#include <intrin.h>

#include "../winkvm.h"

#define WINKVM_TAG 'mvkW'
#define WINKVM_MAX_VMS 32
#define WINKVM_MAX_VCPUS 64
/* The most guest RAM a VM can have. */
#define WINKVM_MAX_MEMORY (64ULL << 30)
/* Guest RAM is locked in pieces this big, as an MDL can't describe 4 GiB. */
#define WINKVM_MDL_CHUNK (256ULL << 20)

/* ---- MSRs ---- */

#define MSR_IA32_FEATURE_CONTROL        0x3A
#define MSR_IA32_SYSENTER_CS            0x174
#define MSR_IA32_SYSENTER_ESP           0x175
#define MSR_IA32_SYSENTER_EIP           0x176
#define MSR_IA32_DEBUGCTL               0x1D9
#define MSR_IA32_VMX_BASIC              0x480
#define MSR_IA32_VMX_PINBASED_CTLS      0x481
#define MSR_IA32_VMX_PROCBASED_CTLS     0x482
#define MSR_IA32_VMX_EXIT_CTLS          0x483
#define MSR_IA32_VMX_ENTRY_CTLS         0x484
#define MSR_IA32_VMX_CR0_FIXED0         0x486
#define MSR_IA32_VMX_CR0_FIXED1         0x487
#define MSR_IA32_VMX_CR4_FIXED0         0x488
#define MSR_IA32_VMX_CR4_FIXED1         0x489
#define MSR_IA32_VMX_PROCBASED_CTLS2    0x48B
#define MSR_IA32_VMX_EPT_VPID_CAP       0x48C
#define MSR_IA32_VMX_TRUE_PINBASED_CTLS 0x48D
#define MSR_IA32_VMX_TRUE_PROCBASED_CTLS 0x48E
#define MSR_IA32_VMX_TRUE_EXIT_CTLS     0x48F
#define MSR_IA32_VMX_TRUE_ENTRY_CTLS    0x490
#define MSR_IA32_EFER                   0xC0000080
#define MSR_STAR                        0xC0000081
#define MSR_LSTAR                       0xC0000082
#define MSR_CSTAR                       0xC0000083
#define MSR_SFMASK                      0xC0000084
#define MSR_FS_BASE                     0xC0000100
#define MSR_GS_BASE                     0xC0000101
#define MSR_KERNEL_GS_BASE              0xC0000102

/* ---- VMCS fields ---- */

#define VMCS_GUEST_ES_SELECTOR          0x0800
#define VMCS_GUEST_CS_SELECTOR          0x0802
#define VMCS_GUEST_SS_SELECTOR          0x0804
#define VMCS_GUEST_DS_SELECTOR          0x0806
#define VMCS_GUEST_FS_SELECTOR          0x0808
#define VMCS_GUEST_GS_SELECTOR          0x080A
#define VMCS_GUEST_LDTR_SELECTOR        0x080C
#define VMCS_GUEST_TR_SELECTOR          0x080E
#define VMCS_HOST_ES_SELECTOR           0x0C00
#define VMCS_HOST_CS_SELECTOR           0x0C02
#define VMCS_HOST_SS_SELECTOR           0x0C04
#define VMCS_HOST_DS_SELECTOR           0x0C06
#define VMCS_HOST_FS_SELECTOR           0x0C08
#define VMCS_HOST_GS_SELECTOR           0x0C0A
#define VMCS_HOST_TR_SELECTOR           0x0C0C

#define VMCS_EXIT_MSR_STORE_ADDR        0x2006
#define VMCS_EXIT_MSR_LOAD_ADDR         0x2008
#define VMCS_ENTRY_MSR_LOAD_ADDR        0x200A
#define VMCS_EPT_POINTER                0x201A
#define VMCS_LINK_POINTER               0x2800
#define VMCS_GUEST_DEBUGCTL             0x2802
#define VMCS_GUEST_EFER                 0x2806
#define VMCS_HOST_EFER                  0x2C02

#define VMCS_PIN_CONTROLS               0x4000
#define VMCS_PROC_CONTROLS              0x4002
#define VMCS_EXCEPTION_BITMAP           0x4004
#define VMCS_CR3_TARGET_COUNT           0x400A
#define VMCS_EXIT_CONTROLS              0x400C
#define VMCS_EXIT_MSR_STORE_COUNT       0x400E
#define VMCS_EXIT_MSR_LOAD_COUNT        0x4010
#define VMCS_ENTRY_CONTROLS             0x4012
#define VMCS_ENTRY_MSR_LOAD_COUNT       0x4014
#define VMCS_ENTRY_INTERRUPTION_INFO    0x4016
#define VMCS_PROC_CONTROLS2             0x401E
#define VMCS_INSTRUCTION_ERROR          0x4400
#define VMCS_EXIT_REASON                0x4402
#define VMCS_EXIT_INSTRUCTION_LENGTH    0x440C
#define VMCS_GUEST_ES_LIMIT             0x4800
#define VMCS_GUEST_CS_LIMIT             0x4802
#define VMCS_GUEST_SS_LIMIT             0x4804
#define VMCS_GUEST_DS_LIMIT             0x4806
#define VMCS_GUEST_FS_LIMIT             0x4808
#define VMCS_GUEST_GS_LIMIT             0x480A
#define VMCS_GUEST_LDTR_LIMIT           0x480C
#define VMCS_GUEST_TR_LIMIT             0x480E
#define VMCS_GUEST_GDTR_LIMIT           0x4810
#define VMCS_GUEST_IDTR_LIMIT           0x4812
#define VMCS_GUEST_ES_AR                0x4814
#define VMCS_GUEST_CS_AR                0x4816
#define VMCS_GUEST_SS_AR                0x4818
#define VMCS_GUEST_DS_AR                0x481A
#define VMCS_GUEST_FS_AR                0x481C
#define VMCS_GUEST_GS_AR                0x481E
#define VMCS_GUEST_LDTR_AR              0x4820
#define VMCS_GUEST_TR_AR                0x4822
#define VMCS_GUEST_INTERRUPTIBILITY     0x4824
#define VMCS_GUEST_ACTIVITY             0x4826
#define VMCS_GUEST_SYSENTER_CS          0x482A
#define VMCS_HOST_SYSENTER_CS           0x4C00

#define VMCS_CR0_MASK                   0x6000
#define VMCS_CR4_MASK                   0x6002
#define VMCS_CR0_SHADOW                 0x6004
#define VMCS_CR4_SHADOW                 0x6006
#define VMCS_EXIT_QUALIFICATION         0x6400
#define VMCS_GUEST_CR0                  0x6800
#define VMCS_GUEST_CR3                  0x6802
#define VMCS_GUEST_CR4                  0x6804
#define VMCS_GUEST_ES_BASE              0x6806
#define VMCS_GUEST_CS_BASE              0x6808
#define VMCS_GUEST_SS_BASE              0x680A
#define VMCS_GUEST_DS_BASE              0x680C
#define VMCS_GUEST_FS_BASE              0x680E
#define VMCS_GUEST_GS_BASE              0x6810
#define VMCS_GUEST_LDTR_BASE            0x6812
#define VMCS_GUEST_TR_BASE              0x6814
#define VMCS_GUEST_GDTR_BASE            0x6816
#define VMCS_GUEST_IDTR_BASE            0x6818
#define VMCS_GUEST_DR7                  0x681A
#define VMCS_GUEST_RSP                  0x681C
#define VMCS_GUEST_RIP                  0x681E
#define VMCS_GUEST_RFLAGS               0x6820
#define VMCS_GUEST_PENDING_DEBUG        0x6822
#define VMCS_GUEST_SYSENTER_ESP         0x6824
#define VMCS_GUEST_SYSENTER_EIP         0x6826
#define VMCS_HOST_CR0                   0x6C00
#define VMCS_HOST_CR3                   0x6C02
#define VMCS_HOST_CR4                   0x6C04
#define VMCS_HOST_FS_BASE               0x6C06
#define VMCS_HOST_GS_BASE               0x6C08
#define VMCS_HOST_TR_BASE               0x6C0A
#define VMCS_HOST_GDTR_BASE             0x6C0C
#define VMCS_HOST_IDTR_BASE             0x6C0E
#define VMCS_HOST_SYSENTER_ESP          0x6C10
#define VMCS_HOST_SYSENTER_EIP          0x6C12
/* HOST_RSP (0x6C14) and HOST_RIP (0x6C16) are written by vmx_enter.asm, just before entering. */

/* ---- Controls ---- */

#define PIN_EXTERNAL_INTERRUPT_EXITING  (1u << 0)

#define PROC_HLT_EXITING                (1u << 7)
#define PROC_MWAIT_EXITING              (1u << 10)
#define PROC_RDPMC_EXITING              (1u << 11)
#define PROC_CR3_LOAD_EXITING           (1u << 15)
#define PROC_CR3_STORE_EXITING          (1u << 16)
#define PROC_MOV_DR_EXITING             (1u << 23)
#define PROC_UNCONDITIONAL_IO_EXITING   (1u << 24)
#define PROC_USE_MSR_BITMAPS            (1u << 28)
#define PROC_MONITOR_EXITING            (1u << 29)
#define PROC_SECONDARY_CONTROLS         (1u << 31)

#define PROC2_ENABLE_EPT                (1u << 1)
#define PROC2_ENABLE_RDTSCP             (1u << 3)
#define PROC2_WBINVD_EXITING            (1u << 6)
#define PROC2_ENABLE_INVPCID            (1u << 12)
#define PROC2_ENABLE_XSAVES             (1u << 20)

#define EXIT_SAVE_DEBUG_CONTROLS        (1u << 2)
#define EXIT_HOST_ADDRESS_SPACE_SIZE    (1u << 9)
#define EXIT_SAVE_EFER                  (1u << 20)
#define EXIT_LOAD_EFER                  (1u << 21)

#define ENTRY_LOAD_DEBUG_CONTROLS       (1u << 2)
#define ENTRY_IA32E_MODE_GUEST          (1u << 9)
#define ENTRY_LOAD_EFER                 (1u << 15)

/* ---- Exit reasons ---- */

#define EXIT_REASON_EXTERNAL_INTERRUPT  1
#define EXIT_REASON_CPUID               10
#define EXIT_REASON_HLT                 12
#define EXIT_REASON_ENTRY_FAILED        0x80000000u

/* ---- EPT ---- */

#define EPT_READ_WRITE_EXECUTE          7ULL
#define EPT_MEMORY_TYPE_WB              (6ULL << 3)
#define EPT_ADDRESS                     0x000FFFFFFFFFF000ULL
#define EPTP_WALK_LENGTH_4              (3ULL << 3)
#define EPTP_MEMORY_TYPE_WB             6ULL

/* ---- Guest control registers it may ask for ---- */

#define CR4_VMXE                        (1ULL << 13)
/* PSE, PAE, PGE, OSFXSR, OSXMMEXCPT, FSGSBASE, OSXSAVE. */
#define CR4_GUEST_ALLOWED               ((1ULL << 4) | (1ULL << 5) | (1ULL << 7) | (1ULL << 9) | (1ULL << 10) | (1ULL << 16) | (1ULL << 18))
/* SCE, LME, LMA, NXE. */
#define EFER_GUEST_ALLOWED              ((1ULL << 0) | (1ULL << 8) | (1ULL << 10) | (1ULL << 11))

/* The MSRs switched on every entry and exit, in this order. */
#define WINKVM_SWITCHED_MSRS 5

/* ---- VMs and vCPUs ---- */

/* What vmx_enter.asm moves in and out of the guest. Its offsets are vmx_enter.asm's too. */
typedef struct _WINKVM_GUEST_CONTEXT {
    UINT64 Gpr[16];        /* encoding order; Gpr[4], rsp, lives in the VMCS */
    PVOID  Xsave;          /* the guest's XSAVE area */
    UINT64 XsaveMask;
    UINT64 Cr2;
} WINKVM_GUEST_CONTEXT;

C_ASSERT(FIELD_OFFSET(WINKVM_GUEST_CONTEXT, Xsave) == 128);
C_ASSERT(FIELD_OFFSET(WINKVM_GUEST_CONTEXT, XsaveMask) == 136);
C_ASSERT(FIELD_OFFSET(WINKVM_GUEST_CONTEXT, Cr2) == 144);

typedef struct _WK_VM WK_VM;

typedef struct _WK_VCPU {
    volatile LONG Used;
    UINT32 Id;
    WK_VM* Vm;

    PVOID Vmcs;
    UINT64 VmcsPhys;
    /* One page: the guest's MSRs (loaded on entry, stored on exit) at 0, the host's (loaded on exit) at 0x800. */
    WINKVM_MSR* GuestMsrs;
    UINT64 GuestMsrsPhys;
    WINKVM_MSR* HostMsrs;
    UINT64 HostMsrsPhys;

    WINKVM_GUEST_CONTEXT Context;
    WINKVM_SYSTEM System;
    BOOLEAN HasSystem;

    volatile LONG Running;
    volatile LONG Cancel;
    /* The processor it's running on, while it is. */
    volatile LONG Cpu;
} WK_VCPU;

#define WINKVM_MEMORY_NONE    0
#define WINKVM_MEMORY_SETTING 1
#define WINKVM_MEMORY_READY   2

struct _WK_VM {
    UINT32 Id;
    PFILE_OBJECT Owner;
    volatile LONG Refs;
    volatile LONG Dying;

    volatile LONG MemoryState;
    UINT64 GuestPhys;
    UINT64 Size;
    PMDL* Mdls;
    ULONG MdlCount;

    UINT64* EptRoot;
    UINT64 Eptp;
    PVOID* EptTables;
    /* Each table's physical address, beside it: how an entry is followed to the table it points at. */
    UINT64* EptTablePhys;
    ULONG EptTableCount;
    ULONG EptTableCapacity;

    FAST_MUTEX VcpuLock;
    ULONG VcpuCount;
    WK_VCPU Vcpus[WINKVM_MAX_VCPUS];
};

typedef struct _WINKVM_GLOBALS {
    PDEVICE_OBJECT Device;
    FAST_MUTEX Lock;                    /* the VM table */
    WK_VM* Vms[WINKVM_MAX_VMS];

    BOOLEAN VmxReady;
    ULONG CpuCount;
    PVOID* VmxonRegions;                /* per processor */
    UINT64* VmxonPhys;
    BOOLEAN* VmxOn;
    BOOLEAN* SetVmxe;
    PKDPC PokeDpcs;                     /* per processor: makes a CPU in a guest exit */

    UINT32 Revision;
    UINT32 Pin, Proc, Proc2, Exit, Entry;
    UINT64 Cr0Fixed0, Cr0Fixed1, Cr4Fixed0, Cr4Fixed1;
    UINT64 XsaveMask;
    ULONG XsaveSize;
    ULONG MxcsrMask;
    ULONG PhysBits;
    UINT64 InveptType;
} WINKVM_GLOBALS;

extern WINKVM_GLOBALS g;

/* ---- vmx.c ---- */
NTSTATUS VmxInitialize(VOID);
VOID VmxShutdown(VOID);
NTSTATUS VcpuInitialize(WK_VCPU* vcpu);
VOID VcpuFree(WK_VCPU* vcpu);
NTSTATUS VcpuSetSystem(WK_VCPU* vcpu, const WINKVM_SYSTEM* system);
VOID VcpuGetFp(WK_VCPU* vcpu, WINKVM_FP* fp);
VOID VcpuSetFp(WK_VCPU* vcpu, const WINKVM_FP* fp);
NTSTATUS VcpuRun(WK_VCPU* vcpu, WINKVM_RUN* run);
VOID VcpuPoke(WK_VCPU* vcpu);

/* ---- ept.c ---- */
NTSTATUS EptSetMemory(WK_VM* vm, const WINKVM_MEMORY* memory);
VOID EptFree(WK_VM* vm);

/* ---- vmx_enter.asm ---- */
UINT64 WinKvmEnter(WINKVM_GUEST_CONTEXT* context, UINT64 launched);
UINT16 WinKvmReadCs(VOID);
UINT16 WinKvmReadSs(VOID);
UINT16 WinKvmReadDs(VOID);
UINT16 WinKvmReadEs(VOID);
UINT16 WinKvmReadFs(VOID);
UINT16 WinKvmReadGs(VOID);
UINT16 WinKvmReadTr(VOID);
VOID WinKvmInvept(UINT64 type, PVOID descriptor);
