//! User-tweakable settings: presets, the individual values, persistence (settings.json), import/export
//! and live access for the rest of the app. Changes take effect immediately - everything that reads a
//! value calls `settings::current()` / `settings::general()` when it needs it.
//!
//! Two groups:
//!  * PRESET values (scan pace, waits, OCR size, ...): chosen by a preset; only "Custom" lets you change them.
//!    Detection accuracy (confirmations + match strictness) is the same in every preset.
//!  * GENERAL values (auto login, restart-loop guard, log size): behaviour, not performance. They do not
//!    depend on the preset and are always editable.

use std::sync::Mutex;
use std::time::Duration;

use windows::Win32::System::Threading::{
    GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
};

use crate::config;
use crate::state::lock;

pub const COUNT: usize = 14;
pub const GEN_COUNT: usize = 17;

// indices into Values / PARAMS (preset-bound)
pub const SCAN_INTERVAL: usize = 0;
pub const IDLE_INTERVAL: usize = 1;
pub const BURST_INTERVAL: usize = 2;
pub const IDLE_AFTER: usize = 3;
pub const NEAR_MISS_BURSTS: usize = 4;
pub const OCR_HEIGHT: usize = 5;
pub const OCR_START_DELAY: usize = 6;
pub const LAUNCH_EXTRA_DELAY: usize = 7;
pub const STOP_STABLE: usize = 8;
pub const STOP_TIMEOUT: usize = 9;
pub const CONFIRMATIONS: usize = 10;
pub const LOGIN_THRESHOLD: usize = 11;
pub const PATCH_THRESHOLD: usize = 12;
pub const LOW_PRIORITY: usize = 13;

// indices into General / GENERAL_PARAMS
pub const AUTO_LOGIN: usize = 0;
pub const AUTO_LOGIN_DELAY: usize = 1;
pub const AUTO_LOGIN_METHOD: usize = 2;
pub const GUARD_MAX: usize = 3;
pub const GUARD_MINUTES: usize = 4;
pub const LOG_MAX_MB: usize = 5;
pub const SAVE_SCREENSHOTS: usize = 6;
pub const SCREENSHOT_EVERY: usize = 7;
pub const KEEP_SCREENSHOTS: usize = 8;
pub const KEEP_LOGS: usize = 9;
pub const FULL_OCR_LOG: usize = 10;
pub const OVERLAY_ON: usize = 11;
pub const OVERLAY_SIZE: usize = 12;
pub const OVERLAY_OPACITY: usize = 13;
pub const OVERLAY_V: usize = 14;
pub const OVERLAY_H: usize = 15;
pub const OVERLAY_MARGIN: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Ms,
    Seconds,
    Minutes,
    Count,
    /// a count where 0 means "off"
    CountOff,
    Percent,
    Pixels,
    Megabytes,
    Px,
    Toggle,
    /// pick one of `options`
    Choice,
}

pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub tip: &'static str,
    pub min: u32,
    pub max: u32,
    pub step: u32,
    pub unit: Unit,
    pub options: &'static [&'static str],
}

pub const PRESET_TIP: &str = "Ready-made profiles for the SCANNING, STARTUP and SYSTEM values. Eco is gentle on slow PCs, \
Balanced suits most PCs, Turbo reacts fastest and uses more CPU. Choose Custom to edit those values yourself. Detection \
accuracy is the same in every preset. Auto login, the restart guard and log size are not part of presets.";
pub const RESET_TIP: &str = "Put all custom preset values back to the Balanced preset.";
pub const IMPORT_TIP: &str = "Load all settings (preset, custom values, auto login, ...) from a settings file exported earlier.";
pub const EXPORT_TIP: &str = "Save all current settings to a file you can keep or copy to another PC.";

