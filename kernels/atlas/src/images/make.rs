//! Making a system DLL's image in memory, from its declaration: PE headers, a `.text` of trap stubs for its host and
//! missing exports, and an export table whose guest exports forward to `ntdll`. File and memory layouts are the same
//! (both aligned to a page), so the bytes are the file `System32` shows and the image a process maps. It's PE32+ for
//! the 64-bit ISAs and PE32 for x86, whose stubs pop a stdcall function's arguments as they return.

use exec::Isa;

use super::pe::{MACHINE_AMD64, MACHINE_ARM64, MACHINE_I386};

const PAGE: usize = 0x1000;
/// Each stub's room in `.text`.
pub const STUB: usize = 32;

/// What one export becomes.
#[derive(Debug, Clone)]
pub enum Body {
    /// A stub that traps into atlas with this number; on x86 it pops `pop` bytes of arguments as it returns.
    Trap { nr: u64, pop: u16 },
    /// A forwarder: `dll.name`.
    Forward(String),
}

/// A stub that traps into atlas with call number `nr`, then returns to its caller with the result, popping `pop`
/// bytes of arguments on x86. With `gate`, an x86-64 stub comes in through atlas's trampoline (`TRAP_AT`) rather than
/// `syscall`.
pub fn stub(isa: Isa, nr: u64, pop: u16, gate: bool) -> Vec<u8> {
    let mut code = Vec::with_capacity(STUB);
    match isa {
        // mov eax, nr; int 0x2e; then ret pop, or ret
        Isa::X86_32 => {
            code.push(0xb8);
            code.extend((nr as u32).to_le_bytes());
            code.extend([0xcd, 0x2e]);
            if pop != 0 {
                code.push(0xc2);
                code.extend(pop.to_le_bytes());
            } else {
                code.push(0xc3);
            }
            code.resize(STUB, 0xcc);
        }
        Isa::Arm64 => {
            // movz x8, #nr; svc #0; ret
            for word in [0xd280_0008 | (nr as u32 & 0xffff) << 5, 0xd400_0001, 0xd65f_03c0] {
                code.extend(u32::to_le_bytes(word));
            }
            code.resize(STUB, 0);
        }
        _ => {
            // mov r10, rcx; mov eax, nr; then syscall, or mov r11, TRAP_AT; call r11; then ret
            code.extend([0x4c, 0x8b, 0xd1, 0xb8]);
            code.extend((nr as u32).to_le_bytes());
            if gate {
                code.extend([0x49, 0xbb]);
                code.extend(atlas_abi::call::TRAP_AT.to_le_bytes());
                code.extend([0x41, 0xff, 0xd3]);
            } else {
                code.extend([0x0f, 0x05]);
            }
            code.push(0xc3);
            code.resize(STUB, 0xcc);
        }
    }
    code
}

