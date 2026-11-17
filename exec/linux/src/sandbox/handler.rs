//! The handler that runs inside every stub, and the memory it shares with Carbon.
//!
//! A program's system calls and faults arrive in its own stub as signals: `SIGSYS` (from Syscall User Dispatch, or
//! from seccomp's `SECCOMP_RET_TRAP`), `SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`, and `SIGURG` for a preempt.
//! Each runs this handler, on an alternate signal stack in memory Carbon also maps, so the host kernel writes the
//! program's registers, FP state and `siginfo` where Carbon reads and changes them directly. The handler copies
//! nothing: it tells Carbon an event is there, waits, runs the system calls Carbon queues for the stub's own address
//! space (mapping guest memory, setting `fs` and `gs`), and returns, which `rt_sigreturn` turns into the program
//! carrying on with whatever registers Carbon left in the frame.
//!
//! The handoff between them is the mailbox's `STATE` word, as a futex, with each side spinning a little before it
//! sleeps (only while the other is on another CPU, and as long as Carbon has lately found it pays: `mailbox.rs`), and
//! waking the other only if it's asleep:
//!
//! ```text
//! stub                                          Carbon
//! RUNNING (the program runs)
//! signal → EVENT, wake Carbon        ─────►     reads the frame, answers
//!                                    ◄─────     OPS: run these calls       (mapping changes, as often as needed)
//! runs them → DONE, wake Carbon      ─────►     reads their results
//!                                    ◄─────     RESUME (and calls to run first: fs, gs)
//! runs them → RUNNING, rt_sigreturn
//! ```
//!
//! The same page holds where the stub starts: a routine that maps the shared memory and makes the calls Carbon queued
//! there to set the stub up (`process.rs`), then faults into the handler. So Carbon needs no `ptrace` to start a stub.
//!
//! The handler is written here as assembly, then copied into each stub's first page, which Syscall User Dispatch
//! exempts and seccomp allows a few calls from. It refers to the mailbox only relative to itself, so it runs wherever
//! the stub goes. The program can jump into it, and can write the mailbox; what that gives it is what seccomp's
//! filter allows (`seccomp.rs`), and Carbon treats everything in the mailbox as untrusted.

use std::arch::global_asm;

/// The stub's layout, from its base. The first page holds the ELF's headers and the handler, and is the one page
/// exempt from dispatch; the trampoline Carbon makes its own calls through is on the next, which isn't.
pub const HANDLER: u64 = 0x100;
pub const TRAMPOLINE_PAGE: u64 = 0x1000;
pub const DATA_PAGE: u64 = 0x2000;
/// The shared memory: the mailbox's page, then the signal stack.
pub const SHARED: u64 = 0x3000;
/// The gate's page (phase 2: `GATE`): the program's registers as it called the gate, laid out as a signal frame's
/// `ucontext`, its FP state as XSAVE wrote it, and a small stack the gate runs on.
pub const GATE_PAGE: u64 = SHARED + 0x1000;
pub const GATE_UC: u64 = GATE_PAGE;
pub const GATE_XSAVE: u64 = GATE_PAGE + 0x100;
pub const GATE_STACK: u64 = GATE_PAGE + 0x1000;
pub const SIGNAL_STACK: u64 = SHARED + 0x2000;
pub const SIGNAL_STACK_SIZE: u64 = 0x1_0000;
pub const SHARED_SIZE: u64 = 0x2000 + SIGNAL_STACK_SIZE;

