//! The "Misc" tab: tests, language tools and small tools, one card each (scrolls when it does not fit).

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    IntersectClipRect, SelectClipRgn, SelectObject, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_TOP, DT_VCENTER, DT_WORDBREAK, HDC, HGDIOBJ,
};

use crate::config;
use crate::gfx::*;
use crate::ui_types::{Action, Ctx, React};

const PAGE_PAD: f32 = 28.0;
const PAGE_TOP: f32 = 22.0;
const TITLE_H: f32 = 44.0;
const HEADING_H: f32 = 38.0;
const CARD_H: f32 = 88.0;
const CARD_GAP: f32 = 12.0;
const CARD_PAD: f32 = 22.0;
const SECTION_GAP: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hot {
    None,
    Card(usize),
}

struct Item {
    heading: Option<&'static str>, // printed above this card (first card of a section)
    title: &'static str,
    desc: String,
    button: &'static str,
    action: Action,
}

pub struct MiscPane {
    hot: Hot,
    scroll: i32,
}

fn items() -> Vec<Item> {
    vec![
        Item {
            heading: Some("TESTS"),
            title: "Self-test (simulated screens)",
            desc: "Runs the real OCR and detection on a simulated patch screen, login screen and a neutral screen and writes PASS / FAIL to Live Logs. No game needed.".to_string(),
            button: "Run self-test",
            action: Action::SelfTest,
        },
        Item {
            heading: None,
            title: "Test an image",
            desc: "Pick a screenshot (png / jpg) and see what the OCR reads and whether it counts as a patch or login screen, using the game language chosen in Settings.".to_string(),
            button: "Choose image...",
            action: Action::TestImage,
        },
        Item {
            heading: None,
            title: "Preview the notification",
            desc: "Shows the 'Patch detected - restarting game' box for a few seconds with your size, opacity and position settings.".to_string(),
            button: "Show preview",
            action: Action::PreviewOverlay,
        },
        Item {
            heading: Some("LANGUAGES"),
            title: "Reload languages",
            desc: "Re-reads languages.json after you edited it (texts, new languages). Details appear in Live Logs.".to_string(),
            button: "Reload",
            action: Action::ReloadLanguages,
        },
        Item {
            heading: None,
            title: "Languages file",
            desc: format!("Texts per language (login, patch notice, user ID label). Edit or add languages here: {}", config::languages_file().display()),
            button: "Open file",
            action: Action::OpenLanguagesFile,
        },
        Item {
            heading: Some("TOOLS"),
            title: "Latest screenshot",
            desc: "Opens the newest screenshot the OCR saved (turn on 'Save screenshots' in Settings > Debugging).".to_string(),
            button: "Show screenshot",
            action: Action::DebugShot,
        },
        Item {
            heading: None,
            title: "Data folder",
            desc: format!("Logs, settings, languages and screenshots live in {}", config::app_dir().display()),
            button: "Open folder",
            action: Action::OpenData,
        },
        Item {
            heading: Some("GAME"),
            title: "Game install",
            desc: "APR remembers which Wuthering Waves install it launches (saved in settings.json). Forget it to pick again the next time APR starts.".to_string(),
            button: "Forget",
            action: Action::ForgetGame,
        },
    ]
}

/// (item index, card rect, button rect, heading rect if any) in client coordinates (scroll applied),
/// plus the total content height.
fn layout(area: &RECT, sc: f32, list: &[Item], scroll: i32) -> (Vec<(usize, RECT, RECT, Option<RECT>)>, i32) {
    let pad = px(PAGE_PAD, sc);
    let top0 = area.top + px(PAGE_TOP, sc) + px(TITLE_H, sc);
    let mut y = top0 - scroll;
    let mut out = Vec::new();
    for (i, it) in list.iter().enumerate() {
        let mut heading = None;
        if it.heading.is_some() {
            if i > 0 {
                y += px(SECTION_GAP, sc);
            }
            heading = Some(rect(area.left + pad + px(4.0, sc), y, area.right - pad, y + px(HEADING_H, sc)));
            y += px(HEADING_H, sc);
        }
        let card = rect(area.left + pad, y, area.right - pad, y + px(CARD_H, sc));
        let (bw, bh) = (px(150.0, sc), px(36.0, sc));
        let by = card.top + (card.bottom - card.top - bh) / 2;
        let button = rect(card.right - px(CARD_PAD, sc) - bw, by, card.right - px(CARD_PAD, sc), by + bh);
        out.push((i, card, button, heading));
        y += px(CARD_H, sc) + px(CARD_GAP, sc);
    }
    let content_h = (y + scroll) - top0 + px(PAGE_PAD, sc);
    (out, content_h)
}

