//! Mapping images into a process: the program, ntdll, atlas's own DLLs, and DLLs from the disk as ntdll's loader asks
//! for them by name. An image that can't go at its preferred base is relocated as it's mapped, as NT's kernel does.
//! Also `PROTECT`, for the loader to open an import table while it fills it in.

use atlas_abi::status;
use exec::{Isa, PAGE, Prot, Turn};
use kernels_core::lock;
use kernels_core::mm::Regions;
use kernels_core::thread::Cx;
use kernels_core::user;

use crate::images::pe::Pe;
use crate::images::{self};
use crate::os;
use crate::process::{Area, AreaKind, GRANULE, Task, disk_path};

/// Where an image goes: its preferred base if that's free, else the lowest room. Relocates `image` there.
pub(crate) fn place(regions: &Regions<Area>, pe: &Pe<'_>, image: &mut [u8]) -> Result<u64, String> {
    let size = u64::from(pe.size_of_image).next_multiple_of(GRANULE);
    let alloc = images::layout(pe.isa).alloc;
    let base = if pe.image_base >= alloc.start && pe.image_base + size < alloc.end && regions.is_free(pe.image_base..pe.image_base + size) {
        pe.image_base
    } else {
        regions.free_above(alloc, size, GRANULE).ok_or("no room for an image")?
    };
    if base != pe.image_base {
        pe.relocate(image, base).map_err(|_| "an image that can't be moved, and its base isn't free")?;
    }
    Ok(base)
}

/// Maps a laid-out image at `base`, each page with its section's protection (headers read-only).
pub(crate) fn map_image(t: &mut Turn<'_>, regions: &mut Regions<Area>, pe: &Pe<'_>, base: u64, image: &[u8]) -> Result<(), String> {
    let pages = image.len().div_ceil(PAGE as usize);
    let mut prots = vec![Prot::NONE; pages];
    prots[..(pe.size_of_headers as usize).div_ceil(PAGE as usize).min(pages)].fill(Prot::R);
    for s in &pe.sections {
        let first = s.rva as usize / PAGE as usize;
        let last = (s.rva as usize + s.virtual_size.max(s.raw_size) as usize).div_ceil(PAGE as usize).min(pages);
        for p in &mut prots[first..last] {
            *p = *p | s.prot;
        }
    }
    let mut page = 0;
    while page < pages {
        let prot = prots[page];
        let mut end = page + 1;
        while end < pages && prots[end] == prot {
            end += 1;
        }
        if prot != Prot::NONE {
            let (from, to) = (page * PAGE as usize, (end * PAGE as usize).min(image.len()));
            let range = base + from as u64..base + (end as u64) * PAGE;
            t.space().map(range.clone(), prot).with(&image[from..to]).map_err(|_| "no memory for the image")?;
            regions.insert(range, Area::of(prot, AreaKind::Image));
        }
        page = end;
    }
    Ok(())
}

/// Maps `atlas-guest`'s ntdll: its base, and its image as laid out.
pub(crate) fn map_ntdll(t: &mut Turn<'_>, regions: &mut Regions<Area>, isa: Isa) -> Result<(u64, Vec<u8>), String> {
    let bytes = images::guest(isa).ok_or("atlas-guest (ntdll) isn't built for this ISA: see kernels/README.md")?;
    let pe = Pe::parse(bytes).map_err(|_| "atlas-guest's image is broken")?;
    let mut image = pe.layout().map_err(|_| "atlas-guest's image is broken")?;
    let base = place(regions, &pe, &mut image)?;
    map_image(t, regions, &pe, base, &image)?;
    Ok((base, image))
}

/// A module's name, as the loader compares them: its file name, lowercased, with `.dll` if it has no extension.
pub(crate) fn module_name(raw: &str) -> String {
    let file = raw.rsplit(['\\', '/']).next().unwrap_or(raw).to_ascii_lowercase();
    if file.contains('.') { file } else { format!("{file}.dll") }
}

