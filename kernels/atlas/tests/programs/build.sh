#!/bin/sh
# Builds the Windows test programs for ARM64 and x86-64 into $1 (default: the current directory), with LLVM's clang,
# lld-link and llvm-dlltool (Homebrew's llvm and lld have them).
set -e
cd "$(dirname "$0")"
out="${1:-.}"
llvm="${LLVM:-/opt/homebrew/opt/llvm/bin}"
for arch in aarch64 x86_64; do
    machine=$([ $arch = aarch64 ] && echo arm64 || echo i386:x86-64)
    for dll in kernel32 user32 ntdll advapi32 gdi32 shell32 version ucrtbase ole32 comctl32; do
        "$llvm/llvm-dlltool" -m "$machine" -d $dll.def -l "$out/$dll-$arch.lib"
    done
    # Unwind tables for every function, as MSVC makes them: SEH walks the stack by them.
    cc="$llvm/clang --target=$arch-pc-windows-msvc -nostdlib -ffreestanding -fno-builtin -fno-stack-protector -funwind-tables -O2 -fuse-ld=lld"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console hello.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" -o "$out/hello-$arch.exe"
    mkdir -p "$out/loader-$arch"
    # mylib.dll goes at the program's own base, so the loader has to move it.
    $cc -shared -Wl,/entry:DllMain -Wl,/base:0x140000000 -Wl,/def:mylib.def mylib.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/mylib.dll"
    $cc -shared -Wl,/entry:DllMain -Wl,/def:mylib2.def mylib2.c -o "$out/loader-$arch/mylib2.dll"
    $cc -shared -Wl,/entry:DllMain -Wl,/def:mylib3.def mylib3.c "$out/loader-$arch/mylib2.lib" -o "$out/loader-$arch/mylib3.dll"
    $cc -shared -Wl,/entry:DllMain -Wl,/def:mylib4.def mylib4.c -o "$out/loader-$arch/mylib4.dll"
    # mylib4 is delay-loaded, through the program's own __delayLoadHelper2.
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console -Wl,/delayload:mylib4.dll loader.c "$out/loader-$arch/mylib.lib" \
        "$out/loader-$arch/mylib4.lib" "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" -o "$out/loader-$arch/loader.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console exceptions.c "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" -o "$out/loader-$arch/exceptions.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console fibers.c "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" -o "$out/loader-$arch/fibers.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console pool.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/pool.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console ansi.c "$out/kernel32-$arch.lib" "$out/advapi32-$arch.lib" "$out/version-$arch.lib" \
        -o "$out/loader-$arch/ansi.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console ansiwin.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" -o "$out/loader-$arch/ansiwin.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console crt.c "$out/ucrtbase-$arch.lib" "$out/kernel32-$arch.lib" -o "$out/loader-$arch/crt.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console threads.c "$out/loader-$arch/mylib.lib" "$out/kernel32-$arch.lib" \
        -o "$out/loader-$arch/threads.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console links.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/links.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console files.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/files.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console registry.c "$out/kernel32-$arch.lib" "$out/advapi32-$arch.lib" \
        -o "$out/loader-$arch/registry.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console process.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/process.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console sections.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/sections.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console io.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/io.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console pipes.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/pipes.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console sysinfo.c "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" -o "$out/loader-$arch/sysinfo.exe"
    # The same, with a manifest that says it supports Windows 10.
    "$llvm/llvm-rc" -no-preprocess -fo "$out/sysinfo10-$arch.res" sysinfo10.rc
    $cc -DWIN10 -Wl,/entry:mainCRTStartup -Wl,/subsystem:console sysinfo.c "$out/kernel32-$arch.lib" "$out/ntdll-$arch.lib" \
        -Wl,"$out/sysinfo10-$arch.res" -o "$out/loader-$arch/sysinfo10.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console security.c "$out/kernel32-$arch.lib" "$out/advapi32-$arch.lib" -o "$out/loader-$arch/security.exe"
    "$llvm/llvm-rc" -no-preprocess -fo "$out/resources-$arch.res" resources.rc
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console resources.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/version-$arch.lib" \
        -Wl,"$out/resources-$arch.res" -o "$out/loader-$arch/resources.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows comctl.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/gdi32-$arch.lib" \
        "$out/comctl32-$arch.lib" -o "$out/loader-$arch/comctl.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows shell.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/shell32-$arch.lib" \
        "$out/ole32-$arch.lib" -o "$out/loader-$arch/shell.exe"
    "$llvm/llvm-rc" -no-preprocess -fo "$out/dialog-$arch.res" dialog.rc
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows dialog.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/gdi32-$arch.lib" \
        -Wl,"$out/dialog-$arch.res" -o "$out/loader-$arch/dialog.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console installer.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/ole32-$arch.lib" \
        "$out/shell32-$arch.lib" -o "$out/loader-$arch/installer.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:console environment.c "$out/kernel32-$arch.lib" -o "$out/loader-$arch/environment.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows desktop.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/shell32-$arch.lib" -o "$out/loader-$arch/desktop.exe"
    $cc -Wl,/entry:mainCRTStartup -Wl,/subsystem:windows windows.c "$out/kernel32-$arch.lib" "$out/user32-$arch.lib" "$out/gdi32-$arch.lib" \
        -o "$out/loader-$arch/windows.exe"
