//! The "Settings" tab: preset dropdown, import/export, and a slider / switch / choice for every value,
//! with a hover tooltip on everything. It only repaints on mouse/keyboard events (no timers, no idle cost).
//!
//! Preset values (SCANNING, STARTUP, DETECTION, SYSTEM) can only be edited in the Custom preset.
//! General values (AUTO LOGIN, SAFETY & LOGS) are always editable and do not depend on the preset.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    DrawTextW, IntersectClipRect, SelectClipRgn, SelectObject, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_TOP, DT_VCENTER, DT_WORDBREAK, HDC, HGDIOBJ,
};

use crate::gfx::*;
use crate::languages;
use crate::settings::{self, General, Param, Preset, Unit, Values, GENERAL_PARAMS, PARAMS};
use crate::ui_types::{Action, Ctx, React};

const PAGE_PAD: f32 = 28.0; // space between the page content and the window edges
const PAGE_TOP: f32 = 22.0;
const TITLE_H: f32 = 44.0;
const PRESET_H: f32 = 128.0; // the preset card
const CARD_PAD: f32 = 22.0; // padding inside a card
const CARD_RADIUS: f32 = 10.0;
const HEADING_H: f32 = 40.0;
const ROW_H: f32 = 52.0;
const SECTION_GAP: f32 = 14.0;

// ---------------- rows ----------------
#[derive(Clone, Copy)]
enum Row {
    /// (text, belongs to the preset-bound values)
    Heading(&'static str, bool),
    Bound(usize),
    Gen(usize),
    /// the "Game language" dropdown
    Language,
}

const LANGUAGE_TIP: &str = "The language the game is set to. It decides which texts APR looks for (login screen, patch notice, user ID label) and which Windows OCR language reads the screen. The list comes from languages.json - Misc > Open languages file to edit it. A red dot means the Windows OCR language pack for it is not installed.";

const ROWS: [Row; 41] = [
    Row::Heading("GAME", false),
    Row::Language,
    Row::Heading("AUTO LOGIN", false),
    Row::Gen(settings::AUTO_LOGIN),
    Row::Gen(settings::AUTO_LOGIN_DELAY),
    Row::Gen(settings::AUTO_LOGIN_METHOD),
    Row::Heading("NOTIFICATION", false),
    Row::Gen(settings::OVERLAY_ON),
    Row::Gen(settings::OVERLAY_SIZE),
    Row::Gen(settings::OVERLAY_OPACITY),
    Row::Gen(settings::OVERLAY_V),
    Row::Gen(settings::OVERLAY_H),
    Row::Gen(settings::OVERLAY_MARGIN),
    Row::Heading("SAFETY", false),
    Row::Gen(settings::GUARD_MAX),
    Row::Gen(settings::GUARD_MINUTES),
    Row::Heading("DEBUGGING", false),
    Row::Gen(settings::LOG_MAX_MB),
    Row::Gen(settings::KEEP_LOGS),
    Row::Gen(settings::FULL_OCR_LOG),
    Row::Gen(settings::SAVE_SCREENSHOTS),
    Row::Gen(settings::SCREENSHOT_EVERY),
    Row::Gen(settings::KEEP_SCREENSHOTS),
    Row::Heading("SCANNING", true),
    Row::Bound(settings::SCAN_INTERVAL),
    Row::Bound(settings::IDLE_INTERVAL),
    Row::Bound(settings::BURST_INTERVAL),
    Row::Bound(settings::IDLE_AFTER),
    Row::Bound(settings::NEAR_MISS_BURSTS),
    Row::Bound(settings::OCR_HEIGHT),
    Row::Heading("STARTUP & RESTART", true),
    Row::Bound(settings::OCR_START_DELAY),
    Row::Bound(settings::LAUNCH_EXTRA_DELAY),
    Row::Bound(settings::STOP_STABLE),
    Row::Bound(settings::STOP_TIMEOUT),
    Row::Heading("DETECTION", true),
    Row::Bound(settings::CONFIRMATIONS),
    Row::Bound(settings::LOGIN_THRESHOLD),
    Row::Bound(settings::PATCH_THRESHOLD),
    Row::Heading("SYSTEM", true),
    Row::Bound(settings::LOW_PRIORITY),
];

struct View {
    param: &'static Param,
    value: u32,
    enabled: bool,
}

fn view_of(row: Row, vals: &Values, gen: &General, custom: bool) -> Option<View> {
    match row {
        Row::Bound(i) => Some(View { param: &PARAMS[i], value: vals.0[i], enabled: custom }),
        Row::Gen(j) => Some(View { param: &GENERAL_PARAMS[j], value: gen.0[j], enabled: true }),
        Row::Heading(..) | Row::Language => None,
    }
}

fn set_row_value(row: Row, v: u32) {
    match row {
        Row::Bound(i) => settings::set_custom_value(i, v),
        Row::Gen(j) => settings::set_general_value(j, v),
        Row::Heading(..) | Row::Language => {}
    }
}

/// True when this row is one of the notification settings.
fn is_overlay_row(row: Row) -> bool {
    row_param(row).map_or(false, |p| p.key.starts_with("overlay_"))
}

/// Current stored value of a general-settings row.
fn row_value(row: Row) -> Option<u32> {
    match row {
        Row::Gen(j) => Some(settings::general().0[j]),
        _ => None,
    }
}

fn row_param(row: Row) -> Option<&'static Param> {
    match row {
        Row::Bound(i) => Some(&PARAMS[i]),
        Row::Gen(j) => Some(&GENERAL_PARAMS[j]),
        Row::Heading(..) | Row::Language => None,
    }
}

