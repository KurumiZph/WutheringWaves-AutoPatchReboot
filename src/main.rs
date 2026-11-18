//! WuWa APR (AutoPatchReboot).
//!
//! Launches Wuthering Waves, watches the window with the built-in Windows OCR, and restarts the game
//! when the "Patching complete / Please restart the game" popup appears. Stops once the login screen
//! ("Tap to land in Solaris-3") or the in-game "User ID" text is seen.

#![windows_subsystem = "windows"]

mod appwin;
mod autologin;
mod cadence;
mod capture;
mod config;
mod detect;
mod game;
mod gfx;
mod image_ops;
mod installs;
mod languages;
mod log;
mod logstyle;
mod monitor;
mod ocr;
mod overlay;
mod settings;
mod shots;
mod state;
mod selftest;
mod sys;
mod tab_logs;
mod tab_misc;
mod tab_settings;
mod textmatch;
mod ui;
mod ui_types;

use std::panic::{catch_unwind, AssertUnwindSafe};

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

fn main() {
    // Only one copy at a time. The handle must stay alive for the whole run.
    let _instance_mutex = unsafe {
        let handle = CreateMutexW(None, false, w!("Local\\WuWaApr_SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            ui::info("WuWa APR is already running (check your system tray).");
            return;
        }
        handle
    };

    // Presets/custom values + process priority; must run before any thread starts.
    settings::load();

    // The previous session's logs are moved to *.old.txt here (and again whenever a file reaches the size limit).
    log::init();
    for note in languages::load() {
        log::log(note); // what was loaded from languages.json (and anything that was skipped)
    }
    languages::check_packs(); // which Windows OCR language packs are installed (background)

    if !ocr::available() {
        log::log("[ERROR] Windows OCR is not available. Exiting.");
        ui::error(
            "Windows OCR is not available on this PC.\n\n\
             Install an OCR-capable language pack: Settings > Time & language > Language & region > \
             Add a language (English), then run WuWa APR again.",
        );
        return;
    }
    log::log("[SYSTEM] Windows OCR found.");
    log::log(format!("[SYSTEM] Live/min log: {}", config::log_min().display()));
    log::log(format!("[SYSTEM] Full OCR log: {}", config::log_full().display()));
    log::log(format!("[SYSTEM] Languages file: {}", config::languages_file().display()));

    let Some(game_exe) = installs::resolve_game_exe() else {
        log::log("[ERROR] No Wuthering Waves executable was selected. Exiting.");
        ui::error("No Wuthering Waves executable was selected. Exiting.");
        return;
    };
    log::log(format!("[SYSTEM] Using game executable: {}", game_exe.display()));

    // APR runs on its own thread; this thread owns the tray icon and the windows.
    let worker = std::thread::Builder::new().name("apr".into()).spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| monitor::run_apr(&game_exe)));
        if result.is_err() {
            log::log("\n[FATAL ERROR] APR crashed.");
            log::log("The game has NOT been intentionally closed.");
            log::set_status("Fatal error - see log.", false);
        }
        state::finish();
    });
    if let Err(e) = worker {
        log::log(format!("[FATAL ERROR] Could not start the APR thread: {e}"));
        ui::error("Could not start the APR thread.");
        return;
    }

    ui::run();
}
