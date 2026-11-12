//! Reading PE images (EXEs and DLLs): headers, sections, imports, exports and relocations, as far as loading needs.

use exec::{Isa, Prot};

pub const MACHINE_I386: u16 = 0x14c;
pub const MACHINE_AMD64: u16 = 0x8664;
pub const MACHINE_ARM64: u16 = 0xaa64;

const DIR_EXPORT: usize = 0;
const DIR_RESOURCE: usize = 2;
const DIR_BASERELOC: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadImage;

type Result<T> = std::result::Result<T, BadImage>;

#[derive(Debug, Clone)]
pub struct Section {
    pub rva: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub prot: Prot,
}

/// What an export is found by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    Name(String),
    #[cfg_attr(not(test), allow(dead_code))]
    Ordinal(u16),
}

/// What an export resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Rva(u32),
    /// Another DLL's export: `dll.name`.
    Forward(String),
}

#[derive(Debug)]
pub struct Pe<'a> {
    bytes: &'a [u8],
    pub isa: Isa,
    pub image_base: u64,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub is_dll: bool,
    /// IMAGE_FILE_RELOCS_STRIPPED: it can only go at its own base.
    pub fixed: bool,
    pub sections: Vec<Section>,
    dirs: Vec<(u32, u32)>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(b.get(at..at + 2).ok_or(BadImage)?.try_into().map_err(|_| BadImage)?))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).ok_or(BadImage)?.try_into().map_err(|_| BadImage)?))
}

fn u64_at(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(b.get(at..at + 8).ok_or(BadImage)?.try_into().map_err(|_| BadImage)?))
}

