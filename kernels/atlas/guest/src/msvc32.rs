//! What MSVC's x86 C runtime gives every program and LLVM calls for 64-bit division on x86: `_alldiv`, `_aulldiv`,
//! `_allrem` and `_aullrem`, each with two 64-bit arguments on the stack that it pops itself, and its result in
//! edx:eax. ntdll has no C runtime under it, so these wrap compiler_builtins' own (cdecl) routines.

macro_rules! helper {
    ($name:literal, $builtin:literal) => {
        concat!(
            ".globl ", $name, "\n",
            $name, ":\n",
            // b's high and low halves, then a's: each was 16 bytes up before the push before it.
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

core::arch::global_asm!(
    helper!("__aulldiv", "___udivdi3"),
    helper!("__aullrem", "___umoddi3"),
    helper!("__alldiv", "___divdi3"),
    helper!("__allrem", "___moddi3"),
);
