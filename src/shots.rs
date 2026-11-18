//! Saved screenshots (Settings -> Debugging -> Save screenshots).
//!   screenshots\scan_<time>_<scan#>.png - what the OCR saw; only the newest N are kept
//!   screenshots\patch_<time>.png        - the moment a patch screen was confirmed; never deleted automatically
//! The user id is blacked out before anything is saved.

use std::path::Path;

use crate::config;
use crate::image_ops::Frame;
use crate::log::{log, timestamp};
use crate::settings;

fn stamp() -> String {
    timestamp().replace(' ', "_").replace(':', "-")
}

/// Keeps only the newest `keep` scan screenshots.
fn prune_scans(dir: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut scans: Vec<_> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name.starts_with("scan_") && name.ends_with(".png")
        })
        .collect();
    scans.sort(); // time stamps sort in time order
    while scans.len() > keep {
        let _ = std::fs::remove_file(scans.remove(0));
    }
}

pub fn save_scan(frame: &Frame, scan: u64) {
    let dir = config::screenshots_dir();
    let path = dir.join(format!("scan_{}_{scan:06}.png", stamp()));
    match frame.save_png(&path) {
        Ok(()) => prune_scans(&dir, settings::general().keep_screenshots()),
        Err(e) => log(format!("[SCREENSHOT] Could not save a screenshot: {e}")),
    }
}

pub fn save_patch(frame: &Frame) {
    let path = config::screenshots_dir().join(format!("patch_{}.png", stamp()));
    match frame.save_png(&path) {
        Ok(()) => log(format!("[SCREENSHOT] Saved the patch screen: {}", path.display())),
        Err(e) => log(format!("[SCREENSHOT] Could not save the patch screenshot: {e}")),
    }
}
