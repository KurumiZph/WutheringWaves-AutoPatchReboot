//! The "Live Logs" tab: title, filter chips, Copy / Export, and below them a plain native text box.
//!
//! The text box is a real read-only Windows Rich Edit control (the one WordPad uses), so selecting, Ctrl+C,
//! the right-click menu, wheel scrolling and the scrollbar are all handled by Windows itself: nothing is
//! painted by hand (no per-frame redraw, no custom scroll animation). Each line gets the colour of its kind
//! (error, warning, ...). If Rich Edit is unavailable it falls back to a plain, uncoloured EDIT box. New lines are appended without touching the user's
//! selection, and the view only follows the newest line while it already sits at the bottom.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, InvalidateRect, SelectObject, SetBkColor, SetTextColor, DT_CENTER, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetScrollInfo, IsWindowVisible, SendMessageW, SetWindowPos, ShowWindow, SB_VERT, SCROLLINFO,
    SIF_ALL, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_VISIBLE, WS_VSCROLL,
};

use crate::gfx::*;
use crate::logstyle::{self, Kind};
use crate::ui_types::{Action, Ctx, React};

const PAGE_PAD: f32 = 28.0; // space between the page content and the window edges
const PAGE_TOP: f32 = 22.0;
const TITLE_H: f32 = 44.0;
const TOOLBAR_H: f32 = 44.0;
const CARD_RADIUS: f32 = 10.0;
const MAX_ENTRIES: usize = 3000;
const DROP_ENTRIES: usize = 500;

// raw Win32 constants (kept as plain numbers so no extra type imports are needed)
const WM_SETREDRAW: u32 = 0x000B;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_SETFONT: u32 = 0x0030;
const WM_VSCROLL: u32 = 0x0115;
const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EM_LINESCROLL: u32 = 0x00B6;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const EM_SETREADONLY: u32 = 0x00CF;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_EXLIMITTEXT: u32 = 0x0435; // Rich Edit
const EM_SETBKGNDCOLOR: u32 = 0x0443; // Rich Edit
const EM_SETCHARFORMAT: u32 = 0x0444; // Rich Edit
const SCF_DEFAULT: usize = 0;
const SCF_SELECTION: usize = 1;
const CFM_COLOR: u32 = 0x4000_0000;
const EM_SETMARGINS: u32 = 0x00D3;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_NOHIDESEL: u32 = 0x0100;
const ES_READONLY: u32 = 0x0800;

