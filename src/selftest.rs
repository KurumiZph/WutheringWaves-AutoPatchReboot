//! Test tools (Misc tab): run the REAL OCR + detection pipeline on a simulated screen or on an image the
//! user picks, and write the result to the log. Lets you check the login / patch detection without waiting
//! for the game to actually patch.
//!
//! "Simulated screens" draw the texts with a normal Windows font, so they prove that OCR, the 720p-style
//! downscale, the fuzzy matching and the position check all work - not that the game's own font reads the
//! same. For that, use "Test an image" with a real screenshot of the patch popup (the Save screenshots
//! setting stores one automatically the first time it is seen).

use std::path::PathBuf;

use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    SetBkMode, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC,
    HGDIOBJ, TRANSPARENT,
};

use crate::detect::{self, Ocr};
use crate::languages::{self, Language};
use crate::gfx::*;
use crate::image_ops::Frame;
use crate::log::log;
use crate::{ocr, settings};

/// What a test screen is expected to be detected as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Patch,
    Login,
    Nothing,
}

const W: i32 = 1920;
const H: i32 = 1080;

// ---------------- rendering a fake screen ----------------
unsafe fn make_frame(draw: impl FnOnce(HDC)) -> Option<Frame> {
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(screen);
    let bmp = CreateCompatibleBitmap(screen, W, H);
    let old = SelectObject(dc, HGDIOBJ(bmp.0));
    SetBkMode(dc, TRANSPARENT);
    draw(dc);
    SelectObject(dc, old); // the bitmap must not be selected while GetDIBits reads it

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = W;
    bmi.bmiHeader.biHeight = -H;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    let mut bgra = vec![0u8; (W * H * 4) as usize];
    let lines = GetDIBits(dc, bmp, 0, H as u32, Some(bgra.as_mut_ptr() as *mut core::ffi::c_void), &mut bmi, DIB_RGB_COLORS);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(dc);
    ReleaseDC(None, screen);
    if lines == 0 {
        return None;
    }
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Some(Frame { width: W, height: H, bgra })
}

