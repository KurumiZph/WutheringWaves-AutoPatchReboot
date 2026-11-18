//! Tunable constants, file locations and the persisted JSON config.

use std::path::PathBuf;
use std::sync::OnceLock;

/// App version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------- detection details that are not in the settings window ----------------
// (every user-tweakable value - scan pace, delays, thresholds, ... - lives in settings.rs)

/// A scan that reaches this similarity to a target, but not the detection threshold, is a "near miss".
pub const NEAR_MISS_THRESHOLD: f64 = 0.5;
/// How closely a "User ID" / "UID" label must match to count as the label.
pub const UID_LABEL_THRESHOLD: f64 = 0.8;
/// OCR texts at least this similar count as "unchanged" (scan pacing).
pub const TEXT_UNCHANGED_SIMILARITY: f64 = 0.92;

// The restart popup must appear inside this fraction of the screen, so corner
// text (like the version watermark) can't trigger a false restart.
pub const PATCH_CENTER_X_RANGE: (f32, f32) = (0.15, 0.85);
pub const PATCH_CENTER_Y_RANGE: (f32, f32) = (0.20, 0.80);

// ---------------- debug ----------------
pub const DEBUG_OCR_CHANGE_THRESHOLD: f64 = 0.92;

// ---------------- game ----------------
pub const GAME_PROCESS_NAMES: [&str; 3] =
    ["Wuthering Waves.exe", "Client-Win64-Shipping.exe", "WutheringWaves.exe"];
pub const EXE_SEARCH_NAMES: [&str; 2] = ["Wuthering Waves.exe", "Client-Win64-Shipping.exe"];


// ---------------- test flags ----------------
pub struct TestFlags {
    pub fake_patch: bool,
    pub dry_run: bool,
}

/// `--fake-patch` (or WUWA_FAKE_PATCH=1) injects a synthetic patch notice.
/// Dry-run is the default; `--fake-patch-live` actually restarts the game.
pub fn test_flags() -> &'static TestFlags {
    static FLAGS: OnceLock<TestFlags> = OnceLock::new();
    FLAGS.get_or_init(|| {
        let args: Vec<String> = std::env::args().collect();
        let has = |name: &str| args.iter().any(|a| a == name);
        let env_is = |key: &str, val: &str| std::env::var(key).map(|v| v == val).unwrap_or(false);
        TestFlags {
            fake_patch: has("--fake-patch") || env_is("WUWA_FAKE_PATCH", "1"),
            dry_run: !has("--fake-patch-live")
                && std::env::var("WUWA_FAKE_PATCH_DRY_RUN").map(|v| v != "0").unwrap_or(true),
        }
    })
}

// ---------------- files ----------------
/// %APPDATA%\WuWa-APR
pub fn app_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("WuWa-APR");
        let _ = std::fs::create_dir_all(&dir);
        dir
    })
}
/// Log files (the current ones and the time-stamped older versions).
pub fn logs_dir() -> PathBuf {
    let dir = app_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
pub fn log_min() -> PathBuf {
    logs_dir().join("ocr_log_min.txt")
}
pub fn log_full() -> PathBuf {
    logs_dir().join("ocr_log_full.txt")
}
/// Saved screenshots: scan_<time>.png (pruned to the newest N) and patch_<time>.png (kept forever).
pub fn screenshots_dir() -> PathBuf {
    let dir = app_dir().join("screenshots");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
/// The most recently saved screenshot, if any.
pub fn latest_screenshot() -> Option<PathBuf> {
    std::fs::read_dir(screenshots_dir())
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "png").unwrap_or(false))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}
/// Exported logs / settings go here.
pub fn exports_dir() -> PathBuf {
    let dir = app_dir().join("exports");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
pub fn settings_file() -> PathBuf {
    app_dir().join("settings.json")
}
pub fn languages_file() -> PathBuf {
    app_dir().join("languages.json")
}
/// The old separate game-install file (versions before the merge). Only read once to migrate, then deleted.
fn old_config_file() -> PathBuf {
    app_dir().join("config.json")
}

// ---------------- settings.json: the settings AND the saved game install ----------------
// settings.json is one JSON object. settings.rs owns the preset / values / language keys; the saved game
// install lives under the "game" key and is owned by installs.rs. Both are written through the helpers
// below, which always keep the other part, so saving one can never wipe the other.

static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_settings_object() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| match v {
            serde_json::Value::Object(o) => Some(o),
            _ => None,
        })
        .unwrap_or_default()
}

fn write_settings_object(obj: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(obj)).map_err(|e| e.to_string())?;
    std::fs::write(settings_file(), text).map_err(|e| e.to_string())
}

/// Writes the settings part (everything except "game") to settings.json; the saved game install stays as it is.
pub fn save_settings_value(settings: &serde_json::Value) -> Result<(), String> {
    let _guard = crate::state::lock(&FILE_LOCK);
    let mut obj = read_settings_object();
    let game = obj.remove("game");
    let mut out = settings.as_object().cloned().unwrap_or_default();
    if let Some(game) = game {
        out.insert("game".to_string(), game);
    }
    write_settings_object(out)
}

/// The saved game install section ({installations, last_used, remember_selection}).
/// First run after the update: it is taken from the old config.json.
pub fn load_config() -> serde_json::Value {
    let _guard = crate::state::lock(&FILE_LOCK);
    if let Some(game) = read_settings_object().remove("game").filter(|v| v.is_object()) {
        return game;
    }
    std::fs::read_to_string(old_config_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Saves the game install section into settings.json (the settings part stays as it is) and removes the
/// old config.json once its content is safely in settings.json.
pub fn save_config(value: &serde_json::Value) {
    let result = {
        let _guard = crate::state::lock(&FILE_LOCK);
        let mut obj = read_settings_object();
        obj.insert("game".to_string(), value.clone());
        write_settings_object(obj)
    };
    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(old_config_file());
        }
        Err(e) => crate::log::log(format!("[CONFIG] Could not save the game install: {e}")),
    }
}

/// Forgets the saved game install (the next start asks again). Also removes a leftover old config.json.
pub fn forget_game() -> Result<(), String> {
    let _guard = crate::state::lock(&FILE_LOCK);
    let mut obj = read_settings_object();
    obj.remove("game");
    let _ = std::fs::remove_file(old_config_file());
    write_settings_object(obj)
}
