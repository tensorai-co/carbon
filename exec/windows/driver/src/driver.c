/*
 * winkvm.sys: the device, its calls, and the table of VMs.
 *
 * \\.\WinKvm is open to SYSTEM and Administrators only. A VM belongs to the handle that made it: no other handle can
 * touch it, and closing that handle destroys it, so a process that dies takes its VMs with it.
 */
#include "winkvm_internal.h"

DRIVER_INITIALIZE DriverEntry;
static DRIVER_UNLOAD WinKvmUnload;
_Dispatch_type_(IRP_MJ_CREATE) _Dispatch_type_(IRP_MJ_CLOSE) static DRIVER_DISPATCH WinKvmCreateClose;
_Dispatch_type_(IRP_MJ_CLEANUP) static DRIVER_DISPATCH WinKvmCleanup;
_Dispatch_type_(IRP_MJ_DEVICE_CONTROL) static DRIVER_DISPATCH WinKvmDeviceControl;

WINKVM_GLOBALS g;

static UNICODE_STRING DeviceName = RTL_CONSTANT_STRING(L"\\Device\\WinKvm");
static UNICODE_STRING LinkName = RTL_CONSTANT_STRING(L"\\DosDevices\\WinKvm");
/* SYSTEM and Administrators, and nobody else. */
static UNICODE_STRING DeviceSddl = RTL_CONSTANT_STRING(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");
/* {6f5a3c1e-2d8b-4b0e-9a57-0c3e5f7b9d21} */
static const GUID DeviceClass = { 0x6f5a3c1e, 0x2d8b, 0x4b0e, { 0x9a, 0x57, 0x0c, 0x3e, 0x5f, 0x7b, 0x9d, 0x21 } };

/* ---- VMs ---- */

/* A VM this handle owns, held so it can't go while it's used. NULL if there's none. */
static WK_VM* AcquireVm(PFILE_OBJECT file, UINT32 id)
{
    if (id == 0 || id > WINKVM_MAX_VMS) {
        return NULL;
    }
    ExAcquireFastMutex(&g.Lock);
    WK_VM* vm = g.Vms[id - 1];
    if (vm && (vm->Owner != file || vm->Dying)) {
        vm = NULL;
    }
    if (vm) {
        InterlockedIncrement(&vm->Refs);
    }
    ExReleaseFastMutex(&g.Lock);
    return vm;
}

static VOID ReleaseVm(WK_VM* vm)
{
    InterlockedDecrement(&vm->Refs);
}

static WK_VCPU* VcpuOf(WK_VM* vm, UINT32 id)
{
    if (id >= WINKVM_MAX_VCPUS || !vm->Vcpus[id].Used) {
        return NULL;
    }
    return &vm->Vcpus[id];
}

static NTSTATUS CreateVm(PFILE_OBJECT file, UINT32* id)
{
    WK_VM* vm = ExAllocatePool2(POOL_FLAG_NON_PAGED, sizeof(WK_VM), WINKVM_TAG);
    if (!vm) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    vm->Owner = file;
    ExInitializeFastMutex(&vm->VcpuLock);
    NTSTATUS status = STATUS_TOO_MANY_SESSIONS;
    ExAcquireFastMutex(&g.Lock);
    for (UINT32 i = 0; i < WINKVM_MAX_VMS; i++) {
        if (!g.Vms[i]) {
            vm->Id = i + 1;
            g.Vms[i] = vm;
            *id = vm->Id;
            status = STATUS_SUCCESS;
            break;
        }
    }
    ExReleaseFastMutex(&g.Lock);
    if (!NT_SUCCESS(status)) {
        ExFreePoolWithTag(vm, WINKVM_TAG);
    }
    return status;
}

/* Destroys a VM already taken out of the table: stops its vCPUs, waits for every call using it, frees it. */
static VOID DestroyVm(WK_VM* vm)
{
    LARGE_INTEGER pause;
    pause.QuadPart = -10 * 1000; /* 1 ms */
    for (;;) {
        for (ULONG i = 0; i < WINKVM_MAX_VCPUS; i++) {
            if (vm->Vcpus[i].Used) {
                InterlockedExchange(&vm->Vcpus[i].Cancel, 1);
                VcpuPoke(&vm->Vcpus[i]);
            }
        }
        if (vm->Refs == 0) {
            break;
        }
        KeDelayExecutionThread(KernelMode, FALSE, &pause);
    }
    for (ULONG i = 0; i < WINKVM_MAX_VCPUS; i++) {
        if (vm->Vcpus[i].Used) {
            VcpuFree(&vm->Vcpus[i]);
        }
    }
    EptFree(vm);
    ExFreePoolWithTag(vm, WINKVM_TAG);
}

/* Takes out of the table every VM `file` owns (every VM, if `file` is NULL), and destroys them. */
static VOID DestroyVms(PFILE_OBJECT file, UINT32 only)
{
    WK_VM* doomed[WINKVM_MAX_VMS];
    ULONG count = 0;
    ExAcquireFastMutex(&g.Lock);
    for (UINT32 i = 0; i < WINKVM_MAX_VMS; i++) {
        WK_VM* vm = g.Vms[i];
        if (vm && (!file || vm->Owner == file) && (!only || vm->Id == only)) {
            InterlockedExchange(&vm->Dying, 1);
            g.Vms[i] = NULL;
            doomed[count++] = vm;
        }
    }
    ExReleaseFastMutex(&g.Lock);
    for (ULONG i = 0; i < count; i++) {
        DestroyVm(doomed[i]);
    }
}

static NTSTATUS CreateVcpu(WK_VM* vm, UINT32* id)
{
    NTSTATUS status = STATUS_TOO_MANY_THREADS;
    ExAcquireFastMutex(&vm->VcpuLock);
    if (vm->VcpuCount < WINKVM_MAX_VCPUS) {
        WK_VCPU* vcpu = &vm->Vcpus[vm->VcpuCount];
        RtlZeroMemory(vcpu, sizeof(*vcpu));
        vcpu->Vm = vm;
        vcpu->Id = vm->VcpuCount;
        status = VcpuInitialize(vcpu);
        if (NT_SUCCESS(status)) {
            *id = vcpu->Id;
            vm->VcpuCount++;
            InterlockedExchange(&vcpu->Used, 1);
        }
    }
    ExReleaseFastMutex(&vm->VcpuLock);
    return status;
}

/* ---- Calls ---- */

/* The input, if there's enough of it. */
#define INPUT(type) (inLength >= sizeof(type) ? (type*)buffer : NULL)
#define OUTPUT_FITS(type) (outLength >= sizeof(type))

static NTSTATUS WinKvmDeviceControl(PDEVICE_OBJECT device, PIRP irp)
{
    UNREFERENCED_PARAMETER(device);
    PIO_STACK_LOCATION stack = IoGetCurrentIrpStackLocation(irp);
    ULONG code = stack->Parameters.DeviceIoControl.IoControlCode;
    ULONG inLength = stack->Parameters.DeviceIoControl.InputBufferLength;
    ULONG outLength = stack->Parameters.DeviceIoControl.OutputBufferLength;
    PVOID buffer = irp->AssociatedIrp.SystemBuffer;
    PFILE_OBJECT file = stack->FileObject;
    NTSTATUS status = STATUS_INVALID_PARAMETER;
    ULONG_PTR information = 0;
    WK_VM* vm = NULL;
    WK_VCPU* vcpu = NULL;

    switch (code) {
    case IOCTL_WINKVM_CAPS:
        if (OUTPUT_FITS(WINKVM_CAPS)) {
            WINKVM_CAPS* caps = buffer;
            caps->Version = WINKVM_API_VERSION;
            caps->Vmx = g.VmxReady;
            caps->Ept = g.VmxReady;
            caps->MaxVcpus = WINKVM_MAX_VCPUS;
            information = sizeof(WINKVM_CAPS);
            status = STATUS_SUCCESS;
        }
        break;

    case IOCTL_WINKVM_CREATE_VM:
        if (!g.VmxReady) {
            status = STATUS_NOT_SUPPORTED;
        } else if (OUTPUT_FITS(WINKVM_VCPU)) {
            UINT32 id = 0;
            status = CreateVm(file, &id);
            if (NT_SUCCESS(status)) {
                WINKVM_VCPU* out = buffer;
                out->Vm = id;
                out->Vcpu = 0;
                information = sizeof(WINKVM_VCPU);
            }
        }
        break;

    case IOCTL_WINKVM_DESTROY_VM: {
        WINKVM_VCPU* in = INPUT(WINKVM_VCPU);
        if (in && (vm = AcquireVm(file, in->Vm))) {
            ReleaseVm(vm);
            vm = NULL;
            DestroyVms(file, in->Vm);
            status = STATUS_SUCCESS;
        }
        break;
    }

    case IOCTL_WINKVM_SET_MEMORY: {
        WINKVM_MEMORY* in = INPUT(WINKVM_MEMORY);
        if (in && (vm = AcquireVm(file, in->Vm))) {
            status = EptSetMemory(vm, in);
        }
        break;
    }

    case IOCTL_WINKVM_CREATE_VCPU: {
        WINKVM_VCPU* in = INPUT(WINKVM_VCPU);
        if (in && OUTPUT_FITS(WINKVM_VCPU) && (vm = AcquireVm(file, in->Vm))) {
            UINT32 vmId = in->Vm, id = 0;
            status = CreateVcpu(vm, &id);
            if (NT_SUCCESS(status)) {
                WINKVM_VCPU* out = buffer;
                out->Vm = vmId;
                out->Vcpu = id;
                information = sizeof(WINKVM_VCPU);
            }
        }
        break;
    }

    case IOCTL_WINKVM_SET_SYSTEM: {
        WINKVM_SYSTEM* in = INPUT(WINKVM_SYSTEM);
        if (in && (vm = AcquireVm(file, in->Vm)) && (vcpu = VcpuOf(vm, in->Vcpu))) {
            status = vcpu->Running ? STATUS_DEVICE_BUSY : VcpuSetSystem(vcpu, in);
        }
        break;
    }

    case IOCTL_WINKVM_GET_FP: {
        WINKVM_VCPU* in = INPUT(WINKVM_VCPU);
        if (in && OUTPUT_FITS(WINKVM_FP) && (vm = AcquireVm(file, in->Vm)) && (vcpu = VcpuOf(vm, in->Vcpu))) {
            if (vcpu->Running) {
                status = STATUS_DEVICE_BUSY;
            } else {
                WINKVM_VCPU id = *in;
                WINKVM_FP* out = buffer;
                VcpuGetFp(vcpu, out);
                out->Vm = id.Vm;
                out->Vcpu = id.Vcpu;
                information = sizeof(WINKVM_FP);
                status = STATUS_SUCCESS;
            }
        }
        break;
    }

    case IOCTL_WINKVM_SET_FP: {
        WINKVM_FP* in = INPUT(WINKVM_FP);
        if (in && (vm = AcquireVm(file, in->Vm)) && (vcpu = VcpuOf(vm, in->Vcpu))) {
            if (vcpu->Running) {
                status = STATUS_DEVICE_BUSY;
            } else {
                VcpuSetFp(vcpu, in);
                status = STATUS_SUCCESS;
            }
        }
        break;
    }

    case IOCTL_WINKVM_RUN: {
        WINKVM_RUN* run = INPUT(WINKVM_RUN);
        if (run && OUTPUT_FITS(WINKVM_RUN) && (vm = AcquireVm(file, run->Vm)) && (vcpu = VcpuOf(vm, run->Vcpu))) {
            if (InterlockedCompareExchange(&vcpu->Running, 1, 0) != 0) {
                status = STATUS_DEVICE_BUSY;
            } else {
                status = VcpuRun(vcpu, run);
                InterlockedExchange(&vcpu->Running, 0);
                if (NT_SUCCESS(status)) {
                    information = sizeof(WINKVM_RUN);
                }
            }
        }
        break;
    }

    case IOCTL_WINKVM_CANCEL: {
        WINKVM_VCPU* in = INPUT(WINKVM_VCPU);
        if (in && (vm = AcquireVm(file, in->Vm)) && (vcpu = VcpuOf(vm, in->Vcpu))) {
            InterlockedExchange(&vcpu->Cancel, 1);
            VcpuPoke(vcpu);
            status = STATUS_SUCCESS;
        }
        break;
    }

    default:
        status = STATUS_INVALID_DEVICE_REQUEST;
        break;
    }

    if (vm) {
        ReleaseVm(vm);
    }
    irp->IoStatus.Status = status;
    irp->IoStatus.Information = information;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return status;
}

/* ---- The device ---- */

static NTSTATUS WinKvmCreateClose(PDEVICE_OBJECT device, PIRP irp)
{
    UNREFERENCED_PARAMETER(device);
    irp->IoStatus.Status = STATUS_SUCCESS;
    irp->IoStatus.Information = 0;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return STATUS_SUCCESS;
}

/* The handle's last close: its VMs go with it. */
static NTSTATUS WinKvmCleanup(PDEVICE_OBJECT device, PIRP irp)
{
    UNREFERENCED_PARAMETER(device);
    DestroyVms(IoGetCurrentIrpStackLocation(irp)->FileObject, 0);
    irp->IoStatus.Status = STATUS_SUCCESS;
    irp->IoStatus.Information = 0;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return STATUS_SUCCESS;
}

static VOID WinKvmUnload(PDRIVER_OBJECT driver)
{
    UNREFERENCED_PARAMETER(driver);
    DestroyVms(NULL, 0);
    IoDeleteSymbolicLink(&LinkName);
    if (g.Device) {
        IoDeleteDevice(g.Device);
    }
    VmxShutdown();
}

NTSTATUS DriverEntry(PDRIVER_OBJECT driver, PUNICODE_STRING registryPath)
{
    UNREFERENCED_PARAMETER(registryPath);
    ExInitializeFastMutex(&g.Lock);

    /* Without VMX the device still loads, so CAPS can say there's none. */
    NTSTATUS status = VmxInitialize();
    if (!NT_SUCCESS(status)) {
        DbgPrintEx(DPFLTR_IHVDRIVER_ID, DPFLTR_WARNING_LEVEL, "winkvm: no VMX here (%08x)\n", status);
    }

    status = IoCreateDeviceSecure(driver, 0, &DeviceName, WINKVM_DEVICE_TYPE, FILE_DEVICE_SECURE_OPEN, FALSE, &DeviceSddl, &DeviceClass, &g.Device);
    if (!NT_SUCCESS(status)) {
        VmxShutdown();
        return status;
    }
    status = IoCreateSymbolicLink(&LinkName, &DeviceName);
    if (!NT_SUCCESS(status)) {
        IoDeleteDevice(g.Device);
        VmxShutdown();
        return status;
    }

    driver->MajorFunction[IRP_MJ_CREATE] = WinKvmCreateClose;
    driver->MajorFunction[IRP_MJ_CLOSE] = WinKvmCreateClose;
    driver->MajorFunction[IRP_MJ_CLEANUP] = WinKvmCleanup;
    driver->MajorFunction[IRP_MJ_DEVICE_CONTROL] = WinKvmDeviceControl;
    driver->DriverUnload = WinKvmUnload;
    g.Device->Flags &= ~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}
