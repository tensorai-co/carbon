//! Painting: `BeginPaint` and `EndPaint`, `GetDC` and `ReleaseDC`, and erasing a window's background with its class's
//! brush.

use kernels_core::thread::Cx;
use kernels_core::user;
use display::Rect;

use super::{WM_ERASEBKGND, rect_bytes};
use crate::os::Reply;
use crate::process::Task;

impl Task {
    /// `GetDC`: a DC for a window's client area (the screen's, for 0).
    pub fn get_dc(&self, hwnd: u64) -> u64 {
        let mut w = self.win32k();
        let client = match w.windows.get(&hwnd) {
            Some(win) => win.client(),
            None if hwnd == 0 => Rect::new(0, 0, super::SCREEN.0, super::SCREEN.1),
            None => return 0,
        };
        w.gdi.new_dc(hwnd, client)
    }

    pub fn release_dc(&self, hdc: u64) -> bool {
        self.win32k().gdi.dcs.remove(&hdc).is_some()
    }

    /// `BeginPaint`: a DC clipped to what needs painting, which it then doesn't; the background erased first if it
    /// needs it. Fills in the PAINTSTRUCT at `ps`.
    pub fn begin_paint(&mut self, cx: &mut Cx<'_>, hwnd: u64, ps: u64) -> Result<u64, Reply> {
        let (invalid, erase) = {
            let mut w = self.win32k();
            let Some(win) = w.windows.get_mut(&hwnd) else { return Ok(0) };
            let taken = (win.invalid, win.erase);
            win.invalid = Rect::default();
            win.erase = false;
            taken
        };
        let hdc = self.win32k().gdi.new_dc(hwnd, invalid);
        let still_to_erase = erase && self.send_message(cx, hwnd, WM_ERASEBKGND, hdc, 0)? == 0;
        // PAINTSTRUCT: hdc (pointer-sized), fErase, rcPaint, fRestore, fIncUpdate, rgbReserved; 72 bytes, or 64.
        let abi = self.abi();
        let w = abi.width as usize;
        let mut b = vec![0u8; if w == 8 { 72 } else { 64 }];
        b[0..w].copy_from_slice(&abi.ptr(hdc));
        b[w..w + 4].copy_from_slice(&u32::from(still_to_erase).to_le_bytes());
        b[w + 4..w + 20].copy_from_slice(&rect_bytes(&invalid));
        let _ = self.mem(cx, true, |t| user::write(&mut t.space(), ps, &b));
        Ok(hdc)
    }

    pub fn end_paint(&mut self, cx: &mut Cx<'_>, ps: u64) -> bool {
        let width = self.abi().width;
        let Ok(hdc) = self.mem(cx, false, |t| user::read_ptr(&t.space(), ps, width)) else { return false };
        self.release_dc(hdc)
    }

    /// WM_ERASEBKGND's default: fills what the DC may draw with the window's class's background. False if it has none.
    pub fn erase_background(&self, hwnd: u64, hdc: u64) -> bool {
        let mut w = self.win32k();
        let Some(class) = w.windows.get(&hwnd).map(|win| win.class.clone()) else { return false };
        let Some(class) = w.class_by_name(&class).cloned() else { return false };
        if class.background == 0 {
            return false;
        }
        let color = self.background_color(&w, &class);
        w.draw(hdc, |s, (dx, dy), clip, dc| s.fill(&dc.clip.offset(dx, dy), color, clip))
    }

    fn background_color(&self, w: &super::Win32k, class: &super::Class) -> u32 {
        match class.background {
            b if b < 0x100 => super::gdi::sys_color(b as u32 - 1),
            b => w.gdi.brush_color(b).unwrap_or(0xffffff),
        }
    }
}