/// (row, top offset inside the list, height). Sections are separated by a gap.
fn row_metrics(sc: f32) -> Vec<(Row, i32, i32)> {
    let mut y = px(4.0, sc);
    let mut out = Vec::with_capacity(ROWS.len());
    for (i, r) in ROWS.iter().enumerate() {
        let h = match r {
            Row::Heading(..) => {
                if i > 0 {
                    y += px(SECTION_GAP, sc);
                }
                px(HEADING_H, sc)
            }
            _ => px(ROW_H, sc),
        };
        out.push((*r, y, h));
        y += h;
    }
    out
}

/// One card per section: (top, bottom) offsets inside the list.
fn card_spans(sc: f32) -> Vec<(i32, i32)> {
    let mut spans = Vec::new();
    let mut start: Option<i32> = None;
    let mut end = 0;
    for (row, top, h) in row_metrics(sc) {
        match row {
            Row::Heading(..) => {
                if let Some(s) = start.take() {
                    spans.push((s, end));
                }
            }
            _ => {
                start.get_or_insert(top);
                end = top + h;
            }
        }
    }
    if let Some(s) = start {
        spans.push((s, end));
    }
    spans
}

fn content_height(sc: f32) -> i32 {
    row_metrics(sc).last().map(|(_, top, h)| top + h).unwrap_or(0) + px(PAGE_PAD, sc)
}

// ---------------- state ----------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hot {
    None,
    Dropdown,
    DropItem(usize),
    Slider(usize),
    Toggle(usize),
    Choice(usize, usize),
    Row(usize),
    Reset,
    Import,
    Export,
    LangBox,
    LangItem(usize),
}

pub struct SettingsPane {
    scroll: i32,
    hot: Hot,
    drag: Option<usize>,
    dropdown_open: bool,
    lang_open: bool,
    mouse: (i32, i32),
}

// ---------------- geometry ----------------
struct Layout {
    title: RECT,
    panel: RECT,
    dropdown: RECT,
    reset: RECT,
    import: RECT,
    export: RECT,
    list: RECT,
}

fn layout(area: &RECT, sc: f32) -> Layout {
    let pad = px(PAGE_PAD, sc);
    let inner_l = area.left + pad;
    let inner_r = area.right - pad;
    let title = rect(inner_l, area.top + px(PAGE_TOP, sc), inner_r, area.top + px(PAGE_TOP, sc) + px(TITLE_H, sc));
    let panel = rect(inner_l, title.bottom, inner_r, title.bottom + px(PRESET_H, sc));
    let cp = px(CARD_PAD, sc);
    let (dw, dh) = (px(250.0, sc), px(34.0, sc));
    let dropdown = rect(panel.right - cp - dw, panel.top + px(20.0, sc), panel.right - cp, panel.top + px(20.0, sc) + dh);
    let (bw, bh, gap) = (px(78.0, sc), px(28.0, sc), px(8.0, sc));
    let by = dropdown.bottom + px(16.0, sc);
    let export = rect(panel.right - cp - bw, by, panel.right - cp, by + bh);
    let import = rect(export.left - gap - bw, by, export.left - gap, by + bh);
    let reset = rect(import.left - gap - bw, by, import.left - gap, by + bh);
    let list = rect(area.left, panel.bottom + px(10.0, sc), area.right, area.bottom);
    Layout { title, panel, dropdown, reset, import, export, list }
}

fn drop_item(lay: &Layout, i: usize, sc: f32) -> RECT {
    let ih = px(34.0, sc);
    let top = lay.dropdown.bottom + px(6.0, sc) + i as i32 * ih;
    rect(lay.dropdown.left, top, lay.dropdown.right, top + ih)
}

struct Geom {
    label: RECT,
    track_l: i32,
    track_r: i32,
    cy: i32,
    value: RECT,
}

