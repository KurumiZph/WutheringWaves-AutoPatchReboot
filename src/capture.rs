//! Finding the game window and grabbing its client area with PrintWindow.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetWindowDC, MonitorFromWindow,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible,
};

use crate::game;
use crate::image_ops::Frame;
use crate::log::log;

pub struct GameWindow {
    pub hwnd: HWND,
    pub title: String,
    pub width: i32,
    pub height: i32,
    /// Minimised windows have no client area to capture; the monitor pauses scanning until restored.
    /// (A window merely hidden behind other windows is NOT minimised and is captured normally.)
    pub minimized: bool,
}

// ---------------- find game windows ----------------

struct EnumCtx {
    pids: Vec<u32>,
    out: Vec<GameWindow>,
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return TRUE;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let minimized = IsIconic(hwnd).as_bool();
    if !minimized && (width < 500 || height < 300) {
        return TRUE;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if !ctx.pids.contains(&pid) {
        return TRUE;
    }
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
    let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    ctx.out.push(GameWindow { hwnd, title, width, height, minimized });
    TRUE
}

/// Visible top-level windows that belong to a WuWa process.
pub fn find_game_windows() -> Vec<GameWindow> {
    let pids = game::game_pids();
    if pids.is_empty() {
        return Vec::new();
    }
    let mut ctx = EnumCtx { pids, out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut EnumCtx as isize));
    }
    ctx.out
}

pub fn find_best_game_window() -> Option<GameWindow> {
    let mut windows = find_game_windows();
    // restored windows first (largest first), minimised ones only if nothing else exists
    windows.sort_by_key(|w| std::cmp::Reverse((!w.minimized, w.width as i64 * w.height as i64)));
    windows.into_iter().next()
}

/// The monitor this window is on (an HMONITOR as a plain number; 0 if unknown).
pub fn monitor_of(hwnd: HWND) -> isize {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as isize }
}

// ---------------- capture ----------------

/// Releases the GDI objects even on early returns.
struct Gdi {
    hwnd: HWND,
    window_dc: HDC,
    mem_dc: HDC,
    bitmap: HBITMAP,
}

impl Drop for Gdi {
    fn drop(&mut self) {
        unsafe {
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            if !self.mem_dc.is_invalid() {
                let _ = DeleteDC(self.mem_dc);
            }
            if !self.window_dc.is_invalid() {
                ReleaseDC(self.hwnd, self.window_dc);
            }
        }
    }
}

/// Full client area in physical pixels (the manifest makes the process per-monitor DPI aware).
pub fn capture_window(window: &GameWindow) -> Option<Frame> {
    let hwnd = window.hwnd;
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return None;
        }
        let mut rc = RECT::default();
        if let Err(e) = GetClientRect(hwnd, &mut rc) {
            log(format!("[CAPTURE] Could not get client size: {e}"));
            return None;
        }
        let (width, height) = (rc.right - rc.left, rc.bottom - rc.top);
        if width <= 0 || height <= 0 {
            log(format!("[CAPTURE] Invalid client size: {width}x{height}"));
            return None;
        }

        let mut gdi = Gdi { hwnd, window_dc: HDC::default(), mem_dc: HDC::default(), bitmap: HBITMAP::default() };
        gdi.window_dc = GetWindowDC(hwnd);
        if gdi.window_dc.is_invalid() {
            return None;
        }
        gdi.mem_dc = CreateCompatibleDC(gdi.window_dc);
        if gdi.mem_dc.is_invalid() {
            return None;
        }
        gdi.bitmap = CreateCompatibleBitmap(gdi.window_dc, width, height);
        if gdi.bitmap.is_invalid() {
            return None;
        }

        let old = SelectObject(gdi.mem_dc, HGDIOBJ(gdi.bitmap.0));
        // PW_RENDERFULLCONTENT (2): needed for DirectX / composited windows
        let printed = PrintWindow(hwnd, gdi.mem_dc, PRINT_WINDOW_FLAGS(2));
        // a bitmap must not be selected into a DC while GetDIBits reads it
        SelectObject(gdi.mem_dc, old);
        if !printed.as_bool() {
            log("[CAPTURE] PrintWindow failed.");
            return None;
        }

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height; // negative = top-down rows
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB

        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        let lines = GetDIBits(
            gdi.mem_dc,
            gdi.bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if lines == 0 {
            log("[CAPTURE] GetDIBits returned no scan lines.");
            return None;
        }
        // the 4th byte is unused by GDI: make it opaque so the OCR engine doesn't see transparency
        for px in bgra.chunks_exact_mut(4) {
            px[3] = 255;
        }
        if crate::settings::general().save_screenshots() && lines != height {
            log(format!("[CAPTURE] Size mismatch: requested {height} lines, got {lines}"));
        }
        Some(Frame { width, height, bgra })
    }
}