pub static PARAMS: [Param; COUNT] = [
    Param { key: "scan_interval_ms", label: "Scan interval", unit: Unit::Ms, min: 500, max: 5000, step: 100, options: &[],
        tip: "How often the game screen is read (OCR) during normal monitoring. Shorter reacts faster but uses more CPU." },
    Param { key: "idle_interval_ms", label: "Idle scan interval", unit: Unit::Ms, min: 500, max: 10000, step: 100, options: &[],
        tip: "Slower pace used when the text on screen has stopped changing. This is also the longest gap between two scans." },
    Param { key: "burst_interval_ms", label: "Fast scan interval", unit: Unit::Ms, min: 100, max: 2000, step: 50, options: &[],
        tip: "Pace used while a login/patch screen is being confirmed, or the screen almost looks like one." },
    Param { key: "idle_after_scans", label: "Scans before idle pace", unit: Unit::Count, min: 1, max: 10, step: 1, options: &[],
        tip: "How many scans in a row must read the same text before switching to the slower idle pace." },
    Param { key: "near_miss_bursts", label: "Fast scans per near-miss", unit: Unit::Count, min: 0, max: 20, step: 1, options: &[],
        tip: "Max number of fast scans when the screen only partly resembles a login/patch screen. 0 turns this off." },
    Param { key: "ocr_height", label: "OCR image height", unit: Unit::Pixels, min: 360, max: 1080, step: 60, options: &[],
        tip: "Screenshots are shrunk to this height before reading. Lower is lighter on the CPU, higher reads small text better." },
    Param { key: "ocr_start_delay_s", label: "Delay before first scan", unit: Unit::Seconds, min: 0, max: 30, step: 1, options: &[],
        tip: "How long after the game process starts before scanning begins (the first seconds are only a loading window). Slow PCs may need more." },
    Param { key: "launch_extra_delay_s", label: "Extra delay before relaunch", unit: Unit::Seconds, min: 0, max: 30, step: 1, options: &[],
        tip: "Extra wait after the game has fully stopped and before it is launched again. Raise it if Steam still says the game is running." },
    Param { key: "stop_stable_ms", label: "'Stopped' confirmation time", unit: Unit::Ms, min: 250, max: 5000, step: 250, options: &[],
        tip: "How long the game must stay fully stopped (no process, Steam idle) before it is relaunched." },
    Param { key: "stop_timeout_s", label: "Max wait for game to stop", unit: Unit::Seconds, min: 10, max: 180, step: 5, options: &[],
        tip: "Give up waiting for the game to stop after this long and relaunch anyway." },
    Param { key: "confirmations", label: "Scans to confirm a screen", unit: Unit::Count, min: 1, max: 5, step: 1, options: &[],
        tip: "How many scans in a row must see the login/patch screen before APR acts. 2 protects against a single misread." },
    Param { key: "login_threshold_pct", label: "Login match strictness", unit: Unit::Percent, min: 50, max: 95, step: 1, options: &[],
        tip: "How closely the screen text must match 'Tap to land in Solaris-3'. Higher is stricter (fewer false alarms), lower is more forgiving." },
    Param { key: "patch_threshold_pct", label: "Patch match strictness", unit: Unit::Percent, min: 50, max: 95, step: 1, options: &[],
        tip: "How closely the screen text must match 'Patching complete' / 'Please restart the game'. Higher is stricter." },
    Param { key: "low_priority", label: "Low CPU priority", unit: Unit::Toggle, min: 0, max: 1, step: 1, options: &[],
        tip: "Run APR below normal priority so it never competes with the game. The game itself always runs at normal priority." },
];