unsafe fn text_line(dc: HDC, text: &str, font: windows::Win32::Graphics::Gdi::HFONT, y: i32, height: i32, color: Rgb) {
    let old = SelectObject(dc, HGDIOBJ(font.0));
    let mut rc = rect(0, y, W, y + height);
    put_text(dc, text, &mut rc, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
}

/// A dark "game scene" with the given centred / bottom texts.
fn simulated_screen(kind: Expect) -> Option<Frame> {
    unsafe {
        let title = make_font(52, 600, "Segoe UI");
        let body = make_font(38, 400, "Segoe UI");
        let small = make_font(26, 400, "Segoe UI");
        let frame = make_frame(|dc| {
            fill(dc, &rect(0, 0, W, H), (22, 24, 30));
            // a few lighter "scene" blocks so the frame is not flat
            fill(dc, &rect(0, H * 7 / 10, W, H), (30, 34, 44));
            fill(dc, &rect(W / 10, H / 8, W / 4, H / 3), (38, 42, 52));
            match kind {
                Expect::Patch => {
                    fill_round(dc, &rect(W * 3 / 10, H * 3 / 10, W * 7 / 10, H * 65 / 100), (46, 50, 62), 18);
                    text_line(dc, "Patching complete", title, H * 36 / 100, 70, (245, 245, 250));
                    text_line(dc, "Please restart the game", body, H * 46 / 100, 60, (220, 222, 230));
                    text_line(dc, "Confirm", small, H * 56 / 100, 44, (150, 200, 255));
                }
                Expect::Login => {
                    text_line(dc, "Tap to land in Solaris-3", body, H * 82 / 100, 64, (240, 240, 245));
                    text_line(dc, "Login Status: Logged in", small, H * 90 / 100, 40, (170, 174, 186));
                }
                Expect::Nothing => {
                    text_line(dc, "Settings   Account   Exit", body, H * 40 / 100, 64, (230, 232, 240));
                    text_line(dc, "Wuthering Waves", title, H * 52 / 100, 70, (200, 205, 220));
                }
            }
        });
        delete_font(title);
        delete_font(body);
        delete_font(small);
        frame
    }
}

// ---------------- reporting ----------------
struct Verdict {
    patch: bool,
    login: bool,
}

/// Strictness for this language: the Custom preset's sliders, otherwise the language's own value.
fn thresholds(lang: &Language) -> (f64, f64) {
    let v = settings::current();
    if settings::is_custom() {
        (v.login_threshold(), v.patch_threshold())
    } else {
        (lang.login_threshold.unwrap_or(v.login_threshold()), lang.patch_threshold.unwrap_or(v.patch_threshold()))
    }
}

fn report(label: &str, frame: &Frame, mut ocr_result: Ocr, lang: &Language) -> Verdict {
    let (login_thr, patch_thr) = thresholds(lang);
    detect::censor_uid(&mut ocr_result, lang); // never log a user id, not even in a test
    let text = ocr_result.text();
    let shown: String = text.chars().take(240).collect();
    log(format!(
        "[TEST] {label} ({}): image {}x{} -> OCR {}x{}, {} words",
        lang.name,
        frame.width,
        frame.height,
        ocr_result.width,
        ocr_result.height,
        ocr_result.words.len()
    ));
    log(format!("[TEST] Text read: {}", if shown.is_empty() { "(nothing)".to_string() } else { shown }));

    let patch = detect::detect_patch(&ocr_result, lang, patch_thr);
    let login = detect::detect_login(&ocr_result, lang, login_thr);
    match &patch {
        Some(h) => log(format!("[TEST] Patch screen: DETECTED (score {:.2}, matched '{}')", h.score, h.candidate)),
        None if lang.patch.is_empty() => {
            log(format!("[TEST] Patch screen: not checked - {} has no patch text yet (add it in languages.json)", lang.name))
        }
        None => {
            let scores: Vec<String> = lang
                .patch
                .iter()
                .map(|p| format!("'{}' {:.2}", p, detect::best_phrase_score(&ocr_result.words, p, lang.mode)))
                .collect();
            log(format!(
                "[TEST] Patch screen: not detected (best similarity: {}; needs {:.2}, in the middle of the screen)",
                scores.join(", "),
                patch_thr
            ));
        }
    }
    match &login {
        Some(h) => log(format!("[TEST] Login screen: DETECTED (score {:.2}, matched '{}')", h.score, h.candidate)),
        None => {
            let best = lang.login.iter().map(|p| detect::best_phrase_score(&ocr_result.words, p, lang.mode)).fold(0.0, f64::max);
            log(format!("[TEST] Login screen: not detected (best similarity {best:.2}; needs {login_thr:.2})"));
        }
    }
    Verdict { patch: patch.is_some(), login: login.is_some() }
}

fn run_case(engine: &windows::Media::Ocr::OcrEngine, name: &str, kind: Expect) -> bool {
    let lang = Language::english(); // the simulated screens are drawn in English
    let Some(frame) = simulated_screen(kind) else {
        log(format!("[TEST] {name}: FAIL (could not draw the test screen)"));
        return false;
    };
    let ocr_result = match ocr::recognize(engine, &frame, settings::current().ocr_height()) {
        Ok(r) => r,
        Err(e) => {
            log(format!("[TEST] {name}: FAIL (OCR error: {e})"));
            return false;
        }
    };
    let verdict = report(name, &frame, ocr_result, &lang);
    let ok = match kind {
        Expect::Patch => verdict.patch,
        Expect::Login => verdict.login,
        Expect::Nothing => !verdict.patch && !verdict.login,
    };
    log(format!(
        "[TEST] {name}: {}",
        if ok { "PASS (detected as expected)" } else { "FAIL (not what was expected - see the lines above)" }
    ));
    ok
}

// ---------------- entry points (run on their own thread) ----------------
/// Simulated patch screen, login screen and a neutral screen through the real pipeline.
pub fn run_simulated() {
    std::thread::spawn(|| {
        log("[TEST] ===== Self-test: simulated screens =====");
        let engine = match ocr::create_engine() {
            Ok(e) => e,
            Err(e) => {
                log(format!("[TEST] FAIL: the Windows OCR engine could not be created: {e}"));
                return;
            }
        };
        let results = [
            run_case(&engine, "Simulated patch screen", Expect::Patch),
            run_case(&engine, "Simulated login screen", Expect::Login),
            run_case(&engine, "Simulated neutral screen (must NOT trigger)", Expect::Nothing),
        ];
        let passed = results.iter().filter(|r| **r).count();
        log(format!(
            "[TEST] ===== Self-test finished: {passed}/{} PASS{} =====",
            results.len(),
            if passed == results.len() { "" } else { " - FAIL: something above did not match" }
        ));
    });
}

/// A real screenshot (or any image) through the real pipeline.
pub fn run_image(path: PathBuf) {
    std::thread::spawn(move || {
        log(format!("[TEST] ===== Testing image: {} =====", path.display()));
        let lang = languages::selected(); // an image is read as the game language chosen in Settings
        let engine = match ocr::create_engine_for(&lang.ocr) {
            Ok(e) => e,
            Err(e) => {
                log(format!("[TEST] FAIL: no Windows OCR for {} ({}): {e}", lang.name, lang.ocr));
                return;
            }
        };
        let frame = match ocr::load_image(&path) {
            Ok(f) => f,
            Err(e) => {
                log(format!("[TEST] FAIL: could not open that image: {e}"));
                return;
            }
        };
        match ocr::recognize(&engine, &frame, settings::current().ocr_height()) {
            Ok(r) => {
                report("Image", &frame, r, &lang);
                // the user id check: a close-up of the bottom-right corner (what APR does on every scan)
                let (crop, _) = frame.uid_corner();
                match ocr::recognize(&engine, &crop, i32::MAX) {
                    Ok(mut corner) => {
                        let uid = detect::censor_uid_corner(&mut corner, &lang);
                        let text: String = corner.text().chars().take(120).collect();
                        log(format!(
                            "[TEST] User ID (bottom-right corner): {} | corner text (masked): {}",
                            if uid.label_found { "DETECTED - APR would stop here" } else { "not found" },
                            if text.is_empty() { "(nothing)".to_string() } else { text }
                        ));
                    }
                    Err(e) => log(format!("[TEST] User ID corner: OCR error: {e}")),
                }
                log("[TEST] ===== Image test finished =====");
            }
            Err(e) => log(format!("[TEST] FAIL: OCR error: {e}")),
        }
    });
}
