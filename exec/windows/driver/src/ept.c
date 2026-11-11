/*
 * Guest RAM: one buffer from the caller's process, locked for as long as the VM lives, and an EPT that maps exactly
 * it at the guest physical address asked for, write-back, readable, writable and executable. Nothing else is in the
 * EPT, so the guest can reach no other host memory.
 */
#include "winkvm_internal.h"

static UINT64* EptNewTable(WK_VM* vm, UINT64* phys)
{
    if (vm->EptTableCount == vm->EptTableCapacity) {
        ULONG capacity = vm->EptTableCapacity ? vm->EptTableCapacity * 2 : 64;
        PVOID* bigger = ExAllocatePool2(POOL_FLAG_NON_PAGED, capacity * sizeof(PVOID), WINKVM_TAG);
        UINT64* biggerPhys = ExAllocatePool2(POOL_FLAG_NON_PAGED, capacity * sizeof(UINT64), WINKVM_TAG);
        if (!bigger || !biggerPhys) {
            if (bigger) {
                ExFreePoolWithTag(bigger, WINKVM_TAG);
            }
            if (biggerPhys) {
                ExFreePoolWithTag(biggerPhys, WINKVM_TAG);
            }
            return NULL;
        }
        if (vm->EptTables) {
            RtlCopyMemory(bigger, vm->EptTables, vm->EptTableCount * sizeof(PVOID));
            RtlCopyMemory(biggerPhys, vm->EptTablePhys, vm->EptTableCount * sizeof(UINT64));
            ExFreePoolWithTag(vm->EptTables, WINKVM_TAG);
            ExFreePoolWithTag(vm->EptTablePhys, WINKVM_TAG);
        }
        vm->EptTables = bigger;
        vm->EptTablePhys = biggerPhys;
        vm->EptTableCapacity = capacity;
    }
    /* A page-sized nonpaged allocation is one zeroed, page-aligned page. */
    UINT64* table = ExAllocatePool2(POOL_FLAG_NON_PAGED, PAGE_SIZE, WINKVM_TAG);
    if (!table) {
        return NULL;
    }
    *phys = (UINT64)MmGetPhysicalAddress(table).QuadPart;
    vm->EptTablePhys[vm->EptTableCount] = *phys;
    vm->EptTables[vm->EptTableCount++] = table;
    return table;
}

/* The table an entry points at, from the tables this VM made: MmGetVirtualForPhysical can't be relied on for pool
 * pages (it gave NULL for them on Windows Server 2022). Newest first, since consecutive pages share their tables. */
static UINT64* EptTableAt(WK_VM* vm, UINT64 entry)
{
    UINT64 phys = entry & EPT_ADDRESS;
    for (ULONG i = vm->EptTableCount; i > 0; i--) {
        if (vm->EptTablePhys[i - 1] == phys) {
            return vm->EptTables[i - 1];
        }
    }
    return NULL;
}

static NTSTATUS EptMapPage(WK_VM* vm, UINT64 gpa, UINT64 hpa)
{
    UINT64* table = vm->EptRoot;
    for (int level = 3; level > 0; level--) {
        ULONG index = (ULONG)((gpa >> (12 + 9 * level)) & 511);
        if (!(table[index] & EPT_READ_WRITE_EXECUTE)) {
            UINT64 phys;
            UINT64* next = EptNewTable(vm, &phys);
            if (!next) {
                return STATUS_INSUFFICIENT_RESOURCES;
            }
            table[index] = phys | EPT_READ_WRITE_EXECUTE;
            table = next;
        } else {
            table = EptTableAt(vm, table[index]);
            if (!table) {
                return STATUS_INTERNAL_ERROR;
            }
        }
    }
    table[(gpa >> 12) & 511] = (hpa & EPT_ADDRESS) | EPT_READ_WRITE_EXECUTE | EPT_MEMORY_TYPE_WB;
    return STATUS_SUCCESS;
}