/// A DLL's image: `name` (as `KERNEL32.dll`), at `base`, with `exports` in ordinal order from 1.
/// Its exports' ordinals start at `first_ordinal`.
pub fn dll(isa: Isa, name: &str, base: u64, first_ordinal: u16, exports: &[(&str, Body)], gate: bool) -> Vec<u8> {
    // .text: one stub per trapping export, in order.
    let mut text = Vec::new();
    let mut functions = Vec::new();
    let text_rva = PAGE;
    for (_, body) in exports {
        functions.push(match body {
            Body::Trap { nr, pop } => {
                let rva = text_rva + text.len();
                text.extend(stub(isa, *nr, *pop, gate));
                Some(rva as u32)
            }
            Body::Forward(_) => None,
        });
    }
    let text_size = text.len().max(1).next_multiple_of(PAGE);
    let edata_rva = text_rva + text_size;

    // .edata: the directory, the three tables, then the strings.
    let n = exports.len();
    let mut sorted: Vec<usize> = (0..n).collect();
    sorted.sort_by(|&a, &b| exports[a].0.cmp(exports[b].0));
    let functions_at = 40;
    let names_at = functions_at + 4 * n;
    let ordinals_at = names_at + 4 * n;
    let mut strings_at = ordinals_at + 2 * n;
    let mut strings = Vec::new();
    let mut add_string = |s: &str, strings: &mut Vec<u8>| -> u32 {
        let rva = (edata_rva + strings_at) as u32;
        strings.extend(s.as_bytes());
        strings.push(0);
        strings_at += s.len() + 1;
        rva
    };
    let name_rva = add_string(name, &mut strings);
    let name_rvas: Vec<u32> = exports.iter().map(|(n, _)| add_string(n, &mut strings)).collect();
    let function_rvas: Vec<u32> = exports
        .iter()
        .zip(&functions)
        .map(|((_, body), rva)| match (rva, body) {
            (Some(rva), _) => *rva,
            (None, Body::Forward(to)) => add_string(to, &mut strings),
            (None, Body::Trap { .. }) => 0,
        })
        .collect();
    // The directory: Characteristics, TimeDateStamp, Major and MinorVersion, Name, Base, NumberOfFunctions,
    // NumberOfNames, and the three tables' RVAs.
    let mut edata = Vec::new();
    for v in [0u32, 0, 0, name_rva, u32::from(first_ordinal), n as u32, n as u32, (edata_rva + functions_at) as u32, (edata_rva + names_at) as u32, (edata_rva + ordinals_at) as u32] {
        edata.extend(v.to_le_bytes());
    }
    for rva in &function_rvas {
        edata.extend(rva.to_le_bytes());
    }
    for &i in &sorted {
        edata.extend(name_rvas[i].to_le_bytes());
    }
    for &i in &sorted {
        edata.extend((i as u16).to_le_bytes());
    }
    edata.extend(&strings);
    let edata_size = edata.len();

    // .rsrc: its version information, as Windows' own DLLs have.
    let rsrc_rva = edata_rva + edata_size.next_multiple_of(PAGE);
    let mut resources = std::collections::BTreeMap::new();
    resources.insert((crate::resources::RT_VERSION, 1, 0x409), crate::resources::system_version(name));
    let rsrc = crate::resources::build(rsrc_rva as u32, &resources);
    let size_of_image = rsrc_rva + rsrc.len().next_multiple_of(PAGE);

    // Headers.
    let mut image = vec![0u8; size_of_image];
    let put = |image: &mut Vec<u8>, at: usize, bytes: &[u8]| image[at..at + bytes.len()].copy_from_slice(bytes);
    put(&mut image, 0, b"MZ");
    put(&mut image, 0x3c, &0x40u32.to_le_bytes());
    put(&mut image, 0x40, b"PE\0\0");
    let coff = 0x44;
    let pe32 = isa == Isa::X86_32;
    let machine = match isa {
        Isa::Arm64 => MACHINE_ARM64,
        Isa::X86_64 => MACHINE_AMD64,
        Isa::X86_32 => MACHINE_I386,
    };
    // PE32's optional header is 16 bytes shorter: a 4-byte image base, stack and heap sizes, and BaseOfData.
    let optional_size: u16 = if pe32 { 224 } else { 240 };
    put(&mut image, coff, &machine.to_le_bytes());
    put(&mut image, coff + 2, &3u16.to_le_bytes());
    put(&mut image, coff + 16, &optional_size.to_le_bytes());
    // An executable DLL: large-address-aware (64-bit), or for a 32-bit machine.
    put(&mut image, coff + 18, &(if pe32 { 0x2102u16 } else { 0x2022u16 }).to_le_bytes());
    let opt = coff + 20;
    put(&mut image, opt, &(if pe32 { 0x10bu16 } else { 0x20bu16 }).to_le_bytes());
    put(&mut image, opt + 4, &(text_size as u32).to_le_bytes());
    put(&mut image, opt + 20, &(text_rva as u32).to_le_bytes());
    if pe32 {
        put(&mut image, opt + 28, &(base as u32).to_le_bytes());
    } else {
        put(&mut image, opt + 24, &base.to_le_bytes());
    }
    put(&mut image, opt + 32, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 36, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 40, &10u16.to_le_bytes());
    put(&mut image, opt + 48, &10u16.to_le_bytes());
    put(&mut image, opt + 56, &(size_of_image as u32).to_le_bytes());
    put(&mut image, opt + 60, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 68, &3u16.to_le_bytes()); // console
    // Dynamic base, NX compatible, and on PE32+ high-entropy addresses.
    put(&mut image, opt + 70, &(if pe32 { 0x140u16 } else { 0x160u16 }).to_le_bytes());
    let dirs = if pe32 { opt + 92 } else { opt + 108 };
    put(&mut image, dirs, &16u32.to_le_bytes());
    put(&mut image, dirs + 4, &(edata_rva as u32).to_le_bytes());
    put(&mut image, dirs + 8, &(edata_size as u32).to_le_bytes());
    put(&mut image, dirs + 20, &(rsrc_rva as u32).to_le_bytes());
    put(&mut image, dirs + 24, &(rsrc.len() as u32).to_le_bytes());
    let table = opt + usize::from(optional_size);
    for (i, (name, rva, size, flags)) in [(b".text\0\0\0", text_rva, text_size, 0x6000_0020u32), (b".edata\0\0", edata_rva, edata_size, 0x4000_0040), (b".rsrc\0\0\0", rsrc_rva, rsrc.len(), 0x4000_0040)].into_iter().enumerate() {
        let at = table + i * 40;
        put(&mut image, at, name);
        put(&mut image, at + 8, &(size as u32).to_le_bytes());
        put(&mut image, at + 12, &(rva as u32).to_le_bytes());
        put(&mut image, at + 16, &(size.next_multiple_of(PAGE) as u32).to_le_bytes());
        put(&mut image, at + 20, &(rva as u32).to_le_bytes());
        put(&mut image, at + 36, &flags.to_le_bytes());
    }
    put(&mut image, text_rva, &text);
    put(&mut image, edata_rva, &edata);
    put(&mut image, rsrc_rva, &rsrc);
    image
}

