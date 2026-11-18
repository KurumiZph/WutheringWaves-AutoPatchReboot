//! The main window: "WuWa APR" with a header, a tab sidebar (Live Logs / Settings / Misc), the active tab
//! on the right and a footer (status, version, "Check for updates").
//!
//! Borderless, dark, with square outer corners (Windows 11 is told not to round them). Drawn by hand into a
//! persistent back buffer; every input event repaints only the part that changed. There are no polling
//! timers: the APR thread / logger wake the window with one message (see state.rs), and the only timers are
//! short-lived (smooth scrolling, tooltip delay, footer notice, the exit countdown).
//!
//! Exit rule: when APR finishes (e.g. login detected) and this window is open, the app stays until the
//! window is closed - or until the user has been inactive for 60 s. If the window is closed, it exits at once.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, EndPaint, GetDC, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, DT_CENTER,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC, HFONT, HGDIOBJ, PAINTSTRUCT,
    SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, GetWindowRect, IsIconic, KillTimer, LoadCursorW,
    SendMessageW, SM_CXICON, SM_CXSMICON, WM_SETICON,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, IDC_ARROW, MINMAXINFO,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL,
    WINDOW_EX_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WM_SIZE,
    WM_TIMER, WNDCLASSW, WS_CLIPCHILDREN, WS_MINIMIZEBOX, WS_POPUP, WS_THICKFRAME,
};

use crate::gfx::*;
use crate::state::{WM_APP_OVERLAY_HIDE, WM_APP_OVERLAY_SHOW, WM_APP_QUIT, WM_APP_SHOW, WM_APP_WAKE};
use crate::tab_logs::LogsPane;
use crate::tab_misc::MiscPane;
use crate::tab_settings::SettingsPane;
use crate::ui_types::{Action, Ctx, Fonts, React};
use crate::{config, languages, log, overlay, settings, state, sys, ui};

// ---------------- tuning ----------------
const ANIM_TIMER: usize = 2; // smooth scrolling (runs only while moving)
const TIP_TIMER: usize = 3; // tooltip delay (one-shot)
const GRACE_TIMER: usize = 4; // exit countdown (1 s, only after APR finished)
const NOTICE_TIMER: usize = 5; // footer notice (one-shot)
const ANIM_MS: u32 = 16;
const TIP_DELAY_MS: u32 = 450;
const NOTICE_MS: u32 = 4000;
/// After APR finished: exit after this long without any interaction (or as soon as the window is closed).
const GRACE: Duration = Duration::from_secs(60);

const HEADER_H: f32 = 56.0;
const FOOTER_H: f32 = 40.0;
const SIDEBAR_W: f32 = 210.0;

// which parts of the back buffer need redrawing
const D_HEADER: u8 = 1;
const D_SIDEBAR: u8 = 2;
const D_FOOTER: u8 = 4;
const D_PANE: u8 = 8; // whole tab
const D_SCROLL: u8 = 16; // Live Logs only: toolbar + newly scrolled lines
const D_ALL: u8 = D_HEADER | D_SIDEBAR | D_FOOTER | D_PANE;

const C_SIDEBAR: Rgb = (25, 25, 28);
const C_SIDE_ACTIVE: Rgb = (46, 46, 52);

// hit-test results
const HT_CLIENT: isize = 1;
const HT_CAPTION: isize = 2;
const HT_LEFT: isize = 10;
const HT_RIGHT: isize = 11;
const HT_TOP: isize = 12;
const HT_TOPLEFT: isize = 13;
const HT_TOPRIGHT: isize = 14;
const HT_BOTTOM: isize = 15;
const HT_BOTTOMLEFT: isize = 16;
const HT_BOTTOMRIGHT: isize = 17;
const WM_MOUSELEAVE_ID: u32 = 0x02A3;

/// Footer status colour: red = error, blue = APR finished (waiting for you to close the UI), green = running normally.
fn status_color(a: &App) -> Rgb {
    let t = a.status_text.to_lowercase();
    let error = ["failed", "error", "not found", "unavailable", "fatal", "crashed", "timed out", "restart loop", "loop detected", "missing"]
        .iter()
        .any(|k| t.contains(k));
    if error {
        C_ERR
    } else if a.exit_pending || t.contains("finished") {
        C_DONE
    } else {
        C_OK
    }
}

/// True while the window is shown (hidden to the tray = false).
static OPEN: AtomicBool = AtomicBool::new(false);

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Logs,
    Settings,
    Misc,
}

const TABS: [(&str, Tab); 3] = [("Live Logs", Tab::Logs), ("Settings", Tab::Settings), ("Misc", Tab::Misc)];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChromeHot {
    None,
    Min,
    Close,
    Tab(usize),
    Update,
}

