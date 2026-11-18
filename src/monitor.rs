//! APR proper: launch the game, watch it with OCR, restart it after a patch.

use std::path::Path;
use std::time::{Duration, Instant};

use windows::Media::Ocr::OcrEngine;

use crate::cadence::Cadence;
use crate::capture::{self, GameWindow};
use crate::config::*;
use crate::detect::{self, Ocr};
use crate::log::{log, log_full_ocr, log_full_ocr_corner, set_status};
use crate::state::{sleep_interruptible, stopping};
use crate::textmatch::similarity;
use crate::languages::{self, Language, MatchMode};
use crate::settings::LoginMethod;
use crate::{autologin, game, ocr, overlay, settings, shots};

fn bar(title: &str) -> String {
    format!("{} {title} {}", "=".repeat(20), "=".repeat(20))
}

// ---------------- waiting helpers ----------------

fn wait_for_game_window(timeout: Option<Duration>) -> Option<GameWindow> {
    log("[WINDOW] Waiting for Wuthering Waves window...");
    let start = Instant::now();
    loop {
        if stopping() {
            return None;
        }
        if let Some(window) = capture::find_best_game_window() {
            log("[WINDOW] Found WuWa window:");
            log(format!("          Title: {}", window.title));
            log(format!("          Size: {}x{}", window.width, window.height));
            return Some(window);
        }
        if let Some(t) = timeout {
            if start.elapsed() > t {
                return None;
            }
        }
        if !sleep_interruptible(Duration::from_secs(1)) {
            return None;
        }
    }
}

/// Waits for the game process, then its window, then until the "delay before first scan" setting has
/// passed since the process first appeared (the first seconds are just a loading window).
fn wait_for_window_after_launch() -> Option<GameWindow> {
    if stopping() || !game::wait_for_game_process(Duration::from_secs(120)) {
        return None;
    }
    let process_seen = Instant::now();
    let window = wait_for_game_window(Some(Duration::from_secs(120)))?;
    overlay::request_hide(); // the game is back: the "restarting" notification is no longer needed
    let remaining = settings::current().ocr_start_delay().saturating_sub(process_seen.elapsed());
    if !remaining.is_zero() {
        log(format!("[OCR] Waiting {:.1}s after game start before the first scan...", remaining.as_secs_f32()));
        if !sleep_interruptible(remaining) {
            return None;
        }
    }
    Some(window)
}

// ---------------- debug OCR logging ----------------

#[derive(Default)]
struct DebugOcrState {
    signature: Option<String>,
    stable_scans: u32,
}

/// Writes only meaningful screen descriptions to the live log (screens that look the same are skipped).
fn log_debug_ocr(ocr: &Ocr, state: &mut DebugOcrState) {
    let signature = ocr.text();
    if signature.is_empty() {
        return;
    }
    if let Some(prev) = &state.signature {
        if similarity(prev, &signature) >= DEBUG_OCR_CHANGE_THRESHOLD {
            state.stable_scans += 1;
            return;
        }
    }
    state.signature = Some(signature);
    state.stable_scans = 0;

    let words: Vec<&str> = ocr.words.iter().map(|w| w.text.as_str()).collect();
    let has = |target: &str, threshold: f64| words.iter().any(|w| similarity(w, target) >= threshold);

    if has("kuro", 0.75) {
        log("[OCR] KURO GAMES");
    }
    if has("photosensitive", 0.80) && has("seizure", 0.80) && has("warning", 0.80) {
        log("[OCR] PHOTOSENSITIVITY WARNING");
    }
}

// ---------------- main loop ----------------