pub fn kind_color(kind: Kind) -> Rgb {
    match kind {
        Kind::Error => (255, 107, 107),
        Kind::Warn => (255, 180, 84),
        Kind::Success => (126, 231, 135),
        Kind::Patch => (210, 168, 255),
        Kind::Login => (86, 212, 221),
        Kind::System => (121, 192, 255),
        Kind::Dim => (139, 148, 158),
        Kind::Normal => C_TEXT,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Errors,
    Warnings,
    Success,
    Patch,
    Login,
    System,
}

/// (label, filter, chip width in 96-dpi px, colour dot)
const CHIPS: [(&str, Filter, f32, Option<Kind>); 7] = [
    ("All", Filter::All, 48.0, None),
    ("Errors", Filter::Errors, 78.0, Some(Kind::Error)),
    ("Warnings", Filter::Warnings, 90.0, Some(Kind::Warn)),
    ("Success", Filter::Success, 84.0, Some(Kind::Success)),
    ("Patch", Filter::Patch, 72.0, Some(Kind::Patch)),
    ("Login", Filter::Login, 72.0, Some(Kind::Login)),
    ("System", Filter::System, 80.0, Some(Kind::System)),
];

fn matches(f: Filter, k: Kind) -> bool {
    match f {
        Filter::All => true,
        Filter::Errors => k == Kind::Error,
        Filter::Warnings => k == Kind::Warn,
        Filter::Success => k == Kind::Success,
        Filter::Patch => k == Kind::Patch,
        Filter::Login => k == Kind::Login,
        Filter::System => k == Kind::System,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hot {
    None,
    Chip(usize),
    Copy,
    Export,
}

/// Win32 CHARFORMATW (92 bytes): only the colour is used.
#[repr(C)]
struct CharFormatW {
    cb_size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    text_color: u32,
    charset: u8,
    pitch: u8,
    face: [u16; 32],
}

/// Sets the text colour for new text (SCF_SELECTION = at the insertion point) or the default (SCF_DEFAULT).
unsafe fn set_text_color(h: HWND, which: usize, c: Rgb) {
    let cf = CharFormatW {
        cb_size: std::mem::size_of::<CharFormatW>() as u32,
        mask: CFM_COLOR,
        effects: 0,
        height: 0,
        offset: 0,
        text_color: rgb(c).0,
        charset: 0,
        pitch: 0,
        face: [0; 32],
    };
    send(h, EM_SETCHARFORMAT, which, &cf as *const CharFormatW as isize);
}

struct Entry {
    raw: String,   // the full original line (with date) - what Copy / Export write
    shown: String, // "HH:MM:SS  message" - what the text box shows
    kind: Kind,
}

pub struct LogsPane {
    entries: Vec<Entry>,
    filter: Filter,
    edit: HWND,
    rich: bool, // false = plain EDIT fallback (no colours)
    placed: (i32, i32, i32, i32),
    shown_lines: usize, // how many lines the text box currently holds
    stale: bool,        // the text box is out of date (it was hidden while lines arrived)
    hot: Hot,
    toolbar_dirty: bool,
}

// ---------------- geometry ----------------
/// The page content, inset from the edges of the tab area.
fn inner_rect(area: &RECT, sc: f32) -> RECT {
    rect(area.left + px(PAGE_PAD, sc), area.top + px(PAGE_TOP, sc), area.right - px(PAGE_PAD, sc), area.bottom - px(PAGE_PAD, sc))
}

fn title_rect(area: &RECT, sc: f32) -> RECT {
    let i = inner_rect(area, sc);
    rect(i.left, i.top, i.right, i.top + px(TITLE_H, sc))
}

fn toolbar_rect(area: &RECT, sc: f32) -> RECT {
    let i = inner_rect(area, sc);
    let top = i.top + px(TITLE_H, sc);
    rect(i.left, top, i.right, top + px(TOOLBAR_H, sc))
}

/// The card behind the text box.
fn body_rect(area: &RECT, sc: f32) -> RECT {
    let i = inner_rect(area, sc);
    rect(i.left, i.top + px(TITLE_H + TOOLBAR_H + 8.0, sc), i.right, i.bottom)
}

/// The text box itself: the card minus a little padding.
fn edit_rect(area: &RECT, sc: f32) -> RECT {
    let b = body_rect(area, sc);
    rect(b.left + px(6.0, sc), b.top + px(8.0, sc), b.right - px(6.0, sc), b.bottom - px(8.0, sc))
}

fn chip_rects(area: &RECT, sc: f32) -> Vec<RECT> {
    let tb = toolbar_rect(area, sc);
    let h = px(30.0, sc);
    let top = tb.top + (tb.bottom - tb.top - h) / 2;
    let mut x = tb.left;
    let mut out = Vec::with_capacity(CHIPS.len());
    for (_, _, w, _) in CHIPS.iter() {
        out.push(rect(x, top, x + px(*w, sc), top + h));
        x += px(*w + 8.0, sc);
    }
    out
}

/// (copy button, export button)
fn button_rects(area: &RECT, sc: f32) -> (RECT, RECT) {
    let tb = toolbar_rect(area, sc);
    let h = px(30.0, sc);
    let top = tb.top + (tb.bottom - tb.top - h) / 2;
    let export = rect(tb.right - px(78.0, sc), top, tb.right, top + h);
    let copy = rect(export.left - px(8.0, sc) - px(68.0, sc), top, export.left - px(8.0, sc), top + h);
    (copy, export)
}

// ---------------- tiny Win32 helpers ----------------
unsafe fn send(h: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    SendMessageW(h, msg, WPARAM(wparam), LPARAM(lparam)).0
}

fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().filter(|&c| c != 0).collect();
    v.push(0);
    v
}

static LOG_BRUSH: AtomicIsize = AtomicIsize::new(0);

/// WM_CTLCOLOREDIT / WM_CTLCOLORSTATIC for the text box: dark background, light text.
/// Returns the brush the control uses to paint its background.
pub unsafe fn ctl_color(hdc: HDC) -> LRESULT {
    SetTextColor(hdc, rgb(C_TEXT));
    SetBkColor(hdc, rgb(C_LOG_BG));
    let mut b = LOG_BRUSH.load(Ordering::Relaxed);
    if b == 0 {
        let brush = CreateSolidBrush(rgb(C_LOG_BG));
        b = brush.0 as isize;
        LOG_BRUSH.store(b, Ordering::Relaxed);
    }
    LRESULT(b)
}

impl LogsPane {
    pub fn new() -> LogsPane {
        LogsPane {
            entries: Vec::new(),
            filter: Filter::All,
            edit: HWND::default(),
            rich: false,
            placed: (0, 0, 0, 0),
            shown_lines: 0,
            stale: true,
            hot: Hot::None,
            toolbar_dirty: true,
        }
    }

    // ---------------- the text box ----------------
    /// Creates the text box as a child of the main window (call once, after the fonts exist).
    pub fn attach(&mut self, parent: HWND, ctx: &Ctx) {
        unsafe {
            let instance = GetModuleHandleW(None).expect("GetModuleHandleW");
            let _ = LoadLibraryW(w!("Msftedit.dll")); // provides the RICHEDIT50W class
            let style = WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_CLIPSIBLINGS
                | WINDOW_STYLE(ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY | ES_NOHIDESEL);
            let make = |class: PCWSTR| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class,
                    PCWSTR::null(),
                    style,
                    0,
                    0,
                    10,
                    10,
                    parent,
                    None,
                    instance,
                    None,
                )
            };
            let (edit, rich) = match make(w!("RICHEDIT50W")) {
                Ok(h) => (h, true),
                Err(_) => match make(w!("EDIT")) {
                    Ok(h) => (h, false),
                    Err(_) => return,
                },
            };
            // dark scrollbar (Windows 10 1809+; older versions just keep the light one)
            let _ = SetWindowTheme(edit, w!("DarkMode_Explorer"), PCWSTR::null());
            if rich {
                send(edit, EM_EXLIMITTEXT, 0, 0x7FFF_FFF0);
                send(edit, EM_SETBKGNDCOLOR, 0, rgb(C_LOG_BG).0 as isize);
            } else {
                send(edit, EM_SETLIMITTEXT, 0x3FFF_FFFF, 0);
            }
            self.edit = edit;
            self.rich = rich;
        }
        self.apply_font(ctx);
    }

    /// (Re)applies the monospaced font and the side margins - after creation and after a DPI change.
    pub fn apply_font(&self, ctx: &Ctx) {
        if self.edit.is_invalid() {
            return;
        }
        unsafe {
            send(self.edit, WM_SETFONT, ctx.fonts.mono.0 as usize, 1);
            let m = px(10.0, ctx.scale) as isize;
            send(self.edit, EM_SETMARGINS, 3, (m << 16) | m); // EC_LEFTMARGIN | EC_RIGHTMARGIN
            if self.rich {
                set_text_color(self.edit, SCF_DEFAULT, C_TEXT);
            }
        }
    }

    /// Shown only while the Live Logs tab is the active one.
    pub fn set_visible(&mut self, on: bool) {
        if self.edit.is_invalid() {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.edit, if on { SW_SHOW } else { SW_HIDE });
        }
        if on && self.stale && self.is_visible() {
            self.rebuild();
        }
    }

    fn is_visible(&self) -> bool {
        !self.edit.is_invalid() && unsafe { IsWindowVisible(self.edit).as_bool() }
    }

    /// True when the view is at (or within a line of) the bottom, or there is nothing to scroll.
    unsafe fn at_bottom(&self) -> bool {
        let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
        if GetScrollInfo(self.edit, SB_VERT, &mut si).is_err() {
            return true;
        }
        si.nPos + si.nPage as i32 >= si.nMax
    }

    /// Line separator of the control (Rich Edit uses a bare \r, a plain EDIT needs \r\n).
    fn nl(&self) -> &'static str {
        if self.rich {
            "\r"
        } else {
            "\r\n"
        }
    }

    /// Inserts one line at the caret (the end), in the colour of its kind.
    unsafe fn put_line(&self, text: &str, kind: Kind, first: bool) {
        let h = self.edit;
        if self.rich {
            set_text_color(h, SCF_SELECTION, kind_color(kind));
        }
        let s = if first { text.to_string() } else { format!("{}{}", self.nl(), text) };
        let w = wide(&s);
        send(h, EM_REPLACESEL, 0, w.as_ptr() as isize);
    }

    /// Replaces the whole text with the lines that pass the filter and jumps to the newest one.
    fn rebuild(&mut self) {
        if self.edit.is_invalid() {
            return;
        }
        let mut n = 0usize;
        unsafe {
            let h = self.edit;
            send(h, WM_SETREDRAW, 0, 0);
            send(h, EM_SETREADONLY, 0, 0); // programmatic edits only while unlocked
            let empty = wide("");
            send(h, WM_SETTEXT, 0, empty.as_ptr() as isize);
            for e in self.entries.iter().filter(|e| matches(self.filter, e.kind)) {
                self.put_line(&e.shown, e.kind, n == 0);
                n += 1;
            }
            let len = send(h, WM_GETTEXTLENGTH, 0, 0);
            send(h, EM_SETSEL, len as usize, len);
            send(h, EM_SCROLLCARET, 0, 0);
            send(h, EM_SETREADONLY, 1, 0);
            send(h, WM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(h, None, true);
        }
        self.shown_lines = n;
        self.stale = false;
    }

    /// Adds lines at the end without disturbing the user's selection or scroll position.
    fn append(&mut self, items: &[(String, Kind)]) {
        unsafe {
            let h = self.edit;
            let follow = self.at_bottom();
            let first = send(h, EM_GETFIRSTVISIBLELINE, 0, 0);
            let (mut sel_start, mut sel_end) = (0u32, 0u32);
            send(h, EM_GETSEL, &mut sel_start as *mut u32 as usize, &mut sel_end as *mut u32 as isize);
            send(h, WM_SETREDRAW, 0, 0);
            send(h, EM_SETREADONLY, 0, 0); // programmatic edits only while unlocked
            let len = send(h, WM_GETTEXTLENGTH, 0, 0);
            send(h, EM_SETSEL, len as usize, len);
            for (i, (text, kind)) in items.iter().enumerate() {
                self.put_line(text, *kind, self.shown_lines == 0 && i == 0);
            }
            if follow {
                let len2 = send(h, WM_GETTEXTLENGTH, 0, 0);
                send(h, EM_SETSEL, len2 as usize, len2);
                send(h, EM_SCROLLCARET, 0, 0);
            } else {
                // the user scrolled up: keep the view exactly where it was
                let now = send(h, EM_GETFIRSTVISIBLELINE, 0, 0);
                if now != first {
                    send(h, EM_LINESCROLL, 0, first - now);
                }
            }
            // put the selection back (EM_SETSEL itself never scrolls)
            send(h, EM_SETSEL, sel_start as usize, sel_end as isize);
            send(h, EM_SETREADONLY, 1, 0);
            send(h, WM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(h, None, true);
        }
    }

    // ---------------- content ----------------
    pub fn push(&mut self, lines: &[String]) {
        let mut added: Vec<(String, Kind)> = Vec::new();
        for line in lines {
            let p = logstyle::parse_line(line);
            let kind = p.kind;
            let shown = if p.time.is_empty() { p.message } else { format!("{}  {}", p.time, p.message) };
            if matches(self.filter, kind) {
                added.push((shown.clone(), kind));
            }
            self.entries.push(Entry { raw: line.clone(), shown, kind });
        }
        let trimmed = self.entries.len() > MAX_ENTRIES;
        if trimmed {
            let drop = self.entries.len() - (MAX_ENTRIES - DROP_ENTRIES);
            self.entries.drain(..drop);
        }
        if !self.is_visible() {
            self.stale = true; // brought up to date the next time the text box is shown
        } else if trimmed || self.stale {
            self.rebuild();
        } else if !added.is_empty() {
            self.append(&added);
            self.shown_lines += added.len();
        }
    }

    /// Positions the text box inside the tab area (only when it actually moved) and refreshes it if needed.
    pub fn relayout(&mut self, ctx: &Ctx, area: &RECT) {
        if self.edit.is_invalid() {
            return;
        }
        let r = edit_rect(area, ctx.scale);
        let placed = (r.left, r.top, r.right, r.bottom);
        if placed != self.placed {
            self.placed = placed;
            unsafe {
                let _ = SetWindowPos(
                    self.edit,
                    None,
                    r.left,
                    r.top,
                    (r.right - r.left).max(1),
                    (r.bottom - r.top).max(1),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        if self.stale && self.is_visible() {
            self.rebuild();
        }
    }

    /// Jump to the newest line (used when the window is shown again).
    pub fn snap_to_end(&mut self) {
        if !self.is_visible() {
            return;
        }
        unsafe {
            let len = send(self.edit, WM_GETTEXTLENGTH, 0, 0);
            send(self.edit, EM_SETSEL, len as usize, len);
            send(self.edit, EM_SCROLLCARET, 0, 0);
        }
    }

    /// The lines currently shown (respecting the filter), one per row, as originally logged.
    pub fn visible_text(&self) -> String {
        self.entries
            .iter()
            .filter(|e| matches(self.filter, e.kind))
            .map(|e| e.raw.as_str())
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    pub fn visible_count(&self) -> usize {
        self.entries.iter().filter(|e| matches(self.filter, e.kind)).count()
    }

    // ---------------- drawing (title + toolbar + the card behind the text box) ----------------
    /// `full` = draw everything (tab switch, resize); otherwise only the toolbar if it changed.
    pub fn render(&mut self, ctx: &Ctx, dc: HDC, area: &RECT, full: bool) {
        let sc = ctx.scale;
        let body = body_rect(area, sc);
        unsafe {
            if full {
                fill(dc, area, C_BG);
            }
            if full || self.toolbar_dirty {
                fill(dc, &rect(area.left, area.top, area.right, body.top), C_BG);
                self.draw_header(ctx, dc, area);
                self.toolbar_dirty = false;
            }
            if full {
                card(dc, &body, C_LOG_BG, C_BORDER, px(CARD_RADIUS, sc));
            }
        }
    }

    /// Page title + toolbar (filter chips, Copy, Export).
    unsafe fn draw_header(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let sc = ctx.scale;
        let old = SelectObject(dc, HGDIOBJ(ctx.fonts.page.0));
        let mut tr = title_rect(area, sc);
        put_text(dc, "Live Logs", &mut tr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
        self.draw_toolbar(ctx, dc, area);
    }

    unsafe fn draw_toolbar(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let sc = ctx.scale;
        let old = SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));

        let chips = chip_rects(area, sc);
        for (i, ((label, f, _, kind), r)) in CHIPS.iter().zip(chips.iter()).enumerate() {
            let active = self.filter == *f;
            let bg = if active { (30, 58, 84) } else if self.hot == Hot::Chip(i) { C_PILL_HOT } else { C_PILL };
            fill_round(dc, r, bg, px(15.0, sc));
            let mut tx = r.left + px(13.0, sc);
            if let Some(k) = kind {
                let cy = (r.top + r.bottom) / 2;
                let rad = px(4.0, sc);
                fill_round(dc, &rect(tx, cy - rad, tx + rad * 2, cy + rad), kind_color(*k), rad);
                tx += px(15.0, sc);
            }
            let mut tr = rect(tx, r.top, r.right - px(6.0, sc), r.bottom);
            put_text(dc, label, &mut tr, if active { C_ACCENT } else { C_TEXT }, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }

        let (copy, export) = button_rects(area, sc);
        for (r, label, hot) in [(copy, "Copy", Hot::Copy), (export, "Export", Hot::Export)] {
            fill_round(dc, &r, if self.hot == hot { C_PILL_HOT } else { C_PILL }, px(8.0, sc));
            let mut tr = r;
            put_text(dc, label, &mut tr, C_TEXT, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }
        SelectObject(dc, old);
    }

    // ---------------- animation hook (kept for the main window; nothing animates any more) ----------------
    pub fn anim_step(&mut self) -> (bool, bool) {
        (false, false)
    }

    // ---------------- input ----------------
    fn hit(&self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> Hot {
        let sc = ctx.scale;
        for (i, r) in chip_rects(area, sc).iter().enumerate() {
            if inside(r, x, y) {
                return Hot::Chip(i);
            }
        }
        let (copy, export) = button_rects(area, sc);
        if inside(&copy, x, y) {
            Hot::Copy
        } else if inside(&export, x, y) {
            Hot::Export
        } else {
            Hot::None
        }
    }

    pub fn mouse_move(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        let hot = self.hit(ctx, area, x, y);
        let mut r = React::default();
        if hot != self.hot {
            self.hot = hot;
            self.toolbar_dirty = true;
            r.redraw = true;
        }
        r
    }

    pub fn leave(&mut self) -> React {
        let mut r = React::default();
        if self.hot != Hot::None {
            self.hot = Hot::None;
            self.toolbar_dirty = true;
            r.redraw = true;
        }
        r
    }

    pub fn mouse_down(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        let mut r = React::default();
        match self.hit(ctx, area, x, y) {
            Hot::Chip(i) => {
                self.filter = CHIPS[i].1;
                if self.is_visible() {
                    self.rebuild();
                } else {
                    self.stale = true;
                }
                self.toolbar_dirty = true;
                r.redraw = true;
            }
            Hot::Copy => r.action = Some(Action::CopyLogs),
            Hot::Export => r.action = Some(Action::ExportLogs),
            Hot::None => {}
        }
        r
    }

    /// Wheel over the title / toolbar area: scroll the text box too.
    pub fn wheel(&mut self, _ctx: &Ctx, notches: f32) -> React {
        if self.is_visible() {
            unsafe {
                send(self.edit, EM_LINESCROLL, 0, (-notches * 3.0).round() as isize);
            }
        }
        React::default()
    }

    /// Keys while the main window (not the text box) has the focus: scroll the text box.
    pub fn key(&mut self, _ctx: &Ctx, vk: u32) -> React {
        let code: usize = match vk {
            0x21 => 2, // Page Up    -> SB_PAGEUP
            0x22 => 3, // Page Down  -> SB_PAGEDOWN
            0x24 => 6, // Home       -> SB_TOP
            0x23 => 7, // End        -> SB_BOTTOM
            0x26 => 0, // Up         -> SB_LINEUP
            0x28 => 1, // Down       -> SB_LINEDOWN
            _ => return React::default(),
        };
        if self.is_visible() {
            unsafe {
                send(self.edit, WM_VSCROLL, code, 0);
            }
        }
        React::default()
    }
}