#[cfg(test)]
mod tests {
    use super::super::pe::{Found, Pe, Wanted};
    use super::*;

    #[test]
    fn made_images_read_back() {
        let exports = [("WriteFile", Body::Trap { nr: 0x1001, pop: 20 }), ("HeapAlloc", Body::Forward("ntdll.RtlAllocateHeap".into())), ("ExitProcess", Body::Trap { nr: 0x1002, pop: 4 })];
        for (isa, base) in [(Isa::Arm64, 0x7ff0_0000_0000), (Isa::X86_64, 0x7ff0_0000_0000), (Isa::X86_32, 0x7040_0000)] {
            let image = dll(isa, "KERNEL32.dll", base, 1, &exports, false);
            let pe = Pe::parse(&image).unwrap();
            assert_eq!((pe.isa, pe.image_base, pe.is_dll), (isa, base, true));
            let laid = pe.layout().unwrap();
            assert_eq!(laid, image);
            let Some(Found::Rva(rva)) = pe.export(&laid, &Wanted::Name("ExitProcess".into())).unwrap() else { panic!() };
            assert_eq!(&laid[rva as usize..rva as usize + 4], &stub(isa, 0x1002, 4, false)[..4]);
            assert_eq!(pe.export(&laid, &Wanted::Name("HeapAlloc".into())).unwrap(), Some(Found::Forward("ntdll.RtlAllocateHeap".into())));
            assert_eq!(pe.export(&laid, &Wanted::Ordinal(1)).unwrap(), pe.export(&laid, &Wanted::Name("WriteFile".into())).unwrap());
            assert_eq!(pe.export(&laid, &Wanted::Name("Nope".into())).unwrap(), None);
            // Its version information, where its resource directory says.
            let (root, _) = pe.directory(2).unwrap();
            let mut tree = crate::resources::Tree::new(root, |at, len| laid.get(at as usize..at as usize + len).map(<[u8]>::to_vec));
            let found = tree.find(&crate::resources::Id::Num(16), &crate::resources::Id::Num(1), None).unwrap();
            let block = &laid[found.data as usize..][..found.size as usize];
            assert!(crate::resources::query(block, "\\StringFileInfo\\040904B0\\OriginalFilename").is_some());
        }
    }
}
