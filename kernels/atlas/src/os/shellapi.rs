//! The shell's core, for shell32: special folders (`SHGetFolderPath`, `SHGetSpecialFolderLocation`), item ID lists,
//! file operations (`SHFileOperation`), running programs (`ShellExecute(Ex)`), choosing a folder
//! (`SHBrowseForFolder`), and what a file is (`SHGetFileInfo`).
//!
//! **Special folders** are the profile's, where the system layer has them: `C:\Users\carbon` and its AppData, the
//! start menu under it, `C:\ProgramData` for everyone's, `C:\Users\Public`, and Windows' own. Program Files and System
//! are the 32-bit ones for a 32-bit program, as WoW64 has them. A folder asked for is made on the disk if it isn't
//! there, as a real profile has them all.
//!
//! **Item ID lists** (PIDLs) are allocated from the program's heap, through ntdll's task allocator, so that
//! `CoTaskMemFree` and `ILFree` free them as Windows' do. Each is one item holding a full path, which is all a file
//! system folder needs; the virtual folders (My Computer, the Control Panel, the network) are an item with no path,
//! which `SHGetPathFromIDList` says has none. The desktop is the empty list.
//!
//! **`ShellExecute`** runs programs (and `runas` the same, as there's one user); documents, folders and URLs need
//! file associations and Explorer, which aren't there yet, so it says there's no association for them, as Windows does
//! for a file nothing opens.
//!
//! **`SHBrowseForFolder`** is a dialog of shell32's own, made by the dialog manager: the caller's text, a folder's path
//! to type or to be given (`BFFM_SETSELECTION`), and OK and Cancel. There's no tree of folders yet.

use kernels_core::thread::Cx;
use kernels_core::user;
use exec::Isa;

use super::Reply;
use crate::process::Task;
use crate::win32k::dialog::{self, IDCANCEL, IDOK, Id, Item, Template};

const MAX_PATH: usize = 260;

/// A PIDL's one item: its size, this mark, what it is (0 a path, else a virtual folder's CSIDL plus 0x100), and a
/// path.
const MARK: &[u8; 4] = b"ATLS";