impl MiscPane {
    pub fn new() -> MiscPane {
        MiscPane { hot: Hot::None, scroll: 0 }
    }

    fn max_scroll(&self, ctx: &Ctx, area: &RECT) -> i32 {
        let list = items();
        let (_, content_h) = layout(area, ctx.scale, &list, 0);
        let viewport = area.bottom - (area.top + px(PAGE_TOP, ctx.scale) + px(TITLE_H, ctx.scale));
        (content_h - viewport).max(0)
    }

    fn hit(&self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> Hot {
        let list = items();
        let top = area.top + px(PAGE_TOP, ctx.scale) + px(TITLE_H, ctx.scale);
        if y < top {
            return Hot::None;
        }
        for (i, _, button, _) in layout(area, ctx.scale, &list, self.scroll).0 {
            if inside(&button, x, y) {
                return Hot::Card(i);
            }
        }
        Hot::None
    }

    pub fn render(&self, ctx: &Ctx, dc: HDC, area: &RECT) {
        let sc = ctx.scale;
        let list = items();
        unsafe {
            fill(dc, area, C_BG);
            let top = area.top + px(PAGE_TOP, sc) + px(TITLE_H, sc);
            let (placed, _) = layout(area, sc, &list, self.scroll);

            // cards (clipped below the title so they scroll underneath it)
            IntersectClipRect(dc, area.left, top, area.right, area.bottom);
            let old = SelectObject(dc, HGDIOBJ(ctx.fonts.bold.0));
            for (i, card_rc, button, heading) in placed {
                if card_rc.bottom < top || card_rc.top > area.bottom {
                    continue;
                }
                let it = &list[i];
                if let (Some(text), Some(hr)) = (it.heading, heading) {
                    SelectObject(dc, HGDIOBJ(ctx.fonts.bold.0));
                    let mut hr = hr;
                    put_text(dc, text, &mut hr, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
                card(dc, &card_rc, C_PANEL, C_BORDER, px(10.0, sc));
                let text_right = button.left - px(18.0, sc);

                SelectObject(dc, HGDIOBJ(ctx.fonts.bold.0));
                let mut tr = rect(card_rc.left + px(CARD_PAD, sc), card_rc.top + px(14.0, sc), text_right, card_rc.top + px(38.0, sc));
                put_text(dc, it.title, &mut tr, C_TEXT, DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
                SelectObject(dc, HGDIOBJ(ctx.fonts.small.0));
                let mut dr = rect(card_rc.left + px(CARD_PAD, sc), card_rc.top + px(40.0, sc), text_right, card_rc.bottom - px(8.0, sc));
                put_text(dc, &it.desc, &mut dr, C_DIM, DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX);

                SelectObject(dc, HGDIOBJ(ctx.fonts.ui.0));
                fill_round(dc, &button, if self.hot == Hot::Card(i) { C_PILL_HOT } else { C_PILL }, px(8.0, sc));
                let mut br = button;
                put_text(dc, it.button, &mut br, C_TEXT, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            }
            let _ = SelectClipRgn(dc, None);

            // title on top of the scrolled content
            fill(dc, &rect(area.left, area.top, area.right, top), C_BG);
            SelectObject(dc, HGDIOBJ(ctx.fonts.page.0));
            let mut title = rect(area.left + px(PAGE_PAD, sc), area.top + px(PAGE_TOP, sc), area.right - px(PAGE_PAD, sc), top);
            put_text(dc, "Misc", &mut title, C_TEXT, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old);
        }
    }

    pub fn mouse_move(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        let hot = self.hit(ctx, area, x, y);
        let mut r = React::default();
        if hot != self.hot {
            self.hot = hot;
            r.redraw = true;
        }
        r
    }

    pub fn leave(&mut self) -> React {
        let mut r = React::default();
        if self.hot != Hot::None {
            self.hot = Hot::None;
            r.redraw = true;
        }
        r
    }

    pub fn mouse_down(&mut self, ctx: &Ctx, area: &RECT, x: i32, y: i32) -> React {
        let mut r = React::default();
        if let Hot::Card(i) = self.hit(ctx, area, x, y) {
            r.action = items().get(i).map(|it| it.action);
        }
        r
    }

    pub fn wheel(&mut self, ctx: &Ctx, area: &RECT, notches: f32) -> React {
        let max = self.max_scroll(ctx, area);
        self.scroll = (self.scroll - (notches * 60.0 * ctx.scale) as i32).clamp(0, max);
        React { redraw: true, ..Default::default() }
    }
}
