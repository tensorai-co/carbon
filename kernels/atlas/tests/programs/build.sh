#!/bin/sh
# Builds the Windows test programs for ARM64 and x86-64 into $1 (default: the current directory), with LLVM's clang,
# lld-link and llvm-dlltool (Homebrew's llvm and lld have them).
set -e
cd "$(dirname "$0")"
out="${1:-.}"
llvm="${LLVM:-/opt/homebrew/opt/llvm/bin}"
for arch in aarch64 x86_64; do
    machine=$([ $arch = aarch64 ] && echo arm64 || echo i386:x86-64)
    for dll in kernel32 user32 ntdll advapi32 gdi32; do
        "$llvm/llvm-dlltool" -m "$machine" -d $dll.def -l "$out/$dll-$arch.lib"
    done
    # Unwind tables for every function, as MSVC makes them: SEH walks the stack by them.
    cc="$llvm/clang --target=$arch-pc-windows-msvc -nostdlib -ffreestanding -fno-builtin -fno-stack-protector -funwind-tables -O2 -fuse-ld=lld"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console hello.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" -o "$out/hello-$arch.exe"
    mkdir -p "$out/loader-$arch"
    # mylib.dll goes at the program's own base, so the loader has to move it.
    $cc -shared -Wl,/entry:DllMain -Wl,/base:0x140000000 -Wl,/def:mylib.def mylib.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/mylib.dll"
    $cc -shared -Wl,/entry:DllMain -Wl,/def:mylib2.def mylib2.c -o "$out/loader-$arch/mylib2.dll"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console loader.c "$out/loader-$arch/mylib.lib" "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" \
        -o "$out/loader-$arch/loader.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console threads.c "$out/loader-$arch/mylib.lib" "$out/kernel32-$arch.lib" \
        -o "$out/loader-$arch/threads.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console files.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/files.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console registry.c "$out/kernel32-$arch.lib" "$out/advapi32-$arch.lib" \
        -o "$out/loader-$arch/registry.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console environment.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/environment.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows desktop.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" -o "$out/loader-$arch/desktop.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows windows.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/gdi32-$arch.lib" \
        -o "$out/loader-$arch/windows.exe"
done
