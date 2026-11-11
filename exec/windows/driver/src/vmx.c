/*
 * VMX: switching it on and off on every processor, setting up a vCPU's VMCS, and running a guest until it halts.
 *
 * A vCPU runs on the thread that calls RUN, held to one processor for the call, since a VMCS is active on one
 * processor at a time. Each RUN loads the VMCS, enters, handles the exits it can (host interrupts, CPUID) and goes
 * back in, and on any other exit reads the guest's state out and clears the VMCS, so the next RUN may be on another
 * processor. Between entries it drops to PASSIVE_LEVEL, so the host's interrupts, DPCs and scheduler all carry on.
 *
 * Everything the guest could use to touch the host exits: MSRs, I/O, debug registers, MWAIT/MONITOR, RDPMC, WBINVD,
 * XSETBV and VMX instructions. The guest's memory is only its EPT.
 */
#include "winkvm_internal.h"

#pragma pack(push, 1)
typedef struct _DESCRIPTOR_TABLE {
    UINT16 Limit;
    UINT64 Base;
} DESCRIPTOR_TABLE;
#pragma pack(pop)

/* The MSRs switched on every entry and exit. */
static const UINT32 SwitchedMsrs[WINKVM_SWITCHED_MSRS] = { MSR_STAR, MSR_LSTAR, MSR_CSTAR, MSR_SFMASK, MSR_KERNEL_GS_BASE };

static UINT64 VmRead(SIZE_T field)
{
    SIZE_T value = 0;
    __vmx_vmread(field, &value);
    return value;
}

static VOID VmWrite(SIZE_T field, UINT64 value)
{
    __vmx_vmwrite(field, (SIZE_T)value);
}

/* What `want` can be, given a VMX capability MSR: its low half must be 1, its high half may be. */
static UINT32 AdjustControls(UINT32 want, ULONG msr)
{
    UINT64 allowed = __readmsr(msr);
    want |= (UINT32)allowed;
    want &= (UINT32)(allowed >> 32);
    return want;
}

/* ---- On and off ---- */

static ULONG_PTR VmxOnIpi(ULONG_PTR argument)
{
    UNREFERENCED_PARAMETER(argument);
    ULONG cpu = KeGetCurrentProcessorNumberEx(NULL);
    if (cpu >= g.CpuCount) {
        return 0;
    }
    UINT64 control = __readmsr(MSR_IA32_FEATURE_CONTROL);
    if (!(control & 1)) {
        /* Unlocked: allow VMXON outside SMX, and lock it. */
        __writemsr(MSR_IA32_FEATURE_CONTROL, control | 1 | 4);
    } else if (!(control & 4)) {
        return 0; /* the firmware switched VMX off */
    }
    UINT64 cr4 = __readcr4();
    if (cr4 & CR4_VMXE) {
        return 0; /* another hypervisor has this processor's VMX: don't share it */
    }
    __writecr0((__readcr0() | g.Cr0Fixed0) & g.Cr0Fixed1);
    __writecr4((cr4 | g.Cr4Fixed0) & g.Cr4Fixed1);
    g.SetVmxe[cpu] = TRUE;
    if (__vmx_on(&g.VmxonPhys[cpu]) == 0) {
        g.VmxOn[cpu] = TRUE;
    }
    return 0;
}

static ULONG_PTR VmxOffIpi(ULONG_PTR argument)
{
    UNREFERENCED_PARAMETER(argument);
    ULONG cpu = KeGetCurrentProcessorNumberEx(NULL);
    if (cpu >= g.CpuCount) {
        return 0;
    }
    if (g.VmxOn[cpu]) {
        __vmx_off();
        g.VmxOn[cpu] = FALSE;
    }
    if (g.SetVmxe[cpu]) {
        __writecr4(__readcr4() & ~CR4_VMXE);
        g.SetVmxe[cpu] = FALSE;
    }
    return 0;
}

/* Does nothing: being sent to a processor running a guest is what makes the guest exit. */
static VOID PokeDpc(PKDPC dpc, PVOID context, PVOID argument1, PVOID argument2)
{
    UNREFERENCED_PARAMETER(dpc);
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(argument1);
    UNREFERENCED_PARAMETER(argument2);
}

static PVOID AllocatePage(UINT64* phys)
{
    /* A page-sized nonpaged allocation is one page, page-aligned, so physically contiguous. */
    PVOID page = ExAllocatePool2(POOL_FLAG_NON_PAGED, PAGE_SIZE, WINKVM_TAG);
    if (page && phys) {
        *phys = (UINT64)MmGetPhysicalAddress(page).QuadPart;
    }
    return page;
}