pub static GENERAL_PARAMS: [Param; GEN_COUNT] = [
    Param { key: "auto_login", label: "Auto login", unit: Unit::Toggle, min: 0, max: 1, step: 1, options: &[],
        tip: "When the 'Tap to land in Solaris-3' screen is confirmed, APR clicks the game (or presses Space) for you. Nothing else is done." },
    Param { key: "auto_login_delay_s", label: "Delay before auto login", unit: Unit::Seconds, min: 0, max: 30, step: 1, options: &[],
        tip: "How long to wait between detecting the login screen and clicking the game." },
    Param { key: "auto_login_method", label: "Auto login action", unit: Unit::Choice, min: 0, max: 1, step: 1,
        options: &["Click centre", "Space key"],
        tip: "Click centre: moves the mouse to the middle of the game window and left-clicks, then puts the mouse back. Space key: sends the Space key to the game. Either way the game window is brought to the front first." },
    Param { key: "restart_guard_max", label: "Restart-loop guard", unit: Unit::CountOff, min: 0, max: 10, step: 1, options: &[],
        tip: "Stop restarting the game if it was already restarted this many times within the time below (a sign of a false detection). 0 turns the guard off." },
    Param { key: "restart_guard_minutes", label: "Guard time window", unit: Unit::Minutes, min: 5, max: 120, step: 5, options: &[],
        tip: "The time window used by the restart-loop guard." },
    Param { key: "log_max_mb", label: "Max log file size", unit: Unit::Megabytes, min: 1, max: 50, step: 1, options: &[],
        tip: "When a log file reaches this size it is moved to '.old' and a fresh one is started, so logs never grow forever." },
    Param { key: "save_screenshots", label: "Save screenshots", unit: Unit::Toggle, min: 0, max: 1, step: 1, options: &[],
        tip: "Saves what the OCR sees into the screenshots folder (user ID blacked out), a patch_<time>.png when a patch screen is confirmed, and adds extra OCR notes to the log. Turn it on when something is not detected, off again afterwards." },
    Param { key: "screenshot_every", label: "Screenshot every N scans", unit: Unit::Count, min: 1, max: 20, step: 1, options: &[],
        tip: "With 'Save screenshots' on, one scan out of this many is saved. 1 = every scan." },
    Param { key: "keep_screenshots", label: "Keep last N screenshots", unit: Unit::Count, min: 1, max: 200, step: 1, options: &[],
        tip: "Older scan screenshots are deleted automatically. Patch screenshots (patch_<time>.png) are never deleted." },
    Param { key: "keep_logs", label: "Keep last N log files", unit: Unit::Count, min: 1, max: 20, step: 1, options: &[],
        tip: "Each start (and each time a log reaches its size limit) the old log is saved with a time stamp. Only this many old logs are kept." },
    Param { key: "full_ocr_log", label: "Full OCR log", unit: Unit::Toggle, min: 0, max: 1, step: 1, options: &[],
        tip: "Also writes ocr_log_full.txt: every word the OCR read on every scan. Very detailed and large - only for debugging." },
    Param { key: "overlay_on", label: "Show notification", unit: Unit::Toggle, min: 0, max: 1, step: 1, options: &[],
        tip: "A small box on the screen (click-through, never takes focus) that shows 'Patch detected - restarting game' until the game window is back." },
    Param { key: "overlay_size", label: "Notification size", unit: Unit::Percent, min: 60, max: 200, step: 10, options: &[],
        tip: "Size of the notification box." },
    Param { key: "overlay_opacity", label: "Notification opacity", unit: Unit::Percent, min: 20, max: 100, step: 5, options: &[],
        tip: "How solid the notification box is. Lower = more see-through." },
    Param { key: "overlay_v", label: "Notification: vertical position", unit: Unit::Choice, min: 0, max: 2, step: 1,
        options: &["Top", "Middle", "Bottom"],
        tip: "Where the notification sits on the monitor the game is on." },
    Param { key: "overlay_h", label: "Notification: horizontal position", unit: Unit::Choice, min: 0, max: 2, step: 1,
        options: &["Left", "Centre", "Right"],
        tip: "Where the notification sits on the monitor the game is on." },
    Param { key: "overlay_margin", label: "Notification margin", unit: Unit::Px, min: 0, max: 200, step: 4, options: &[],
        tip: "Distance between the notification and the edge of the monitor." },
];

// ---------------- values ----------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Values(pub [u32; COUNT]);

//                                scan idle burst idleN nearN  ocrH  start extra stable tmo  conf login patch lowprio
const ECO: Values = Values([2500, 3500, 600, 2, 5, 720, 12, 3, 1500, 90, 2, 72, 69, 1]);
const BALANCED: Values = Values([1500, 2000, 300, 3, 8, 720, 8, 0, 750, 60, 2, 72, 69, 1]);
const TURBO: Values = Values([800, 1200, 200, 4, 12, 900, 6, 0, 500, 60, 2, 72, 69, 0]);

