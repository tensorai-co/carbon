//! Making a system DLL's image in memory, from its declaration: PE headers, a `.text` of trap stubs for its host and
//! missing exports, and an export table whose guest exports forward to `ntdll`. File and memory layouts are the same
//! (both aligned to a page), so the bytes are the file `System32` shows and the image a process maps.

use exec::Isa;

use super::pe::{MACHINE_AMD64, MACHINE_ARM64};

const PAGE: usize = 0x1000;
/// Each stub's room in `.text`.
pub const STUB: usize = 32;

/// What one export becomes.
#[derive(Debug, Clone)]
pub enum Body {
    /// A stub that traps into atlas with this number.
    Trap(u64),
    /// A forwarder: `dll.name`.
    Forward(String),
}

/// A stub that traps into atlas with call number `nr`, then returns to its caller with the result.
/// With `gate`, an x86-64 stub comes in through atlas's trampoline (`TRAP_AT`) rather than `syscall`.
pub fn stub(isa: Isa, nr: u64, gate: bool) -> Vec<u8> {
    let mut code = Vec::with_capacity(STUB);
    match isa {
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
pub fn dll(isa: Isa, name: &str, base: u64, exports: &[(&str, Body)], gate: bool) -> Vec<u8> {
    // .text: one stub per trapping export, in order.
    let mut text = Vec::new();
    let mut functions = Vec::new();
    let text_rva = PAGE;
    for (_, body) in exports {
        functions.push(match body {
            Body::Trap(nr) => {
                let rva = text_rva + text.len();
                text.extend(stub(isa, *nr, gate));
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
            (None, Body::Trap(_)) => 0,
        })
        .collect();
    // The directory: Characteristics, TimeDateStamp, Major and MinorVersion, Name, Base, NumberOfFunctions,
    // NumberOfNames, and the three tables' RVAs.
    let mut edata = Vec::new();
    for v in [0u32, 0, 0, name_rva, 1, n as u32, n as u32, (edata_rva + functions_at) as u32, (edata_rva + names_at) as u32, (edata_rva + ordinals_at) as u32] {
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
    let size_of_image = edata_rva + edata_size.next_multiple_of(PAGE);

    // Headers.
    let mut image = vec![0u8; size_of_image];
    let put = |image: &mut Vec<u8>, at: usize, bytes: &[u8]| image[at..at + bytes.len()].copy_from_slice(bytes);
    put(&mut image, 0, b"MZ");
    put(&mut image, 0x3c, &0x40u32.to_le_bytes());
    put(&mut image, 0x40, b"PE\0\0");
    let coff = 0x44;
    let machine = if isa == Isa::Arm64 { MACHINE_ARM64 } else { MACHINE_AMD64 };
    put(&mut image, coff, &machine.to_le_bytes());
    put(&mut image, coff + 2, &2u16.to_le_bytes());
    put(&mut image, coff + 16, &240u16.to_le_bytes());
    put(&mut image, coff + 18, &0x2022u16.to_le_bytes()); // an executable, large-address-aware DLL
    let opt = coff + 20;
    put(&mut image, opt, &0x20bu16.to_le_bytes());
    put(&mut image, opt + 4, &(text_size as u32).to_le_bytes());
    put(&mut image, opt + 20, &(text_rva as u32).to_le_bytes());
    put(&mut image, opt + 24, &base.to_le_bytes());
    put(&mut image, opt + 32, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 36, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 40, &10u16.to_le_bytes());
    put(&mut image, opt + 48, &10u16.to_le_bytes());
    put(&mut image, opt + 56, &(size_of_image as u32).to_le_bytes());
    put(&mut image, opt + 60, &(PAGE as u32).to_le_bytes());
    put(&mut image, opt + 68, &3u16.to_le_bytes()); // console
    put(&mut image, opt + 70, &0x160u16.to_le_bytes());
    put(&mut image, opt + 108, &16u32.to_le_bytes());
    put(&mut image, opt + 112, &(edata_rva as u32).to_le_bytes());
    put(&mut image, opt + 116, &(edata_size as u32).to_le_bytes());
    let table = opt + 240;
    for (i, (name, rva, size, flags)) in [(b".text\0\0\0", text_rva, text_size, 0x6000_0020u32), (b".edata\0\0", edata_rva, edata_size, 0x4000_0040)].into_iter().enumerate() {
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
    image
}

#[cfg(test)]
mod tests {
    use super::super::pe::{Found, Pe, Wanted};
    use super::*;

    #[test]
    fn made_images_read_back() {
        let exports = [("WriteFile", Body::Trap(0x1001)), ("HeapAlloc", Body::Forward("ntdll.RtlAllocateHeap".into())), ("ExitProcess", Body::Trap(0x1002))];
        for isa in [Isa::Arm64, Isa::X86_64] {
            let image = dll(isa, "KERNEL32.dll", 0x7ff0_0000_0000, &exports, false);
            let pe = Pe::parse(&image).unwrap();
            assert_eq!((pe.isa, pe.image_base, pe.is_dll), (isa, 0x7ff0_0000_0000, true));
            let laid = pe.layout().unwrap();
            assert_eq!(laid, image);
            let Some(Found::Rva(rva)) = pe.export(&laid, &Wanted::Name("ExitProcess".into())).unwrap() else { panic!() };
            assert_eq!(&laid[rva as usize..rva as usize + 4], &stub(isa, 0x1002, false)[..4]);
            assert_eq!(pe.export(&laid, &Wanted::Name("HeapAlloc".into())).unwrap(), Some(Found::Forward("ntdll.RtlAllocateHeap".into())));
            assert_eq!(pe.export(&laid, &Wanted::Ordinal(1)).unwrap(), pe.export(&laid, &Wanted::Name("WriteFile".into())).unwrap());
            assert_eq!(pe.export(&laid, &Wanted::Name("Nope".into())).unwrap(), None);
        }
    }
}