static VOID FreeArrays(VOID)
{
    if (g.VmxonRegions) {
        for (ULONG i = 0; i < g.CpuCount; i++) {
            if (g.VmxonRegions[i]) {
                ExFreePoolWithTag(g.VmxonRegions[i], WINKVM_TAG);
            }
        }
    }
    PVOID arrays[] = { g.VmxonRegions, g.VmxonPhys, g.VmxOn, g.SetVmxe, g.PokeDpcs };
    for (ULONG i = 0; i < RTL_NUMBER_OF(arrays); i++) {
        if (arrays[i]) {
            ExFreePoolWithTag(arrays[i], WINKVM_TAG);
        }
    }
    g.VmxonRegions = NULL;
    g.VmxonPhys = NULL;
    g.VmxOn = NULL;
    g.SetVmxe = NULL;
    g.PokeDpcs = NULL;
}

NTSTATUS VmxInitialize(VOID)
{
    int r[4];
    __cpuid(r, 1);
    if (!(r[2] & (1 << 5))) {
        return STATUS_NOT_SUPPORTED; /* no VT-x */
    }
    if (!(r[2] & (1 << 26)) || !(r[2] & (1 << 27))) {
        return STATUS_NOT_SUPPORTED; /* no XSAVE, or the OS hasn't switched it on */
    }

    UINT64 basic = __readmsr(MSR_IA32_VMX_BASIC);
    if (((basic >> 50) & 0xF) != 6) {
        return STATUS_NOT_SUPPORTED; /* VMCS must be write-back memory */
    }
    g.Revision = (UINT32)(basic & 0x7FFFFFFF);
    BOOLEAN useTrue = (basic >> 55) & 1;

    g.Pin = AdjustControls(PIN_EXTERNAL_INTERRUPT_EXITING, useTrue ? MSR_IA32_VMX_TRUE_PINBASED_CTLS : MSR_IA32_VMX_PINBASED_CTLS);
    g.Proc = AdjustControls(PROC_HLT_EXITING | PROC_MWAIT_EXITING | PROC_RDPMC_EXITING | PROC_MOV_DR_EXITING | PROC_UNCONDITIONAL_IO_EXITING
                                | PROC_MONITOR_EXITING | PROC_SECONDARY_CONTROLS,
                            useTrue ? MSR_IA32_VMX_TRUE_PROCBASED_CTLS : MSR_IA32_VMX_PROCBASED_CTLS);
    g.Proc2 = AdjustControls(PROC2_ENABLE_EPT | PROC2_ENABLE_RDTSCP | PROC2_WBINVD_EXITING | PROC2_ENABLE_INVPCID | PROC2_ENABLE_XSAVES,
                             MSR_IA32_VMX_PROCBASED_CTLS2);
    g.Exit = AdjustControls(EXIT_SAVE_DEBUG_CONTROLS | EXIT_HOST_ADDRESS_SPACE_SIZE | EXIT_SAVE_EFER | EXIT_LOAD_EFER,
                            useTrue ? MSR_IA32_VMX_TRUE_EXIT_CTLS : MSR_IA32_VMX_EXIT_CTLS);
    g.Entry = AdjustControls(ENTRY_LOAD_DEBUG_CONTROLS | ENTRY_IA32E_MODE_GUEST | ENTRY_LOAD_EFER,
                             useTrue ? MSR_IA32_VMX_TRUE_ENTRY_CTLS : MSR_IA32_VMX_ENTRY_CTLS);

    /* What it can't do without. MSR bitmaps stay off, so every MSR access exits. */
    UINT32 proc = PROC_HLT_EXITING | PROC_MOV_DR_EXITING | PROC_UNCONDITIONAL_IO_EXITING | PROC_SECONDARY_CONTROLS;
    if (!(g.Pin & PIN_EXTERNAL_INTERRUPT_EXITING) || (g.Proc & proc) != proc || (g.Proc & PROC_USE_MSR_BITMAPS)
        || !(g.Proc2 & PROC2_ENABLE_EPT) || !(g.Proc2 & PROC2_WBINVD_EXITING)
        || (g.Exit & (EXIT_HOST_ADDRESS_SPACE_SIZE | EXIT_SAVE_EFER | EXIT_LOAD_EFER)) != (EXIT_HOST_ADDRESS_SPACE_SIZE | EXIT_SAVE_EFER | EXIT_LOAD_EFER)
        || (g.Entry & (ENTRY_IA32E_MODE_GUEST | ENTRY_LOAD_EFER)) != (ENTRY_IA32E_MODE_GUEST | ENTRY_LOAD_EFER)) {
        return STATUS_NOT_SUPPORTED;
    }

    /* EPT: 4-level walks, write-back tables, and INVEPT. */
    UINT64 ept = __readmsr(MSR_IA32_VMX_EPT_VPID_CAP);
    if (!(ept & (1ULL << 6)) || !(ept & (1ULL << 14)) || !(ept & (1ULL << 20))) {
        return STATUS_NOT_SUPPORTED;
    }
    if (ept & (1ULL << 25)) {
        g.InveptType = 1; /* single context */
    } else if (ept & (1ULL << 26)) {
        g.InveptType = 2; /* all contexts */
    } else {
        return STATUS_NOT_SUPPORTED;
    }

    g.Cr0Fixed0 = __readmsr(MSR_IA32_VMX_CR0_FIXED0);
    g.Cr0Fixed1 = __readmsr(MSR_IA32_VMX_CR0_FIXED1);
    g.Cr4Fixed0 = __readmsr(MSR_IA32_VMX_CR4_FIXED0);
    g.Cr4Fixed1 = __readmsr(MSR_IA32_VMX_CR4_FIXED1);

    /* The guest's extended state: x87, SSE, AVX and AVX-512, as the host has them. Not AMX, whose state the kernel
       may not touch. */
    g.XsaveMask = _xgetbv(0) & 0xE7;
    __cpuidex(r, 0xD, 0);
    g.XsaveSize = (ULONG)r[1];
    if (g.XsaveSize > PAGE_SIZE) {
        return STATUS_NOT_SUPPORTED;
    }
    DECLSPEC_ALIGN(16) UCHAR fx[512] = { 0 };
    _fxsave(fx);
    g.MxcsrMask = *(ULONG*)(fx + 28);
    if (g.MxcsrMask == 0) {
        g.MxcsrMask = 0xFFBF;
    }

    __cpuid(r, 0x80000000);
    g.PhysBits = 36;
    if ((ULONG)r[0] >= 0x80000008) {
        __cpuid(r, 0x80000008);
        g.PhysBits = r[0] & 0xFF;
    }

    g.CpuCount = KeQueryActiveProcessorCountEx(ALL_PROCESSOR_GROUPS);
    g.VmxonRegions = ExAllocatePool2(POOL_FLAG_NON_PAGED, g.CpuCount * sizeof(PVOID), WINKVM_TAG);
    g.VmxonPhys = ExAllocatePool2(POOL_FLAG_NON_PAGED, g.CpuCount * sizeof(UINT64), WINKVM_TAG);
    g.VmxOn = ExAllocatePool2(POOL_FLAG_NON_PAGED, g.CpuCount * sizeof(BOOLEAN), WINKVM_TAG);
    g.SetVmxe = ExAllocatePool2(POOL_FLAG_NON_PAGED, g.CpuCount * sizeof(BOOLEAN), WINKVM_TAG);
    g.PokeDpcs = ExAllocatePool2(POOL_FLAG_NON_PAGED, g.CpuCount * sizeof(KDPC), WINKVM_TAG);
    if (!g.VmxonRegions || !g.VmxonPhys || !g.VmxOn || !g.SetVmxe || !g.PokeDpcs) {
        FreeArrays();
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    for (ULONG i = 0; i < g.CpuCount; i++) {
        g.VmxonRegions[i] = AllocatePage(&g.VmxonPhys[i]);
        if (!g.VmxonRegions[i]) {
            FreeArrays();
            return STATUS_INSUFFICIENT_RESOURCES;
        }
        *(UINT32*)g.VmxonRegions[i] = g.Revision;

        PROCESSOR_NUMBER number;
        KeInitializeDpc(&g.PokeDpcs[i], PokeDpc, NULL);
        KeSetImportanceDpc(&g.PokeDpcs[i], HighImportance);
        if (NT_SUCCESS(KeGetProcessorNumberFromIndex(i, &number))) {
            KeSetTargetProcessorDpcEx(&g.PokeDpcs[i], &number);
        }
    }

    KeIpiGenericCall(VmxOnIpi, 0);
    for (ULONG i = 0; i < g.CpuCount; i++) {
        if (!g.VmxOn[i]) {
            KeIpiGenericCall(VmxOffIpi, 0);
            FreeArrays();
            return STATUS_NOT_SUPPORTED;
        }
    }
    g.VmxReady = TRUE;
    return STATUS_SUCCESS;
}

VOID VmxShutdown(VOID)
{
    if (!g.VmxonRegions) {
        return;
    }
    KeFlushQueuedDpcs();
    KeIpiGenericCall(VmxOffIpi, 0);
    FreeArrays();
    g.VmxReady = FALSE;
}

/* ---- vCPUs ---- */

NTSTATUS VcpuInitialize(WK_VCPU* vcpu)
{
    vcpu->Cpu = -1;
    vcpu->Vmcs = AllocatePage(&vcpu->VmcsPhys);
    UINT64 msrsPhys = 0;
    PUCHAR msrs = AllocatePage(&msrsPhys);
    vcpu->Context.Xsave = AllocatePage(NULL);
    if (!vcpu->Vmcs || !msrs || !vcpu->Context.Xsave) {
        if (msrs) {
            ExFreePoolWithTag(msrs, WINKVM_TAG);
        }
        VcpuFree(vcpu);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    *(UINT32*)vcpu->Vmcs = g.Revision;
    vcpu->GuestMsrs = (WINKVM_MSR*)msrs;
    vcpu->GuestMsrsPhys = msrsPhys;
    vcpu->HostMsrs = (WINKVM_MSR*)(msrs + 0x800);
    vcpu->HostMsrsPhys = msrsPhys + 0x800;
    for (ULONG i = 0; i < WINKVM_SWITCHED_MSRS; i++) {
        vcpu->GuestMsrs[i].Index = SwitchedMsrs[i];
        vcpu->HostMsrs[i].Index = SwitchedMsrs[i];
    }
    /* A zeroed XSAVE header: the first XRSTOR puts every component in its initial state. */
    vcpu->Context.XsaveMask = g.XsaveMask;
    /* Its launch state is "clear" from here, as VMPTRLD needs. */
    if (__vmx_vmclear(&vcpu->VmcsPhys) != 0) {
        VcpuFree(vcpu);
        return STATUS_UNSUCCESSFUL;
    }
    return STATUS_SUCCESS;
}

VOID VcpuFree(WK_VCPU* vcpu)
{
    if (vcpu->Vmcs) {
        ExFreePoolWithTag(vcpu->Vmcs, WINKVM_TAG);
    }
    if (vcpu->GuestMsrs) {
        ExFreePoolWithTag(vcpu->GuestMsrs, WINKVM_TAG);
    }
    if (vcpu->Context.Xsave) {
        ExFreePoolWithTag(vcpu->Context.Xsave, WINKVM_TAG);
    }
    vcpu->Vmcs = NULL;
    vcpu->GuestMsrs = NULL;
    vcpu->HostMsrs = NULL;
    vcpu->Context.Xsave = NULL;
}

NTSTATUS VcpuSetSystem(WK_VCPU* vcpu, const WINKVM_SYSTEM* system)
{
    if (system->MsrCount > RTL_NUMBER_OF(system->Msrs)) {
        return STATUS_INVALID_PARAMETER;
    }
    UINT64 values[WINKVM_SWITCHED_MSRS] = { 0 };
    for (ULONG i = 0; i < system->MsrCount; i++) {
        const WINKVM_MSR* msr = &system->Msrs[i];
        if (msr->Index == MSR_FS_BASE || msr->Index == MSR_GS_BASE) {
            continue; /* these are the segments' bases, in RUN */
        }
        ULONG slot = 0;
        while (slot < WINKVM_SWITCHED_MSRS && SwitchedMsrs[slot] != msr->Index) {
            slot++;
        }
        if (slot == WINKVM_SWITCHED_MSRS) {
            return STATUS_INVALID_PARAMETER;
        }
        values[slot] = msr->Value;
    }
    for (ULONG i = 0; i < WINKVM_SWITCHED_MSRS; i++) {
        vcpu->GuestMsrs[i].Value = values[i];
    }
    vcpu->System = *system;
    vcpu->HasSystem = TRUE;
    return STATUS_SUCCESS;
}

VOID VcpuGetFp(WK_VCPU* vcpu, WINKVM_FP* fp)
{
    RtlCopyMemory(fp->Fxsave, vcpu->Context.Xsave, sizeof(fp->Fxsave));
}

VOID VcpuSetFp(WK_VCPU* vcpu, const WINKVM_FP* fp)
{
    PUCHAR area = vcpu->Context.Xsave;
    RtlCopyMemory(area, fp->Fxsave, sizeof(fp->Fxsave));
    /* XRSTOR faults on reserved MXCSR bits, which would be the host's fault, in the kernel: never let them through. */
    *(ULONG*)(area + 24) &= g.MxcsrMask;
    /* x87 and SSE come from the legacy area now; the header's other fields stay standard. */
    *(UINT64*)(area + 512) |= 3;
    *(UINT64*)(area + 520) = 0;
}

/* Interrupts this vCPU if it's in its guest. */
VOID VcpuPoke(WK_VCPU* vcpu)
{
    LONG cpu = vcpu->Cpu;
    if (cpu >= 0 && (ULONG)cpu < g.CpuCount && (ULONG)cpu != KeGetCurrentProcessorNumberEx(NULL)) {
        KeInsertQueueDpc(&g.PokeDpcs[cpu], NULL, NULL);
    }
}

/* ---- The VMCS ---- */

static UINT64 SegmentBase(UINT64 gdt, UINT16 selector)
{
    PUCHAR d = (PUCHAR)gdt + (selector & ~7);
    UINT64 base = (UINT64)d[2] | (UINT64)d[3] << 8 | (UINT64)d[4] << 16 | (UINT64)d[7] << 24;
    if (!(d[5] & 0x10)) {
        base |= (UINT64)*(UINT32*)(d + 8) << 32; /* a system descriptor is 16 bytes */
    }
    return base;
}

/* VMX's access rights, from a WINKVM_SEGMENT. */
static UINT32 AccessRights(const WINKVM_SEGMENT* s)
{
    if (!(s->Flags & 1)) {
        return 1u << 16; /* unusable */
    }
    return (s->Type & 0xF) | ((s->Flags >> 4) & 1) << 4 | ((s->Flags >> 1) & 3) << 5 | 1u << 7 | ((s->Flags >> 5) & 1) << 13
         | ((s->Flags >> 3) & 1) << 14 | ((s->Flags >> 6) & 1) << 15;
}

/* Segment registers by VMX's numbering: ES, CS, SS, DS, FS, GS, LDTR, TR. */
static VOID WriteSegment(ULONG index, const WINKVM_SEGMENT* s)
{
    VmWrite(VMCS_GUEST_ES_SELECTOR + 2 * index, s->Selector);
    VmWrite(VMCS_GUEST_ES_BASE + 2 * index, s->Base);
    VmWrite(VMCS_GUEST_ES_LIMIT + 2 * index, s->Limit);
    VmWrite(VMCS_GUEST_ES_AR + 2 * index, AccessRights(s));
}

static VOID ReadSegment(ULONG index, WINKVM_SEGMENT* s)
{
    UINT32 ar = (UINT32)VmRead(VMCS_GUEST_ES_AR + 2 * index);
    s->Selector = (UINT16)VmRead(VMCS_GUEST_ES_SELECTOR + 2 * index);
    s->Base = VmRead(VMCS_GUEST_ES_BASE + 2 * index);
    s->Limit = (UINT32)VmRead(VMCS_GUEST_ES_LIMIT + 2 * index);
    s->Type = (UINT8)(ar & 0xF);
    s->Flags = (ar & (1u << 16)) ? 0
             : (UINT8)(((ar >> 7) & 1) | ((ar >> 5) & 3) << 1 | ((ar >> 14) & 1) << 3 | ((ar >> 4) & 1) << 4 | ((ar >> 13) & 1) << 5
                       | ((ar >> 15) & 1) << 6);
}

static VOID WriteControls(WK_VCPU* vcpu)
{
    const WINKVM_SYSTEM* s = &vcpu->System;
    VmWrite(VMCS_PIN_CONTROLS, g.Pin);
    VmWrite(VMCS_PROC_CONTROLS, g.Proc);
    VmWrite(VMCS_PROC_CONTROLS2, g.Proc2);
    VmWrite(VMCS_EXIT_CONTROLS, g.Exit);
    VmWrite(VMCS_ENTRY_CONTROLS, g.Entry);
    VmWrite(VMCS_EXCEPTION_BITMAP, 0);
    VmWrite(VMCS_CR3_TARGET_COUNT, 0);
    VmWrite(VMCS_CR0_MASK, 0);
    VmWrite(VMCS_CR0_SHADOW, s->Cr0);
    /* VMXE must be on in the guest's CR4, but it never sees it. */
    VmWrite(VMCS_CR4_MASK, CR4_VMXE);
    VmWrite(VMCS_CR4_SHADOW, s->Cr4 & CR4_GUEST_ALLOWED);
    VmWrite(VMCS_EPT_POINTER, vcpu->Vm->Eptp);
    VmWrite(VMCS_ENTRY_MSR_LOAD_ADDR, vcpu->GuestMsrsPhys);
    VmWrite(VMCS_ENTRY_MSR_LOAD_COUNT, WINKVM_SWITCHED_MSRS);
    VmWrite(VMCS_EXIT_MSR_STORE_ADDR, vcpu->GuestMsrsPhys);
    VmWrite(VMCS_EXIT_MSR_STORE_COUNT, WINKVM_SWITCHED_MSRS);
    VmWrite(VMCS_EXIT_MSR_LOAD_ADDR, vcpu->HostMsrsPhys);
    VmWrite(VMCS_EXIT_MSR_LOAD_COUNT, WINKVM_SWITCHED_MSRS);
    VmWrite(VMCS_ENTRY_INTERRUPTION_INFO, 0);
    VmWrite(VMCS_LINK_POINTER, ~0ULL);
}

/* The host as it is on this processor, for this thread. */
static VOID WriteHost(VOID)
{
    DESCRIPTOR_TABLE gdt, idt;
    _sgdt(&gdt);
    __sidt(&idt);
    UINT16 tr = WinKvmReadTr();
    VmWrite(VMCS_HOST_CR0, __readcr0());
    VmWrite(VMCS_HOST_CR3, __readcr3());
    VmWrite(VMCS_HOST_CR4, __readcr4());
    VmWrite(VMCS_HOST_CS_SELECTOR, WinKvmReadCs() & 0xF8);
    VmWrite(VMCS_HOST_SS_SELECTOR, WinKvmReadSs() & 0xF8);
    VmWrite(VMCS_HOST_DS_SELECTOR, WinKvmReadDs() & 0xF8);
    VmWrite(VMCS_HOST_ES_SELECTOR, WinKvmReadEs() & 0xF8);
    VmWrite(VMCS_HOST_FS_SELECTOR, WinKvmReadFs() & 0xF8);
    VmWrite(VMCS_HOST_GS_SELECTOR, WinKvmReadGs() & 0xF8);
    VmWrite(VMCS_HOST_TR_SELECTOR, tr & 0xF8);
    VmWrite(VMCS_HOST_FS_BASE, __readmsr(MSR_FS_BASE));
    VmWrite(VMCS_HOST_GS_BASE, __readmsr(MSR_GS_BASE));
    VmWrite(VMCS_HOST_TR_BASE, SegmentBase(gdt.Base, tr));
    VmWrite(VMCS_HOST_GDTR_BASE, gdt.Base);
    VmWrite(VMCS_HOST_IDTR_BASE, idt.Base);
    VmWrite(VMCS_HOST_SYSENTER_CS, __readmsr(MSR_IA32_SYSENTER_CS));
    VmWrite(VMCS_HOST_SYSENTER_ESP, __readmsr(MSR_IA32_SYSENTER_ESP));
    VmWrite(VMCS_HOST_SYSENTER_EIP, __readmsr(MSR_IA32_SYSENTER_EIP));
    VmWrite(VMCS_HOST_EFER, __readmsr(MSR_IA32_EFER));
}

/* The guest, from its system state and the registers RUN brought. Anything inconsistent fails VM entry's checks,
   which is reported, never acted on. */
static VOID WriteGuest(WK_VCPU* vcpu, const WINKVM_REGS* r)
{
    const WINKVM_SYSTEM* s = &vcpu->System;
    VmWrite(VMCS_GUEST_CR0, (s->Cr0 | g.Cr0Fixed0) & g.Cr0Fixed1);
    VmWrite(VMCS_GUEST_CR4, ((s->Cr4 & CR4_GUEST_ALLOWED) | g.Cr4Fixed0) & g.Cr4Fixed1);
    VmWrite(VMCS_GUEST_CR3, r->Cr3);
    VmWrite(VMCS_GUEST_EFER, s->Efer & EFER_GUEST_ALLOWED);
    VmWrite(VMCS_GUEST_DR7, 0x400);
    VmWrite(VMCS_GUEST_DEBUGCTL, 0);
    VmWrite(VMCS_GUEST_GDTR_BASE, s->GdtBase);
    VmWrite(VMCS_GUEST_GDTR_LIMIT, s->GdtLimit);
    VmWrite(VMCS_GUEST_IDTR_BASE, s->IdtBase);
    VmWrite(VMCS_GUEST_IDTR_LIMIT, s->IdtLimit);

    WINKVM_SEGMENT ldtr = { 0 };
    WriteSegment(0, &r->Es);
    WriteSegment(1, &r->Cs);
    WriteSegment(2, &r->Ss);
    WriteSegment(3, &r->Ds);
    WriteSegment(4, &r->Fs);
    WriteSegment(5, &r->Gs);
    WriteSegment(6, &ldtr);
    WriteSegment(7, &s->Tr);

    VmWrite(VMCS_GUEST_RSP, r->Gpr[4]);
    VmWrite(VMCS_GUEST_RIP, r->Rip);
    /* Bit 1 is always set; VM (virtual-8086) never is. */
    VmWrite(VMCS_GUEST_RFLAGS, (r->Rflags & 0x3F7FD5 & ~(1ULL << 17)) | 2);
    VmWrite(VMCS_GUEST_INTERRUPTIBILITY, 0);
    VmWrite(VMCS_GUEST_ACTIVITY, 0);
    VmWrite(VMCS_GUEST_PENDING_DEBUG, 0);
    VmWrite(VMCS_GUEST_SYSENTER_CS, 0);
    VmWrite(VMCS_GUEST_SYSENTER_ESP, 0);
    VmWrite(VMCS_GUEST_SYSENTER_EIP, 0);
}

static VOID ReadGuest(WK_VCPU* vcpu, WINKVM_REGS* r)
{
    RtlCopyMemory(r->Gpr, vcpu->Context.Gpr, sizeof(r->Gpr));
    r->Gpr[4] = VmRead(VMCS_GUEST_RSP);
    r->Rip = VmRead(VMCS_GUEST_RIP);
    r->Rflags = VmRead(VMCS_GUEST_RFLAGS);
    r->Cr2 = vcpu->Context.Cr2;
    r->Cr3 = VmRead(VMCS_GUEST_CR3);
    ReadSegment(0, &r->Es);
    ReadSegment(1, &r->Cs);
    ReadSegment(2, &r->Ss);
    ReadSegment(3, &r->Ds);
    ReadSegment(4, &r->Fs);
    ReadSegment(5, &r->Gs);
}

static VOID SkipInstruction(VOID)
{
    VmWrite(VMCS_GUEST_RIP, VmRead(VMCS_GUEST_RIP) + VmRead(VMCS_EXIT_INSTRUCTION_LENGTH));
}

/* CPUID always exits. The host's answer, saying there's no hypervisor and no VMX, with OSXSAVE as the guest's CR4. */
static VOID EmulateCpuid(WK_VCPU* vcpu)
{
    UINT64* gpr = vcpu->Context.Gpr;
    UINT32 leaf = (UINT32)gpr[0];
    UINT32 subleaf = (UINT32)gpr[1];
    int r[4] = { 0 };
    if (leaf < 0x40000000 || leaf > 0x4FFFFFFF) {
        __cpuidex(r, (int)leaf, (int)subleaf);
    }
    if (leaf == 1) {
        r[2] &= ~(1 << 5);           /* VMX */
        r[2] &= ~(int)0x80000000;    /* hypervisor present */
        if (vcpu->System.Cr4 & (1ULL << 18)) {
            r[2] |= 1 << 27;
        } else {
            r[2] &= ~(1 << 27);
        }
    }
    gpr[0] = (UINT32)r[0]; /* rax */
    gpr[3] = (UINT32)r[1]; /* rbx */
    gpr[1] = (UINT32)r[2]; /* rcx */
    gpr[2] = (UINT32)r[3]; /* rdx */
}

/* ---- Running ---- */

NTSTATUS VcpuRun(WK_VCPU* vcpu, WINKVM_RUN* run)
{
    WK_VM* vm = vcpu->Vm;
    if (!vcpu->HasSystem || vm->MemoryState != WINKVM_MEMORY_READY) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    RtlCopyMemory(vcpu->Context.Gpr, run->Regs.Gpr, sizeof(vcpu->Context.Gpr));
    vcpu->Context.Cr2 = run->Regs.Cr2;
    run->ExitReason = 0;
    run->Qualification = 0;

    /* One processor for the whole call: the VMCS is active only where it was loaded. */
    PROCESSOR_NUMBER here;
    KeGetCurrentProcessorNumberEx(&here);
    GROUP_AFFINITY affinity = { 0 }, previous;
    affinity.Group = here.Group;
    affinity.Mask = (KAFFINITY)1 << here.Number;
    KeSetSystemGroupAffinityThread(&affinity, &previous);

    /* The calling thread's own extended state, which the guest's will replace in the registers. */
    XSTATE_SAVE save;
    NTSTATUS status = KeSaveExtendedProcessorState(g.XsaveMask, &save);
    if (!NT_SUCCESS(status)) {
        KeRevertToUserGroupAffinityThread(&previous);
        return status;
    }
    for (ULONG i = 0; i < WINKVM_SWITCHED_MSRS; i++) {
        vcpu->HostMsrs[i].Value = __readmsr(vcpu->HostMsrs[i].Index);
    }
    InterlockedExchange(&vcpu->Cpu, (LONG)KeGetCurrentProcessorNumberEx(NULL));

    BOOLEAN loaded = FALSE, launched = FALSE;
    KIRQL irql;
    for (;;) {
        if (InterlockedExchange(&vcpu->Cancel, 0) || vm->Dying || PsIsThreadTerminating(PsGetCurrentThread())) {
            run->ExitReason = WINKVM_EXIT_CANCELED;
            break;
        }
        KeRaiseIrql(DISPATCH_LEVEL, &irql);
        if (!loaded) {
            if (__vmx_vmptrld(&vcpu->VmcsPhys) != 0) {
                KeLowerIrql(irql);
                run->ExitReason = WINKVM_EXIT_FAILED_ENTRY;
                run->Qualification = 0xFFFF0001;
                break;
            }
            loaded = TRUE;
            WriteControls(vcpu);
            WriteHost();
            WriteGuest(vcpu, &run->Regs);
            /* Translations from an EPT whose tables were another VM's must go. */
            UINT64 descriptor[2] = { vm->Eptp, 0 };
            WinKvmInvept(g.InveptType, descriptor);
        } else {
            /* Another vCPU may have run on this processor since: make this one current again. */
            UINT64 current = 0;
            __vmx_vmptrst(&current);
            if (current != vcpu->VmcsPhys) {
                __vmx_vmptrld(&vcpu->VmcsPhys);
            }
        }

        /* What a VM exit doesn't put back as it was. */
        DESCRIPTOR_TABLE gdt, idt;
        _sgdt(&gdt);
        __sidt(&idt);
        UINT64 dr7 = __readdr(7);
        UINT64 debugctl = __readmsr(MSR_IA32_DEBUGCTL);

        UINT64 failed = WinKvmEnter(&vcpu->Context, launched);

        _lgdt(&gdt);
        __lidt(&idt);
        __writemsr(MSR_IA32_DEBUGCTL, debugctl);
        __writedr(7, dr7);

        if (failed) {
            run->ExitReason = WINKVM_EXIT_FAILED_ENTRY;
            run->Qualification = VmRead(VMCS_INSTRUCTION_ERROR);
            KeLowerIrql(irql);
            break;
        }
        launched = TRUE;
        UINT32 reason = (UINT32)VmRead(VMCS_EXIT_REASON);
        if (reason & EXIT_REASON_ENTRY_FAILED) {
            run->ExitReason = WINKVM_EXIT_FAILED_ENTRY;
            run->Qualification = reason;
            KeLowerIrql(irql);
            break;
        }
        switch (reason & 0xFFFF) {
        case EXIT_REASON_EXTERNAL_INTERRUPT:
            /* The host takes it as soon as interrupts are back on, which they are; dropping to PASSIVE_LEVEL lets its
               DPCs and the scheduler run too. */
            KeLowerIrql(irql);
            continue;
        case EXIT_REASON_CPUID:
            EmulateCpuid(vcpu);
            SkipInstruction();
            KeLowerIrql(irql);
            continue;
        case EXIT_REASON_HLT:
            /* As KVM does: past the halt. */
            SkipInstruction();
            run->ExitReason = WINKVM_EXIT_HLT;
            break;
        default:
            run->ExitReason = WINKVM_EXIT_OTHER;
            run->Qualification = VmRead(VMCS_EXIT_QUALIFICATION) << 16 | (reason & 0xFFFF);
            break;
        }
        KeLowerIrql(irql);
        break;
    }

    if (loaded) {
        KeRaiseIrql(DISPATCH_LEVEL, &irql);
        UINT64 current = 0;
        __vmx_vmptrst(&current);
        if (current != vcpu->VmcsPhys) {
            __vmx_vmptrld(&vcpu->VmcsPhys);
        }
        if (launched) {
            ReadGuest(vcpu, &run->Regs);
        }
        /* Written back to memory and inactive, so the next RUN can be on any processor. */
        __vmx_vmclear(&vcpu->VmcsPhys);
        KeLowerIrql(irql);
    }
    InterlockedExchange(&vcpu->Cpu, -1);
    KeRestoreExtendedProcessorState(&save);
    KeRevertToUserGroupAffinityThread(&previous);
    return STATUS_SUCCESS;
}
