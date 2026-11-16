//! What a C runtime gives every program and LLVM calls on Windows: `__chkstk`, which touches each page of a stack frame
//! bigger than one, in order, so the stack's guard page grows it; and on x86, MSVC's 64-bit division helpers.

// x86-64: the frame's size in rax; every register kept.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl __chkstk",
    "__chkstk:",
    "push rcx",
    "push rax",
    "lea rcx, [rsp + 24]",
    "2:",
    "cmp rax, 0x1000",
    "jb 3f",
    "sub rcx, 0x1000",
    "test [rcx], rcx",
    "sub rax, 0x1000",
    "jmp 2b",
    "3:",
    "sub rcx, rax",
    "test [rcx], rcx",
    "pop rax",
    "pop rcx",
    "ret",
);

// ARM64: the frame's size in 16-byte units in x15; x16 and x17 may be used.
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".globl __chkstk",
    "__chkstk:",
    "lsl x16, x15, #4",
    "mov x17, sp",
    "2:",
    "sub x17, x17, #4096",
    "subs x16, x16, #4096",
    "ldr xzr, [x17]",
    "b.gt 2b",
    "ret",
);

// x86: the frame's size in eax; it moves esp down by that much itself, as MSVC's does.
#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl __chkstk",
    "__chkstk:",
    "push ecx",
    "lea ecx, [esp + 8]",
    "2:",
    "cmp eax, 0x1000",
    "jb 3f",
    "sub ecx, 0x1000",
    "test [ecx], eax",
    "sub eax, 0x1000",
    "jmp 2b",
    "3:",
    "sub ecx, eax",
    "test [ecx], eax",
    "mov eax, esp",
    "mov esp, ecx",
    "mov ecx, [eax]",
    "mov eax, [eax + 4]",
    "jmp eax",
);

#[cfg(target_arch = "x86")]
macro_rules! helper {
    ($name:literal, $builtin:literal) => {
        concat!(
            ".globl ", $name, "\n",
            $name, ":\n",
            "push dword ptr [esp + 16]\n",
            "push dword ptr [esp + 16]\n",
            "push dword ptr [esp + 16]\n",
            "push dword ptr [esp + 16]\n",
            "call ", $builtin, "\n",
            "add esp, 16\n",
            "ret 16\n",
        )
    };
}

#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    helper!("__aulldiv", "___udivdi3"),
    helper!("__aullrem", "___umoddi3"),
    helper!("__alldiv", "___divdi3"),
    helper!("__allrem", "___moddi3"),
);