/// Where a special folder is: a path, or None for a virtual folder (one that isn't on the disk). None for a CSIDL
/// there's no such folder for.
pub fn special_folder(csidl: u32, isa: Isa) -> Option<Option<String>> {
    const PROFILE: &str = "C:\\Users\\carbon";
    const ROAMING: &str = "C:\\Users\\carbon\\AppData\\Roaming";
    const LOCAL: &str = "C:\\Users\\carbon\\AppData\\Local";
    const START: &str = "C:\\Users\\carbon\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu";
    const COMMON_START: &str = "C:\\ProgramData\\Microsoft\\Windows\\Start Menu";
    let x86 = isa == Isa::X86_32;
    let path = |s: &str| Some(Some(s.to_string()));
    let joined = |a: &str, b: &str| Some(Some(format!("{a}\\{b}")));
    match csidl & 0xff {
        // The desktop's folder is the user's Desktop.
        0x00 | 0x10 => joined(PROFILE, "Desktop"),
        0x02 => joined(START, "Programs"),
        0x05 => joined(PROFILE, "Documents"),
        0x06 => joined(PROFILE, "Favorites"),
        0x07 => joined(START, "Programs\\Startup"),
        0x08 => joined(ROAMING, "Microsoft\\Windows\\Recent"),
        0x09 => joined(ROAMING, "Microsoft\\Windows\\SendTo"),
        0x0b => path(START),
        0x0d => joined(PROFILE, "Music"),
        0x0e => joined(PROFILE, "Videos"),
        0x13 => joined(ROAMING, "Microsoft\\Windows\\Network Shortcuts"),
        0x14 => path("C:\\Windows\\Fonts"),
        0x15 => joined(ROAMING, "Microsoft\\Windows\\Templates"),
        0x16 => path(COMMON_START),
        0x17 => joined(COMMON_START, "Programs"),
        0x18 => joined(COMMON_START, "Programs\\Startup"),
        0x19 => path("C:\\Users\\Public\\Desktop"),
        0x1a => path(ROAMING),
        0x1b => joined(ROAMING, "Microsoft\\Windows\\Printer Shortcuts"),
        0x1c => path(LOCAL),
        0x1f => joined(PROFILE, "Favorites"),
        0x20 => joined(LOCAL, "Microsoft\\Windows\\INetCache"),
        0x21 => joined(LOCAL, "Microsoft\\Windows\\INetCookies"),
        0x22 => joined(LOCAL, "Microsoft\\Windows\\History"),
        0x23 => path("C:\\ProgramData"),
        0x24 => path("C:\\Windows"),
        0x25 => path(if x86 { "C:\\Windows\\SysWOW64" } else { "C:\\Windows\\System32" }),
        0x26 => path(if x86 { "C:\\Program Files (x86)" } else { "C:\\Program Files" }),
        0x27 => joined(PROFILE, "Pictures"),
        0x28 => path(PROFILE),
        0x29 => path("C:\\Windows\\SysWOW64"),
        0x2a => path("C:\\Program Files (x86)"),
        0x2b => path(if x86 { "C:\\Program Files (x86)\\Common Files" } else { "C:\\Program Files\\Common Files" }),
        0x2c => path("C:\\Program Files (x86)\\Common Files"),
        0x2d => path("C:\\ProgramData\\Microsoft\\Windows\\Templates"),
        0x2e => path("C:\\Users\\Public\\Documents"),
        0x2f => joined(COMMON_START, "Programs\\Administrative Tools"),
        0x30 => joined(START, "Programs\\Administrative Tools"),
        0x35 => path("C:\\Users\\Public\\Music"),
        0x36 => path("C:\\Users\\Public\\Pictures"),
        0x37 => path("C:\\Users\\Public\\Videos"),
        0x38 => path("C:\\Windows\\Resources"),
        0x3b => joined(LOCAL, "Microsoft\\Windows\\Burn\\Burn"),
        // The Internet, the Control Panel, Printers, the Recycle Bin, My Computer, the network.
        0x01 | 0x03 | 0x04 | 0x0a | 0x11 | 0x12 | 0x3d => Some(None),
        _ => None,
    }
}

/// A path's last part, as a folder's display name is.
fn display_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    match trimmed.rsplit_once('\\') {
        Some((_, last)) if !last.is_empty() => last.to_string(),
        _ => trimmed.to_string(),
    }
}

/// `SHBrowseForFolder`'s state, kept with the dialog it shows.
#[derive(Debug, Clone, Default)]
pub struct Browse {
    /// `BROWSEINFO`'s callback and its `lParam`.
    pub callback: u64,
    pub param: u64,
    /// Text arguments in UTF-16 (W) or ANSI (A).
    pub wide: bool,
}

/// The browse dialog's controls: the caller's text, its status line, the folder's path.
const BROWSE_TITLE: u64 = 0x3742;
const BROWSE_STATUS: u64 = 0x3743;
const BROWSE_PATH: u64 = 0x3744;
const BFFM_INITIALIZED: u64 = 1;
const BFFM_SELCHANGED: u64 = 2;
const BFFM_SETSTATUSTEXTA: u32 = 0x464;
const BFFM_ENABLEOK: u32 = 0x465;
const BFFM_SETSELECTIONA: u32 = 0x466;
const BFFM_SETSELECTIONW: u32 = 0x467;
const BFFM_SETSTATUSTEXTW: u32 = 0x468;
const BFFM_SETOKTEXT: u32 = 0x469;
const BFFM_SETEXPANDED: u32 = 0x46a;

impl Task {
    // ---- ntdll's task allocator ----

    /// Where one of ntdll's exports is in this process.
    pub fn guest_export(&self, name: &str) -> Option<u64> {
        use crate::images::pe::{Found, Pe, Wanted};
        let base = kernels_core::lock(&self.process.state).mapped.get("ntdll.dll").map(|m| m.0)?;
        let bytes = crate::images::guest(self.isa)?;
        let laid = Pe::parse(bytes).ok()?.layout().ok()?;
        match Pe::parse(&laid).ok()?.export(&laid, &Wanted::Name(name.to_string())).ok()?? {
            Found::Rva(rva) => Some(base + u64::from(rva)),
            Found::Forward(_) => None,
        }
    }