/// The mailbox's fields, by offset.
pub const STATE: u64 = 0x00;
pub const STUB_SLEEPING: u64 = 0x04;
pub const CARBON_SLEEPING: u64 = 0x08;
pub const SIGNO: u64 = 0x10;
pub const INFO: u64 = 0x18;
pub const UC: u64 = 0x20;
/// For a general-protection fault, the two bytes at `rip` (atlas's `int 0x2e` and `int 0x29`).
pub const CODE: u64 = 0x28;
pub const FS_BASE: u64 = 0x30;
pub const GS_BASE: u64 = 0x38;
pub const NOPS: u64 = 0x40;
/// How many times the handler looks before it sleeps.
pub const SPIN: u64 = 0x48;
/// `FLAG_FSGSBASE`: the handler reads and writes `fs` and `gs` with `rdfsbase` and the rest (the kernel allows them:
/// Linux 5.9 on a CPU that has them). Without it, a program can't change them but through Carbon (`arch_prctl`, which
/// it answers), so Carbon knows them, and queues calls to set them.
pub const FLAGS: u64 = 0x50;
pub const FLAG_FSGSBASE: u64 = 1;
/// The host CPU each side was last on, as Linux's per-CPU segment says (`lsl`, as the vDSO's `getcpu` does), or
/// `u32::MAX` if it can't tell: neither spins while the other's on its own CPU, where it would only keep it from running.
pub const CARBON_CPU: u64 = 0x58;
pub const STUB_CPU: u64 = 0x5c;
/// Linux's per-CPU segment selector (`__CPUNODE_SEG`), whose limit is the CPU's number.
pub const CPU_SEGMENT: u32 = 0x7b;
/// Nonzero while the event Carbon has came by the gate, not a signal: the handler returns through the gate's way.
pub const VIA_GATE: u64 = 0x60;
/// Nonzero from the gate's first instruction until it starts giving the program its registers back: a preempt that
/// comes meanwhile is dropped, as an event is on its way to Carbon anyway.
pub const IN_GATE: u64 = 0x64;
/// `SIGNO` for an event that came by the gate.
pub const GATE_SIGNO: u64 = 0;
/// The calls to run: each is the number, six arguments, and the result, eight words.
pub const OPS: u64 = 0x100;
pub const OP_SIZE: u64 = 64;
pub const MAX_OPS: u64 = (0x1000 - OPS) / OP_SIZE;

/// `STATE`'s values.
pub const RUNNING: u32 = 0;
pub const EVENT: u32 = 1;
pub const DO_OPS: u32 = 2;
pub const DONE: u32 = 3;
pub const RESUME: u32 = 4;

/// In a signal frame's `ucontext`: the registers (`struct sigcontext`) start at 40, in the kernel's order.
pub mod uc {
    pub const R8: usize = 40;
    pub const R9: usize = 48;
    pub const R10: usize = 56;
    pub const R11: usize = 64;
    pub const R12: usize = 72;
    pub const R13: usize = 80;
    pub const R14: usize = 88;
    pub const R15: usize = 96;
    pub const RDI: usize = 104;
    pub const RSI: usize = 112;
    pub const RBP: usize = 120;
    pub const RBX: usize = 128;
    pub const RDX: usize = 136;
    pub const RAX: usize = 144;
    pub const RCX: usize = 152;
    pub const RSP: usize = 160;
    pub const RIP: usize = 168;
    pub const EFLAGS: usize = 176;
    /// Where the FP state (FXSAVE's layout, and XSAVE's past it) is.
    pub const FPSTATE: usize = 224;
    /// The whole `ucontext`, as far as Carbon reads it.
    pub const SIZE: usize = 232;
}