impl Values {
    fn ms(&self, i: usize) -> Duration {
        Duration::from_millis(self.0[i] as u64)
    }
    fn secs(&self, i: usize) -> Duration {
        Duration::from_secs(self.0[i] as u64)
    }
    pub fn scan_interval(&self) -> Duration {
        self.ms(SCAN_INTERVAL)
    }
    /// never faster than the normal pace
    pub fn idle_interval(&self) -> Duration {
        self.ms(IDLE_INTERVAL).max(self.scan_interval())
    }
    pub fn burst_interval(&self) -> Duration {
        self.ms(BURST_INTERVAL)
    }
    pub fn idle_after(&self) -> u32 {
        self.0[IDLE_AFTER]
    }
    pub fn near_miss_bursts(&self) -> u32 {
        self.0[NEAR_MISS_BURSTS]
    }
    pub fn ocr_height(&self) -> i32 {
        self.0[OCR_HEIGHT] as i32
    }
    pub fn ocr_start_delay(&self) -> Duration {
        self.secs(OCR_START_DELAY)
    }
    pub fn launch_extra_delay(&self) -> Duration {
        self.secs(LAUNCH_EXTRA_DELAY)
    }
    pub fn stop_stable(&self) -> Duration {
        self.ms(STOP_STABLE)
    }
    pub fn stop_timeout(&self) -> Duration {
        self.secs(STOP_TIMEOUT)
    }
    pub fn confirmations(&self) -> u32 {
        self.0[CONFIRMATIONS]
    }
    pub fn login_threshold(&self) -> f64 {
        self.0[LOGIN_THRESHOLD] as f64 / 100.0
    }
    pub fn patch_threshold(&self) -> f64 {
        self.0[PATCH_THRESHOLD] as f64 / 100.0
    }
    pub fn low_priority(&self) -> bool {
        self.0[LOW_PRIORITY] != 0
    }

    pub fn balanced() -> Values {
        BALANCED
    }
}

// ---------------- general values ----------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoginMethod {
    Click,
    Space,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct General(pub [u32; GEN_COUNT]);

//                                       login delay method guardN guardMin logMB shots every keepS keepL fullLog ovOn size opac v h margin
const GENERAL_DEFAULT: General = General([0, 2, 0, 3, 30, 5, 0, 2, 20, 5, 0, 1, 100, 85, 2, 2, 24]);

impl General {
    pub fn auto_login(&self) -> bool {
        self.0[AUTO_LOGIN] != 0
    }
    pub fn auto_login_delay(&self) -> Duration {
        Duration::from_secs(self.0[AUTO_LOGIN_DELAY] as u64)
    }
    pub fn auto_login_method(&self) -> LoginMethod {
        if self.0[AUTO_LOGIN_METHOD] == 1 {
            LoginMethod::Space
        } else {
            LoginMethod::Click
        }
    }
    /// (max restarts, time window); max 0 = guard off
    pub fn restart_guard(&self) -> (u32, Duration) {
        (self.0[GUARD_MAX], Duration::from_secs(self.0[GUARD_MINUTES] as u64 * 60))
    }
    pub fn save_screenshots(&self) -> bool {
        self.0[SAVE_SCREENSHOTS] != 0
    }
    pub fn screenshot_every(&self) -> u32 {
        self.0[SCREENSHOT_EVERY].max(1)
    }
    pub fn keep_screenshots(&self) -> usize {
        self.0[KEEP_SCREENSHOTS] as usize
    }
    pub fn keep_logs(&self) -> usize {
        self.0[KEEP_LOGS] as usize
    }
    pub fn full_ocr_log(&self) -> bool {
        self.0[FULL_OCR_LOG] != 0
    }
    pub fn overlay(&self) -> OverlaySettings {
        OverlaySettings {
            on: self.0[OVERLAY_ON] != 0,
            size_pct: self.0[OVERLAY_SIZE],
            opacity_pct: self.0[OVERLAY_OPACITY],
            v: self.0[OVERLAY_V],
            h: self.0[OVERLAY_H],
            margin: self.0[OVERLAY_MARGIN],
        }
    }
    pub fn log_max_bytes(&self) -> u64 {
        self.0[LOG_MAX_MB] as u64 * 1024 * 1024
    }
}

/// The notification box settings (position: v 0 top / 1 middle / 2 bottom, h 0 left / 1 centre / 2 right).
#[derive(Clone, Copy, Debug)]
pub struct OverlaySettings {
    pub on: bool,
    pub size_pct: u32,
    pub opacity_pct: u32,
    pub v: u32,
    pub h: u32,
    pub margin: u32,
}