struct App {
    ctx: Ctx,
    client_w: i32,
    client_h: i32,
    back: Option<BackBuffer>,
    dirty: u8,
    minimized: bool,
    tab: Tab,
    hot: ChromeHot,
    tracking: bool,
    capturing: bool,
    tip_visible: bool,
    status_text: String,
    status_version: u64,
    notice: Option<String>,
    exit_note: Option<String>,
    exit_pending: bool,
    last_interaction: Instant,
    last_mouse: (i32, i32),
    logs: LogsPane,
    settings: SettingsPane,
    misc: MiscPane,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the state. Returns None (instead of panicking) if a message handler re-entered us.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

// ---------------- geometry ----------------
struct Chrome {
    header_h: i32,
    sidebar_w: i32,
    footer_top: i32,
    content: RECT,
    min: RECT,
    close: RECT,
    tabs: Vec<RECT>,
    update: RECT,
    version: RECT,
    status: RECT,
}

fn chrome(ctx: &Ctx, w: i32, h: i32) -> Chrome {
    let sc = ctx.scale;
    let header_h = px(HEADER_H, sc);
    let footer_h = px(FOOTER_H, sc);
    let sidebar_w = px(SIDEBAR_W, sc);
    let footer_top = h - footer_h;
    let (bw, bh) = (px(40.0, sc), px(28.0, sc));
    let top = (header_h - bh) / 2;
    let right = w - px(14.0, sc);
    let close = rect(right - bw, top, right, top + bh);
    let min = rect(close.left - px(4.0, sc) - bw, top, close.left - px(4.0, sc), top + bh);
    let tabs = (0..TABS.len() as i32)
        .map(|i| {
            let y = header_h + px(20.0, sc) + i * px(50.0, sc);
            rect(px(14.0, sc), y, sidebar_w - px(14.0, sc), y + px(44.0, sc))
        })
        .collect();
    let (uw, uh) = (px(156.0, sc), px(26.0, sc));
    let uy = footer_top + (footer_h - uh) / 2;
    let update = rect(w - px(24.0, sc) - uw, uy, w - px(24.0, sc), uy + uh);
    let version = rect(update.left - px(80.0, sc), footer_top, update.left - px(12.0, sc), h);
    let status = rect(px(46.0, sc), footer_top, version.left - px(12.0, sc), h);
    Chrome {
        header_h,
        sidebar_w,
        footer_top,
        content: rect(sidebar_w, header_h, w, footer_top),
        min,
        close,
        tabs,
        update,
        version,
        status,
    }
}

fn chrome_hit(ch: &Chrome, x: i32, y: i32) -> ChromeHot {
    if inside(&ch.close, x, y) {
        ChromeHot::Close
    } else if inside(&ch.min, x, y) {
        ChromeHot::Min
    } else if inside(&ch.update, x, y) {
        ChromeHot::Update
    } else {
        match ch.tabs.iter().position(|r| inside(r, x, y)) {
            Some(i) => ChromeHot::Tab(i),
            None => ChromeHot::None,
        }
    }
}

fn pane_mark(tab: Tab) -> u8 {
    if tab == Tab::Logs {
        D_SCROLL
    } else {
        D_PANE
    }
}

/// Marks parts of the back buffer stale and asks Windows to repaint just those screen areas.
fn mark(hwnd: HWND, a: &mut App, parts: u8) {
    if parts == 0 {
        return;
    }
    a.dirty |= parts;
    if !is_open() || a.minimized || a.client_w <= 0 {
        return;
    }
    let (w, h) = (a.client_w, a.client_h);
    let ch = chrome(&a.ctx, w, h);
    unsafe {
        let inv = |rc: RECT| {
            let _ = InvalidateRect(hwnd, Some(&rc as *const RECT), false);
        };
        if parts & D_HEADER != 0 {
            inv(rect(0, 0, w, ch.header_h));
        }
        if parts & D_SIDEBAR != 0 {
            inv(rect(0, ch.header_h, ch.sidebar_w, ch.footer_top));
        }
        if parts & D_FOOTER != 0 {
            inv(rect(0, ch.footer_top, w, h));
        }
        if parts & (D_PANE | D_SCROLL) != 0 {
            inv(ch.content);
        }
    }
}

// ---------------- fonts ----------------
unsafe fn create_fonts(a: &mut App, hwnd: HWND) {
    let f = a.ctx.fonts;
    for font in [f.ui, f.bold, f.small, f.mono, f.title, f.page] {
        delete_font(font);
    }
    let sc = a.ctx.scale;
    a.ctx.fonts = Fonts {
        ui: make_font(px(13.0, sc), 400, "Segoe UI"),
        bold: make_font(px(15.0, sc), 600, "Segoe UI"),
        small: make_font(px(12.0, sc), 400, "Segoe UI"),
        mono: make_font(px(13.0, sc), 400, "Consolas"),
        title: make_font(px(17.0, sc), 600, "Segoe UI"),
        page: make_font(px(24.0, sc), 600, "Segoe UI"),
    };
    a.ctx.line_h = px(13.0 * 1.3, sc);
    let hdc = GetDC(hwnd);
    if !hdc.is_invalid() {
        a.ctx.char_w = char_width(hdc, a.ctx.fonts.mono);
        ReleaseDC(hwnd, hdc);
    }
    a.logs.apply_font(&a.ctx); // the text box must not keep a deleted font
}

// ---------------- painting ----------------
unsafe fn draw_header(a: &App, dc: HDC, w: i32, ch: &Chrome) {
    let sc = a.ctx.scale;
    fill(dc, &rect(0, 0, w, ch.header_h), C_BAR);
    fill(dc, &rect(0, ch.header_h - 1, w, ch.header_h), C_BORDER);
    // title
    let old = SelectObject(dc, HGDIOBJ(a.ctx.fonts.title.0));
    let mut title = rect(px(24.0, sc), 0, ch.min.left - px(10.0, sc), ch.header_h);
    put_text(dc, "WuWa APR", &mut title, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);

    let radius = px(6.0, sc);
    let bg = |h: ChromeHot, c: Rgb| if a.hot == h { c } else { C_BAR };
    fill_round(dc, &ch.min, bg(ChromeHot::Min, C_BTN_HOT), radius);
    let (cx, cy) = ((ch.min.left + ch.min.right) / 2, (ch.min.top + ch.min.bottom) / 2);
    let half = px(5.0, sc);
    line(dc, (cx - half, cy + px(2.0, sc)), (cx + half, cy + px(2.0, sc)), C_TEXT, px(1.5, sc).max(1));

    fill_round(dc, &ch.close, bg(ChromeHot::Close, C_CLOSE_HOT), radius);
    let (cx, cy) = ((ch.close.left + ch.close.right) / 2, (ch.close.top + ch.close.bottom) / 2);
    let half = px(4.5, sc);
    line(dc, (cx - half, cy - half), (cx + half, cy + half), C_TEXT, px(1.5, sc).max(1));
    line(dc, (cx - half, cy + half), (cx + half, cy - half), C_TEXT, px(1.5, sc).max(1));
}

/// Simple line-art icons for the sidebar (no icon font needed).
unsafe fn draw_tab_icon(dc: HDC, tab: Tab, cx: i32, cy: i32, s: i32, c: Rgb) {
    let wd = (s / 9).max(1);
    match tab {
        Tab::Logs => {
            // three text lines
            let h = s / 2;
            line(dc, (cx - h, cy - s / 3), (cx + h, cy - s / 3), c, wd);
            line(dc, (cx - h, cy), (cx + h - s / 5, cy), c, wd);
            line(dc, (cx - h, cy + s / 3), (cx + h, cy + s / 3), c, wd);
        }
        Tab::Settings => {
            // two sliders with knobs
            let h = s / 2;
            let r = (s / 6).max(2);
            line(dc, (cx - h, cy - s / 4), (cx + h, cy - s / 4), c, wd);
            line(dc, (cx - h, cy + s / 4), (cx + h, cy + s / 4), c, wd);
            fill_round(dc, &rect(cx - h / 2 - r, cy - s / 4 - r, cx - h / 2 + r, cy - s / 4 + r), c, r);
            fill_round(dc, &rect(cx + h / 2 - r, cy + s / 4 - r, cx + h / 2 + r, cy + s / 4 + r), c, r);
        }
        Tab::Misc => {
            // 2 x 2 tiles
            let t = s / 3;
            let g = (s / 8).max(1);
            for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let x = cx + dx * (t / 2 + g / 2);
                let y = cy + dy * (t / 2 + g / 2);
                fill_round(dc, &rect(x - t / 2, y - t / 2, x + t / 2, y + t / 2), c, (t / 4).max(1));
            }
        }
    }
}