impl<'a> Pe<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Pe<'a>> {
        if bytes.get(..2) != Some(b"MZ") {
            return Err(BadImage);
        }
        let pe = u32_at(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(BadImage);
        }
        let coff = pe + 4;
        let isa = match u16_at(bytes, coff)? {
            MACHINE_ARM64 => Isa::Arm64,
            MACHINE_AMD64 => Isa::X86_64,
            MACHINE_I386 => Isa::X86_32,
            _ => return Err(BadImage),
        };
        let sections = u16_at(bytes, coff + 2)? as usize;
        let optional_size = u16_at(bytes, coff + 16)? as usize;
        let characteristics = u16_at(bytes, coff + 18)?;
        let (is_dll, fixed) = (characteristics & 0x2000 != 0, characteristics & 1 != 0);
        let opt = coff + 20;
        let pe32_plus = match u16_at(bytes, opt)? {
            0x20b => true,
            0x10b => false,
            _ => return Err(BadImage),
        };
        let image_base = if pe32_plus { u64_at(bytes, opt + 24)? } else { u64::from(u32_at(bytes, opt + 28)?) };
        let size_of_image = u32_at(bytes, opt + 56)?;
        let size_of_headers = u32_at(bytes, opt + 60)?;
        let (count_at, dirs_at) = if pe32_plus { (opt + 108, opt + 112) } else { (opt + 92, opt + 96) };
        let count = (u32_at(bytes, count_at)? as usize).min(16);
        let dirs = (0..count).map(|i| Ok((u32_at(bytes, dirs_at + i * 8)?, u32_at(bytes, dirs_at + i * 8 + 4)?))).collect::<Result<_>>()?;
        let table = opt + optional_size;
        let sections = (0..sections)
            .map(|i| {
                let at = table + i * 40;
                let flags = u32_at(bytes, at + 36)?;
                let mut prot = Prot::NONE;
                for (bit, p) in [(0x4000_0000, Prot::R), (0x8000_0000, Prot::W), (0x2000_0000, Prot::X)] {
                    if flags & bit != 0 {
                        prot = prot | p;
                    }
                }
                Ok(Section { virtual_size: u32_at(bytes, at + 8)?, rva: u32_at(bytes, at + 12)?, raw_size: u32_at(bytes, at + 16)?, raw_offset: u32_at(bytes, at + 20)?, prot })
            })
            .collect::<Result<_>>()?;
        Ok(Pe { bytes, isa, image_base, size_of_image, size_of_headers, is_dll, fixed, sections, dirs })
    }

    /// The image as it's laid out in memory: headers, then each section at its RVA.
    pub fn layout(&self) -> Result<Vec<u8>> {
        let mut image = vec![0u8; self.size_of_image as usize];
        let headers = (self.size_of_headers as usize).min(self.bytes.len()).min(image.len());
        image[..headers].copy_from_slice(&self.bytes[..headers]);
        for s in &self.sections {
            let size = s.raw_size.min(s.virtual_size.max(s.raw_size)) as usize;
            let from = self.bytes.get(s.raw_offset as usize..s.raw_offset as usize + size).ok_or(BadImage)?;
            let to = image.get_mut(s.rva as usize..s.rva as usize + size.min(self.size_of_image as usize - s.rva as usize)).ok_or(BadImage)?;
            let n = to.len();
            to.copy_from_slice(&from[..n]);
        }
        Ok(image)
    }

    fn dir(&self, index: usize) -> Option<(u32, u32)> {
        self.dirs.get(index).copied().filter(|(rva, size)| *rva != 0 && *size != 0)
    }

    /// The subsystem it's for (2 GUI, 3 console), and that subsystem's version.
    pub fn subsystem(&self) -> (u16, u16, u16) {
        let opt = u32_at(self.bytes, 0x3c).map(|pe| pe as usize + 24).unwrap_or(0);
        let at = |o: usize| u16_at(self.bytes, opt + o).unwrap_or(0);
        (at(68), at(48), at(50))
    }

    /// A resource's bytes, in the image as laid out: of type `kind` (24 is a manifest), its first name and language.
    pub fn resource<'i>(&self, image: &'i [u8], kind: u16) -> Option<&'i [u8]> {
        let (root, _) = self.dir(DIR_RESOURCE)?;
        let root = root as usize;
        // A directory's entries: (name or ID, offset from the root, whether that's another directory).
        let entries = |at: usize| -> Option<Vec<(u32, usize, bool)>> {
            let count = usize::from(u16_at(image, at + 12).ok()?) + usize::from(u16_at(image, at + 14).ok()?);
            (0..count.min(4096))
                .map(|i| {
                    let e = at + 16 + i * 8;
                    let (name, offset) = (u32_at(image, e).ok()?, u32_at(image, e + 4).ok()?);
                    Some((name, root + (offset & 0x7fff_ffff) as usize, offset & 0x8000_0000 != 0))
                })
                .collect()
        };
        let (_, names, sub) = entries(root)?.into_iter().find(|(name, _, _)| *name == u32::from(kind))?;
        let (_, languages, sub2) = *entries(names).filter(|_| sub)?.first()?;
        let (_, leaf, sub3) = *entries(languages).filter(|_| sub2)?.first()?;
        if sub3 {
            return None;
        }
        let (rva, size) = (u32_at(image, leaf).ok()? as usize, u32_at(image, leaf + 4).ok()? as usize);
        image.get(rva..rva + size)
    }

    /// A data directory's RVA and size (1 imports, 13 delay-load imports, …), if it has one.
    pub fn directory(&self, index: usize) -> Option<(u32, u32)> {
        self.dir(index)
    }

    /// Every export's name, in the image as laid out in memory.
    pub fn export_names(&self, image: &[u8]) -> Result<Vec<String>> {
        let Some((rva, _)) = self.dir(DIR_EXPORT) else { return Ok(Vec::new()) };
        let at = rva as usize;
        let (names, name_table) = (u32_at(image, at + 24)?, u32_at(image, at + 32)?);
        (0..names as usize).map(|i| cstr(image, u32_at(image, name_table as usize + i * 4)? as usize)).collect()
    }

    /// Finds an export, in the image as laid out in memory.
    pub fn export(&self, image: &[u8], wanted: &Wanted) -> Result<Option<Found>> {
        let Some((rva, size)) = self.dir(DIR_EXPORT) else { return Ok(None) };
        let at = rva as usize;
        let base = u32_at(image, at + 16)?;
        let (functions, names) = (u32_at(image, at + 20)?, u32_at(image, at + 24)?);
        let (function_table, name_table, ordinal_table) = (u32_at(image, at + 28)?, u32_at(image, at + 32)?, u32_at(image, at + 36)?);
        let index = match wanted {
            Wanted::Ordinal(ordinal) => u32::from(*ordinal).checked_sub(base),
            Wanted::Name(name) => {
                let mut found = None;
                for i in 0..names as usize {
                    if cstr(image, u32_at(image, name_table as usize + i * 4)? as usize)? == *name {
                        found = Some(u32::from(u16_at(image, ordinal_table as usize + i * 2)?));
                        break;
                    }
                }
                found
            }
        };
        let Some(index) = index.filter(|i| *i < functions) else { return Ok(None) };
        let target = u32_at(image, function_table as usize + index as usize * 4)?;
        Ok(Some(if target >= rva && target < rva + size { Found::Forward(cstr(image, target as usize)?) } else { Found::Rva(target) }))
    }

    /// Moves a laid-out image to `new_base`, by its base relocations.
    pub fn relocate(&self, image: &mut [u8], new_base: u64) -> Result<()> {
        let delta = new_base.wrapping_sub(self.image_base);
        let Some((rva, size)) = self.dir(DIR_BASERELOC) else {
            // Nothing to fix up: it goes anywhere, unless it says its relocations were stripped.
            return if delta == 0 || !self.fixed { Ok(()) } else { Err(BadImage) };
        };
        let mut at = rva as usize;
        while at < (rva + size) as usize {
            let (page, block) = (u32_at(image, at)? as usize, u32_at(image, at + 4)? as usize);
            if block < 8 {
                break;
            }
            for i in 0..(block - 8) / 2 {
                let entry = u16_at(image, at + 8 + i * 2)?;
                let target = page + (entry & 0xfff) as usize;
                match entry >> 12 {
                    0 => {}
                    3 => {
                        let v = u32_at(image, target)?.wrapping_add(delta as u32);
                        image.get_mut(target..target + 4).ok_or(BadImage)?.copy_from_slice(&v.to_le_bytes());
                    }
                    10 => {
                        let v = u64_at(image, target)?.wrapping_add(delta);
                        image.get_mut(target..target + 8).ok_or(BadImage)?.copy_from_slice(&v.to_le_bytes());
                    }
                    _ => return Err(BadImage),
                }
            }
            at += block;
        }
        Ok(())
    }
}

fn cstr(b: &[u8], at: usize) -> Result<String> {
    let rest = b.get(at..).ok_or(BadImage)?;
    let end = rest.iter().position(|&c| c == 0).ok_or(BadImage)?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}