/// Clamp to the allowed range and snap to the step.
fn snap_with(p: &Param, v: u32) -> u32 {
    let v = v.clamp(p.min, p.max);
    let snapped = p.min + ((v - p.min + p.step / 2) / p.step) * p.step;
    snapped.min(p.max)
}

pub fn snap(idx: usize, v: u32) -> u32 {
    snap_with(&PARAMS[idx], v)
}

pub fn snap_general(idx: usize, v: u32) -> u32 {
    snap_with(&GENERAL_PARAMS[idx], v)
}

pub fn format_value(p: &Param, v: u32) -> String {
    match p.unit {
        Unit::Ms => format!("{v} ms"),
        Unit::Seconds => format!("{v} s"),
        Unit::Minutes => format!("{v} min"),
        Unit::Count => format!("{v}"),
        Unit::CountOff => if v == 0 { "Off".to_string() } else { format!("{v}") },
        Unit::Percent => format!("{v} %"),
        Unit::Pixels => format!("{v}p"),
        Unit::Megabytes => format!("{v} MB"),
        Unit::Px => format!("{v} px"),
        Unit::Toggle => (if v != 0 { "On" } else { "Off" }).to_string(),
        Unit::Choice => p.options.get(v as usize).copied().unwrap_or("?").to_string(),
    }
}

// ---------------- presets ----------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Eco,
    Balanced,
    Turbo,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Eco, Preset::Balanced, Preset::Turbo, Preset::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Eco => "Eco - low-end PCs",
            Preset::Balanced => "Balanced",
            Preset::Turbo => "Turbo - high-end PCs",
            Preset::Custom => "Custom",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Preset::Eco => "Slower scans and longer waits for gentle CPU use. Best for slow or busy PCs. Detection accuracy is unchanged.",
            Preset::Balanced => "Good reaction time with low CPU use. Recommended for most PCs.",
            Preset::Turbo => "Fastest reactions: frequent scans and short waits, using more CPU. Detection accuracy is unchanged.",
            Preset::Custom => "Your own values. The scanning, startup and system sliders below can only be changed in this mode.",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Preset::Eco => "eco",
            Preset::Balanced => "balanced",
            Preset::Turbo => "turbo",
            Preset::Custom => "custom",
        }
    }

    fn from_key(k: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.key() == k)
    }

    fn fixed_values(self) -> Option<Values> {
        match self {
            Preset::Eco => Some(ECO),
            Preset::Balanced => Some(BALANCED),
            Preset::Turbo => Some(TURBO),
            Preset::Custom => None,
        }
    }
}

// ---------------- store ----------------
#[derive(Clone)]
pub struct Store {
    pub preset: Preset,
    pub custom: Values,
    /// false until the user has used Custom once; then Custom starts from the preset they came from.
    pub custom_saved: bool,
    pub general: General,
    /// id of the game language (see languages.rs); empty = English
    pub language: String,
}

static STORE: Mutex<Store> =
    Mutex::new(Store { preset: Preset::Balanced, custom: BALANCED, custom_saved: false, general: GENERAL_DEFAULT, language: String::new() });
static EFFECTIVE: Mutex<Values> = Mutex::new(BALANCED);
static GENERAL: Mutex<General> = Mutex::new(GENERAL_DEFAULT);

fn effective_of(st: &Store) -> Values {
    st.preset.fixed_values().unwrap_or(st.custom)
}

fn refresh(st: &Store) {
    let v = effective_of(st);
    *lock(&EFFECTIVE) = v;
    *lock(&GENERAL) = st.general;
    apply_priority(v.low_priority());
}

