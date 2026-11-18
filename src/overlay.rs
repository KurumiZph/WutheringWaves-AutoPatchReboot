//! The notification box: a small rounded, click-through box ("[spinner] Patch detected - restarting game")
//! that appears on the monitor the game is on and disappears when the game window is back.
//!
//! It never takes focus and ignores the mouse (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT), stays on top, and only
//! exists while something is shown: the spinner timer (80 ms) runs only then. Other threads ask for it
//! with `request_show` / `request_hide`; the UI thread does the actual window work.
//! (Exclusive-fullscreen games hide any window on top of them; windowed / borderless is fine.)

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, DrawTextW, EndPaint,
    GetDC, GetMonitorInfoW, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, DT_CALCRECT, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HGDIOBJ, HMONITOR, MONITORINFO, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, KillTimer, RegisterClassW, SetLayeredWindowAttributes,
    SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, LWA_ALPHA, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SW_HIDE, WM_DESTROY, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::gfx::*;
use crate::settings::{self, OverlaySettings};
use crate::state::{lock, post_ui, WM_APP_OVERLAY_HIDE, WM_APP_OVERLAY_SHOW};

pub const TEXT_PATCH: &str = "Patch detected - restarting game";

const SPIN_TIMER: usize = 1;
const HIDE_TIMER: usize = 2;
const SPIN_MS: u32 = 80;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// something is in progress: spinning ring
    Progress,
    /// something went wrong: red "!"
    Error,
}

struct Request {
    text: String,
    kind: Kind,
    monitor: isize,
    auto_hide_ms: u32,
    force: bool,
}

static REQUEST: Mutex<Option<Request>> = Mutex::new(None);

/// true while a real (no auto-hide) notification is up, so a settings preview never hides it
static REAL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn real_active() -> bool {
    REAL_ACTIVE.load(Ordering::Relaxed)
}

/// Shows the box (any thread). `monitor` = HMONITOR as a number (0 = primary). `auto_hide_ms` 0 = until hidden.
/// `force` shows it even if the notification is switched off in Settings (used by "Preview").
pub fn request_show(text: &str, kind: Kind, monitor: isize, auto_hide_ms: u32, force: bool) {
    REAL_ACTIVE.store(auto_hide_ms == 0, Ordering::Relaxed);
    *lock(&REQUEST) = Some(Request { text: text.to_string(), kind, monitor, auto_hide_ms, force });
    post_ui(WM_APP_OVERLAY_SHOW);
}

/// Hides the box (any thread).
pub fn request_hide() {
    REAL_ACTIVE.store(false, Ordering::Relaxed);
    post_ui(WM_APP_OVERLAY_HIDE);
}

// ---------------- UI-thread part ----------------
struct Ov {
    hwnd: HWND,
    text: String,
    kind: Kind,
    phase: u32,
    scale: f32,
}

thread_local! {
    static OV: RefCell<Option<Ov>> = const { RefCell::new(None) };
}

unsafe fn ensure_window() -> HWND {
    if let Some(h) = OV.with(|c| c.borrow().as_ref().map(|o| o.hwnd)) {
        return h;
    }
    let instance = GetModuleHandleW(None).expect("GetModuleHandleW");
    let class_name = w!("WuWaAprOverlay");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class_name, ..Default::default() };
    RegisterClassW(&wc); // a second registration (after a destroy) simply fails harmlessly
    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
        class_name,
        w!("WuWa APR notification"),
        WS_POPUP,
        0,
        0,
        10,
        10,
        None,
        None,
        instance,
        None,
    )
    .expect("create overlay window");
    OV.with(|c| *c.borrow_mut() = Some(Ov { hwnd, text: String::new(), kind: Kind::Progress, phase: 0, scale: 1.0 }));
    hwnd
}

