//! The stub program: a tiny PE with no imports, whose only code is the gate. Windows' loader runs it like any program,
//! so the stub comes up with its own ntdll and a thread with a real TEB, none of which runs again once Carbon takes the
//! thread over.
//!
//! The gate is `int3; ret`. atlas's runtime calls it (it's `Layout::gate`) in place of `int 0x2e`; the `int3` stops the
//! thread for Carbon, and core pops the return address as `ret` would, once the kernel has answered.

use std::path::PathBuf;

/// Where the image is linked, above every guest address (`SPACE_TOP`). It has no relocations, so it loads here or not
/// at all.
pub const BASE: u64 = 0x7f40_0000_0000;
pub const GATE: u64 = BASE + 0x1010;
const IMAGE_SIZE: u32 = 0x2000;

pub fn image() -> Vec<u8> {
    let mut file = vec![0u8; 0x400];
    let put = |file: &mut Vec<u8>, at: usize, bytes: &[u8]| file[at..at + bytes.len()].copy_from_slice(bytes);
    // DOS header, pointing at the PE header after it.
    put(&mut file, 0, b"MZ");
    put(&mut file, 0x3c, &0x40u32.to_le_bytes());
    put(&mut file, 0x40, b"PE\0\0");
    // COFF: x86-64, one section, a 240-byte optional header; executable, large-address aware, no relocations.
    let coff = 0x44;
    put(&mut file, coff, &0x8664u16.to_le_bytes());
    put(&mut file, coff + 2, &1u16.to_le_bytes());
    put(&mut file, coff + 16, &240u16.to_le_bytes());
    put(&mut file, coff + 18, &0x0023u16.to_le_bytes());
    // Optional header, PE32+.
    let mut o: Vec<u8> = Vec::with_capacity(240);
    o.extend_from_slice(&0x20bu16.to_le_bytes());
    o.extend_from_slice(&[14, 0]); // linker version
    o.extend_from_slice(&0x200u32.to_le_bytes()); // size of code
    o.extend_from_slice(&0u32.to_le_bytes()); // initialized data
    o.extend_from_slice(&0u32.to_le_bytes()); // uninitialized data
    o.extend_from_slice(&0x1000u32.to_le_bytes()); // entry point: never runs, Carbon takes the thread first
    o.extend_from_slice(&0x1000u32.to_le_bytes()); // base of code
    o.extend_from_slice(&BASE.to_le_bytes());
    o.extend_from_slice(&0x1000u32.to_le_bytes()); // section alignment
    o.extend_from_slice(&0x200u32.to_le_bytes()); // file alignment
    for version in [6u16, 0, 0, 0, 6, 0] {
        o.extend_from_slice(&version.to_le_bytes());
    }
    o.extend_from_slice(&0u32.to_le_bytes()); // Win32 version
    o.extend_from_slice(&IMAGE_SIZE.to_le_bytes());
    o.extend_from_slice(&0x200u32.to_le_bytes()); // size of headers
    o.extend_from_slice(&0u32.to_le_bytes()); // checksum
    o.extend_from_slice(&2u16.to_le_bytes()); // GUI: no console of its own
    o.extend_from_slice(&0x8100u16.to_le_bytes()); // NX compatible, terminal-server aware, no ASLR
    for size in [0x10_0000u64, 0x1000, 0x10_0000, 0x1000] {
        o.extend_from_slice(&size.to_le_bytes()); // stack and heap reserve and commit
    }
    o.extend_from_slice(&0u32.to_le_bytes()); // loader flags
    o.extend_from_slice(&16u32.to_le_bytes()); // data directories, all empty
    o.extend_from_slice(&[0; 128]);
    debug_assert_eq!(o.len(), 240);
    put(&mut file, coff + 20, &o);
    // `.text`: RVA 0x1000, file offset 0x200, code, readable and executable.
    let section = coff + 20 + 240;
    put(&mut file, section, b".text\0\0\0");
    put(&mut file, section + 8, &0x1000u32.to_le_bytes());
    put(&mut file, section + 12, &0x1000u32.to_le_bytes());
    put(&mut file, section + 16, &0x200u32.to_le_bytes());
    put(&mut file, section + 20, &0x200u32.to_le_bytes());
    put(&mut file, section + 36, &0x6000_0020u32.to_le_bytes());
    // The code: breakpoints, with the gate, `int3; ret`, at RVA 0x1010.
    file[0x200..].fill(0xcc);
    put(&mut file, 0x200 + (GATE - BASE - 0x1000) as usize, &[0xcc, 0xc3]);
    file
}

/// Writes the stub where Carbon keeps its files, once per version, and gives its path.
pub fn path() -> Result<PathBuf, String> {
    let image = image();
    let dir = std::env::temp_dir().join("carbon");
    std::fs::create_dir_all(&dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    let hash = image.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    let path = dir.join(format!("stub-{hash:016x}.exe"));
    if std::fs::read(&path).ok().as_deref() != Some(&image[..]) {
        let partial = dir.join(format!("stub-{hash:016x}.{}.tmp", std::process::id()));
        std::fs::write(&partial, &image).map_err(|e| format!("writing {}: {e}", partial.display()))?;
        if std::fs::rename(&partial, &path).is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn image_is_an_x64_exe_with_the_gate() {
        let image = super::image();
        assert_eq!(&image[..2], b"MZ");
        assert_eq!(&image[0x40..0x44], b"PE\0\0");
        assert_eq!(u16::from_le_bytes([image[0x44], image[0x45]]), 0x8664);
        assert_eq!(u64::from_le_bytes(image[0x44 + 20 + 24..0x44 + 20 + 32].try_into().unwrap()), super::BASE);
        let gate = 0x200 + (super::GATE - super::BASE - 0x1000) as usize;
        assert_eq!(&image[gate..gate + 2], &[0xcc, 0xc3]);
        assert_eq!(image.len(), 0x400);
    }
}