fn monitor_game(exe: &Path) {
    let flags = test_flags();
    let mut login_count = 0u32;
    let mut patch_count = 0u32;
    let mut scan_number = 0u64;
    let mut debug_state = DebugOcrState::default();
    let mut cadence = Cadence::new();
    // times of the recent patch restarts (restart-loop guard)
    let mut restarts: std::collections::VecDeque<Instant> = std::collections::VecDeque::new();
    let mut minimized_logged = false;
    let mut blank_streak = 0u32;
    // game language: the OCR engine follows the language chosen in Settings (and its OCR language tag)
    let mut engine: Option<OcrEngine> = None;
    let mut active_key: (String, String) = (String::new(), String::new());
    let mut last_engine_try = Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(Instant::now);
    let mut fallback_logged = false;

    loop {
        if stopping() {
            log("[WuWa] APR stopped by user (tray quit).");
            return;
        }
        let v = settings::current(); // re-read every loop: the settings tab changes it live
        let save_shots = settings::general().save_screenshots();
        // ---- game language: texts + OCR engine (switches live when changed in Settings / languages.json) ----
        let (lang, fell_back) = languages::resolve(&settings::language_id());
        if fell_back && !fallback_logged {
            log(format!("[LANG] The selected language '{}' is not in languages.json - using English.", settings::language_id()));
            fallback_logged = true;
        }
        if !fell_back {
            fallback_logged = false;
        }
        let key = (lang.id.clone(), lang.ocr.clone());
        let switched = key != active_key;
        if switched || (engine.is_none() && last_engine_try.elapsed() >= Duration::from_secs(5)) {
            last_engine_try = Instant::now();
            if switched {
                log(format!(
                    "[LANG] Game language: {} (OCR language {}, {} matching)",
                    lang.name,
                    lang.ocr,
                    if lang.mode == MatchMode::Chars { "character" } else { "word" }
                ));
                log(format!("[LANG] Login text: {}", lang.login.join("  |  ")));
                if lang.patch.is_empty() {
                    log(format!(
                        "[LANG] No patch text for {} yet - patch detection is OFF for this language (login detection works). Add it in languages.json.",
                        lang.name
                    ));
                }
                login_count = 0;
                patch_count = 0;
                cadence.reset();
                active_key = key;
            }
            engine = match ocr::create_engine_for(&lang.ocr) {
                Ok(e) => Some(e),
                Err(e) => {
                    let english = if lang.mode == MatchMode::Words { ocr::create_engine_for("en-US").ok() } else { None };
                    match english {
                        Some(en) => {
                            log(format!(
                                "[LANG] WARNING: the Windows OCR pack for {} ({}) is not installed - reading with the English OCR instead (accented letters may be misread).",
                                lang.name, lang.ocr
                            ));
                            Some(en)
                        }
                        None => {
                            log(format!("[ERROR] No Windows OCR for {} ({}): {e}", lang.name, lang.ocr));
                            None
                        }
                    }
                }
            };
            if engine.is_none() {
                set_status(&format!("OCR language pack missing for {} - install it in Windows", lang.name), true);
            } else if switched {
                set_status("Monitoring for login/patch screen...", false);
            }
        }
        let Some(engine_ref) = engine.as_ref() else {
            sleep_interruptible(Duration::from_secs(2));
            continue;
        };

        // Re-find the window every loop: restarts change the HWND.
        let window = match capture::find_best_game_window() {
            Some(w) => w,
            None => {
                if !game::is_game_running() {
                    log("[WuWa] Game process disappeared. Waiting for it to return...");
                    game::wait_for_game_process(Duration::from_secs(120));
                }
                match wait_for_game_window(Some(Duration::from_secs(30))) {
                    Some(w) => w,
                    None => {
                        sleep_interruptible(Duration::from_secs(1));
                        continue;
                    }
                }
            }
        };

        // Minimised = nothing to capture. (A window hidden behind other apps is still captured.)
        if window.minimized {
            if !minimized_logged {
                log("[CAPTURE] Game window is minimized - scanning paused until it is restored.");
                minimized_logged = true;
            }
            sleep_interruptible(v.scan_interval());
            continue;
        }
        if minimized_logged {
            log("[CAPTURE] Game window restored - scanning resumed.");
            minimized_logged = false;
        }

        let Some(mut frame) = capture::capture_window(&window) else {
            sleep_interruptible(Duration::from_secs(1));
            continue;
        };

        // A flat/black frame has nothing to read: skip it and leave the confirmation counters alone.
        if frame.looks_blank() {
            blank_streak += 1;
            if blank_streak == 1 {
                log("[CAPTURE] Frame is blank (black screen, loading, or no capture available) - skipping OCR.");
            }
            sleep_interruptible(v.scan_interval());
            continue;
        }
        if blank_streak > 0 {
            log(format!("[CAPTURE] Frame content is back after {blank_streak} blank scan(s)."));
            blank_streak = 0;
        }

        scan_number += 1;

        let mut ocr_result = match ocr::recognize(engine_ref, &frame, v.ocr_height()) {
            Ok(r) => r,
            Err(e) => {
                log(format!("[OCR ERROR] {e}"));
                Ocr::empty(frame.width, frame.height)
            }
        };

        let patch_lang = if flags.fake_patch { Language::english() } else { lang.clone() };
        if flags.fake_patch && scan_number <= v.confirmations() as u64 {
            ocr_result = detect::make_fake_patch(&ocr_result, &patch_lang);
            if scan_number == 1 {
                log("[TEST] Fake patch notice injected into OCR detection path.");
            }
        }

        // Mask the user id BEFORE anything is logged or saved.
        let mut uid = detect::censor_uid(&mut ocr_result, &lang);
        if !uid.label_found {
            // The user id is tiny text in the bottom-right corner and gets lost when the whole frame is shrunk
            // for OCR, so read that corner again at full resolution, enlarged.
            let corner = scan_uid_corner(engine_ref, &frame, &lang, ocr_result.width, scan_number);
            uid.masked.extend(corner.masked);
            uid.label_found |= corner.label_found;
        }
        log_full_ocr(&ocr_result, scan_number);

        if save_shots && (scan_number % settings::general().screenshot_every() as u64 == 0 || !uid.masked.is_empty()) {
            let boxes: Vec<(f32, f32, f32, f32)> =
                uid.masked.iter().map(|w| (w.left, w.top, w.width, w.height)).collect();
            frame.blackout(&boxes, ocr_result.width);
            shots::save_scan(&frame, scan_number);
        }
        if save_shots {
            log_debug_ocr(&ocr_result, &mut debug_state);
        }

        if uid.label_found {
            log(format!("\n{}", bar("[OCR] User ID detected (game already running)")));
            log("\nWuthering Waves will remain running.");
            log("APR is exiting.\n");
            set_status("User ID detected - APR finished.", false);
            return;
        }

        let need = v.confirmations();
        // Custom preset: the sliders decide. Otherwise the language's own strictness (short CJK phrases need more care).
        let (login_thr, patch_thr) = if settings::is_custom() {
            (v.login_threshold(), v.patch_threshold())
        } else {
            (lang.login_threshold.unwrap_or(v.login_threshold()), lang.patch_threshold.unwrap_or(v.patch_threshold()))
        };
        let login = detect::detect_login(&ocr_result, &lang, login_thr);
        let patch = detect::detect_patch(&ocr_result, &patch_lang, patch_thr);

        match &login {
            Some(l) => {
                login_count += 1;
                log(format!(
                    "[LOGIN] Possible detection {login_count}/{need} | score={:.2} | text='{}'",
                    l.score, l.candidate
                ));
            }
            None => login_count = 0,
        }
        match &patch {
            Some(p) => {
                patch_count += 1;
                log(format!(
                    "[PATCH] Possible detection {patch_count}/{need} | score={:.2} | text='{}'",
                    p.score, p.candidate
                ));
            }
            None => patch_count = 0,
        }
        if let (Some(l), true) = (&login, login_count >= need) {
            log(format!("\n{}", bar("[SUCCESS] LOGIN SCREEN CONFIRMED")));
            log(format!("\nDetected:\n    {}\n", l.candidate));
            log("Wuthering Waves will remain running.");

            let general = settings::general();
            if general.auto_login() {
                let delay = general.auto_login_delay();
                let method = general.auto_login_method();
                let what = if method == LoginMethod::Click { "click in the middle of the game window" } else { "Space key" };
                log(format!("[LOGIN] Auto login in {}s ({what}).", delay.as_secs()));
                set_status(&format!("Login detected - auto login in {}s...", delay.as_secs()), false);
                if !sleep_interruptible(delay) {
                    log("[LOGIN] Auto login cancelled (APR was stopped).");
                    return;
                }
                let sent = match capture::find_best_game_window() {
                    Some(window) => match autologin::perform(&window, method) {
                        Ok(message) => {
                            log(message);
                            true
                        }
                        Err(e) => {
                            log(format!("[LOGIN] Auto login failed: {e}"));
                            false
                        }
                    },
                    None => {
                        log("[LOGIN] Auto login failed: the game window was not found.");
                        false
                    }
                };
                log("APR is finishing.\n");
                set_status(
                    if sent {
                        "Login detected - auto login sent - APR finished."
                    } else {
                        "Login detected - auto login failed - APR finished."
                    },
                    false,
                );
            } else {
                log("APR is finishing.\n");
                set_status("Login detected - APR finished.", false);
            }
            return;
        }

        if let (Some(p), true) = (&patch, patch_count >= need) {
            log(format!("\n{}", bar("[PATCH] RESTART MESSAGE CONFIRMED")));
            log(format!("Detected: {}", p.candidate));

            if save_shots {
                shots::save_patch(&frame); // kept forever: a real patch popup is rare - handy as an OCR test image
            }

            if flags.fake_patch && flags.dry_run {
                log("[TEST] Fake patch detection succeeded.");
                log("[TEST] Dry-run enabled; game was NOT closed or restarted.");
                set_status("Fake patch test passed.", false);
                return;
            }

            let monitor = capture::monitor_of(window.hwnd); // the notification appears on the game's monitor

            // Restart-loop guard: a false detection must not keep killing the game forever.
            let (guard_max, guard_window) = settings::general().restart_guard();
            if guard_max > 0 {
                let now = Instant::now();
                while restarts.front().map(|t| now.duration_since(*t) > guard_window).unwrap_or(false) {
                    restarts.pop_front();
                }
                if restarts.len() as u32 >= guard_max {
                    log(format!(
                        "[ERROR] Restart-loop guard: the game was already restarted {} times in the last {} minutes. \
                         NOT restarting again (false detection?). The game was left running.",
                        restarts.len(),
                        guard_window.as_secs() / 60
                    ));
                    set_status("Restart loop detected - APR stopped.", false);
                    overlay::request_show("Restart loop detected - APR stopped", overlay::Kind::Error, monitor, 10_000, false);
                    return;
                }
                restarts.push_back(now);
            }

            set_status("Patch detected - restarting game...", false);
            overlay::request_show(overlay::TEXT_PATCH, overlay::Kind::Progress, monitor, 300_000, false);
            login_count = 0;
            patch_count = 0;

            game::close_game();
            game::wait_until_launchable(exe);

            let extra = settings::current().launch_extra_delay();
            if !extra.is_zero() {
                log(format!("[WuWa] Extra delay before relaunch: {}s", extra.as_secs()));
                sleep_interruptible(extra);
            }

            if stopping() {
                log("[WuWa] APR stopped by user (tray quit).");
                overlay::request_hide();
                return;
            }

            if game::launch_game(exe) {
                log("[WuWa] Restart launched.");
                let new_window = wait_for_window_after_launch();
                log(if new_window.is_some() {
                    "[WuWa] New game window detected."
                } else {
                    "[WuWa] WARNING: Game window was not found yet."
                });
                cadence.reset();
                set_status("Monitoring for login/patch screen...", false);
            } else {
                log("[WuWa] Restart failed.");
                set_status("Restart failed - see log.", false);
                overlay::request_show("Restart failed - see the log", overlay::Kind::Error, monitor, 8_000, false);
                return;
            }
            continue;
        }

        // Pace the next scan by what was just read (see cadence.rs).
        let candidate = login.is_some() || patch.is_some();
        let near = !candidate && detect::near_miss(&ocr_result, &lang);
        let wait = cadence.next_interval(&ocr_result.text(), near, candidate, &v);
        sleep_interruptible(wait);
    }
}