fn geom(area: &RECT, sc: f32, top: i32, rh: i32) -> Geom {
    let inset = px(PAGE_PAD + CARD_PAD, sc); // page padding + card padding
    let w = area.right - area.left;
    let label_w = ((w as f32 * 0.32) as i32).clamp(px(170.0, sc), px(250.0, sc));
    let value_w = px(92.0, sc);
    let track_l = area.left + inset + label_w + px(14.0, sc);
    let track_r = (area.right - inset - value_w - px(14.0, sc)).max(track_l + px(60.0, sc));
    Geom {
        label: rect(area.left + inset, top, area.left + inset + label_w, top + rh),
        track_l,
        track_r,
        cy: top + rh / 2,
        value: rect(area.right - inset - value_w, top, area.right - inset, top + rh),
    }
}

fn switch_rect(g: &Geom, sc: f32) -> RECT {
    rect(g.track_l, g.cy - px(10.0, sc), g.track_l + px(40.0, sc), g.cy + px(10.0, sc))
}

fn choice_rects(g: &Geom, sc: f32, n: usize) -> Vec<RECT> {
    let seg_w = (px(124.0, sc)).min((g.track_r - g.track_l) / n.max(1) as i32);
    (0..n)
        .map(|i| rect(g.track_l + i as i32 * seg_w, g.cy - px(14.0, sc), g.track_l + (i as i32 + 1) * seg_w, g.cy + px(14.0, sc)))
        .collect()
}

fn slider_value_from_x(p: &Param, x: i32, g: &Geom) -> u32 {
    let ratio = ((x - g.track_l) as f32 / (g.track_r - g.track_l).max(1) as f32).clamp(0.0, 1.0);
    (p.min as f32 + ratio * (p.max - p.min) as f32).round() as u32
}

fn tip_text(hot: Hot) -> Option<&'static str> {
    match hot {
        Hot::Dropdown => Some(settings::PRESET_TIP),
        Hot::DropItem(i) => Some(Preset::ALL[i].blurb()),
        Hot::Row(r) if matches!(ROWS[r], Row::Language) => Some(LANGUAGE_TIP),
        Hot::LangBox | Hot::LangItem(_) => Some(LANGUAGE_TIP),
        Hot::Slider(r) | Hot::Toggle(r) | Hot::Choice(r, _) | Hot::Row(r) => row_param(ROWS[r]).map(|p| p.tip),
        Hot::Reset => Some(settings::RESET_TIP),
        Hot::Import => Some(settings::IMPORT_TIP),
        Hot::Export => Some(settings::EXPORT_TIP),
        Hot::None => None,
    }
}

// ---------------- language dropdown ----------------
/// The dropdown box inside the language row.
fn lang_box(g: &Geom, sc: f32) -> RECT {
    let w = px(300.0, sc).min((g.track_r - g.track_l).max(px(160.0, sc)));
    rect(g.track_l, g.cy - px(17.0, sc), g.track_l + w, g.cy + px(17.0, sc))
}

/// The open list under (or, near the bottom, above) the box: (outer rect, item rects).
fn lang_overlay(area: &RECT, sc: f32, bx: &RECT, n: usize) -> (RECT, Vec<RECT>) {
    let ih = px(34.0, sc);
    let total = n as i32 * ih + px(8.0, sc);
    let below = bx.bottom + px(4.0, sc);
    let top = if below + total <= area.bottom { below } else { (bx.top - px(4.0, sc) - total).max(area.top) };
    let outer = rect(bx.left, top, bx.right, top + total);
    let items = (0..n as i32).map(|i| rect(bx.left, top + px(4.0, sc) + i * ih, bx.right, top + px(4.0, sc) + (i + 1) * ih)).collect();
    (outer, items)
}