unsafe fn draw_sidebar(a: &App, dc: HDC, ch: &Chrome) {
    let sc = a.ctx.scale;
    fill(dc, &rect(0, ch.header_h, ch.sidebar_w, ch.footer_top), C_SIDEBAR);
    fill(dc, &rect(ch.sidebar_w - 1, ch.header_h, ch.sidebar_w, ch.footer_top), C_BORDER);
    let old = SelectObject(dc, HGDIOBJ(a.ctx.fonts.ui.0));
    for (i, ((label, tab), r)) in TABS.iter().zip(ch.tabs.iter()).enumerate() {
        let active = a.tab == *tab;
        if active {
            fill_round(dc, r, C_SIDE_ACTIVE, px(8.0, sc));
            fill_round(dc, &rect(r.left, r.top + px(12.0, sc), r.left + px(3.0, sc), r.bottom - px(12.0, sc)), C_ACCENT, px(1.0, sc));
        } else if a.hot == ChromeHot::Tab(i) {
            fill_round(dc, r, (34, 34, 38), px(8.0, sc));
        }
        let color = if active { C_TEXT } else { C_DIM };
        draw_tab_icon(dc, *tab, r.left + px(28.0, sc), (r.top + r.bottom) / 2, px(18.0, sc), if active { C_ACCENT } else { C_DIM });
        let mut tr = rect(r.left + px(52.0, sc), r.top, r.right - px(8.0, sc), r.bottom);
        put_text(dc, label, &mut tr, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }
    SelectObject(dc, old);
}

unsafe fn draw_footer(a: &App, dc: HDC, w: i32, h: i32, ch: &Chrome) {
    let sc = a.ctx.scale;
    fill(dc, &rect(0, ch.footer_top, w, h), C_BAR);
    fill(dc, &rect(0, ch.footer_top, w, ch.footer_top + 1), C_BORDER);
    let old = SelectObject(dc, HGDIOBJ(a.ctx.fonts.small.0));

    // status dot (red = error, blue = finished, green = running) + text (a notice / the exit countdown take over for a moment)
    let (text, color) = if let Some(n) = &a.exit_note {
        (n.as_str(), C_DONE)
    } else if let Some(n) = &a.notice {
        (n.as_str(), C_ACCENT)
    } else {
        (a.status_text.as_str(), C_DIM)
    };
    let cy = (ch.footer_top + h) / 2;
    let r = px(5.0, sc);
    fill_round(dc, &rect(px(24.0, sc), cy - r, px(24.0, sc) + r * 2, cy + r), status_color(a), r);
    let mut sr = ch.status;
    put_text(dc, text, &mut sr, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);

    let mut vr = ch.version;
    put_text(dc, &format!("v{}", config::VERSION), &mut vr, C_DIM, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

    let hot = a.hot == ChromeHot::Update;
    fill_round(dc, &ch.update, if hot { C_PILL_HOT } else { C_PILL }, px(8.0, sc));
    let mut ur = ch.update;
    put_text(dc, "Check for updates", &mut ur, C_TEXT, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn draw_frame(a: &mut App, dc: HDC, w: i32, h: i32, dirty: u8) {
    let ch = chrome(&a.ctx, w, h);
    SetBkMode(dc, TRANSPARENT);
    if dirty & D_HEADER != 0 {
        draw_header(a, dc, w, &ch);
    }
    if dirty & D_SIDEBAR != 0 {
        draw_sidebar(a, dc, &ch);
    }
    if dirty & D_FOOTER != 0 {
        draw_footer(a, dc, w, h, &ch);
    }
    if dirty & (D_PANE | D_SCROLL) != 0 {
        let full = dirty & D_PANE != 0;
        match a.tab {
            Tab::Logs => a.logs.render(&a.ctx, dc, &ch.content, full),
            Tab::Settings => {
                a.settings.render(&a.ctx, dc, &ch.content);
                if a.tip_visible {
                    a.settings.draw_tooltip(&a.ctx, dc, &ch.content);
                }
            }
            Tab::Misc => a.misc.render(&a.ctx, dc, &ch.content),
        }
    }
    // 1 px outline
    fill(dc, &rect(0, 0, w, 1), C_BORDER);
    fill(dc, &rect(0, h - 1, w, h), C_BORDER);
    fill(dc, &rect(0, 0, 1, h), C_BORDER);
    fill(dc, &rect(w - 1, 0, w, h), C_BORDER);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let pr = ps.rcPaint;
    let _ = with_app(|a| {
        let (w, h) = (a.client_w, a.client_h);
        if w <= 0 || h <= 0 {
            return;
        }
        if a.back.as_ref().map(|b| b.w != w || b.h != h).unwrap_or(true) {
            a.back = Some(BackBuffer::new(hdc, w, h));
            a.dirty = D_ALL;
        }
        let dirty = a.dirty;
        a.dirty = 0;
        let Some(dc) = a.back.as_ref().map(|b| b.dc) else { return };
        if dirty != 0 {
            draw_frame(a, dc, w, h, dirty);
        }
        // copy only the area Windows asked for
        let _ = BitBlt(hdc, pr.left, pr.top, pr.right - pr.left, pr.bottom - pr.top, dc, pr.left, pr.top, SRCCOPY);
    });
    let _ = EndPaint(hwnd, &ps);
}

// ---------------- input plumbing ----------------
/// What still has to be done outside the state borrow (timers, mouse capture, dialogs, ...).
#[derive(Default)]
struct Post {
    anim: bool,
    tip: bool,
    capture: bool,
    release: bool,
    track: bool,
    close: bool,
    minimize: bool,
    action: Option<Action>,
    notice: Option<String>,
}

/// Folds a tab's reaction into the app state. Returns the parts that need repainting.
fn absorb(a: &mut App, r: &React, post: &mut Post) -> u8 {
    let mut parts = 0;
    if r.redraw {
        a.tip_visible = false; // anything that changes the tab also hides the tooltip
        parts |= pane_mark(a.tab);
    }
    post.anim |= r.anim;
    post.tip |= r.tip;
    post.capture |= r.capture;
    post.release |= r.release;
    if r.capture {
        a.capturing = true;
    }
    if r.release {
        a.capturing = false;
    }
    if r.action.is_some() {
        post.action = r.action;
    }
    parts
}

fn pane_leave(a: &mut App) -> React {
    match a.tab {
        Tab::Logs => a.logs.leave(),
        Tab::Settings => a.settings.leave(),
        Tab::Misc => a.misc.leave(),
    }
}

fn set_notice(hwnd: HWND, text: &str) {
    with_app(|a| {
        a.notice = Some(text.to_string());
        mark(hwnd, a, D_FOOTER);
    });
    unsafe {
        SetTimer(hwnd, NOTICE_TIMER, NOTICE_MS, None);
    }
}

fn start_anim(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, ANIM_TIMER, ANIM_MS, None);
    }
}

fn switch_tab(a: &mut App, tab: Tab) -> u8 {
    if a.tab == tab {
        return 0;
    }
    a.tab = tab;
    a.tip_visible = false;
    a.settings.escape();
    a.logs.set_visible(tab == Tab::Logs);
    D_SIDEBAR | D_PANE
}

/// Timers / capture / actions that must run after the state borrow is released.
unsafe fn finish_post(hwnd: HWND, post: Post) {
    if post.track {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tme);
    }
    if post.anim {
        start_anim(hwnd);
    }
    if post.tip {
        SetTimer(hwnd, TIP_TIMER, TIP_DELAY_MS, None);
    }
    if post.capture {
        SetCapture(hwnd);
    }
    if post.release {
        let _ = ReleaseCapture();
    }
    if let Some(n) = post.notice {
        set_notice(hwnd, &n);
    }
    if post.minimize {
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
    }
    if post.close {
        request_close(hwnd);
    }
    if let Some(action) = post.action {
        run_action(hwnd, action);
    }
}

/// Switches to the Live Logs tab (used when a test starts, so the result is visible).
fn show_logs_tab(hwnd: HWND) {
    with_app(|a| {
        let parts = switch_tab(a, Tab::Logs);
        mark(hwnd, a, parts);
    });
}

fn run_action(hwnd: HWND, action: Action) {
    match action {
        Action::CopyLogs => {
            let (text, n) = with_app(|a| (a.logs.visible_text(), a.logs.visible_count())).unwrap_or_default();
            if n == 0 {
                set_notice(hwnd, "Nothing to copy");
            } else if sys::clipboard_set_text(hwnd, &text) {
                set_notice(hwnd, &format!("Copied {n} lines to the clipboard"));
            } else {
                set_notice(hwnd, "Could not open the clipboard");
            }
        }
        Action::ExportLogs => {
            let (text, n) = with_app(|a| (a.logs.visible_text(), a.logs.visible_count())).unwrap_or_default();
            if n == 0 {
                set_notice(hwnd, "Nothing to export");
                return;
            }
            let path = config::exports_dir().join(format!("apr-log-{}.txt", sys::file_stamp()));
            match std::fs::write(&path, text) {
                Ok(()) => {
                    sys::reveal(&path);
                    set_notice(hwnd, &format!("Exported {n} lines to {}", path.display()));
                }
                Err(e) => set_notice(hwnd, &format!("Export failed: {e}")),
            }
        }
        Action::ImportSettings => {
            let Some(path) = sys::pick_open_file(hwnd, "Import WuWa APR settings") else { return };
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| settings::import_json(&t)) {
                Ok(()) => {
                    with_app(|a| mark(hwnd, a, D_PANE));
                    set_notice(hwnd, "Settings imported");
                }
                Err(e) => set_notice(hwnd, &format!("Import failed: {e}")),
            }
        }
        Action::ExportSettings => {
            let name = format!("wuwa-apr-settings-{}.json", sys::file_stamp());
            let Some(path) = sys::pick_save_file(hwnd, "Export WuWa APR settings", &config::exports_dir(), &name) else {
                return;
            };
            match std::fs::write(&path, settings::export_json()) {
                Ok(()) => set_notice(hwnd, &format!("Settings exported to {}", path.display())),
                Err(e) => set_notice(hwnd, &format!("Export failed: {e}")),
            }
        }
        Action::DebugShot => match config::latest_screenshot() {
            Some(png) => sys::open_path(&png),
            None => set_notice(hwnd, "No screenshot saved yet - turn on 'Save screenshots' in Settings > Debugging"),
        },
        Action::PreviewOverlay => {
            overlay::request_show(overlay::TEXT_PATCH, overlay::Kind::Progress, crate::capture::monitor_of(hwnd), 5000, true);
            set_notice(hwnd, "Showing the notification for 5 seconds");
        }
        Action::ReloadLanguages => {
            for note in languages::load() {
                log::log(note);
            }
            languages::check_packs();
            with_app(|a| mark(hwnd, a, D_PANE));
            set_notice(hwnd, "Languages reloaded - see Live Logs for details");
        }
        Action::OpenLanguagesFile => sys::open_path(&config::languages_file()),
        Action::OpenData => sys::open_path(config::app_dir()),
        Action::ForgetGame => match config::forget_game() {
            Ok(()) => set_notice(hwnd, "Game install forgotten - restart APR to choose it again"),
            Err(e) => set_notice(hwnd, &format!("Could not forget the game install: {e}")),
        },
        Action::SelfTest => {
            show_logs_tab(hwnd);
            crate::selftest::run_simulated();
            set_notice(hwnd, "Self-test running - results appear in Live Logs");
        }
        Action::TestImage => {
            let Some(path) = sys::pick_image_file(hwnd, "Choose a screenshot to test") else { return };
            show_logs_tab(hwnd);
            crate::selftest::run_image(path);
            set_notice(hwnd, "Testing the image - results appear in Live Logs");
        }
		Action::LiveOverlayPreview => {
			if !overlay::real_active() {
				if settings::general().overlay().on {
					overlay::request_show(overlay::TEXT_PATCH, overlay::Kind::Progress,
						crate::capture::monitor_of(hwnd), 2000, false);
				} else {
					overlay::request_hide(); // switched the notification off: hide it
				}
			}
		}
    }
}

