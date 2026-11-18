//! Message boxes, the tray icon + menu and the message loop. The window itself is in appwin.rs.
//!
//! The tray menu is just: status line / Show UI / Quit. Menu clicks arrive through a callback that posts a
//! message to the main window, so nothing here polls or runs timers.

use std::cell::RefCell;

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, DispatchMessageW, GetMessageW, GetSystemMetrics, MessageBoxW, TranslateMessage,
    LR_DEFAULTCOLOR, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, MSG,
    SM_CXSMICON,
};

use crate::{appwin, state};

// ---------------- message boxes ----------------

pub fn message_box(text: &str, caption: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(caption), style) }
}

pub fn info(text: &str) {
    message_box(text, "WuWa APR", MB_OK | MB_ICONINFORMATION);
}

pub fn error(text: &str) {
    message_box(text, "WuWa APR", MB_OK | MB_ICONERROR);
}

// ---------------- tray ----------------

struct TrayState {
    tray: TrayIcon,
    status_item: MenuItem,
}

thread_local! {
    static TRAY: RefCell<Option<TrayState>> = const { RefCell::new(None) };
}

/// Shows the current status in the tray menu's first line and in the icon's tooltip.
pub fn set_tray_status(text: &str) {
    TRAY.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut() {
            if let Some(t) = guard.as_mut() {
                t.status_item.set_text(text);
                let tip: String = format!("WuWa APR: {text}").chars().take(120).collect();
                let _ = t.tray.set_tooltip(Some(tip));
            }
        }
    });
}

fn distance_to_segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Fallback icon: blue disc with a white "W".
fn generated_icon() -> Icon {
    const N: u32 = 64;
    let strokes = [
        ((16.0, 20.0), (25.0, 46.0)),
        ((25.0, 46.0), (32.0, 30.0)),
        ((32.0, 30.0), (39.0, 46.0)),
        ((39.0, 46.0), (48.0, 20.0)),
    ];
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = ((fx - 32.0).powi(2) + (fy - 32.0).powi(2)).sqrt() <= 30.0;
            if !inside {
                continue;
            }
            let on_w = strokes.iter().any(|&(a, b)| distance_to_segment(fx, fy, a, b) <= 2.6);
            let px: [u8; 4] = if on_w { [255, 255, 255, 255] } else { [60, 110, 220, 255] };
            let i = ((y * N + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, N, N).expect("tray icon")
}

/// The tray icon file, compiled into the program (it is NOT a resource of the exe, so the exe's own icon
/// stays assets/icon.ico). Put it at assets/tray.ico.
static TRAY_ICO: &[u8] = include_bytes!("../assets/tray.ico");

/// Picks the image of an .ico file that best fits `want` px (the smallest one that is big enough,
/// else the biggest) and turns it into an icon handle (HICON as a number; the caller owns it).
fn hicon_from_ico(data: &[u8], want: u32) -> Option<isize> {
    let u16le = |at: usize| -> Option<u32> { Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]) as u32) };
    let u32le = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes([*data.get(at)?, *data.get(at + 1)?, *data.get(at + 2)?, *data.get(at + 3)?]))
    };
    if u16le(2)? != 1 {
        return None; // not an icon file
    }
    let count = u16le(4)? as usize;
    let mut best: Option<(u32, usize, usize, u32, u32)> = None; // (score, offset, size, w, h)
    for i in 0..count {
        let e = 6 + i * 16;
        let w = match *data.get(e)? {
            0 => 256,
            v => v as u32,
        };
        let h = match *data.get(e + 1)? {
            0 => 256,
            v => v as u32,
        };
        let size = u32le(e + 8)? as usize;
        let off = u32le(e + 12)? as usize;
        if off.checked_add(size)? > data.len() {
            continue;
        }
        let score = if w >= want { w - want } else { 10_000 + (want - w) };
        if best.map_or(true, |b| score < b.0) {
            best = Some((score, off, size, w, h));
        }
    }
    let (_, off, size, w, h) = best?;
    unsafe {
        let hicon =
            CreateIconFromResourceEx(&data[off..off + size], true, 0x0003_0000, w as i32, h as i32, LR_DEFAULTCOLOR).ok()?;
        Some(hicon.0 as isize)
    }
}

/// A fresh HICON of tray.ico at about `want` px. Used for the tray and for the window's own icon
/// (Alt+Tab / taskbar). Every caller gets its own handle.
pub fn tray_hicon(want: u32) -> Option<isize> {
    hicon_from_ico(TRAY_ICO, want)
}

/// tray.ico (built in) -> else the exe's own icon (resource #1) -> else the generated one.
fn tray_icon_image() -> Icon {
    let want = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    tray_hicon(want)
        .map(Icon::from_handle)
        .or_else(|| Icon::from_resource(1, None).ok())
        .unwrap_or_else(generated_icon)
}

/// Builds the main window (hidden) and the tray icon, then runs the message loop until the app quits.
pub fn run() {
    let _main_window = appwin::create();

    let menu = Menu::new();
    let status_item = MenuItem::new("Starting...", false, None);
    let show_item = MenuItem::with_id(MenuId::new("apr_show"), "Show UI", true, None);
    let quit_item = MenuItem::with_id(MenuId::new("apr_quit"), "Quit", true, None);
    let _ = menu.append(&status_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&show_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit_item);

    // Menu clicks -> a message to the main window (no polling anywhere).
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        if event.id == MenuId::new("apr_show") {
            state::post_ui(state::WM_APP_SHOW);
        } else if event.id == MenuId::new("apr_quit") {
            state::post_ui(state::WM_APP_QUIT);
        }
    }));

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("WuWa APR: Starting...")
        .with_icon(tray_icon_image())
        .build()
        .expect("create tray icon");
    TRAY.with(|cell| *cell.borrow_mut() = Some(TrayState { tray, status_item }));

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // remove the tray icon right away (no ghost icon left behind)
    TRAY.with(|cell| {
        cell.borrow_mut().take();
    });
}