fn apply_priority(low: bool) {
    unsafe {
        let class = if low { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
        let _ = SetPriorityClass(GetCurrentProcess(), class);
    }
}

/// The chosen game language id ("en" until one is chosen).
pub fn language_id() -> String {
    let id = lock(&STORE).language.clone();
    if id.trim().is_empty() {
        "en".to_string()
    } else {
        id
    }
}

pub fn set_language(id: &str) {
    lock(&STORE).language = id.to_string();
    save();
}

pub fn is_custom() -> bool {
    lock(&STORE).preset == Preset::Custom
}

/// The preset values in effect right now (cheap: copies 14 numbers).
pub fn current() -> Values {
    *lock(&EFFECTIVE)
}

pub fn general() -> General {
    *lock(&GENERAL)
}

pub fn store() -> Store {
    lock(&STORE).clone()
}

pub fn set_preset(p: Preset) {
    {
        let mut st = lock(&STORE);
        if p == Preset::Custom && !st.custom_saved {
            st.custom = effective_of(&st); // start from whatever was active
            st.custom_saved = true;
        }
        st.preset = p;
        refresh(&st);
    }
    save();
}

/// Live change while a slider is dragged (no disk write; call `save()` when the drag ends).
pub fn set_custom_value(idx: usize, v: u32) {
    let mut st = lock(&STORE);
    st.custom.0[idx] = snap(idx, v);
    st.custom_saved = true;
    st.preset = Preset::Custom;
    refresh(&st);
}

pub fn set_general_value(idx: usize, v: u32) {
    let mut st = lock(&STORE);
    st.general.0[idx] = snap_general(idx, v);
    refresh(&st);
}

pub fn reset_custom() {
    {
        let mut st = lock(&STORE);
        st.custom = BALANCED;
        st.custom_saved = true;
        refresh(&st);
    }
    save();
}

// ---------------- json (settings.json and import/export share the format) ----------------
fn to_json(st: &Store) -> serde_json::Value {
    let mut custom = serde_json::Map::new();
    for (i, p) in PARAMS.iter().enumerate() {
        custom.insert(p.key.to_string(), serde_json::json!(st.custom.0[i]));
    }
    let mut general = serde_json::Map::new();
    for (i, p) in GENERAL_PARAMS.iter().enumerate() {
        general.insert(p.key.to_string(), serde_json::json!(st.general.0[i]));
    }
    serde_json::json!({
        "app": "wuwa-apr",
        "preset": st.preset.key(),
        "custom_saved": st.custom_saved,
        "language": if st.language.is_empty() { "en" } else { st.language.as_str() },
        "custom": custom,
        "general": general,
    })
}

/// Applies whatever recognised keys `data` contains on top of `st`. Returns how many it recognised.
fn apply_json(st: &mut Store, data: &serde_json::Value) -> usize {
    let mut found = 0;
    if let Some(p) = data.get("preset").and_then(|v| v.as_str()).and_then(Preset::from_key) {
        st.preset = p;
        found += 1;
    }
    if let Some(l) = data.get("language").and_then(|v| v.as_str()) {
        st.language = l.to_string();
        found += 1;
    }
    if let Some(b) = data.get("custom_saved").and_then(|v| v.as_bool()) {
        st.custom_saved = b;
    }
    if let Some(custom) = data.get("custom") {
        for (i, p) in PARAMS.iter().enumerate() {
            if let Some(v) = custom.get(p.key).and_then(|v| v.as_u64()) {
                st.custom.0[i] = snap(i, v.min(u32::MAX as u64) as u32);
                found += 1;
            }
        }
    }
    if let Some(general) = data.get("general") {
        for (i, p) in GENERAL_PARAMS.iter().enumerate() {
            if let Some(v) = general.get(p.key).and_then(|v| v.as_u64()) {
                st.general.0[i] = snap_general(i, v.min(u32::MAX as u64) as u32);
                found += 1;
            }
        }
    }
    found
}

pub fn export_json() -> String {
    serde_json::to_string_pretty(&to_json(&store())).unwrap_or_default()
}

/// Replaces the current settings with the ones in `text` (an exported settings file). Saves them too.
pub fn import_json(text: &str) -> Result<(), String> {
    let data: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("not a valid settings file ({e})"))?;
    if !data.is_object() {
        return Err("not a valid settings file".to_string());
    }
    let mut st = store();
    if apply_json(&mut st, &data) == 0 {
        return Err("no WuWa APR settings found in that file".to_string());
    }
    if st.preset == Preset::Custom {
        st.custom_saved = true;
    }
    refresh(&st);
    *lock(&STORE) = st;
    save();
    Ok(())
}

pub fn save() {
    let data = to_json(&store());
    if let Err(e) = config::save_settings_value(&data) {
        crate::log::log(format!("[CONFIG] Could not save settings: {e}"));
    }
}