NTSTATUS EptSetMemory(WK_VM* vm, const WINKVM_MEMORY* memory)
{
    if (InterlockedCompareExchange(&vm->MemoryState, WINKVM_MEMORY_SETTING, WINKVM_MEMORY_NONE) != WINKVM_MEMORY_NONE) {
        return STATUS_INVALID_DEVICE_STATE; /* one region per VM */
    }
    UINT64 limit = 1ULL << min(g.PhysBits, 48);
    UINT64 userEnd = memory->UserAddress + memory->Size;
    UINT64 guestEnd = memory->GuestPhys + memory->Size;
    if ((memory->GuestPhys | memory->Size | memory->UserAddress) & (PAGE_SIZE - 1) || memory->Size == 0 || memory->Size > WINKVM_MAX_MEMORY
        || guestEnd < memory->GuestPhys || guestEnd > limit || userEnd < memory->UserAddress
        || userEnd > (UINT64)(ULONG_PTR)MmUserProbeAddress) {
        InterlockedExchange(&vm->MemoryState, WINKVM_MEMORY_NONE);
        return STATUS_INVALID_PARAMETER;
    }

    NTSTATUS status = STATUS_SUCCESS;
    ULONG chunks = (ULONG)((memory->Size + WINKVM_MDL_CHUNK - 1) / WINKVM_MDL_CHUNK);
    vm->Mdls = ExAllocatePool2(POOL_FLAG_NON_PAGED, chunks * sizeof(PMDL), WINKVM_TAG);
    if (!vm->Mdls) {
        status = STATUS_INSUFFICIENT_RESOURCES;
        goto failed;
    }
    for (ULONG i = 0; i < chunks; i++) {
        UINT64 offset = (UINT64)i * WINKVM_MDL_CHUNK;
        ULONG length = (ULONG)min(WINKVM_MDL_CHUNK, memory->Size - offset);
        PMDL mdl = IoAllocateMdl((PVOID)(ULONG_PTR)(memory->UserAddress + offset), length, FALSE, FALSE, NULL);
        if (!mdl) {
            status = STATUS_INSUFFICIENT_RESOURCES;
            goto failed;
        }
        __try {
            MmProbeAndLockPages(mdl, UserMode, IoWriteAccess);
        } __except (EXCEPTION_EXECUTE_HANDLER) {
            IoFreeMdl(mdl);
            status = STATUS_ACCESS_VIOLATION;
            goto failed;
        }
        vm->Mdls[vm->MdlCount++] = mdl;
    }

    UINT64 rootPhys;
    vm->EptRoot = EptNewTable(vm, &rootPhys);
    if (!vm->EptRoot) {
        status = STATUS_INSUFFICIENT_RESOURCES;
        goto failed;
    }
    for (ULONG i = 0; i < vm->MdlCount; i++) {
        PPFN_NUMBER pfns = MmGetMdlPfnArray(vm->Mdls[i]);
        ULONG pages = (ULONG)(MmGetMdlByteCount(vm->Mdls[i]) / PAGE_SIZE);
        UINT64 gpa = memory->GuestPhys + (UINT64)i * WINKVM_MDL_CHUNK;
        for (ULONG page = 0; page < pages; page++) {
            status = EptMapPage(vm, gpa + (UINT64)page * PAGE_SIZE, (UINT64)pfns[page] << PAGE_SHIFT);
            if (!NT_SUCCESS(status)) {
                goto failed;
            }
        }
    }
    vm->Eptp = rootPhys | EPTP_WALK_LENGTH_4 | EPTP_MEMORY_TYPE_WB;
    vm->GuestPhys = memory->GuestPhys;
    vm->Size = memory->Size;
    InterlockedExchange(&vm->MemoryState, WINKVM_MEMORY_READY);
    return STATUS_SUCCESS;

failed:
    EptFree(vm);
    InterlockedExchange(&vm->MemoryState, WINKVM_MEMORY_NONE);
    return status;
}

VOID EptFree(WK_VM* vm)
{
    if (vm->Mdls) {
        for (ULONG i = 0; i < vm->MdlCount; i++) {
            MmUnlockPages(vm->Mdls[i]);
            IoFreeMdl(vm->Mdls[i]);
        }
        ExFreePoolWithTag(vm->Mdls, WINKVM_TAG);
    }
    if (vm->EptTables) {
        for (ULONG i = 0; i < vm->EptTableCount; i++) {
            ExFreePoolWithTag(vm->EptTables[i], WINKVM_TAG);
        }
        ExFreePoolWithTag(vm->EptTables, WINKVM_TAG);
        ExFreePoolWithTag(vm->EptTablePhys, WINKVM_TAG);
    }
    vm->Mdls = NULL;
    vm->MdlCount = 0;
    vm->EptTables = NULL;
    vm->EptTablePhys = NULL;
    vm->EptTableCount = 0;
    vm->EptTableCapacity = 0;
    vm->EptRoot = NULL;
    vm->Eptp = 0;
}
