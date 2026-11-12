//! Sections: `CreateFileMapping`'s memory, which every view of it sees, in one process or several. A section's
//! frames are atlas's, made as they're first touched (zeros for one the page file backs, the file's bytes for a file
//! mapping), and a fault maps the frame itself into a view (`Map::frame`), or copy-on-write for a `FILE_MAP_COPY` one.
//! A file mapping's pages go back to the file when a view is flushed (`FlushViewOfFile`) or unmapped, and when the
//! process ends.
//!
//! Not yet: a `WriteFile` to the file after a page is in memory isn't seen by views (Windows has one cache for both),
//! image sections (`SEC_IMAGE`), and `SEC_RESERVE`'s committing as it goes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use exec::{Frame, PAGE, Prot};
use kernels_core::lock;
use kernels_core::thread::{Cx, SharedExec};
use layers::Ino;

use crate::process::{Area, AreaKind, Task};

/// A section's memory.
pub struct Section {
    exec: SharedExec,
    pub size: u64,
    /// The most a view may be: writable (`PAGE_READWRITE`), and executable.
    pub writable: bool,
    pub executable: bool,
    /// The file it maps, if not the page file.
    pub file: Option<Ino>,
    /// Its frames by page, made as they're first wanted.
    frames: Mutex<BTreeMap<u64, Frame>>,
    /// The descriptor it was made with or given since; None for its maker's token's default.
    pub security: Mutex<Option<crate::security::Descriptor>>,
}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Section({} bytes)", self.size)
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        let frames = std::mem::take(self.frames.get_mut().unwrap_or_else(|p| p.into_inner()));
        if frames.is_empty() {
            return;
        }
        let mut exec = lock(&self.exec);
        let mut memory = exec.memory();
        for frame in frames.into_values() {
            memory.free_frame(frame);
        }
    }
}

impl Section {
    pub fn new(exec: SharedExec, size: u64, writable: bool, executable: bool, file: Option<Ino>) -> Arc<Section> {
        Arc::new(Section { exec, size, writable, executable, file, frames: Mutex::new(BTreeMap::new()), security: Mutex::new(None) })
    }

    pub fn pages(&self) -> u64 {
        self.size.div_ceil(PAGE)
    }
}

/// Where a region of a view is in its section.
#[derive(Debug, Clone)]
pub struct View {
    pub section: Arc<Section>,
    /// The section's offset at this region's start.
    pub offset: u64,
    /// `FILE_MAP_COPY`: its writes are its own.
    pub copy: bool,
}

impl Task {
    /// Writes a file mapping's pages in `offsets` back to its file, as far as the file goes.
    pub(crate) fn write_back(&mut self, cx: &mut Cx<'_>, section: &Section, offsets: std::ops::Range<u64>) {
        let Some(ino) = section.file.filter(|_| section.writable) else { return };
        let pages: Vec<(u64, Frame)> = {
            let frames = lock(&section.frames);
            frames.range(offsets.start / PAGE..offsets.end.div_ceil(PAGE)).map(|(i, f)| (*i, *f)).collect()
        };
        let Ok(size) = lock(&self.k.disk).stat(ino).map(|s| s.size) else { return };
        for (index, frame) in pages {
            let at = index * PAGE;
            if at >= size {
                break;
            }
            let mut data = vec![0u8; PAGE.min(size - at) as usize];
            cx.turn(|t| t.memory().frame_read(frame, 0, &mut data));
            let now = self.k.host.wall();
            let _ = lock(&self.k.disk).write(ino, at, &data, now);
        }
    }

    /// The view a base address starts: its range (each of its regions, together) and its section.
    pub(crate) fn view_at(&self, base: u64) -> Option<(std::ops::Range<u64>, Arc<Section>, bool)> {
        let state = lock(&self.process.state);
        let (first, area) = state.regions.find(base)?;
        if area.kind != AreaKind::View || area.base != base || first.start != base {
            return None;
        }
        let view = area.view.clone()?;
        let mut end = first.end;
        while let Some((next, a)) = state.regions.find(end) {
            if a.kind != AreaKind::View || a.base != base {
                break;
            }
            end = next.end;
        }
        Some((base..end, view.section, view.copy))
    }

    /// Every view a process has, by its range, for writing back as it ends.
    pub(crate) fn views(&self) -> Vec<(std::ops::Range<u64>, Arc<Section>, u64)> {
        let state = lock(&self.process.state);
        state
            .regions
            .iter()
            .filter_map(|(range, a): (std::ops::Range<u64>, &Area)| a.view.as_ref().filter(|v| !v.copy).map(|v| (range, v.section.clone(), v.offset)))
            .collect()
    }
}

/// A fault in a view: the section's page, from the file if it's a file mapping and not in memory yet.
pub(crate) fn fault_view(k: &crate::Kernel, cx: &mut Cx<'_>, view: &View, page: u64, prot: Prot, write: bool) -> bool {
    let index = view.offset / PAGE;
    let section = &view.section;
    if index >= section.pages() {
        return false;
    }
    // A page of the file, read before exec's lock is taken, if the section doesn't have it yet.
    let mut frames = lock(&section.frames);
    let data = match (frames.contains_key(&index), section.file) {
        (false, Some(ino)) => {
            let mut data = vec![0u8; PAGE as usize];
            let n = lock(&k.disk).read(ino, index * PAGE, &mut data).unwrap_or(0);
            data.truncate(n);
            data
        }
        _ => Vec::new(),
    };
    cx.turn(|t| {
        let frame = match frames.get(&index) {
            Some(frame) => *frame,
            None => {
                let frame = t.memory().new_frame()?;
                if !data.is_empty() {
                    t.memory().frame_write(frame, 0, &data);
                }
                frames.insert(index, frame);
                frame
            }
        };
        if !view.copy {
            return t.space().map(page..page + PAGE, prot).frame(frame);
        }
        t.space().map(page..page + PAGE, prot).copy_on_write(frame)?;
        if write {
            t.space().make_writable(page, prot)?;
        }
        Ok::<(), exec::Error>(())
    })
    .is_ok()
}