global_asm!(
    ".pushsection .text.carbon_stub, \"ax\", @progbits",
    ".p2align 4",
    ".globl carbon_stub_start",
    ".hidden carbon_stub_start",
    "carbon_stub_start:",
    // A signal: rdi is its number, rsi its siginfo, rdx the ucontext, all on the shared signal stack. Every register
    // is free: rt_sigreturn puts the program's back from the frame.
    "lea rbx, [rip + carbon_stub_start + {mailbox}]",
    // In the gate's own code, where signals aren't blocked (`GATE`): before it starts giving the program its
    // registers back, a preempt is dropped (the gate's event is on its way to Carbon, which sees the preempt before it
    // lets the program run), and anything else, a fault in the gate, ends the stub; after, it's the program's, as the
    // gate's frame has it.
    "mov rax, [rdx + {uc_rip}]",
    "lea rcx, [rip + .Lcarbon_gate_window]",
    "cmp rax, rcx",
    "jb .Lcarbon_gate_early",
    "lea rcx, [rip + .Lcarbon_gate_end]",
    "cmp rax, rcx",
    "jb .Lcarbon_gate_late",
    ".Lcarbon_gate_early:",
    "cmp dword ptr [rbx + {in_gate}], 0",
    "je .Lcarbon_signal",
    "lea rcx, [rip + carbon_stub_start]",
    "and rcx, -4096",
    "xor rcx, rax",
    "cmp rcx, 4096",
    "jae .Lcarbon_signal",
    "cmp edi, {preempt}",
    "jne .Lcarbon_gate_broken",
    "ret",
    ".Lcarbon_gate_broken:",
    "ud2",
    ".Lcarbon_gate_late:",
    "mov r8, rdi",
    "mov r9, rsi",
    "lea rsi, [rip + carbon_stub_start + {gate_frame}]",
    "lea rdi, [rdx + {uc_r8}]",
    "mov ecx, {frame_words}",
    "cld",
    "rep movsq",
    "mov rdi, r8",
    "mov rsi, r9",
    ".Lcarbon_signal:",
    "mov dword ptr [rbx + {via_gate}], 0",
    "mov [rbx + {signo}], rdi",
    "mov [rbx + {info}], rsi",
    "mov [rbx + {uc}], rdx",
    "mov qword ptr [rbx + {code}], 0",
    // A general-protection fault (SIGSEGV, SI_KERNEL): the instruction's first two bytes, unless they'd run onto the
    // next page, which may not be there.
    "cmp edi, 11",
    "jne .Lcarbon_bases",
    "cmp dword ptr [rsi + 8], 0x80",
    "jne .Lcarbon_bases",
    "mov rax, [rdx + {uc_rip}]",
    "mov ecx, eax",
    "and ecx, 0xfff",
    "cmp ecx, 0xfff",
    "je .Lcarbon_bases",
    "movzx ecx, word ptr [rax]",
    "mov [rbx + {code}], rcx",
    // fs and gs, which a signal frame doesn't carry.
    ".Lcarbon_bases:",
    "test byte ptr [rbx + {flags}], {fsgsbase}",
    "jz .Lcarbon_bases_read",
    "rdfsbase rax",
    "mov [rbx + {fs}], rax",
    "rdgsbase rax",
    "mov [rbx + {gs}], rax",
    ".Lcarbon_bases_read:",
    "mov dword ptr [rbx + {state}], {event}",
    "mfence",
    "call .Lcarbon_wake",
    // Wait for Carbon: look `SPIN` times, then sleep until it changes the state.
    ".Lcarbon_wait:",
    "mov rcx, [rbx + {spin}]",
    "mov eax, {cpu_segment}",
    "lsl eax, eax",
    "jnz .Lcarbon_look",
    "and eax, 0xfff",
    "mov dword ptr [rbx + {stub_cpu}], eax",
    "cmp eax, dword ptr [rbx + {carbon_cpu}]",
    "jne .Lcarbon_look",
    "xor ecx, ecx",
    ".Lcarbon_look:",
    "mov eax, dword ptr [rbx + {state}]",
    "cmp eax, {do_ops}",
    "je .Lcarbon_ops",
    "cmp eax, {resume}",
    "je .Lcarbon_resume",
    "test rcx, rcx",
    "jz .Lcarbon_sleep",
    "dec rcx",
    "pause",
    "jmp .Lcarbon_look",
    ".Lcarbon_sleep:",
    "mov dword ptr [rbx + {stub_sleeping}], 1",
    "mfence",
    "mov edx, dword ptr [rbx + {state}]",
    "cmp edx, {do_ops}",
    "je .Lcarbon_awake",
    "cmp edx, {resume}",
    "je .Lcarbon_awake",
    "mov eax, 202",
    "lea rdi, [rbx + {state}]",
    "xor esi, esi",
    "xor r10d, r10d",
    "syscall",
    ".Lcarbon_awake:",
    "mov dword ptr [rbx + {stub_sleeping}], 0",
    "jmp .Lcarbon_wait",
    ".Lcarbon_ops:",
    "call .Lcarbon_run",
    "mov dword ptr [rbx + {state}], {done}",
    "mfence",
    "call .Lcarbon_wake",
    "jmp .Lcarbon_wait",
    ".Lcarbon_resume:",
    "call .Lcarbon_run",
    "test byte ptr [rbx + {flags}], {fsgsbase}",
    "jz .Lcarbon_bases_set",
    "mov rax, [rbx + {fs}]",
    "wrfsbase rax",
    "mov rax, [rbx + {gs}]",
    "wrgsbase rax",
    ".Lcarbon_bases_set:",
    "cmp dword ptr [rbx + {via_gate}], 0",
    "jne .Lcarbon_gate_return",
    "mov dword ptr [rbx + {state}], {running}",
    // To the restorer the kernel left on the stack, which makes the rt_sigreturn.
    "ret",
    // The queued calls, at most MAX_OPS of them, each result written after its call.
    ".Lcarbon_run:",
    "mov r12, [rbx + {nops}]",
    "cmp r12, {max_ops}",
    "jbe .Lcarbon_run_some",
    "mov r12, {max_ops}",
    ".Lcarbon_run_some:",
    "lea r13, [rbx + {ops}]",
    ".Lcarbon_run_next:",
    "test r12, r12",
    "jz .Lcarbon_run_done",
    "mov rax, [r13]",
    "mov rdi, [r13 + 8]",
    "mov rsi, [r13 + 16]",
    "mov rdx, [r13 + 24]",
    "mov r10, [r13 + 32]",
    "mov r8, [r13 + 40]",
    "mov r9, [r13 + 48]",
    "syscall",
    "mov [r13 + 56], rax",
    "add r13, {op_size}",
    "dec r12",
    "jmp .Lcarbon_run_next",
    ".Lcarbon_run_done:",
    "mov qword ptr [rbx + {nops}], 0",
    "ret",
    // Wakes Carbon, if it's asleep.
    ".Lcarbon_wake:",
    "cmp dword ptr [rbx + {carbon_sleeping}], 0",
    "je .Lcarbon_wake_done",
    "mov eax, 202",
    "lea rdi, [rbx + {state}]",
    "mov esi, 1",
    "mov edx, 1",
    "syscall",
    ".Lcarbon_wake_done:",
    "ret",
    // The gate (phase 2): where atlas's ntdll calls, through the atlas page, instead of making a `syscall`, which
    // saves the signal and `rt_sigreturn`. Every register but rsp and rip is the program's, and its return address is
    // on its stack. The registers go to the gate's frame, laid out as a signal frame's, its FP state by XSAVE (x87,
    // SSE and AVX, as the frame's software area says), and it's an event like any other, of signal GATE_SIGNO. It
    // runs on a stack of its own in the gate's page, never the program's, with signals unblocked: the handler sees to
    // one that comes meanwhile (above).
    ".globl carbon_stub_gate",
    ".hidden carbon_stub_gate",
    "carbon_stub_gate:",
    "mov dword ptr [rip + carbon_stub_start + {mailbox} + {in_gate}], 1",
    "mov [rip + carbon_stub_start + {g_rax}], rax",
    "mov [rip + carbon_stub_start + {g_rcx}], rcx",
    "mov [rip + carbon_stub_start + {g_rdx}], rdx",
    "mov [rip + carbon_stub_start + {g_rbx}], rbx",
    "mov [rip + carbon_stub_start + {g_rbp}], rbp",
    "mov [rip + carbon_stub_start + {g_rsi}], rsi",
    "mov [rip + carbon_stub_start + {g_rdi}], rdi",
    "mov [rip + carbon_stub_start + {g_r8}], r8",
    "mov [rip + carbon_stub_start + {g_r9}], r9",
    "mov [rip + carbon_stub_start + {g_r10}], r10",
    "mov [rip + carbon_stub_start + {g_r11}], r11",
    "mov [rip + carbon_stub_start + {g_r12}], r12",
    "mov [rip + carbon_stub_start + {g_r13}], r13",
    "mov [rip + carbon_stub_start + {g_r14}], r14",
    "mov [rip + carbon_stub_start + {g_r15}], r15",
    "pop qword ptr [rip + carbon_stub_start + {g_rip}]",
    "mov [rip + carbon_stub_start + {g_rsp}], rsp",
    "lea rsp, [rip + carbon_stub_start + {gate_stack}]",
    "pushfq",
    "pop qword ptr [rip + carbon_stub_start + {g_eflags}]",
    "xor ecx, ecx",
    "xgetbv",
    "and eax, 7",
    "xor edx, edx",
    "mov [rip + carbon_stub_start + {gate_xsave} + 472], rax",
    "xsave64 [rip + carbon_stub_start + {gate_xsave}]",
    "mov dword ptr [rip + carbon_stub_start + {gate_xsave} + 464], {xstate_magic}",
    "mov ecx, 576",
    "mov edx, 832",
    "test eax, 4",
    "cmovnz ecx, edx",
    "mov dword ptr [rip + carbon_stub_start + {gate_xsave} + 468], ecx",
    "lea rax, [rip + carbon_stub_start + {gate_xsave}]",
    "mov [rip + carbon_stub_start + {g_fpstate}], rax",
    "lea rbx, [rip + carbon_stub_start + {mailbox}]",
    "mov dword ptr [rbx + {via_gate}], 1",
    "mov qword ptr [rbx + {signo}], {gate_signo}",
    "mov qword ptr [rbx + {info}], 0",
    "lea rax, [rip + carbon_stub_start + {gate_uc}]",
    "mov [rbx + {uc}], rax",
    "mov qword ptr [rbx + {code}], 0",
    "jmp .Lcarbon_bases",
    // Back to the program from the gate's frame, as Carbon left it: the FP state, the flags, then (from the window
    // on, where a signal takes the frame as the program's) every register, rsp, and a jump to rip.
    ".Lcarbon_gate_return:",
    "mov eax, [rip + carbon_stub_start + {gate_xsave} + 472]",
    "xor edx, edx",
    "xrstor64 [rip + carbon_stub_start + {gate_xsave}]",
    "push qword ptr [rip + carbon_stub_start + {g_eflags}]",
    "popfq",
    "mov dword ptr [rbx + {state}], {running}",
    "mov dword ptr [rbx + {in_gate}], 0",
    ".Lcarbon_gate_window:",
    "mov rax, [rip + carbon_stub_start + {g_rax}]",
    "mov rcx, [rip + carbon_stub_start + {g_rcx}]",
    "mov rdx, [rip + carbon_stub_start + {g_rdx}]",
    "mov rbx, [rip + carbon_stub_start + {g_rbx}]",
    "mov rbp, [rip + carbon_stub_start + {g_rbp}]",
    "mov rsi, [rip + carbon_stub_start + {g_rsi}]",
    "mov rdi, [rip + carbon_stub_start + {g_rdi}]",
    "mov r8, [rip + carbon_stub_start + {g_r8}]",
    "mov r9, [rip + carbon_stub_start + {g_r9}]",
    "mov r10, [rip + carbon_stub_start + {g_r10}]",
    "mov r11, [rip + carbon_stub_start + {g_r11}]",
    "mov r12, [rip + carbon_stub_start + {g_r12}]",
    "mov r13, [rip + carbon_stub_start + {g_r13}]",
    "mov r14, [rip + carbon_stub_start + {g_r14}]",
    "mov r15, [rip + carbon_stub_start + {g_r15}]",
    "mov rsp, [rip + carbon_stub_start + {g_rsp}]",
    "jmp qword ptr [rip + carbon_stub_start + {g_rip}]",
    ".Lcarbon_gate_end:",
    // The restorer the kernel returns to: rt_sigreturn.
    ".globl carbon_stub_restorer",
    ".hidden carbon_stub_restorer",
    "carbon_stub_restorer:",
    "xor eax, eax",
    "mov al, 15",
    "syscall",
    // Where the stub starts: it maps the shared memory from MAILBOX_FD, then makes the calls Carbon queued there, in
    // order, each result written after its call, and ends with the failed one's number (from 1) as its exit code if one
    // fails, or 255 if the shared memory can't be mapped. It uses no stack, as one of the calls unmaps it.
    ".globl carbon_stub_setup",
    ".hidden carbon_stub_setup",
    "carbon_stub_setup:",
    // arch_prctl(ARCH_SET_GS, 0): nothing on Linux, but gVisor stops rewriting this process's system calls for good
    // when its gs is set, which it would otherwise do to any `mov $nr, %eax; syscall` it runs, the program's included.
    "mov eax, 158",
    "mov edi, 0x1001",
    "xor esi, esi",
    "syscall",
    "lea rdi, [rip + carbon_stub_start + {mailbox}]",
    "mov eax, 9",
    "mov esi, {shared_size}",
    "mov edx, 3",
    "mov r10d, 0x11",
    "mov r8d, {mailbox_fd}",
    "xor r9d, r9d",
    "syscall",
    "cmp rax, rdi",
    "jne .Lcarbon_setup_unmapped",
    "mov rbx, rdi",
    "mov r12, [rbx + {nops}]",
    "cmp r12, {max_ops}",
    "jbe .Lcarbon_setup_some",
    "mov r12, {max_ops}",
    ".Lcarbon_setup_some:",
    "lea r13, [rbx + {ops}]",
    "xor r14d, r14d",
    ".Lcarbon_setup_next:",
    "test r12, r12",
    "jz .Lcarbon_setup_done",
    "mov rax, [r13]",
    "mov rdi, [r13 + 8]",
    "mov rsi, [r13 + 16]",
    "mov rdx, [r13 + 24]",
    "mov r10, [r13 + 32]",
    "mov r8, [r13 + 40]",
    "mov r9, [r13 + 48]",
    "syscall",
    "mov [r13 + 56], rax",
    "cmp rax, -4095",
    "jae .Lcarbon_setup_failed",
    "add r13, {op_size}",
    "inc r14",
    "dec r12",
    "jmp .Lcarbon_setup_next",
    ".Lcarbon_setup_failed:",
    "mov eax, 231",
    "lea edi, [r14 + 1]",
    "syscall",
    ".Lcarbon_setup_unmapped:",
    "mov eax, 231",
    "mov edi, 255",
    "syscall",
    ".Lcarbon_setup_done:",
    "mov qword ptr [rbx + {nops}], 0",
    // Then a fault, so it's in the handler, waiting, from the start.
    ".globl carbon_stub_kick",
    ".hidden carbon_stub_kick",
    "carbon_stub_kick:",
    "ud2",
    ".globl carbon_stub_end",
    ".hidden carbon_stub_end",
    "carbon_stub_end:",
    ".popsection",
    mailbox = const SHARED - HANDLER,
    g_rax = const GATE_UC - HANDLER + uc::RAX as u64,
    g_rcx = const GATE_UC - HANDLER + uc::RCX as u64,
    g_rdx = const GATE_UC - HANDLER + uc::RDX as u64,
    g_rbx = const GATE_UC - HANDLER + uc::RBX as u64,
    g_rbp = const GATE_UC - HANDLER + uc::RBP as u64,
    g_rsi = const GATE_UC - HANDLER + uc::RSI as u64,
    g_rdi = const GATE_UC - HANDLER + uc::RDI as u64,
    g_r8 = const GATE_UC - HANDLER + uc::R8 as u64,
    g_r9 = const GATE_UC - HANDLER + uc::R9 as u64,
    g_r10 = const GATE_UC - HANDLER + uc::R10 as u64,
    g_r11 = const GATE_UC - HANDLER + uc::R11 as u64,
    g_r12 = const GATE_UC - HANDLER + uc::R12 as u64,
    g_r13 = const GATE_UC - HANDLER + uc::R13 as u64,
    g_r14 = const GATE_UC - HANDLER + uc::R14 as u64,
    g_r15 = const GATE_UC - HANDLER + uc::R15 as u64,
    g_rip = const GATE_UC - HANDLER + uc::RIP as u64,
    g_rsp = const GATE_UC - HANDLER + uc::RSP as u64,
    g_eflags = const GATE_UC - HANDLER + uc::EFLAGS as u64,
    g_fpstate = const GATE_UC - HANDLER + uc::FPSTATE as u64,
    gate_frame = const GATE_UC - HANDLER + uc::R8 as u64,
    gate_uc = const GATE_UC - HANDLER,
    gate_xsave = const GATE_XSAVE - HANDLER,
    gate_stack = const GATE_STACK - HANDLER,
    uc_r8 = const uc::R8,
    frame_words = const (uc::EFLAGS - uc::R8) / 8 + 1,
    xstate_magic = const 0x4650_5853u32,
    gate_signo = const GATE_SIGNO,
    via_gate = const VIA_GATE,
    in_gate = const IN_GATE,
    preempt = const libc::SIGURG,
    shared_size = const SHARED_SIZE,
    mailbox_fd = const super::process::MAILBOX_FD,
    flags = const FLAGS,
    cpu_segment = const CPU_SEGMENT,
    stub_cpu = const STUB_CPU,
    carbon_cpu = const CARBON_CPU,
    fsgsbase = const FLAG_FSGSBASE,
    signo = const SIGNO,
    info = const INFO,
    uc = const UC,
    code = const CODE,
    uc_rip = const uc::RIP,
    fs = const FS_BASE,
    gs = const GS_BASE,
    state = const STATE,
    stub_sleeping = const STUB_SLEEPING,
    carbon_sleeping = const CARBON_SLEEPING,
    spin = const SPIN,
    nops = const NOPS,
    ops = const OPS,
    op_size = const OP_SIZE,
    max_ops = const MAX_OPS,
    event = const EVENT,
    do_ops = const DO_OPS,
    done = const DONE,
    resume = const RESUME,
    running = const RUNNING,
);