pub(crate) fn prot_to_page(prot: Prot) -> u32 {
    match (prot.read(), prot.write(), prot.exec()) {
        (_, true, true) => 0x40,
        (_, false, true) if prot.read() => 0x20,
        (_, false, true) => 0x10,
        (_, true, false) => 0x04,
        (true, false, false) => 0x02,
        _ => 0x01,
    }
}

impl Task {
    /// `MAP_IMAGE`: a DLL by name, for the loader. Its status; its base and path go where it says.
    pub(crate) fn map_named(&mut self, cx: &mut Cx<'_>, name_at: u64, len: u64, base_out: u64, path_out: u64) -> u32 {
        let Ok(raw) = self.mem(cx, false, |t| user::read_vec(&t.space(), name_at, len.min(259) as usize)) else { return status::ACCESS_VIOLATION };
        let raw = String::from_utf8_lossy(&raw).into_owned();
        let name = module_name(&raw);
        // An API set is its DLL's other name: the loader learns which from the path it's given.
        let (raw, name) = match crate::imports::api_set(&name) {
            Some(dll) => (dll.to_string(), dll.to_string()),
            None => (raw, name),
        };
        let found = self.find_or_map(cx, &raw, &name);
        let (base, path) = match found {
            Ok(found) => found,
            Err(code) => {
                if self.k.options.trace {
                    self.k.host.write(host::Stream::Err, format!("atlas: no DLL {raw} ({code:#x})\n").as_bytes());
                }
                return code;
            }
        };
        let path16: Vec<u8> = path.encode_utf16().take(259).chain([0]).flat_map(u16::to_le_bytes).collect();
        // The loader's base is pointer-sized.
        let base_bytes = self.abi().ptr(base);
        let wrote = self.mem(cx, true, |t| {
            user::write(&mut t.space(), base_out, &base_bytes)?;
            user::write(&mut t.space(), path_out, &path16)
        });
        if wrote.is_err() { status::ACCESS_VIOLATION } else { status::SUCCESS }
    }

    fn find_or_map(&mut self, cx: &mut Cx<'_>, raw: &str, name: &str) -> Result<(u64, String), u32> {
        if let Some(found) = lock(&self.process.state).mapped.get(name) {
            return Ok(found.clone());
        }
        let isa = self.isa;
        // atlas's own: made for this ISA, always from System32, whatever the program brings.
        if let Some(dll) = os::find(name) {
            let image = self.k.images.get(isa, dll);
            let base = images::layout(isa).dll_base(dll);
            let path = format!("C:\\Windows\\System32\\{}", os::DLLS[dll].name);
            let mut state = lock(&self.process.state);
            let state = &mut *state;
            let pe = Pe::parse(&image).map_err(|_| status::INVALID_IMAGE_FORMAT)?;
            if !state.regions.is_free(base..base + image.len() as u64) {
                return Err(status::INVALID_IMAGE_FORMAT);
            }
            cx.turn(|t| map_image(t, &mut state.regions, &pe, base, &image)).map_err(|_| status::NO_MEMORY)?;
            state.mapped.insert(name.to_string(), (base, path.clone()));
            return Ok((base, path));
        }
        // A file: by its path, or found as Windows looks: the program's folder, then System32.
        let candidates: Vec<String> = if raw.contains(['\\', '/']) {
            vec![disk_path(raw)]
        } else {
            let dir = lock(&self.process.state).exe_dir.clone();
            // A 32-bit program's System32 is SysWOW64.
            let system = if self.isa == Isa::X86_32 { "/Windows/SysWOW64" } else { "/Windows/System32" };
            vec![format!("{dir}/{raw}"), format!("{system}/{raw}"), format!("{dir}/{name}"), format!("{system}/{name}")]
        };
        let found = {
            let disk = lock(&self.k.disk);
            candidates.iter().find_map(|path| {
                let ino = disk.resolve(layers::ROOT, path, true).ok()?;
                Some((path.clone(), disk.contents(ino).ok()?.to_vec()))
            })
        };
        let (path, bytes) = found.ok_or(status::DLL_NOT_FOUND)?;
        let pe = Pe::parse(&bytes).map_err(|_| status::INVALID_IMAGE_FORMAT)?;
        if pe.isa != isa || !pe.is_dll {
            return Err(status::INVALID_IMAGE_FORMAT);
        }
        let mut image = pe.layout().map_err(|_| status::INVALID_IMAGE_FORMAT)?;
        let mut state = lock(&self.process.state);
        let state = &mut *state;
        let base = place(&state.regions, &pe, &mut image).map_err(|_| status::NO_MEMORY)?;
        cx.turn(|t| map_image(t, &mut state.regions, &pe, base, &image)).map_err(|_| status::NO_MEMORY)?;
        let windows_path = format!("C:{}", path.replace('/', "\\"));
        state.mapped.insert(name.to_string(), (base, windows_path.clone()));
        Ok((base, windows_path))
    }