// ---------------- public API ----------------
/// Creates the (hidden) window.
pub fn create() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).expect("GetModuleHandleW");
        let class_name = w!("WuWaAprMainWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let scale = GetDpiForSystem().max(96) as f32 / 96.0;
        let (screen_w, screen_h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let width = px(1100.0, scale).min(screen_w - 40);
        let height = px(720.0, scale).min((screen_h as f32 * 0.92) as i32);
        let (x, y) = (((screen_w - width) / 2).max(0), ((screen_h - height) / 2).max(0));

        let empty = HFONT::default();
        let app = App {
            ctx: Ctx {
                scale,
                fonts: Fonts { ui: empty, bold: empty, small: empty, mono: empty, title: empty, page: empty },
                char_w: 7,
                line_h: 16,
            },
            client_w: 0,
            client_h: 0,
            back: None,
            dirty: D_ALL,
            minimized: false,
            tab: Tab::Logs,
            hot: ChromeHot::None,
            tracking: false,
            capturing: false,
            tip_visible: false,
            status_text: String::new(),
            status_version: 0,
            notice: None,
            exit_note: None,
            exit_pending: false,
            last_interaction: Instant::now(),
            last_mouse: (-1, -1),
            logs: LogsPane::new(),
            settings: SettingsPane::new(),
            misc: MiscPane::new(),
        };
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        // WS_THICKFRAME + a zero non-client area (WM_NCCALCSIZE) = no title bar but native edge resizing + shadow
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("WuWa APR"),
            WS_POPUP | WS_THICKFRAME | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            None,
            None,
            instance,
            None,
        )
        .expect("create main window");

        let _ = square_corners(hwnd);
        set_window_icons(hwnd);
        state::set_ui_hwnd(hwnd.0 as isize);
        hwnd
    }
}