unsafe extern "C" {
    static carbon_stub_start: u8;
    static carbon_stub_restorer: u8;
    #[cfg(test)]
    static carbon_stub_kick: u8;
    static carbon_stub_gate: u8;
    static carbon_stub_setup: u8;
    static carbon_stub_end: u8;
}

fn address(symbol: &u8) -> usize {
    std::ptr::from_ref(symbol) as usize
}

/// The handler's machine code, to copy to `HANDLER` in a stub.
pub fn code() -> &'static [u8] {
    // SAFETY: the bytes between two labels of the same section, in this binary's text.
    unsafe {
        let start = address(&carbon_stub_start);
        std::slice::from_raw_parts(start as *const u8, address(&carbon_stub_end) - start)
    }
}

/// Where, from the stub's base, the handler, its restorer and the kick are.
pub fn handler_at() -> u64 {
    HANDLER
}

pub fn restorer_at() -> u64 {
    // SAFETY: addresses of labels, not reads.
    HANDLER + unsafe { (address(&carbon_stub_restorer) - address(&carbon_stub_start)) as u64 }
}

/// Where the gate is, from the stub's base.
pub fn gate_at() -> u64 {
    // SAFETY: addresses of labels, not reads.
    HANDLER + unsafe { (address(&carbon_stub_gate) - address(&carbon_stub_start)) as u64 }
}