/// Width of `text` in the notification font.
unsafe fn text_width(text: &str, scale: f32) -> i32 {
    let font = make_font(px(15.0, scale), 600, "Segoe UI");
    let screen = GetDC(None);
    let old = SelectObject(screen, HGDIOBJ(font.0));
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rc = rect(0, 0, 0, 0);
    DrawTextW(screen, &mut wide, &mut rc, DT_CALCRECT | DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(screen, old);
    ReleaseDC(None, screen);
    delete_font(font);
    rc.right
}

/// Work area (without the taskbar) of the monitor, or the primary screen if unknown.
unsafe fn work_area(monitor: isize) -> RECT {
    if monitor != 0 {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(HMONITOR(monitor as _), &mut mi).as_bool() {
            return mi.rcWork;
        }
    }
    rect(0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
}

/// (x, y) of a box of size (w, h) inside `area` for the configured position.
fn place(area: &RECT, w: i32, h: i32, cfg: &OverlaySettings, margin: i32) -> (i32, i32) {
    let x = match cfg.h {
        0 => area.left + margin,
        1 => area.left + (area.right - area.left - w) / 2,
        _ => area.right - w - margin,
    };
    let y = match cfg.v {
        0 => area.top + margin,
        1 => area.top + (area.bottom - area.top - h) / 2,
        _ => area.bottom - h - margin,
    };
    (x, y)
}

/// UI thread: handles WM_APP_OVERLAY_SHOW.
pub fn on_show() {
    let Some(req) = lock(&REQUEST).take() else { return };
    let cfg = settings::general().overlay();
    if !cfg.on && !req.force {
        return;
    }
    unsafe {
        let hwnd = ensure_window();
        let scale = GetDpiForSystem().max(96) as f32 / 96.0 * cfg.size_pct as f32 / 100.0;
        let (pad, icon, gap) = (px(16.0, scale), px(26.0, scale), px(12.0, scale));
        let width = pad + icon + gap + text_width(&req.text, scale) + pad;
        let height = px(56.0, scale);
        let area = work_area(req.monitor);
        let (x, y) = place(&area, width, height, &cfg, px(cfg.margin as f32, scale / (cfg.size_pct as f32 / 100.0)));

        OV.with(|c| {
            if let Some(o) = c.borrow_mut().as_mut() {
                o.text = req.text.clone();
                o.kind = req.kind;
                o.phase = 0;
                o.scale = scale;
            }
        });
        let alpha = (cfg.opacity_pct.clamp(20, 100) * 255 / 100) as u8;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        if !round_corners(hwnd) {
            apply_region(hwnd, px(12.0, scale));
        }
        if req.kind == Kind::Progress {
            SetTimer(hwnd, SPIN_TIMER, SPIN_MS, None);
        } else {
            let _ = KillTimer(hwnd, SPIN_TIMER);
        }
        if req.auto_hide_ms > 0 {
            SetTimer(hwnd, HIDE_TIMER, req.auto_hide_ms, None);
        } else {
            let _ = KillTimer(hwnd, HIDE_TIMER);
        }
        let _ = InvalidateRect(hwnd, None, false);
    }
}

/// UI thread: handles WM_APP_OVERLAY_HIDE.
pub fn on_hide() {
    if let Some(hwnd) = OV.with(|c| c.borrow().as_ref().map(|o| o.hwnd)) {
        unsafe { hide(hwnd) };
    }
}

unsafe fn hide(hwnd: HWND) {
    let _ = KillTimer(hwnd, SPIN_TIMER);
    let _ = KillTimer(hwnd, HIDE_TIMER);
    let _ = ShowWindow(hwnd, SW_HIDE);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let snapshot = OV.with(|c| c.borrow().as_ref().map(|o| (o.text.clone(), o.kind, o.phase, o.scale)));
    if let (true, Some((text, kind, phase, scale))) = (w > 0 && h > 0, snapshot) {
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        SetBkMode(mem, TRANSPARENT);

        fill(mem, &rect(0, 0, w, h), C_BAR);
        card(mem, &rect(0, 0, w, h), (30, 30, 34), C_BORDER, px(12.0, scale));

        let (pad, icon, gap) = (px(16.0, scale), px(26.0, scale), px(12.0, scale));
        let (cx, cy) = (pad + icon / 2, h / 2);
        match kind {
            Kind::Progress => {
                // 8 dots on a ring; the "head" moves one step per tick and the tail fades out
                let ring = icon as f32 * 0.38;
                for i in 0..8u32 {
                    let age = (phase + 8 - i) % 8; // 0 = head
                    let t = 1.0 - age as f32 / 8.0;
                    let ang = i as f32 * std::f32::consts::TAU / 8.0 - std::f32::consts::FRAC_PI_2;
                    let (dx, dy) = ((cx as f32 + ring * ang.cos()) as i32, (cy as f32 + ring * ang.sin()) as i32);
                    let r = (px(2.0, scale) as f32 + t * px(1.6, scale) as f32).round() as i32;
                    fill_round(mem, &rect(dx - r, dy - r, dx + r, dy + r), lerp((70, 74, 86), C_ACCENT, t), r);
                }
            }
            Kind::Error => {
                let r = icon / 2 - px(2.0, scale);
                fill_round(mem, &rect(cx - r, cy - r, cx + r, cy + r), C_ERR, r);
                line(mem, (cx, cy - r / 2), (cx, cy + px(1.0, scale)), (30, 30, 34), px(2.5, scale).max(2));
                fill_round(mem, &rect(cx - 1, cy + r / 2 - 1, cx + 2, cy + r / 2 + 2), (30, 30, 34), 1);
            }
        }

        let font = make_font(px(15.0, scale), 600, "Segoe UI");
        let prev = SelectObject(mem, HGDIOBJ(font.0));
        let mut tr = rect(pad + icon + gap, 0, w - pad, h);
        put_text(mem, &text, &mut tr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(mem, prev);
        delete_font(font);

        let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
    }
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // the mouse goes straight through to whatever is underneath
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
        WM_TIMER => {
            match wparam.0 {
                SPIN_TIMER => {
                    OV.with(|c| {
                        if let Some(o) = c.borrow_mut().as_mut() {
                            o.phase = (o.phase + 1) % 8;
                        }
                    });
                    let _ = InvalidateRect(hwnd, None, false);
                }
                HIDE_TIMER => hide(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            OV.with(|c| *c.borrow_mut() = None);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
