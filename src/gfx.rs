//! Small GDI helpers shared by the live-log window and the settings window.
//!
//! Rounded shapes and lines drawn into the main window's back buffer are anti-aliased by hand
//! (smooth edges instead of GDI's jagged 1-bit regions). The back buffer is a DIB section, so its pixels
//! can be blended directly. Any other device context (e.g. the overlay) keeps using plain GDI.

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicPtr, Ordering};

use windows::Win32::Foundation::{COLORREF, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, CreatePen, CreateRoundRectRgn,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, FillRect, FillRgn, GdiFlush, LineTo, MoveToEx, SelectObject,
    SetTextColor, SetWindowRgn, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_TOP, HDC, HFONT, HGDIOBJ, LOGFONTW,
    PS_SOLID,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

pub type Rgb = (u8, u8, u8);

// ---------------- shared dark palette (Fluent-style: soft greys, one blue accent) ----------------
pub const C_BG: Rgb = (28, 28, 31); // page background
pub const C_BAR: Rgb = (23, 23, 26); // header / footer
pub const C_PANEL: Rgb = (40, 40, 44); // cards
pub const C_LOG_BG: Rgb = (21, 21, 24); // the log "console"
pub const C_BORDER: Rgb = (58, 58, 64);
pub const C_TEXT: Rgb = (240, 240, 244);
pub const C_DIM: Rgb = (148, 148, 156);
pub const C_BTN_HOT: Rgb = (56, 56, 62);
pub const C_CLOSE_HOT: Rgb = (229, 72, 77);
pub const C_PILL: Rgb = (52, 52, 58);
pub const C_PILL_HOT: Rgb = (68, 68, 76);
pub const C_THUMB: Rgb = (96, 96, 104);
pub const C_WARN: Rgb = (255, 180, 84);
pub const C_ACCENT: Rgb = (76, 194, 255);
pub const C_ERR: Rgb = (255, 107, 107);
pub const C_OK: Rgb = (126, 231, 135);
pub const C_DONE: Rgb = (121, 192, 255);

/// Corner radius of the rounded windows (96-dpi px).
pub const CORNER: f32 = 10.0;

// ---------------- tiny helpers ----------------
pub fn rgb(c: Rgb) -> COLORREF {
    COLORREF((c.2 as u32) << 16 | (c.1 as u32) << 8 | c.0 as u32)
}

pub fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

pub fn px(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

pub fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT { left: l, top: t, right: r, bottom: b }
}

pub fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub fn lo_signed(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}

pub fn hi_signed(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}

// ---------------- anti-aliasing (pure Rust, on the back buffer's pixels) ----------------
static AA_DC: AtomicIsize = AtomicIsize::new(0);
static AA_BITS: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static AA_W: AtomicI32 = AtomicI32::new(0);
static AA_H: AtomicI32 = AtomicI32::new(0);

/// (pixels, width, height) when `hdc` is the main window's back buffer, else None (-> plain GDI).
fn aa_target(hdc: HDC) -> Option<(*mut u8, i32, i32)> {
    if hdc.0 as isize != AA_DC.load(Ordering::Relaxed) {
        return None;
    }
    let p = AA_BITS.load(Ordering::Relaxed);
    if p.is_null() {
        return None;
    }
    Some((p, AA_W.load(Ordering::Relaxed), AA_H.load(Ordering::Relaxed)))
}

/// Mixes colour `c` into one BGRA pixel with the given coverage (0..1).
unsafe fn blend_px(p: *mut u8, c: Rgb, cov: f32) {
    if cov >= 0.999 {
        *p = c.2;
        *p.add(1) = c.1;
        *p.add(2) = c.0;
    } else if cov > 0.001 {
        let mix = |dst: u8, src: u8| -> u8 { (dst as f32 + (src as f32 - dst as f32) * cov).round() as u8 };
        *p = mix(*p, c.2);
        *p.add(1) = mix(*p.add(1), c.1);
        *p.add(2) = mix(*p.add(2), c.0);
    }
}

/// Rounded rectangle covering x in [l, r), y in [t, b), smooth edges.
unsafe fn aa_round_rect(buf: (*mut u8, i32, i32), l: i32, t: i32, r: i32, b: i32, radius: f32, c: Rgb) {
    let (bits, bw, bh) = buf;
    let _ = GdiFlush(); // finish pending GDI drawing before touching the pixels ourselves
    let (lf, tf, rf, bf) = (l as f32, t as f32, r as f32, b as f32);
    let rad = radius.min((rf - lf) / 2.0).min((bf - tf) / 2.0).max(0.0);
    let (x0, x1, y0, y1) = (l.max(0), r.min(bw), t.max(0), b.min(bh));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for y in y0..y1 {
        let cy = y as f32 + 0.5;
        let dy = if cy < tf + rad {
            tf + rad - cy
        } else if cy > bf - rad {
            cy - (bf - rad)
        } else {
            0.0
        };
        let row = bits.add(y as usize * bw as usize * 4);
        if dy <= 0.0 {
            // straight part of the shape: solid
            let mut p = row.add(x0 as usize * 4);
            for _ in x0..x1 {
                *p = c.2;
                *p.add(1) = c.1;
                *p.add(2) = c.0;
                p = p.add(4);
            }
        } else {
            // rows that touch the rounded corners: per-pixel coverage
            for x in x0..x1 {
                let cx = x as f32 + 0.5;
                let dx = if cx < lf + rad {
                    lf + rad - cx
                } else if cx > rf - rad {
                    cx - (rf - rad)
                } else {
                    0.0
                };
                let cov = (rad - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                blend_px(row.add(x as usize * 4), c, cov);
            }
        }
    }
}

/// Smooth line with round ends. Coordinates are GDI pixel positions (the line runs through pixel centres).
unsafe fn aa_line(buf: (*mut u8, i32, i32), a: (i32, i32), b: (i32, i32), c: Rgb, width: f32) {
    let (bits, bw, bh) = buf;
    let _ = GdiFlush();
    let (ax, ay, bx, by) = (a.0 as f32 + 0.5, a.1 as f32 + 0.5, b.0 as f32 + 0.5, b.1 as f32 + 0.5);
    let half = (width / 2.0).max(0.5);
    let pad = half + 1.0;
    let x0 = ((ax.min(bx) - pad).floor() as i32).max(0);
    let x1 = ((ax.max(bx) + pad).ceil() as i32).min(bw);
    let y0 = ((ay.min(by) - pad).floor() as i32).max(0);
    let y1 = ((ay.max(by) + pad).ceil() as i32).min(bh);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    for y in y0..y1 {
        for x in x0..x1 {
            let (pxc, pyc) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if len2 > 0.0 { (((pxc - ax) * dx + (pyc - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (qx, qy) = (ax + t * dx, ay + t * dy);
            let d = ((pxc - qx) * (pxc - qx) + (pyc - qy) * (pyc - qy)).sqrt();
            let cov = (half - d + 0.5).clamp(0.0, 1.0);
            blend_px(bits.add((y as usize * bw as usize + x as usize) * 4), c, cov);
        }
    }
}

// ---------------- drawing ----------------
pub unsafe fn fill(hdc: HDC, rc: &RECT, c: Rgb) {
    let brush = CreateSolidBrush(rgb(c));
    FillRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

pub unsafe fn fill_round(hdc: HDC, rc: &RECT, c: Rgb, radius: i32) {
    if let Some(buf) = aa_target(hdc) {
        // same footprint as the GDI region below (it includes the right / bottom pixel row)
        aa_round_rect(buf, rc.left, rc.top, rc.right + 1, rc.bottom + 1, radius as f32, c);
        return;
    }
    let rgn = CreateRoundRectRgn(rc.left, rc.top, rc.right + 1, rc.bottom + 1, radius * 2, radius * 2);
    let brush = CreateSolidBrush(rgb(c));
    let _ = FillRgn(hdc, rgn, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
    let _ = DeleteObject(HGDIOBJ(rgn.0));
}

/// Rounded card: 1 px border + fill.
pub unsafe fn card(hdc: HDC, rc: &RECT, bg: Rgb, border: Rgb, radius: i32) {
    fill_round(hdc, rc, border, radius);
    fill_round(hdc, &rect(rc.left + 1, rc.top + 1, rc.right - 1, rc.bottom - 1), bg, (radius - 1).max(1));
}

/// Rounded-rectangle region (caller deletes it) - used to clip drawing to a rounded card.
pub unsafe fn round_region(rc: &RECT, radius: i32) -> windows::Win32::Graphics::Gdi::HRGN {
    CreateRoundRectRgn(rc.left, rc.top, rc.right + 1, rc.bottom + 1, radius * 2, radius * 2)
}

/// 1 px rounded outline.
pub unsafe fn frame_round(hdc: HDC, rc: &RECT, c: Rgb, radius: i32) {
    let rgn = round_region(rc, radius);
    let brush = CreateSolidBrush(rgb(c));
    let _ = windows::Win32::Graphics::Gdi::FrameRgn(hdc, rgn, brush, 1, 1);
    let _ = DeleteObject(HGDIOBJ(brush.0));
    let _ = DeleteObject(HGDIOBJ(rgn.0));
}

pub unsafe fn put_text(hdc: HDC, text: &str, rc: &mut RECT, c: Rgb, flags: DRAW_TEXT_FORMAT) {
    if text.is_empty() {
        return;
    }
    SetTextColor(hdc, rgb(c));
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut wide, rc, flags);
}

pub unsafe fn line(hdc: HDC, a: (i32, i32), b: (i32, i32), c: Rgb, width: i32) {
    if let Some(buf) = aa_target(hdc) {
        aa_line(buf, a, b, c, width as f32);
        return;
    }
    let pen = CreatePen(PS_SOLID, width, rgb(c));
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let _ = MoveToEx(hdc, a.0, a.1, None);
    let _ = LineTo(hdc, b.0, b.1);
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

// ---------------- fonts ----------------
pub unsafe fn make_font(height_px: i32, weight: i32, face: &str) -> HFONT {
    let mut lf = LOGFONTW::default();
    lf.lfHeight = -height_px;
    lf.lfWeight = weight;
    let wide: Vec<u16> = face.encode_utf16().collect();
    let n = wide.len().min(lf.lfFaceName.len() - 1);
    lf.lfFaceName[..n].copy_from_slice(&wide[..n]);
    CreateFontIndirectW(&lf)
}

pub unsafe fn delete_font(f: HFONT) {
    if !f.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(f.0));
    }
}

/// Width of one character of a monospaced font.
pub unsafe fn char_width(hdc: HDC, font: HFONT) -> i32 {
    let old = SelectObject(hdc, HGDIOBJ(font.0));
    let mut wide: Vec<u16> = "0000000000".encode_utf16().collect();
    let mut rc = rect(0, 0, 0, 0);
    DrawTextW(hdc, &mut wide, &mut rc, DT_CALCRECT | DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(hdc, old);
    (rc.right / 10).max(1)
}

// ---------------- rounded windows ----------------
/// Native rounded corners (Windows 11). Returns false if unsupported (Windows 10).
pub unsafe fn round_corners(hwnd: HWND) -> bool {
    let preference: i32 = 2; // DWMWCP_ROUND
    DwmSetWindowAttribute(
        hwnd,
        DWMWINDOWATTRIBUTE(33), // DWMWA_WINDOW_CORNER_PREFERENCE
        &preference as *const i32 as *const core::ffi::c_void,
        4,
    )
    .is_ok()
}

/// Main window: ask Windows 11 NOT to round the window's outer corners (Windows 10 never rounds them).
pub unsafe fn square_corners(hwnd: HWND) -> bool {
    let preference: i32 = 1; // DWMWCP_DONOTROUND
    DwmSetWindowAttribute(
        hwnd,
        DWMWINDOWATTRIBUTE(33), // DWMWA_WINDOW_CORNER_PREFERENCE
        &preference as *const i32 as *const core::ffi::c_void,
        4,
    )
    .is_ok()
}

/// Windows 10 fallback: clip the window to a rounded region.
pub unsafe fn apply_region(hwnd: HWND, radius_px: i32) {
    let mut rc = RECT::default();
    if GetWindowRect(hwnd, &mut rc).is_err() {
        return;
    }
    let d = radius_px * 2;
    let rgn = CreateRoundRectRgn(0, 0, rc.right - rc.left + 1, rc.bottom - rc.top + 1, d, d);
    SetWindowRgn(hwnd, rgn, true); // the system owns the region from here on
}

// ---------------- persistent back buffer ----------------
/// Off-screen bitmap kept between paints: scrolling/hovering redraws only what changed into it,
/// and WM_PAINT just copies the invalid rectangle to the screen.
pub struct BackBuffer {
    pub dc: HDC,
    bmp: windows::Win32::Graphics::Gdi::HBITMAP,
    old: HGDIOBJ,
    pub w: i32,
    pub h: i32,
}

impl BackBuffer {
    pub unsafe fn new(like: HDC, w: i32, h: i32) -> BackBuffer {
        let dc = CreateCompatibleDC(like);

        // A DIB section: a normal bitmap whose pixels we can reach directly (needed for anti-aliasing).
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h; // top-down rows
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(like, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                bits = std::ptr::null_mut(); // no direct pixels: everything falls back to plain GDI shapes
                CreateCompatibleBitmap(like, w, h)
            }
        };
        let old = SelectObject(dc, HGDIOBJ(bmp.0));
        AA_BITS.store(bits as *mut u8, Ordering::Relaxed);
        AA_W.store(w, Ordering::Relaxed);
        AA_H.store(h, Ordering::Relaxed);
        AA_DC.store(if bits.is_null() { 0 } else { dc.0 as isize }, Ordering::Relaxed);
        BackBuffer { dc, bmp, old, w, h }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            if AA_DC.load(Ordering::Relaxed) == self.dc.0 as isize {
                AA_DC.store(0, Ordering::Relaxed); // the pixels are about to disappear
            }
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