    /// Bytes in a block from the program's heap, as `CoTaskMemAlloc` gives one: the program frees it.
    pub fn task_alloc(&mut self, cx: &mut Cx<'_>, bytes: &[u8]) -> Result<u64, Reply> {
        let Some(alloc) = self.guest_export("AtlasCoTaskMemAlloc") else { return Ok(0) };
        let at = self.upcall(cx, alloc, &[bytes.len() as u64])?;
        if at != 0 {
            let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, bytes));
        }
        Ok(at)
    }

    pub fn task_free(&mut self, cx: &mut Cx<'_>, at: u64) -> Result<(), Reply> {
        if at != 0
            && let Some(free) = self.guest_export("AtlasCoTaskMemFree")
        {
            self.upcall(cx, free, &[at])?;
        }
        Ok(())
    }

    // ---- Special folders and item ID lists ----

    /// A folder made, and those it's in, if they aren't there.
    pub fn make_folders(&self, path: &str) {
        let mut at = String::new();
        for part in path.split('\\') {
            if !at.is_empty() {
                at.push('\\');
            }
            at.push_str(part);
            if at.len() > 2 && self.path_info(&at).is_err() {
                let _ = self.create_directory(&at);
            }
        }
    }

    /// A PIDL for a path (or, with None, the virtual folder `csidl`), in the program's heap.
    pub fn make_pidl(&mut self, cx: &mut Cx<'_>, path: Option<&str>, csidl: u32) -> Result<u64, Reply> {
        let mut item = Vec::new();
        // The desktop is the empty list.
        if path.is_some() || csidl & 0xff != 0 {
            let kind: u16 = if path.is_some() { 0 } else { 0x100 + (csidl & 0xff) as u16 };
            let text: Vec<u8> = path.unwrap_or("").encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            let size = (2 + MARK.len() + 2 + text.len()) as u16;
            item.extend(size.to_le_bytes());
            item.extend(MARK);
            item.extend(kind.to_le_bytes());
            item.extend(text);
        }
        item.extend([0, 0]);
        self.task_alloc(cx, &item)
    }

    /// What a PIDL is: Some(path) for a folder or file on the disk, None for a virtual folder or one that isn't ours.
    pub fn pidl_path(&mut self, cx: &mut Cx<'_>, pidl: u64) -> Option<String> {
        if pidl == 0 {
            return None;
        }
        let head = self.mem(cx, false, |t| user::read_vec(&t.space(), pidl, 2)).ok()?;
        let size = u16::from_le_bytes([head[0], head[1]]) as usize;
        if size == 0 {
            return special_folder(0, self.isa).flatten();
        }
        let item = self.mem(cx, false, |t| user::read_vec(&t.space(), pidl, size)).ok()?;
        if item.get(2..6) != Some(MARK) || item.get(6..8) != Some(&[0, 0]) {
            return None;
        }
        let units: Vec<u16> = item[8..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
        Some(String::from_utf16_lossy(&units))
    }

    /// Writes a path into a program's buffer of MAX_PATH characters.
    pub fn put_path_text(&mut self, cx: &mut Cx<'_>, at: u64, path: &str, wide: bool) {
        let bytes: Vec<u8> = if wide {
            path.encode_utf16().take(MAX_PATH - 1).chain([0]).flat_map(u16::to_le_bytes).collect()
        } else {
            crate::ansi::bytes(path).into_iter().take(MAX_PATH - 1).chain([0]).collect()
        };
        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), at, &bytes));
    }

    // ---- File operations ----

    /// A double-NUL-terminated list of paths.
    pub fn path_list_of(&mut self, cx: &mut Cx<'_>, mut at: u64, wide: bool) -> Vec<String> {
        let mut out = Vec::new();
        if at == 0 {
            return out;
        }
        for _ in 0..4096 {
            let s = self.text_arg(cx, at, wide);
            if s.is_empty() {
                break;
            }
            at += if wide { 2 * (s.encode_utf16().count() as u64 + 1) } else { crate::ansi::bytes(&s).len() as u64 + 1 };
            out.push(s);
        }
        out
    }

    /// What a source names: the paths a wildcard matches (but `.` and `..`, and folders if `files_only`), or the path.
    fn expand(&self, source: &str, files_only: bool) -> Result<Vec<(String, bool)>, u32> {
        let (dir, name) = source.rsplit_once('\\').unwrap_or((".", source));
        if name.contains(['*', '?']) {
            let found = self.find(source).unwrap_or_default();
            return Ok(found
                .into_iter()
                .filter(|f| f.name != "." && f.name != "..")
                .map(|f| (format!("{dir}\\{}", f.name), f.attributes & 0x10 != 0))
                .filter(|(_, folder)| !files_only || !folder)
                .collect());
        }
        let info = self.path_info(source)?;
        Ok(vec![(source.to_string(), info.attributes & 0x10 != 0)])
    }

    /// Copies a file or a whole folder.
    fn copy_tree(&self, from: &str, to: &str, folder: bool) -> Result<(), u32> {
        if !folder {
            if let Some((parent, _)) = to.rsplit_once('\\') {
                self.make_folders(parent);
            }
            return self.copy_file(from, to, false);
        }
        self.make_folders(to);
        for f in self.find(&format!("{from}\\*")).unwrap_or_default() {
            if f.name != "." && f.name != ".." {
                self.copy_tree(&format!("{from}\\{}", f.name), &format!("{to}\\{}", f.name), f.attributes & 0x10 != 0)?;
            }
        }
        Ok(())
    }

    /// Deletes a file or a whole folder.
    fn delete_tree(&self, path: &str, folder: bool) -> Result<(), u32> {
        if !folder {
            return self.delete_file(path);
        }
        for f in self.find(&format!("{path}\\*")).unwrap_or_default() {
            if f.name != "." && f.name != ".." {
                self.delete_tree(&format!("{path}\\{}", f.name), f.attributes & 0x10 != 0)?;
            }
        }
        self.remove_directory(path)
    }

    /// `SHFileOperation`'s work: FO_MOVE (1), FO_COPY (2), FO_DELETE (3) or FO_RENAME (4) of `from` to `to`. 0, or a
    /// Win32 error.
    pub fn file_operation(&self, op: u32, from: &[String], to: &[String], flags: u16) -> u32 {
        const FOF_MULTIDESTFILES: u16 = 0x1;
        const FOF_FILESONLY: u16 = 0x80;
        // A wildcard's folder is resolved, and the pattern kept as it is.
        let resolved = |p: &str| match p.rsplit_once('\\') {
            Some((dir, name)) if name.contains(['*', '?']) => self.resolve(dir).map(|(full, _)| format!("{}\\{name}", full.trim_end_matches('\\'))),
            _ => self.resolve(p).map(|(full, _)| full),
        };
        let mut sources = Vec::new();
        for (i, f) in from.iter().enumerate() {
            let Ok(full) = resolved(f) else { return 3 };
            match self.expand(&full, flags & FOF_FILESONLY != 0) {
                Ok(found) => sources.extend(found.into_iter().map(|s| (i, s, f.contains(['*', '?'])))),
                Err(e) => return e,
            }
        }
        let run = || -> Result<(), u32> {
            match op {
                3 => {
                    for (_, (path, folder), _) in &sources {
                        self.delete_tree(path, *folder)?;
                    }
                }
                1 | 2 | 4 => {
                    for (i, (path, folder), wild) in &sources {
                        let dest = if flags & FOF_MULTIDESTFILES != 0 { to.get(*i) } else { to.first() };
                        let Some(dest) = dest else { return Err(87) };
                        let dest = resolved(dest)?;
                        // Into the destination if it's a folder, or there are several, or a wildcard chose them.
                        let into = op != 4 && (self.path_info(&dest).is_ok_and(|d| d.attributes & 0x10 != 0) || *wild || (from.len() > 1 && flags & FOF_MULTIDESTFILES == 0));
                        let target = if into { format!("{dest}\\{}", display_name(path)) } else { dest };
                        match op {
                            2 => self.copy_tree(path, &target, *folder)?,
                            4 => self.move_file(path, &target, false)?,
                            _ => {
                                if let Some((parent, _)) = target.rsplit_once('\\') {
                                    self.make_folders(parent);
                                }
                                // A folder onto one that's there merges into it.
                                if self.move_file(path, &target, !*folder).is_err() {
                                    self.copy_tree(path, &target, *folder)?;
                                    self.delete_tree(path, *folder)?;
                                }
                            }
                        }
                    }
                }
                _ => return Err(87),
            }
            Ok(())
        };
        run().err().unwrap_or(0)
    }

    // ---- Running programs ----

    /// `ShellExecute`'s work: runs `file` (with `params`, in `dir`) if it's a program; the new process's handle, or a
    /// `SE_ERR_*` (2 not found, 31 no association) as Windows' ShellExecute gives them.
    pub fn shell_execute(&mut self, cx: &mut Cx<'_>, verb: &str, file: &str, params: &str, dir: Option<&str>, show: u32) -> Result<Result<u64, u64>, Reply> {
        const SE_ERR_FNF: u64 = 2;
        const SE_ERR_NOASSOC: u64 = 31;
        let verb = verb.to_ascii_lowercase();
        if !matches!(verb.as_str(), "" | "open" | "runas") || file.contains("://") || file.to_ascii_lowercase().starts_with("mailto:") {
            return Ok(Err(SE_ERR_NOASSOC));
        }
        let path = file.trim_matches('"');
        let is_program = |task: &Task, p: &str| task.read_whole(p).ok().and_then(|b| crate::images::pe::Pe::parse(&b).ok().map(|pe| !pe.is_dll)).unwrap_or(false);
        // As it's named, or with .exe, or found as CreateProcess finds a program.
        let candidates = [path.to_string(), format!("{path}.exe")];
        let found = candidates.iter().find(|p| self.path_info(p).is_ok()).cloned();
        let program = match found {
            Some(p) if self.path_info(&p).is_ok_and(|i| i.attributes & 0x10 != 0) => return Ok(Err(SE_ERR_NOASSOC)),
            Some(p) if is_program(self, &p) => p,
            Some(_) => return Ok(Err(SE_ERR_NOASSOC)),
            None if !path.contains('\\') => path.to_string(),
            None => return Ok(Err(SE_ERR_FNF)),
        };
        let line = if params.is_empty() { format!("\"{program}\"") } else { format!("\"{program}\" {params}") };
        let wide = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() };
        let (line_w, dir_w) = (wide(&line), dir.map(wide).unwrap_or_default());
        let (startup_size, flags_at, _) = super::procapi::startup(self.isa);
        let width = self.abi().width;
        let info_size = 2 * width + 8;
        // The command line, the folder, STARTUPINFO (SW_* in wShowWindow) and PROCESS_INFORMATION, on the stack.
        let line_at = 0u64;
        let dir_at = line_w.len() as u64;
        let startup_at = (dir_at + dir_w.len() as u64).next_multiple_of(8);
        let info_at = startup_at + startup_size;
        let len = (info_at + info_size) as usize;
        let build = |_: u64| {
            let mut b = vec![0u8; len];
            b[..line_w.len()].copy_from_slice(&line_w);
            b[dir_at as usize..dir_at as usize + dir_w.len()].copy_from_slice(&dir_w);
            let s = startup_at as usize;
            b[s..s + 4].copy_from_slice(&(startup_size as u32).to_le_bytes());
            let f = s + flags_at as usize;
            b[f..f + 4].copy_from_slice(&1u32.to_le_bytes()); // STARTF_USESHOWWINDOW
            b[f + 4..f + 6].copy_from_slice(&(show as u16).to_le_bytes());
            b
        };
        let has_dir = dir.is_some();
        self.with_scratch(cx, len, build, |task, cx, at| {
            let a = [0, at + line_at, 0, 0, 0, 0, 0, if has_dir { at + dir_at } else { 0 }, at + startup_at, at + info_at];
            match super::procapi::create_process_w(task, cx, &a) {
                Reply::Value(1) => {}
                Reply::Value(_) => return Ok(Err(SE_ERR_FNF)),
                other => return Err(other),
            }
            let info = task.mem(cx, false, |t| user::read_vec(&t.space(), at + info_at, info_size as usize)).unwrap_or_default();
            let abi = task.abi();
            let (process, thread) = (abi.ptr_at(&info, 0), abi.ptr_at(&info, width as usize));
            kernels_core::lock(&task.process.state).handles.close(thread);
            Ok(Ok(process))
        })
    }

    // ---- Choosing a folder ----

    /// `SHBrowseForFolder`: a modal dialog to choose a folder in; its PIDL, or 0 if it was cancelled.
    #[allow(clippy::too_many_arguments)]
    pub fn browse_for_folder(&mut self, cx: &mut Cx<'_>, owner: u64, title: &str, display: u64, callback: u64, param: u64, wide: bool, image_at: u64) -> Result<u64, Reply> {
        const WS_POPUP: u32 = 0x8000_0000;
        const WS_CAPTION: u32 = 0x00c0_0000;
        const WS_SYSMENU: u32 = 0x0008_0000;
        const WS_VISIBLE: u32 = 0x1000_0000;
        const WS_BORDER: u32 = 0x0080_0000;
        const WS_TABSTOP: u32 = 0x0001_0000;
        const DS_MODALFRAME: u32 = 0x80;
        const DS_CENTER: u32 = 0x800;
        let item = |style: u32, rect: [i32; 4], id: u64, class: u16, title: &str| Item {
            style: style | WS_VISIBLE,
            ex_style: 0,
            rect,
            id: id as u32,
            class: Id::Num(class),
            title: Id::Name(title.to_string()),
            data: None,
        };
        let t = Template {
            style: WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | dialog::DS_SETFONT,
            ex_style: 0,
            rect: [0, 0, 260, 92],
            menu: Id::None,
            class: Id::None,
            title: "Browse For Folder".to_string(),
            font: Some((8, "MS Shell Dlg".to_string())),
            items: vec![
                item(0, [7, 7, 246, 24], BROWSE_TITLE, 0x82, title),
                item(WS_BORDER | WS_TABSTOP | 0x80, [7, 34, 246, 12], BROWSE_PATH, 0x81, ""),
                item(0, [7, 50, 246, 10], BROWSE_STATUS, 0x82, ""),
                item(dialog::BS_DEFPUSHBUTTON | WS_TABSTOP, [146, 70, 50, 14], IDOK, 0x80, "OK"),
                item(WS_TABSTOP, [203, 70, 50, 14], IDCANCEL, 0x80, "Cancel"),
            ],
        };
        let browse = Browse { callback, param, wide };
        let setup = move |task: &mut Task, cx: &mut Cx<'_>, hwnd: u64| -> Result<(), Reply> {
            if let Some(d) = task.win32k().dialogs.get_mut(&hwnd) {
                d.browse = Some(browse);
            }
            // The Documents folder first, until the callback says otherwise.
            if let Some(Some(start)) = special_folder(0x05, task.isa) {
                task.set_item_text(cx, hwnd, BROWSE_PATH, &start)?;
            }
            if callback != 0 {
                task.upcall(cx, callback, &[hwnd, BFFM_INITIALIZED, 0, param])?;
            }
            Ok(())
        };
        let result = self.dialog_box_with(cx, &t, 0, 0, owner, 0, 0, setup)?;
        let tid = self.tid;
        let chosen = self.win32k().browsed.remove(&tid);
        let Some(path) = chosen.filter(|_| result == 1) else { return Ok(0) };
        if display != 0 {
            self.put_path_text(cx, display, &display_name(&path), wide);
        }
        if image_at != 0 {
            let _ = self.mem(cx, true, |t| user::write(&mut t.space(), image_at, &0u32.to_le_bytes()));
        }
        self.make_pidl(cx, Some(&path), 0)
    }

    /// Sets a dialog's control's text from win32k's own string.
    fn set_item_text(&mut self, cx: &mut Cx<'_>, dialog: u64, id: u64, text: &str) -> Result<(), Reply> {
        let item = self.dialog_item(dialog, id);
        if item == 0 {
            return Ok(());
        }
        let wide: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        self.with_scratch(cx, wide.len(), |_| wide.clone(), |task, cx, at| task.send_message(cx, item, crate::win32k::WM_SETTEXT, 0, at))?;
        Ok(())
    }

    /// What the browse dialog does with a message, before `DefDlgProc`'s own: OK takes the folder typed or given
    /// (if it's one), Cancel ends it, and the `BFFM_*` messages its callback sends set the folder, the status line,
    /// OK's text and whether it's enabled. None for what it leaves to `DefDlgProc`.
    pub(crate) fn browse_message(&mut self, cx: &mut Cx<'_>, hwnd: u64, message: u32, wparam: u64, lparam: u64) -> Result<Option<u64>, Reply> {
        let wide = self.win32k().dialogs.get(&hwnd).and_then(|d| d.browse.as_ref()).is_some_and(|b| b.wide);
        Ok(Some(match message {
            crate::win32k::WM_COMMAND if wparam & 0xffff == IDOK => {
                let path_item = self.dialog_item(hwnd, BROWSE_PATH);
                let typed = self.win32k().windows.get(&path_item).map(|w| w.text.trim().to_string()).unwrap_or_default();
                let full = self.resolve(&typed).map(|(full, _)| full);
                match full {
                    Ok(full) if self.path_info(&full).is_ok_and(|i| i.attributes & 0x10 != 0) => {
                        let tid = self.tid;
                        self.win32k().browsed.insert(tid, full);
                        self.end_dialog(cx, hwnd, 1)?;
                    }
                    // Not a folder: it stays open, saying so.
                    _ => self.set_item_text(cx, hwnd, BROWSE_STATUS, "That isn't a folder.")?,
                }
                0
            }
            crate::win32k::WM_COMMAND if wparam & 0xffff == IDCANCEL => {
                self.end_dialog(cx, hwnd, 0)?;
                0
            }
            BFFM_SETSELECTIONW | BFFM_SETSELECTIONA => {
                let path = if wparam != 0 { Some(self.text_arg(cx, lparam, message == BFFM_SETSELECTIONW)) } else { self.pidl_path(cx, lparam) };
                if let Some(path) = path {
                    self.set_item_text(cx, hwnd, BROWSE_PATH, &path)?;
                    let callback = self.win32k().dialogs.get(&hwnd).and_then(|d| d.browse.as_ref()).map(|b| (b.callback, b.param));
                    if let Some((callback, param)) = callback.filter(|c| c.0 != 0) {
                        let pidl = self.make_pidl(cx, Some(&path), 0)?;
                        self.upcall(cx, callback, &[hwnd, BFFM_SELCHANGED, pidl, param])?;
                        self.task_free(cx, pidl)?;
                    }
                }
                1
            }
            BFFM_SETSTATUSTEXTW | BFFM_SETSTATUSTEXTA => {
                let text = self.text_arg(cx, lparam, message == BFFM_SETSTATUSTEXTW);
                self.set_item_text(cx, hwnd, BROWSE_STATUS, &text)?;
                1
            }
            BFFM_ENABLEOK => {
                let ok = self.dialog_item(hwnd, IDOK);
                self.enable_window(cx, ok, lparam != 0)?;
                1
            }
            BFFM_SETOKTEXT => {
                let text = self.text_arg(cx, lparam, wide);
                self.set_item_text(cx, hwnd, IDOK, &text)?;
                1
            }
            BFFM_SETEXPANDED => 1,
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_folders() {
        assert_eq!(special_folder(0x1a, Isa::X86_64), Some(Some("C:\\Users\\carbon\\AppData\\Roaming".into())));
        assert_eq!(special_folder(0x26, Isa::X86_32), Some(Some("C:\\Program Files (x86)".into())));
        assert_eq!(special_folder(0x26, Isa::Arm64), Some(Some("C:\\Program Files".into())));
        assert_eq!(special_folder(0x8000 | 0x17, Isa::X86_64), Some(Some("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs".into())));
        assert_eq!(special_folder(0x11, Isa::X86_64), Some(None));
        assert_eq!(special_folder(0xfe, Isa::X86_64), None);
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name("C:\\Program Files\\App\\"), "App");
        assert_eq!(display_name("C:\\"), "C:");
    }
}