    /// `UNMAP_IMAGE`: a DLL the loader has unloaded. Only one it mapped by name, from the disk: atlas's own and
    /// ntdll stay, as do the program and anything that isn't an image.
    pub(crate) fn unmap_image(&mut self, cx: &mut Cx<'_>, base: u64, size: u64) -> u32 {
        if base >= self.layout().system_base || !base.is_multiple_of(GRANULE) || size == 0 {
            return status::INVALID_PARAMETER;
        }
        let range = base..(base + size).next_multiple_of(GRANULE);
        {
            let mut state = lock(&self.process.state);
            let Some(name) = state.mapped.iter().find(|(_, (b, _))| *b == base).map(|(n, _)| n.clone()) else { return status::INVALID_PARAMETER };
            let program = module_name(&state.image);
            if name == "ntdll.dll" || name == program {
                return status::INVALID_PARAMETER;
            }
            if state.regions.iter().any(|(r, a)| r.start < range.end && r.end > range.start && a.kind != AreaKind::Image) {
                return status::INVALID_PARAMETER;
            }
            state.mapped.remove(&name);
            state.regions.remove(range.clone());
        }
        cx.turn(|t| t.space().unmap(range));
        status::SUCCESS
    }

    /// `VirtualProtect` and `PROTECT`: gives pages a new protection, and writes the old one.
    pub(crate) fn virtual_protect(&mut self, cx: &mut Cx<'_>, addr: u64, size: u64, protect: u32, old_out: u64) -> u32 {
        let Some(prot) = crate::process::prot_from(protect) else { return status::INVALID_PARAMETER };
        let guard = protect & crate::process::PAGE_GUARD != 0;
        let range = addr & !(PAGE - 1)..(addr + size.max(1)).next_multiple_of(PAGE);
        let old = {
            let mut state = lock(&self.process.state);
            let mut at = range.start;
            let mut old = None;
            while at < range.end {
                let Some((r, a)) = state.regions.find(at) else { return status::INVALID_PARAMETER };
                if matches!(a.kind, AreaKind::Kept | AreaKind::Reserved | AreaKind::System) {
                    return status::INVALID_PARAMETER;
                }
                // A shared view is no more writable than its section.
                if prot.write() && a.view.as_ref().is_some_and(|v| !v.copy && !v.section.writable) {
                    return status::INVALID_PARAMETER;
                }
                old.get_or_insert((a.prot, a.guard));
                at = r.end;
            }
            state.regions.update(range.clone(), |a| {
                a.prot = prot;
                a.guard = guard;
            });
            old.unwrap_or((Prot::NONE, false))
        };
        cx.turn(|t| t.space().protect(range, if guard { Prot::NONE } else { prot }));
        let old = prot_to_page(old.0) | if old.1 { crate::process::PAGE_GUARD } else { 0 };
        if old_out != 0 && self.mem(cx, true, |t| user::write_u32(&mut t.space(), old_out, old)).is_err() {
            return status::ACCESS_VIOLATION;
        }
        status::SUCCESS
    }
}
