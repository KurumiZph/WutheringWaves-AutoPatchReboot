//! Process-wide flags and the wake-up channel between the APR thread (and logging) and the UI thread.
//!
//! The UI never polls: anything that changes what it shows (a log line, the status text, "finished")
//! calls `wake()`, which posts ONE message to the UI window (further wakes are coalesced until the UI
//! has handled it). With nothing happening, the UI thread sleeps in GetMessage and uses no CPU.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Messages posted to the UI window (WM_APP = 0x8000).
pub const WM_APP_WAKE: u32 = 0x8001;
pub const WM_APP_SHOW: u32 = 0x8002;
pub const WM_APP_QUIT: u32 = 0x8003;
pub const WM_APP_OVERLAY_SHOW: u32 = 0x8004;
pub const WM_APP_OVERLAY_HIDE: u32 = 0x8005;

/// Set by tray "Quit".
static STOP: AtomicBool = AtomicBool::new(false);
/// Set by the APR thread when it has finished (login found, UID found, fatal error...).
static DONE: AtomicBool = AtomicBool::new(false);

static UI_HWND: AtomicIsize = AtomicIsize::new(0);
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

static STATUS: Mutex<String> = Mutex::new(String::new());
static STATUS_VERSION: AtomicU64 = AtomicU64::new(0);

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------- UI wake-up ----------------
/// Registers the UI window. Everything that happened before it existed (log lines, status, even
/// "finished") is waiting in the queues, so wake the UI once to pick it all up.
pub fn set_ui_hwnd(hwnd: isize) {
    UI_HWND.store(hwnd, Ordering::Relaxed);
    WAKE_PENDING.store(false, Ordering::Release);
    wake();
}

/// Posts a message to the UI window. Returns false if there is no window (yet) or posting failed.
pub fn post_ui(msg: u32) -> bool {
    let h = UI_HWND.load(Ordering::Relaxed);
    if h == 0 {
        return false;
    }
    unsafe { PostMessageW(HWND(h as _), msg, WPARAM(0), LPARAM(0)).is_ok() }
}

/// Ask the UI to refresh (logs / status / finished). Cheap; coalesced.
/// (The "already pending" flag is only set when a message was really posted - otherwise it would stay
/// set forever and every later wake-up would be swallowed.)
pub fn wake() {
    if UI_HWND.load(Ordering::Relaxed) == 0 {
        return; // no window yet: set_ui_hwnd() wakes the UI once it exists
    }
    if !WAKE_PENDING.swap(true, Ordering::AcqRel) && !post_ui(WM_APP_WAKE) {
        WAKE_PENDING.store(false, Ordering::Release);
    }
}

/// UI side: call first thing when handling WM_APP_WAKE.
pub fn clear_wake() {
    WAKE_PENDING.store(false, Ordering::Release);
}

// ---------------- flags ----------------
pub fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}
pub fn request_stop() {
    STOP.store(true, Ordering::Relaxed);
}
pub fn finish() {
    DONE.store(true, Ordering::Relaxed);
    wake();
}
pub fn is_done() -> bool {
    DONE.load(Ordering::Relaxed)
}

pub fn put_status(text: &str) {
    *lock(&STATUS) = text.to_string();
    STATUS_VERSION.fetch_add(1, Ordering::Relaxed);
    wake();
}
/// (version, text) - the UI re-applies the text only when the version changes.
pub fn status() -> (u64, String) {
    (STATUS_VERSION.load(Ordering::Relaxed), lock(&STATUS).clone())
}

/// Sleeps in small slices. Returns `false` if a stop was requested meanwhile.
pub fn sleep_interruptible(total: Duration) -> bool {
    let end = Instant::now() + total;
    loop {
        if stopping() {
            return false;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}