done
# The same C runtime test as an ordinary C program, built by MinGW-w64 against the UCRT, if it's there.
if command -v x86_64-w64-mingw32-gcc >/dev/null; then
    x86_64-w64-mingw32-gcc -O2 crt.c -o "$out/loader-x86_64/crt-mingw.exe"
fi

# 32-bit x86 (WoW64). x86's imports are stdcall names, `_Name@N`, whose N (the bytes of arguments) the .def files
# don't say: each program's object says which it imports (through `__imp_`, or directly, as `__try`'s
# `_except_handler3`), and each goes in an import library for the system DLL whose .def has it, decorated, with `-k`
# so the DLL's own name is the plain one, as on Windows. The test DLLs' own import libraries come from linking them.
cc32="$llvm/clang --target=i686-pc-windows-msvc -nostdlib -ffreestanding -fno-builtin -fno-stack-protector -O2 -fuse-ld=lld"
system_dlls="kernel32 user32 ntdll advapi32 gdi32 shell32 version ucrtbase ole32 comctl32"
# The system import libraries an object needs: i686_imports object folder, printing the libraries.
i686_imports() {
    obj=$1
    tmp=$2
    for sym in $("$llvm/llvm-nm" -u "$obj" | awk '{print $2}'); do
        imp=${sym#__imp_}
        imp=${imp#_}
        for dll in $system_dlls; do
            line=$(grep -E "^${imp%@*}( DATA)?\$" "$dll.def" | head -1)
            if [ -n "$line" ]; then
                [ -f "$tmp/$dll.def" ] || head -2 "$dll.def" > "$tmp/$dll.def"
                grep -qx "$imp${line#${imp%@*}}" "$tmp/$dll.def" || echo "$imp${line#${imp%@*}}" >> "$tmp/$dll.def"
                break
            fi
        done
    done
    for def in "$tmp"/*.def; do
        [ -e "$def" ] || continue
        "$llvm/llvm-dlltool" -m i386 -k -d "$def" -l "${def%.def}.lib"
        echo "${def%.def}.lib"
    done
}
# A program: i686 source.c out.exe [more to link].
i686() {
    src=$1
    exe=$2
    shift 2
    tmp=$(mktemp -d)
    $cc32 -c "$src" -o "$tmp/prog.o"
    libs=$(i686_imports "$tmp/prog.o" "$tmp")
    # shellcheck disable=SC2086
    $cc32 "$tmp/prog.o" $libs -Wl,/entry:mainCRTStartup -Wl,/subsystem:console "$@" -o "$exe"
    rm -rf "$tmp"
}
# A DLL, with its .def and its import library beside it: i686dll source.c out.dll [more to link].
i686dll() {
    src=$1
    dll=$2
    shift 2
    tmp=$(mktemp -d)
    $cc32 -c "$src" -o "$tmp/dll.o"
    libs=$(i686_imports "$tmp/dll.o" "$tmp")
    # shellcheck disable=SC2086
    $cc32 -shared "$tmp/dll.o" $libs -Wl,/entry:DllMain -Wl,/def:"${src%.c}.def" "$@" -o "$dll"
    rm -rf "$tmp"
}
mkdir -p "$out/loader-i686"
for prog in shell comctl installer wow64 files io pipes process sections registry links environment security pool windows desktop ansiwin exceptions fibers crt ansi hello; do
    i686 $prog.c "$out/loader-i686/$prog.exe" || echo "i686: $prog.c doesn't build"
done
# The loader's DLLs, as the 64-bit ones are made.
o="$out/loader-i686"
i686dll mylib.c "$o/mylib.dll" -Wl,/base:0x400000 || echo "i686: mylib.c doesn't build"
i686dll mylib2.c "$o/mylib2.dll" || echo "i686: mylib2.c doesn't build"
i686dll mylib3.c "$o/mylib3.dll" "$o/mylib2.lib" || echo "i686: mylib3.c doesn't build"
i686dll mylib4.c "$o/mylib4.dll" || echo "i686: mylib4.c doesn't build"
i686 loader.c "$o/loader.exe" -Wl,/delayload:mylib4.dll "$o/mylib.lib" "$o/mylib4.lib" || echo "i686: loader.c doesn't build"
i686 threads.c "$o/threads.exe" "$o/mylib.lib" || echo "i686: threads.c doesn't build"
# wow64.exe's 64-bit partner, beside it: a 32-bit program starting a 64-bit one, which starts a 32-bit one.
"$llvm/clang" --target=x86_64-pc-windows-msvc -nostdlib -ffreestanding -fno-builtin -fno-stack-protector -O2 -fuse-ld=lld \
    -Wl,/entry:mainCRTStartup -Wl,/subsystem:console wowchild.c "$out/kernel32-x86_64.lib" -o "$out/loader-i686/wowchild.exe"
"$llvm/llvm-rc" -no-preprocess -fo "$out/resources-i686.res" resources.rc
i686 resources.c "$out/loader-i686/resources.exe" -Wl,"$out/resources-i686.res"
"$llvm/llvm-rc" -no-preprocess -fo "$out/dialog-i686.res" dialog.rc
i686 dialog.c "$out/loader-i686/dialog.exe" -Wl,"$out/dialog-i686.res"