/// (status text, colour) for the selected language, shown under the row label.
fn lang_status(l: &languages::Language, fell_back: bool) -> (String, Rgb) {
    if fell_back {
        return ("Not in languages.json - using English".to_string(), C_WARN);
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut color = C_DIM;
    match languages::pack_installed(&l.ocr) {
        Some(true) => parts.push("OCR ready"),
        Some(false) if l.mode == languages::MatchMode::Chars => {
            parts.push("OCR pack missing - install it in Windows");
            color = C_ERR;
        }
        Some(false) => {
            parts.push("OCR pack missing - English OCR is used");
            color = C_WARN;
        }
        None => parts.push("checking OCR..."),
    }
    if l.patch.is_empty() {
        parts.push("no patch text - patch detection off");
        if color == C_DIM {
            color = C_WARN;
        }
    }
    (parts.join("  -  "), color)
}

impl SettingsPane {
    pub fn new() -> SettingsPane {
        SettingsPane { scroll: 0, hot: Hot::None, drag: None, dropdown_open: false, lang_open: false, mouse: (0, 0) }
    }

    pub fn dropdown_open(&self) -> bool {
        self.dropdown_open
    }

    pub fn tooltip(&self) -> Option<&'static str> {
        if self.drag.is_some() {
            None
        } else {
            tip_text(self.hot)
        }
    }

    pub fn max_scroll(&self, ctx: &Ctx, area: &RECT) -> i32 {
        let lay = layout(area, ctx.scale);
        (content_height(ctx.scale) - (lay.list.bottom - lay.list.top)).max(0)
    }

    pub fn clamp_scroll(&mut self, ctx: &Ctx, area: &RECT) {
        self.scroll = self.scroll.clamp(0, self.max_scroll(ctx, area));
    }

    /// (top, height) of the language row in client coordinates.
    fn language_row(&self, ctx: &Ctx, area: &RECT) -> Option<(i32, i32)> {
        let lay = layout(area, ctx.scale);
        row_metrics(ctx.scale)
            .into_iter()
            .find(|(r, _, _)| matches!(r, Row::Language))
            .map(|(_, top, rh)| (lay.list.top + top - self.scroll, rh))
    }

    /// Geometry of the row at index `row`, in client coordinates.
    fn geom_for(&self, ctx: &Ctx, area: &RECT, row: usize) -> Option<Geom> {
        let lay = layout(area, ctx.scale);
        row_metrics(ctx.scale)
            .get(row)
            .map(|(_, top, rh)| geom(area, ctx.scale, lay.list.top + top - self.scroll, *rh))
    }

    fn hit(&self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> Hot {
        let sc = ctx.scale;
        let lay = layout(area, sc);
        if self.lang_open {
            if let Some((ry, rh)) = self.language_row(ctx, area) {
                let bx = lang_box(&geom(area, sc, ry, rh), sc);
                let (_, items) = lang_overlay(area, sc, &bx, languages::list().len());
                for (i, r) in items.iter().enumerate() {
                    if inside(r, x, y) {
                        return Hot::LangItem(i);
                    }
                }
                return if inside(&bx, x, y) { Hot::LangBox } else { Hot::None };
            }
        }
        if self.dropdown_open {
            for i in 0..Preset::ALL.len() {
                if inside(&drop_item(&lay, i, sc), x, y) {
                    return Hot::DropItem(i);
                }
            }
            return if inside(&lay.dropdown, x, y) { Hot::Dropdown } else { Hot::None };
        }
        if inside(&lay.dropdown, x, y) {
            return Hot::Dropdown;
        }
        if inside(&lay.reset, x, y) {
            return Hot::Reset;
        }
        if inside(&lay.import, x, y) {
            return Hot::Import;
        }
        if inside(&lay.export, x, y) {
            return Hot::Export;
        }
        if y >= lay.list.top && y < lay.list.bottom {
            for (idx, (row, top, rh)) in row_metrics(sc).into_iter().enumerate() {
                let ry = lay.list.top + top - self.scroll;
                if y < ry || y >= ry + rh {
                    continue;
                }
                if let Row::Language = row {
                    let bx = lang_box(&geom(area, sc, ry, rh), sc);
                    return if inside(&bx, x, y) { Hot::LangBox } else { Hot::Row(idx) };
                }
                let Some(p) = row_param(row) else { return Hot::None };
                let g = geom(area, sc, ry, rh);
                return match p.unit {
                    Unit::Toggle => {
                        let sw = switch_rect(&g, sc);
                        let grown = rect(sw.left - 6, sw.top - 6, sw.right + 6, sw.bottom + 6);
                        if inside(&grown, x, y) { Hot::Toggle(idx) } else { Hot::Row(idx) }
                    }
                    Unit::Choice => {
                        match choice_rects(&g, sc, p.options.len()).iter().position(|r| inside(r, x, y)) {
                            Some(o) => Hot::Choice(idx, o),
                            None => Hot::Row(idx),
                        }
                    }
                    _ => {
                        let near = x >= g.track_l - px(10.0, sc)
                            && x <= g.track_r + px(10.0, sc)
                            && (y - g.cy).abs() <= px(13.0, sc);
                        if near { Hot::Slider(idx) } else { Hot::Row(idx) }
                    }
                };
            }
        }
        Hot::None
    }

    // ---------------- drawing ----------------
    pub fn render(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        unsafe {
            self.draw(ctx, dc, area);
        }
    }

    unsafe fn draw_row(&self, ctx: &Ctx, dc: HDC, area: &RECT, y: i32, rh: i32, idx: usize, v: &View) {
        let sc = ctx.scale;
        let pad = px(PAGE_PAD, sc);
        let hovered = matches!(
            self.hot,
            Hot::Row(j) | Hot::Slider(j) | Hot::Toggle(j) | Hot::Choice(j, _) if j == idx
        );
        if hovered {
            fill_round(dc, &rect(area.left + pad + px(6.0, sc), y + 3, area.right - pad - px(6.0, sc), y + rh - 3), (47, 47, 52), px(8.0, sc));
        }
        let g = geom(area, sc, y, rh);
        let p = v.param;
        SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        let mut lr = g.label;
        put_text(dc, p.label, &mut lr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);

        let (track_col, fill_col, thumb_col): (Rgb, Rgb, Rgb) = if v.enabled {
            (C_THUMB, C_ACCENT, (235, 240, 250))
        } else {
            (C_PILL, (66, 78, 100), (120, 128, 142))
        };

        match p.unit {
            Unit::Toggle => {
                let sw = switch_rect(&g, sc);
                let on = v.value != 0;
                fill_round(dc, &sw, if on { fill_col } else { track_col }, px(10.0, sc));
                let kr = px(7.0, sc);
                let kx = if on { sw.right - px(10.0, sc) } else { sw.left + px(10.0, sc) };
                fill_round(dc, &rect(kx - kr, g.cy - kr, kx + kr, g.cy + kr), thumb_col, kr);
            }
            Unit::Choice => {
                let rects = choice_rects(&g, sc, p.options.len());
                for (o, r) in rects.iter().enumerate() {
                    let selected = v.value as usize == o;
                    let hot = matches!(self.hot, Hot::Choice(j, k) if j == idx && k == o);
                    let bg = if selected {
                        if v.enabled { (33, 52, 80) } else { (30, 36, 48) }
                    } else if hot {
                        C_PILL_HOT
                    } else {
                        C_PILL
                    };
                    fill_round(dc, r, bg, px(8.0, sc));
                    let mut tr = *r;
                    let col = if selected { if v.enabled { C_ACCENT } else { C_DIM } } else { C_TEXT };
                    put_text(dc, p.options[o], &mut tr, col, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
            }
            _ => {
                let th = px(6.0, sc);
                fill_round(dc, &rect(g.track_l, g.cy - th / 2, g.track_r, g.cy + th / 2), track_col, th / 2);
                let ratio = (v.value - p.min) as f32 / (p.max - p.min).max(1) as f32;
                let tx = g.track_l + (ratio * (g.track_r - g.track_l) as f32) as i32;
                if tx > g.track_l {
                    fill_round(dc, &rect(g.track_l, g.cy - th / 2, tx, g.cy + th / 2), fill_col, th / 2);
                }
                let kr = px(8.0, sc);
                fill_round(dc, &rect(tx - kr, g.cy - kr, tx + kr, g.cy + kr), thumb_col, kr);
            }
        }

        if p.unit != Unit::Choice {
            let mut vr = g.value;
            put_text(
                dc,
                &settings::format_value(p, v.value),
                &mut vr,
                if v.enabled { C_TEXT } else { C_DIM },
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    unsafe fn draw_language_row(&self, ctx: &Ctx, dc: HDC, area: &RECT, y: i32, rh: i32, idx: usize) {
        let sc = ctx.scale;
        let pad = px(PAGE_PAD, sc);
        let hovered = matches!(self.hot, Hot::Row(j) if j == idx) || self.hot == Hot::LangBox;
        if hovered {
            fill_round(dc, &rect(area.left + pad + px(6.0, sc), y + 3, area.right - pad - px(6.0, sc), y + rh - 3), (47, 47, 52), px(8.0, sc));
        }
        let g = geom(area, sc, y, rh);
        let (l, fell_back) = languages::resolve(&settings::language_id());
        let (status, status_color) = lang_status(&l, fell_back);

        SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        let mut lr = rect(g.label.left, y + px(7.0, sc), g.label.right, y + px(27.0, sc));
        put_text(dc, "Game language", &mut lr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
        let mut sr = rect(g.label.left, y + px(27.0, sc), area.right - pad - px(22.0, sc), y + rh - px(4.0, sc));
        // the status line sits under the label and may run under the dropdown box area: keep it to the label column
        sr.right = sr.right.min(g.label.right + px(90.0, sc));
        put_text(dc, &status, &mut sr, status_color, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);

        let bx = lang_box(&g, sc);
        fill_round(dc, &bx, if self.hot == Hot::LangBox || self.lang_open { C_PILL_HOT } else { C_PILL }, px(8.0, sc));
        SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        let mut nr = rect(bx.left + px(14.0, sc), bx.top, bx.right - px(36.0, sc), bx.bottom);
        put_text(dc, &l.name, &mut nr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
        let (ax, ay) = (bx.right - px(20.0, sc), (bx.top + bx.bottom) / 2);
        let a = px(4.0, sc);
        line(dc, (ax - a, ay - a / 2), (ax, ay + a / 2), C_TEXT, px(1.5, sc).max(1));
        line(dc, (ax, ay + a / 2), (ax + a, ay - a / 2), C_TEXT, px(1.5, sc).max(1));
    }

    /// The open language list (drawn last, on top of the list).
    unsafe fn draw_language_overlay(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let sc = ctx.scale;
        let Some((ry, rh)) = self.language_row(ctx, area) else { return };
        let bx = lang_box(&geom(area, sc, ry, rh), sc);
        let langs = languages::list();
        let (outer, items) = lang_overlay(area, sc, &bx, langs.len());
        let selected = settings::language_id();
        card(dc, &outer, (34, 34, 38), C_BORDER, px(8.0, sc));
        SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        for (i, (l, item)) in langs.iter().zip(items.iter()).enumerate() {
            if self.hot == Hot::LangItem(i) {
                fill_round(dc, &rect(item.left + 4, item.top, item.right - 4, item.bottom), C_PILL_HOT, px(6.0, sc));
            }
            // dot: green = OCR pack installed, amber = Latin language read by the English OCR, red = pack missing
            let dot = match languages::pack_installed(&l.ocr) {
                Some(true) => C_OK,
                Some(false) if l.mode == languages::MatchMode::Chars => C_ERR,
                Some(false) => C_WARN,
                None => C_DIM,
            };
            let r = px(4.0, sc);
            let (dx, dy) = (item.left + px(16.0, sc), (item.top + item.bottom) / 2);
            fill_round(dc, &rect(dx - r, dy - r, dx + r, dy + r), dot, r);
            let selected_row = l.id == selected.to_lowercase();
            let mut tr = rect(item.left + px(30.0, sc), item.top, item.right - px(8.0, sc), item.bottom);
            put_text(dc, &l.name, &mut tr, if selected_row { C_ACCENT } else { C_TEXT }, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
        }
    }

    unsafe fn draw(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let sc = ctx.scale;
        let lay = layout(area, sc);
        let st = settings::store();
        let custom = st.preset == Preset::Custom;
        let vals = settings::current();
        let gen = settings::general();
        let page_pad = px(PAGE_PAD, sc);
        let cp = px(CARD_PAD, sc);
        let radius = px(CARD_RADIUS, sc);
        let initial_font = SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        fill(dc, area, C_BG);

        // ---------- scrolling list (drawn first, clipped; the title + preset card paint over any overflow) ----------
        IntersectClipRect(dc, lay.list.left, lay.list.top, lay.list.right, lay.list.bottom);
        for (top, bottom) in card_spans(sc) {
            let card_rc = rect(area.left + page_pad, lay.list.top + top - self.scroll, area.right - page_pad, lay.list.top + bottom - self.scroll);
            if card_rc.bottom < lay.list.top || card_rc.top > lay.list.bottom {
                continue;
            }
            card(dc, &card_rc, C_PANEL, C_BORDER, radius);
        }
        let mut prev_was_row = false;
        for (idx, (row, top, rh)) in row_metrics(sc).into_iter().enumerate() {
            let y = lay.list.top + top - self.scroll;
            let visible = !(y + rh < lay.list.top || y > lay.list.bottom);
            match row {
                Row::Heading(text, bound) => {
                    prev_was_row = false;
                    if visible {
                        SelectObject(dc, HGDIOBJ(ctx.fonts.bold.0));
                        let mut rc = rect(area.left + page_pad + px(4.0, sc), y, area.right - page_pad, y + rh);
                        put_text(dc, text, &mut rc, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                        if bound && !custom {
                            SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
                            let mut note = rect(area.left + page_pad, y, area.right - page_pad - px(4.0, sc), y + rh);
                            put_text(dc, "editable in Custom", &mut note, C_DIM, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                        }
                    }
                }
                Row::Language => {
                    if visible {
                        if prev_was_row {
                            fill(dc, &rect(area.left + page_pad + cp, y, area.right - page_pad - cp, y + 1), (52, 52, 58));
                        }
                        self.draw_language_row(ctx, dc, area, y, rh, idx);
                    }
                    prev_was_row = true;
                }
                _ => {
                    if visible {
                        if prev_was_row {
                            // thin divider between two rows of the same card
                            fill(dc, &rect(area.left + page_pad + cp, y, area.right - page_pad - cp, y + 1), (52, 52, 58));
                        }
                        if let Some(v) = view_of(row, &vals, &gen, custom) {
                            self.draw_row(ctx, dc, area, y, rh, idx, &v);
                        }
                    }
                    prev_was_row = true;
                }
            }
        }
        let viewport = (lay.list.bottom - lay.list.top).max(1);
        let content = content_height(sc);
        if content > viewport {
            let thumb_h = ((viewport as f32 * viewport as f32 / content as f32) as i32).max(px(30.0, sc));
            let max = (content - viewport).max(1);
            let thumb_y = lay.list.top + (self.scroll as f32 / max as f32 * (viewport - thumb_h) as f32) as i32;
            let x = area.right - px(12.0, sc);
            fill_round(dc, &rect(x, thumb_y, x + px(4.0, sc), thumb_y + thumb_h), C_THUMB, px(2.0, sc));
        }
        let _ = SelectClipRgn(dc, None);

        // ---------- title ----------
        fill(dc, &rect(area.left, area.top, area.right, lay.panel.top), C_BG);
        SelectObject(dc, HGDIOBJ(ctx.fonts.page.0));
        let mut tr = lay.title;
        put_text(dc, "Settings", &mut tr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

        // ---------- preset card ----------
        fill(dc, &rect(area.left, lay.panel.top, area.right, lay.list.top), C_BG);
        card(dc, &lay.panel, C_PANEL, C_BORDER, radius);
        SelectObject(dc, HGDIOBJ(ctx.fonts.bold.0));
        let mut pl = rect(lay.panel.left + cp, lay.panel.top + px(20.0, sc), lay.dropdown.left - cp, lay.panel.top + px(44.0, sc));
        put_text(dc, "Preset", &mut pl, C_TEXT, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
        let mut blurb = rect(lay.panel.left + cp, lay.panel.top + px(48.0, sc), lay.dropdown.left - cp, lay.panel.bottom - px(12.0, sc));
        put_text(dc, st.preset.blurb(), &mut blurb, C_DIM, DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX);

        let box_bg = if self.hot == Hot::Dropdown || self.dropdown_open { C_PILL_HOT } else { C_PILL };
        fill_round(dc, &lay.dropdown, box_bg, px(8.0, sc));
        SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
        let mut dt = rect(lay.dropdown.left + px(14.0, sc), lay.dropdown.top, lay.dropdown.right - px(36.0, sc), lay.dropdown.bottom);
        put_text(dc, st.preset.label(), &mut dt, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
        let (ax, ay) = (lay.dropdown.right - px(20.0, sc), (lay.dropdown.top + lay.dropdown.bottom) / 2);
        let a = px(4.0, sc);
        line(dc, (ax - a, ay - a / 2), (ax, ay + a / 2), C_TEXT, px(1.5, sc).max(1));
        line(dc, (ax, ay + a / 2), (ax + a, ay - a / 2), C_TEXT, px(1.5, sc).max(1));

        SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
        for (r, label, hot, enabled) in [
            (lay.reset, "Reset", Hot::Reset, custom),
            (lay.import, "Import", Hot::Import, true),
            (lay.export, "Export", Hot::Export, true),
        ] {
            let bg = if !enabled { (34, 34, 38) } else if self.hot == hot { C_PILL_HOT } else { C_PILL };
            fill_round(dc, &r, bg, px(8.0, sc));
            let mut tr = r;
            put_text(dc, label, &mut tr, if enabled { C_TEXT } else { C_DIM }, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        }

        // ---------- dropdown list (on top of everything) ----------
        if self.dropdown_open {
            let n = Preset::ALL.len() as i32;
            let ih = px(34.0, sc);
            let outer = rect(lay.dropdown.left, lay.dropdown.bottom + px(2.0, sc), lay.dropdown.right, lay.dropdown.bottom + px(10.0, sc) + n * ih);
            card(dc, &outer, (34, 34, 38), C_BORDER, px(8.0, sc));
            SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
            for (i, p) in Preset::ALL.iter().enumerate() {
                let item = drop_item(&lay, i, sc);
                if self.hot == Hot::DropItem(i) {
                    fill_round(dc, &rect(item.left + 4, item.top, item.right - 4, item.bottom), C_PILL_HOT, px(6.0, sc));
                }
                if *p == st.preset {
                    let r = px(3.0, sc);
                    let (dx, dy) = (item.left + px(14.0, sc), (item.top + item.bottom) / 2);
                    fill_round(dc, &rect(dx - r, dy - r, dx + r, dy + r), C_ACCENT, r);
                }
                let mut tr = rect(item.left + px(28.0, sc), item.top, item.right - px(8.0, sc), item.bottom);
                put_text(dc, p.label(), &mut tr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
            }
        }
        if self.lang_open {
            self.draw_language_overlay(ctx, dc, area);
        }
        SelectObject(dc, initial_font);
    }

    /// Drawn by the main window after everything else, so it sits on top.
    pub fn draw_tooltip(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let Some(text) = self.tooltip() else { return };
        let sc = ctx.scale;
        unsafe {
            let old = SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
            let max_w = px(300.0, sc).min(area.right - area.left - px(24.0, sc));
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            let mut m = rect(0, 0, max_w, 0);
            DrawTextW(dc, &mut wide, &mut m, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_LEFT);
            let (tw, th) = (m.right + px(20.0, sc), m.bottom + px(16.0, sc));
            let mut x = self.mouse.0 + px(16.0, sc);
            let mut y = self.mouse.1 + px(20.0, sc);
            if x + tw > area.right - 6 {
                x = (area.right - tw - 6).max(area.left + 6);
            }
            if y + th > area.bottom - 6 {
                y = (self.mouse.1 - th - px(10.0, sc)).max(area.top + 6);
            }
            fill_round(dc, &rect(x, y, x + tw, y + th), C_BORDER, px(6.0, sc));
            fill_round(dc, &rect(x + 1, y + 1, x + tw - 1, y + th - 1), (30, 34, 44), px(6.0, sc));
            let mut tr = rect(x + px(10.0, sc), y + px(8.0, sc), x + tw - px(10.0, sc), y + th - px(8.0, sc));
            put_text(dc, text, &mut tr, C_TEXT, DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX);
            SelectObject(dc, old);
        }
    }

    // ---------------- input ----------------
    pub fn mouse_move(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        self.mouse = (x, y);
        let mut r = React::default();
        if let Some(row) = self.drag {
            if let (Some(g), Some(p)) = (self.geom_for(ctx, area, row), row_param(ROWS[row])) {
                let v = slider_value_from_x(p, x, &g);
                let changed = row_value(ROWS[row]) != Some(v);
                set_row_value(ROWS[row], v); // live, no disk write
                if changed && is_overlay_row(ROWS[row]) {
                    r.action = Some(Action::LiveOverlayPreview);
                }
                r.redraw = true;
            }
        }
        let hot = match self.drag {
            Some(row) => Hot::Slider(row),
            None => self.hit(ctx, area, x, y),
        };
        if hot != self.hot {
            self.hot = hot;
            r.redraw = true;
            r.tip = self.drag.is_none() && tip_text(hot).is_some();
        }
        r
    }

    pub fn leave(&mut self) -> React {
        let mut r = React::default();
        if self.drag.is_none() && self.hot != Hot::None {
            self.hot = Hot::None;
            r.redraw = true;
        }
        r
    }

    pub fn mouse_down(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        let mut r = React { redraw: true, ..Default::default() };
        let hot = self.hit(ctx, area, x, y);
        if self.lang_open {
            self.lang_open = false;
            if let Hot::LangItem(i) = hot {
                if let Some(l) = languages::list().get(i) {
                    settings::set_language(&l.id);
                }
            }
            return r;
        }
        if self.dropdown_open {
            self.dropdown_open = false;
            if let Hot::DropItem(i) = hot {
                settings::set_preset(Preset::ALL[i]);
            }
            return r;
        }
        let custom = settings::store().preset == Preset::Custom;
        let vals = settings::current();
        let gen = settings::general();
        let enabled = |row: usize| view_of(ROWS[row], &vals, &gen, custom).map(|v| v.enabled).unwrap_or(false);
        match hot {
            Hot::Dropdown => self.dropdown_open = true,
            Hot::LangBox => self.lang_open = true,
            Hot::Slider(row) if enabled(row) => {
                self.drag = Some(row);
                if let (Some(g), Some(p)) = (self.geom_for(ctx, area, row), row_param(ROWS[row])) {
                    set_row_value(ROWS[row], slider_value_from_x(p, x, &g));
                    if is_overlay_row(ROWS[row]) {
                        r.action = Some(Action::LiveOverlayPreview);
                    }
                }
                r.capture = true;
            }
            Hot::Toggle(row) if enabled(row) => {
                let on = view_of(ROWS[row], &vals, &gen, custom).map(|v| v.value != 0).unwrap_or(false);
                set_row_value(ROWS[row], if on { 0 } else { 1 });
                settings::save();
                if is_overlay_row(ROWS[row]) {
                    r.action = Some(Action::LiveOverlayPreview);
                }
            }
            Hot::Choice(row, opt) if enabled(row) => {
                set_row_value(ROWS[row], opt as u32);
                settings::save();
                if is_overlay_row(ROWS[row]) {
                    r.action = Some(Action::LiveOverlayPreview);
                }
            }
            Hot::Reset if custom => settings::reset_custom(),
            Hot::Import => r.action = Some(Action::ImportSettings),
            Hot::Export => r.action = Some(Action::ExportSettings),
            _ => r.redraw = false,
        }
        r
    }

    pub fn mouse_up(&mut self) -> React {
        let mut r = React::default();
        if self.drag.take().is_some() {
            settings::save(); // one disk write per drag
            r.release = true;
            r.redraw = true;
        }
        r
    }

    pub fn wheel(&mut self, ctx: &Ctx, area: &RECT, notches: f32) -> React {
        let max = self.max_scroll(ctx, area);
        self.scroll = (self.scroll - (notches * 60.0 * ctx.scale) as i32).clamp(0, max);
        React { redraw: true, ..Default::default() }
    }

    /// Esc closes an open dropdown; returns true if it did (so the window stays open).
    pub fn escape(&mut self) -> bool {
        if self.dropdown_open || self.lang_open {
            self.dropdown_open = false;
            self.lang_open = false;
            true
        } else {
            false
        }
    }
}