/// Where the stub starts.
pub fn setup_at() -> u64 {
    // SAFETY: as above.
    HANDLER + unsafe { (address(&carbon_stub_setup) - address(&carbon_stub_start)) as u64 }
}

#[cfg(test)]
pub fn kick_at() -> u64 {
    // SAFETY: as above.
    HANDLER + unsafe { (address(&carbon_stub_kick) - address(&carbon_stub_start)) as u64 }
}

#[cfg(test)]
mod tests {
    /// gVisor's systrap rewrites a `syscall` whose seventh byte back is `mov $nr, %eax`'s opcode, into a jump to code
    /// of its own (before the stub turns that off): none of the stub's may look like that, or gVisor would write into
    /// the handler.
    #[test]
    fn gvisor_would_rewrite_none_of_the_handler_s_calls() {
        let code = super::code();
        for at in 5..code.len() - 1 {
            if code[at..at + 2] == [0x0f, 0x05] {
                assert_ne!(code[at - 5], 0xb8, "a syscall at {at:#x}");
            }
        }
    }

    #[test]
    fn the_handler_fits_its_page_and_ends_as_it_should() {
        let code = super::code();
        assert!(super::HANDLER as usize + code.len() <= 0x1000, "{} bytes", code.len());
        // The restorer is `xor eax, eax; mov al, 15; syscall`, the kick `ud2`.
        let restorer = (super::restorer_at() - super::HANDLER) as usize;
        assert_eq!(&code[restorer..restorer + 6], &[0x31, 0xc0, 0xb0, 15, 0x0f, 0x05]);
        let kick = (super::kick_at() - super::HANDLER) as usize;
        assert_eq!(&code[kick..kick + 2], &[0x0f, 0x0b]);
        // The first instruction is `lea rbx, [rip + disp32]`, whose displacement reaches the mailbox from its end.
        assert_eq!(&code[..3], &[0x48, 0x8d, 0x1d]);
        let disp = i32::from_le_bytes(code[3..7].try_into().unwrap());
        assert_eq!(7 + disp as i64, (super::SHARED - super::HANDLER) as i64);
        // So does the setup's, from where it is, after its first call.
        let setup = (super::setup_at() - super::HANDLER) as usize + 14;
        assert_eq!(&code[setup..setup + 3], &[0x48, 0x8d, 0x3d]);
        let disp = i32::from_le_bytes(code[setup + 3..setup + 7].try_into().unwrap());
        assert_eq!((setup + 7) as i64 + disp as i64, (super::SHARED - super::HANDLER) as i64);
    }
}
