//! Logging.  Two files are kept:
//!   logs\ocr_log_min.txt  - concise live log (also shown in the Live Logs tab)
//!   logs\ocr_log_full.txt - only with "Full OCR log" on: every line plus the per-scan OCR dump
//! Versioning: at every start the previous file becomes `<name>_<time>.txt`, and so does a file that
//! reaches the size limit while running. Only the newest N versions are kept (Settings -> Debugging).

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::config;
use crate::detect::Ocr;
use crate::state::{lock, put_status};

struct LogFile {
    file: Option<File>,
    path: PathBuf,
    bytes: u64,
}

struct Files {
    min: Option<LogFile>,
    full: Option<LogFile>,
}

static FILES: Mutex<Files> = Mutex::new(Files { min: None, full: None });
static QUEUE: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const QUEUE_CAP: usize = 5000;

/// "ocr_log_min.txt" + time stamp -> "ocr_log_min_2026-11-09_12-00-00.txt"
fn archive_name(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "log".to_string());
    let stamp = timestamp().replace(' ', "_").replace(':', "-");
    path.with_file_name(format!("{stem}_{stamp}.txt"))
}

/// Deletes the oldest time-stamped versions of `path`, keeping the newest `keep`.
fn prune(path: &Path, keep: usize) {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().map(|s| s.to_string_lossy().into_owned())) else { return };
    let prefix = format!("{stem}_");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    name.starts_with(&prefix) && name.ends_with(".txt")
                })
                .collect()
        })
        .unwrap_or_default();
    versions.sort(); // time stamps sort in time order
    while versions.len() > keep {
        let _ = std::fs::remove_file(versions.remove(0));
    }
}

/// Moves the current log aside as a time-stamped version and prunes the old ones.
fn rotate(path: &Path) {
    let _ = std::fs::rename(path, archive_name(path));
    prune(path, crate::settings::general().keep_logs());
}

fn open_fresh(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn open_rotating(path: PathBuf) -> Option<LogFile> {
    if path.exists() {
        rotate(&path); // the previous session becomes a version
    }
    let file = open_fresh(&path)?;
    Some(LogFile { file: Some(file), path, bytes: 0 })
}

/// Call once at startup (before the first log() call). The full OCR log is opened only when it is turned on.
pub fn init() {
    let mut files = lock(&FILES);
    files.min = open_rotating(config::log_min());
}

pub fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

fn write_line(slot: &mut Option<LogFile>, line: &str) {
    let Some(lf) = slot.as_mut() else { return };
    if lf.bytes >= crate::settings::general().log_max_bytes() {
        lf.file = None; // close, move aside, start again
        rotate(&lf.path);
        lf.file = open_fresh(&lf.path);
        lf.bytes = 0;
    }
    if let Some(f) = lf.file.as_mut() {
        let _ = writeln!(f, "{line}");
        lf.bytes += line.len() as u64 + 2;
    }
}

/// Timestamped line -> MIN file, FULL file and the Live Logs tab.
pub fn log(message: impl AsRef<str>) {
    let message = message.as_ref();
    let ts = timestamp();
    let lines: Vec<&str> = if message.is_empty() { vec![""] } else { message.lines().collect() };
    for raw in lines {
        let line = if raw.is_empty() { format!("[{ts}]") } else { format!("[{ts}] {raw}") };
        {
            let mut files = lock(&FILES);
            write_line(&mut files.min, &line);
            write_full(&mut files, &line);
        }
        let mut q = lock(&QUEUE);
        if q.len() >= QUEUE_CAP {
            q.pop_front();
        }
        q.push_back(line);
    }
    crate::state::wake();
}

/// The full OCR log (only while "Full OCR log" is on in Settings; the file is created on first use).
fn write_full(files: &mut Files, line: &str) {
    if !crate::settings::general().full_ocr_log() {
        return;
    }
    if files.full.is_none() {
        files.full = open_rotating(config::log_full());
    }
    write_line(&mut files.full, line);
}

/// Full-log file only.
pub fn full_line(line: &str) {
    let mut files = lock(&FILES);
    write_full(&mut files, line);
}

/// Everything not yet shown in the Live Logs tab.
pub fn drain_queue() -> Vec<String> {
    lock(&QUEUE).drain(..).collect()
}

/// Raw per-scan OCR dump for the FULL log (already UID-masked, tokens are normalised).
pub fn log_full_ocr(ocr: &Ocr, scan_number: u64) {
    let ts = timestamp();
    full_line(&format!("[{ts}] [OCR FULL] SCAN {scan_number:06} ({}x{})", ocr.width, ocr.height));
    for w in &ocr.words {
        full_line(&format!(
            "[{ts}] TEXT={:?} | BOX=({:.0},{:.0},{:.0},{:.0})",
            w.text, w.left, w.top, w.width, w.height
        ));
    }
    full_line(&format!("[{ts}] RAW TEXT: {}", ocr.text()));
}

/// Same for the close-up of the bottom-right corner (the user id area), already masked.
pub fn log_full_ocr_corner(ocr: &Ocr, scan_number: u64) {
    let ts = timestamp();
    full_line(&format!("[{ts}] [OCR FULL] SCAN {scan_number:06} CORNER ({}x{}) RAW TEXT: {}", ocr.width, ocr.height, ocr.text()));
}

/// Updates the status text shown in the UI (and optionally the log).
pub fn set_status(text: &str, also_log: bool) {
    put_status(text);
    if also_log {
        log(text);
    }
}