/// The window's own icon (shown in Alt+Tab and on the taskbar) = the tray icon.
unsafe fn set_window_icons(hwnd: HWND) {
    for (which, metric) in [(1usize, SM_CXICON), (0usize, SM_CXSMICON)] {
        // 1 = ICON_BIG (Alt+Tab), 0 = ICON_SMALL (title area / taskbar)
        let want = GetSystemMetrics(metric).max(16) as u32;
        if let Some(h) = crate::ui::tray_hicon(want) {
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(which), LPARAM(h));
        }
    }
}

fn show(hwnd: HWND) {
    let was_open = OPEN.swap(true, Ordering::Relaxed);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }
        let _ = SetForegroundWindow(hwnd);
    }
    with_app(|a| {
        a.last_interaction = Instant::now();
        a.minimized = false;
        if !was_open {
            let ch = chrome(&a.ctx, a.client_w, a.client_h);
            a.logs.relayout(&a.ctx, &ch.content);
            a.logs.snap_to_end(); // opening jumps to the newest line instead of scrolling there
        }
        mark(hwnd, a, D_ALL);
    });
}

/// The X button / Esc / Alt+F4: hide to the tray - or, if APR already finished, exit.
fn request_close(hwnd: HWND) {
    let finished = with_app(|a| a.exit_pending).unwrap_or(false);
    if finished {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return;
    }
    OPEN.store(false, Ordering::Relaxed);
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

// ---------------- wake-ups from the APR thread ----------------
unsafe fn on_wake(hwnd: HWND) {
    state::clear_wake();
    let lines = log::drain_queue();
    let (anim, new_status) = with_app(|a| {
        let mut parts = 0u8;
        let anim = false; // the native text box repaints and scrolls itself
        if !lines.is_empty() {
            a.logs.push(&lines);
        }
        let (version, text) = state::status();
        let mut new_status = None;
        if version != a.status_version {
            a.status_version = version;
            a.status_text = text.clone();
            parts |= D_FOOTER;
            new_status = Some(text);
        }
        if languages::take_packs_changed() && a.tab == Tab::Settings {
            parts |= D_PANE; // the OCR pack check finished: update the language status
        }
        mark(hwnd, a, parts);
        (anim, new_status)
    })
    .unwrap_or((false, None));
    if let Some(text) = new_status {
        ui::set_tray_status(&text);
    }
    if anim {
        start_anim(hwnd);
    }
    if state::is_done() {
        handle_finish(hwnd);
    }
}

/// APR is done. Exit now if nobody is looking; otherwise wait for the user (see the module docs).
unsafe fn handle_finish(hwnd: HWND) {
    let first = with_app(|a| {
        let first = !a.exit_pending;
        a.exit_pending = true;
        first
    })
    .unwrap_or(false);
    if !first {
        return;
    }
    if !is_open() {
        let _ = DestroyWindow(hwnd);
        return;
    }
    let (_, status) = state::status();
    log::log(format!(
        "[SYSTEM] {status} APR will close when you close this window, or after {} seconds without activity.",
        GRACE.as_secs()
    ));
    SetTimer(hwnd, GRACE_TIMER, 1000, None);
    grace_tick(hwnd);
}

unsafe fn grace_tick(hwnd: HWND) {
    let idle = with_app(|a| a.last_interaction.elapsed()).unwrap_or(GRACE);
    if !is_open() || idle >= GRACE {
        let _ = DestroyWindow(hwnd);
        return;
    }
    let secs = ((GRACE - idle).as_millis() + 999) / 1000;
    with_app(|a| {
        a.exit_note = Some(format!("APR finished - closing in {secs}s without activity (or when you close this window)"));
        mark(hwnd, a, D_FOOTER);
    });
}

unsafe fn on_anim(hwnd: HWND) {
    let keep = with_app(|a| {
        if !is_open() || a.minimized {
            while a.logs.anim_step().1 {} // nobody is watching: finish instantly
            return false;
        }
        let (changed, more) = a.logs.anim_step();
        if changed && a.tab == Tab::Logs {
            mark(hwnd, a, D_SCROLL);
        }
        more
    })
    .unwrap_or(false);
    if !keep {
        let _ = KillTimer(hwnd, ANIM_TIMER);
    }
}

// ---------------- window procedure ----------------
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            with_app(|a| {
                create_fonts(a, hwnd);
                a.logs.attach(hwnd, &a.ctx);
            });
            LRESULT(0)
        }
        // client area = whole window: no title bar, no frame
        WM_NCCALCSIZE => {
            if wparam.0 != 0 {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_ERASEBKGND => LRESULT(1),
        // the log text box asks its parent for its colours (a read-only EDIT uses the "static" message)
        0x0133 | 0x0138 => crate::tab_logs::ctl_color(HDC(wparam.0 as _)),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            if wparam.0 == 1 {
                with_app(|a| a.minimized = true); // SIZE_MINIMIZED
            } else {
                let (cw, ch_) = ((lparam.0 & 0xFFFF) as i32, ((lparam.0 >> 16) & 0xFFFF) as i32);
                with_app(|a| {
                    a.minimized = false;
                    a.client_w = cw;
                    a.client_h = ch_;
                    a.back = None; // size changed: new buffer on the next paint
                    let ch = chrome(&a.ctx, cw, ch_);
                    a.logs.relayout(&a.ctx, &ch.content);
                    a.settings.clamp_scroll(&a.ctx, &ch.content);
                    a.dirty = D_ALL;
                });
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let scale = with_app(|a| a.ctx.scale).unwrap_or(1.0);
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = px(960.0, scale);
            info.ptMinTrackSize.y = px(580.0, scale);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as f32;
            let suggested = *(lparam.0 as *const RECT);
            with_app(|a| {
                a.ctx.scale = dpi / 96.0;
                create_fonts(a, hwnd);
            });
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let (sx, sy) = (lo_signed(lparam), hi_signed(lparam));
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let (x, y) = (sx - wr.left, sy - wr.top);
            let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
            let verdict = with_app(|a| {
                let b = px(6.0, a.ctx.scale);
                let ch = chrome(&a.ctx, w, h);
                let (l, rt, t, bt) = (x < b, x >= w - b, y < b, y >= h - b);
                if t && l {
                    HT_TOPLEFT
                } else if t && rt {
                    HT_TOPRIGHT
                } else if bt && l {
                    HT_BOTTOMLEFT
                } else if bt && rt {
                    HT_BOTTOMRIGHT
                } else if l {
                    HT_LEFT
                } else if rt {
                    HT_RIGHT
                } else if t {
                    HT_TOP
                } else if bt {
                    HT_BOTTOM
                } else if inside(&ch.close, x, y) || inside(&ch.min, x, y) {
                    HT_CLIENT
                } else if y < ch.header_h {
                    HT_CAPTION // drag the window by its header
                } else {
                    HT_CLIENT
                }
            })
            .unwrap_or(HT_CLIENT);
            LRESULT(verdict)
        }
        WM_MOUSEMOVE => {
            let (x, y) = (lo_signed(lparam), hi_signed(lparam));
            let post = with_app(|a| {
                let mut post = Post::default();
                if (x, y) != a.last_mouse {
                    a.last_mouse = (x, y);
                    a.last_interaction = Instant::now();
                }
                let ch = chrome(&a.ctx, a.client_w, a.client_h);
                let mut parts = 0u8;
                let hot = chrome_hit(&ch, x, y);
                if hot != a.hot {
                    a.hot = hot;
                    parts |= D_HEADER | D_SIDEBAR | D_FOOTER;
                }
                let r = if a.capturing || inside(&ch.content, x, y) {
                    match a.tab {
                        Tab::Logs => a.logs.mouse_move(&a.ctx, &ch.content, x, y),
                        Tab::Settings => a.settings.mouse_move(&a.ctx, &ch.content, x, y),
                        Tab::Misc => a.misc.mouse_move(&a.ctx, &ch.content, x, y),
                    }
                } else {
                    pane_leave(a)
                };
                parts |= absorb(a, &r, &mut post);
                post.track = !a.tracking;
                a.tracking = true;
                mark(hwnd, a, parts);
                post
            })
            .unwrap_or_default();
            finish_post(hwnd, post);
            LRESULT(0)
        }
        WM_MOUSELEAVE_ID => {
            with_app(|a| {
                a.tracking = false;
                if !a.capturing {
                    let mut parts = 0u8;
                    if a.hot != ChromeHot::None {
                        a.hot = ChromeHot::None;
                        parts |= D_HEADER | D_SIDEBAR | D_FOOTER;
                    }
                    let mut post = Post::default();
                    let r = pane_leave(a);
                    parts |= absorb(a, &r, &mut post);
                    mark(hwnd, a, parts);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(hwnd);
            let (x, y) = (lo_signed(lparam), hi_signed(lparam));
            let post = with_app(|a| {
                a.last_interaction = Instant::now();
                let mut post = Post::default();
                let ch = chrome(&a.ctx, a.client_w, a.client_h);
                let mut parts = 0u8;
                match chrome_hit(&ch, x, y) {
                    ChromeHot::Tab(i) => parts |= switch_tab(a, TABS[i].1),
                    ChromeHot::Update => {
                        post.notice = Some("Update checks aren't available yet - coming in a future version.".to_string());
                    }
                    ChromeHot::Min | ChromeHot::Close => {} // handled on button-up
                    ChromeHot::None => {
                        if inside(&ch.content, x, y) {
                            let r = match a.tab {
                                Tab::Logs => a.logs.mouse_down(&a.ctx, &ch.content, x, y),
                                Tab::Settings => a.settings.mouse_down(&a.ctx, &ch.content, x, y),
                                Tab::Misc => a.misc.mouse_down(&a.ctx, &ch.content, x, y),
                            };
                            parts |= absorb(a, &r, &mut post);
                        }
                    }
                }
                mark(hwnd, a, parts);
                post
            })
            .unwrap_or_default();
            finish_post(hwnd, post);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let post = with_app(|a| {
                a.last_interaction = Instant::now();
                let mut post = Post::default();
                let mut parts = 0u8;
                if a.capturing && a.tab == Tab::Settings {
                    let r = a.settings.mouse_up();
                    parts |= absorb(a, &r, &mut post);
                } else {
                    match a.hot {
                        ChromeHot::Close => post.close = true,
                        ChromeHot::Min => post.minimize = true,
                        _ => {}
                    }
                }
                mark(hwnd, a, parts);
                post
            })
            .unwrap_or_default();
            finish_post(hwnd, post);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let notches = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            let post = with_app(|a| {
                a.last_interaction = Instant::now();
                let mut post = Post::default();
                let ch = chrome(&a.ctx, a.client_w, a.client_h);
                let r = match a.tab {
                    Tab::Logs => a.logs.wheel(&a.ctx, notches),
                    Tab::Settings => a.settings.wheel(&a.ctx, &ch.content, notches),
                    Tab::Misc => a.misc.wheel(&a.ctx, &ch.content, notches),
                };
                let parts = absorb(a, &r, &mut post);
                mark(hwnd, a, parts);
                post
            })
            .unwrap_or_default();
            finish_post(hwnd, post);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            let post = with_app(|a| {
                a.last_interaction = Instant::now();
                let mut post = Post::default();
                let mut parts = 0u8;
                if vk == 0x1B {
                    // Esc: close an open dropdown first, then the window
                    if a.tab == Tab::Settings && a.settings.escape() {
                        parts |= D_PANE;
                    } else {
                        post.close = true;
                    }
                } else if a.tab == Tab::Logs {
                    let r = a.logs.key(&a.ctx, vk);
                    parts |= absorb(a, &r, &mut post);
                }
                mark(hwnd, a, parts);
                post
            })
            .unwrap_or_default();
            finish_post(hwnd, post);
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                ANIM_TIMER => on_anim(hwnd),
                TIP_TIMER => {
                    let _ = KillTimer(hwnd, TIP_TIMER);
                    with_app(|a| {
                        if a.tab == Tab::Settings && a.settings.tooltip().is_some() {
                            a.tip_visible = true;
                            mark(hwnd, a, D_PANE);
                        }
                    });
                }
                GRACE_TIMER => grace_tick(hwnd),
                NOTICE_TIMER => {
                    let _ = KillTimer(hwnd, NOTICE_TIMER);
                    with_app(|a| {
                        a.notice = None;
                        mark(hwnd, a, D_FOOTER);
                    });
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_WAKE => {
            on_wake(hwnd);
            LRESULT(0)
        }
        WM_APP_SHOW => {
            show(hwnd);
            LRESULT(0)
        }
        WM_APP_OVERLAY_SHOW => {
            overlay::on_show();
            LRESULT(0)
        }
        WM_APP_OVERLAY_HIDE => {
            overlay::on_hide();
            LRESULT(0)
        }
        WM_APP_QUIT => {
            log::set_status("Stopping (quit requested)...", true);
            state::request_stop();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        // Alt+F4 / taskbar close behave like the X button
        WM_CLOSE => {
            request_close(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app(|a| {
                a.back = None;
                let f = a.ctx.fonts;
                for font in [f.ui, f.bold, f.small, f.mono, f.title, f.page] {
                    delete_font(font);
                }
            });
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