/// Read settings.json (missing/broken -> defaults) and apply it. Call once at startup.
pub fn load() {
    let mut st = Store { preset: Preset::Balanced, custom: BALANCED, custom_saved: false, general: GENERAL_DEFAULT, language: String::new() };
    if let Ok(text) = std::fs::read_to_string(config::settings_file()) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
            apply_json(&mut st, &data);
        }
    }
    refresh(&st);
    *lock(&STORE) = st;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_inside_every_slider_range_and_on_the_step() {
        for v in [ECO, BALANCED, TURBO] {
            for (i, p) in PARAMS.iter().enumerate() {
                assert!(v.0[i] >= p.min && v.0[i] <= p.max, "{} out of range", p.key);
                assert_eq!((v.0[i] - p.min) % p.step, 0, "{} not on its step", p.key);
            }
        }
        for (i, p) in GENERAL_PARAMS.iter().enumerate() {
            let v = GENERAL_DEFAULT.0[i];
            assert!(v >= p.min && v <= p.max, "{} default out of range", p.key);
            assert_eq!((v - p.min) % p.step, 0, "{} default not on its step", p.key);
        }
    }

    #[test]
    fn detection_accuracy_is_identical_in_all_presets() {
        for i in [CONFIRMATIONS, LOGIN_THRESHOLD, PATCH_THRESHOLD] {
            assert_eq!(ECO.0[i], BALANCED.0[i]);
            assert_eq!(TURBO.0[i], BALANCED.0[i]);
        }
    }

    #[test]
    fn snap_clamps_and_rounds_to_the_step() {
        assert_eq!(snap(SCAN_INTERVAL, 10), 500);
        assert_eq!(snap(SCAN_INTERVAL, 99999), 5000);
        assert_eq!(snap(SCAN_INTERVAL, 1549), 1500);
        assert_eq!(snap(SCAN_INTERVAL, 1551), 1600);
        assert_eq!(snap(OCR_HEIGHT, 700), 720);
        assert_eq!(snap_general(GUARD_MINUTES, 7), 5);
        assert_eq!(snap_general(AUTO_LOGIN_METHOD, 9), 1);
    }

    #[test]
    fn idle_interval_is_never_faster_than_the_scan_interval() {
        let mut v = BALANCED;
        v.0[IDLE_INTERVAL] = 500;
        v.0[SCAN_INTERVAL] = 2000;
        assert_eq!(v.idle_interval(), v.scan_interval());
    }

    #[test]
    fn export_then_apply_round_trips() {
        let mut st = Store { preset: Preset::Custom, custom: TURBO, custom_saved: true, general: GENERAL_DEFAULT, language: String::new() };
        st.general.0[AUTO_LOGIN] = 1;
        st.general.0[AUTO_LOGIN_DELAY] = 7;
        let json = to_json(&st);
        let mut back = Store { preset: Preset::Eco, custom: ECO, custom_saved: false, general: GENERAL_DEFAULT, language: String::new() };
        assert!(apply_json(&mut back, &json) > 0);
        assert_eq!(back.preset, Preset::Custom);
        assert_eq!(back.custom, TURBO);
        assert_eq!(back.general.0[AUTO_LOGIN_DELAY], 7);
        assert!(back.general.auto_login());
        let mut with_lang = back.clone();
        with_lang.language = "ja".to_string();
        let mut other = Store { preset: Preset::Eco, custom: ECO, custom_saved: false, general: GENERAL_DEFAULT, language: String::new() };
        assert!(apply_json(&mut other, &to_json(&with_lang)) > 0);
        assert_eq!(other.language, "ja");
    }

    #[test]
    fn unrelated_json_is_rejected() {
        let mut st = Store { preset: Preset::Eco, custom: ECO, custom_saved: false, general: GENERAL_DEFAULT, language: String::new() };
        assert_eq!(apply_json(&mut st, &serde_json::json!({"hello": 1})), 0);
    }

    #[test]
    fn format_value_handles_off_and_choices() {
        assert_eq!(format_value(&GENERAL_PARAMS[GUARD_MAX], 0), "Off");
        assert_eq!(format_value(&GENERAL_PARAMS[GUARD_MAX], 3), "3");
        assert_eq!(format_value(&GENERAL_PARAMS[AUTO_LOGIN_METHOD], 1), "Space key");
    }
}