// ---------------- entry point of the APR thread ----------------

pub fn run_apr(exe: &Path) {
    if game::is_game_running() {
        set_status("Game already running.", true);
    } else if !game::launch_game(exe) {
        set_status("Failed to launch game - see log.", true);
        return;
    }

    set_status("Waiting for game window...", true);
    if wait_for_window_after_launch().is_none() {
        set_status("Game window not found - see log.", true);
        log("The game may still be starting.");
        return;
    }

    let line = "=".repeat(72);
    log(&line);
    log("[OCR] MONITORING STARTED");
    log("  SCAN  : Entire Wuthering Waves window");
    log(&line);

    set_status("Monitoring for login/patch screen...", true);
    monitor_game(exe);
}

/// Second, close-up OCR of the bottom-right corner (where the game shows the user id) at full resolution.
/// The returned boxes are in the coordinates of the main OCR image (`ocr_width` wide), so the screenshot
/// blackout works on them. Nothing of the number is ever logged.
fn scan_uid_corner(engine: &OcrEngine, frame: &crate::image_ops::Frame, lang: &Language, ocr_width: i32, scan_number: u64) -> detect::UidResult {
    let (crop, map) = frame.uid_corner();
    let mut corner = match ocr::recognize(engine, &crop, i32::MAX) {
        Ok(r) => r,
        Err(e) => {
            log(format!("[OCR ERROR] corner scan: {e}"));
            return detect::UidResult { masked: Vec::new(), label_found: false };
        }
    };
    let mut uid = detect::censor_uid_corner(&mut corner, lang);
    log_full_ocr_corner(&corner, scan_number);
    let to_ocr = ocr_width.max(1) as f32 / frame.width.max(1) as f32;
    for w in uid.masked.iter_mut() {
        let (l, t, bw, bh) = map.to_frame(w.left, w.top, w.width, w.height);
        *w = detect::Word { text: String::new(), left: l * to_ocr, top: t * to_ocr, width: bw * to_ocr, height: bh * to_ocr };
    }
    uid
}
