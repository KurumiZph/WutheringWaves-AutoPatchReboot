//! Game process handling: detect, close, launch.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, NORMAL_PRIORITY_CLASS, PROCESS_TERMINATE};

use crate::config::GAME_PROCESS_NAMES;
use crate::installs;
use crate::log::log;
use crate::settings;
use crate::state::{sleep_interruptible, stopping};

fn utf16_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// (pid, exe name) of every running WuWa process.
pub fn game_processes() -> Vec<(u32, String)> {
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return found;
        };
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = utf16_to_string(&entry.szExeFile);
                if GAME_PROCESS_NAMES.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    found.push((entry.th32ProcessID, name));
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

pub fn game_pids() -> Vec<u32> {
    game_processes().into_iter().map(|(pid, _)| pid).collect()
}

pub fn is_game_running() -> bool {
    !game_processes().is_empty()
}

fn terminate(pid: u32) {
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
        }
    }
}

/// True once no game process has been present for `stable` in a row.
fn wait_until_absent(timeout: Duration, stable: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut absent_since: Option<Instant> = None;
    while Instant::now() < deadline {
        if !is_game_running() {
            let since = *absent_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= stable {
                return true;
            }
        } else {
            absent_since = None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

pub fn close_game() {
    let processes = game_processes();
    if processes.is_empty() {
        log("[WuWa] No game process found.");
        return;
    }
    log("[WuWa] Closing Wuthering Waves...");
    for (pid, name) in &processes {
        log(format!("[WuWa] Terminating {name} (PID {pid})"));
        terminate(*pid);
    }
    if wait_until_absent(Duration::from_secs(8), Duration::from_millis(300)) {
        log("[WuWa] Game process is gone.");
        return;
    }

    log("[WuWa] Game did not close normally. Force closing...");
    for pid in game_pids() {
        terminate(pid);
    }
    if wait_until_absent(Duration::from_secs(6), Duration::from_millis(300)) {
        log("[WuWa] Game process is gone (force closed).");
        return;
    }
    log("[WuWa] WARNING: WuWa process may still be running.");
}

/// Blocks until the game can be started again: no WuWa process AND (Steam installs) Steam no longer
/// marks the app as running. Returns as soon as that has held for the 'stopped confirmation time' setting, so fast PCs
/// don't wait and slow PCs don't hit Steam's "already running" error.
pub fn wait_until_launchable(exe: &Path) {
    let steam_app = if installs::get_game_source(exe) == Some("steam") { installs::steam_app_id(exe) } else { None };
    match steam_app {
        Some(id) => {
            log(format!("[WuWa] Waiting for the game and Steam (app {id}) to report 'stopped'..."));
            if installs::steam_app_running(id).is_none() {
                log("[WuWa] Steam's running-state registry value was not found; using the process check only.");
            }
        }
        None => log("[WuWa] Waiting for the game to stop completely..."),
    }

    let start = Instant::now();
    let mut idle_since: Option<Instant> = None;
    let mut last_report = Instant::now();
    loop {
        if stopping() {
            return;
        }
        let v = settings::current(); // re-read every pass: the settings window changes it live
        let process_up = is_game_running();
        let steam_up = steam_app.and_then(installs::steam_app_running);
        if process_up || steam_up == Some(true) {
            idle_since = None;
            if last_report.elapsed() >= Duration::from_secs(5) {
                last_report = Instant::now();
                log(format!("[WuWa] Still stopping... (process running: {process_up}, Steam says running: {steam_up:?})"));
            }
        } else {
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= v.stop_stable() {
                log(format!("[WuWa] Game fully stopped after {:.1}s. Relaunching.", start.elapsed().as_secs_f32()));
                return;
            }
        }
        if start.elapsed() >= v.stop_timeout() {
            log("[WuWa] WARNING: timed out waiting for the game to stop; launching anyway.");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Launches exactly one game process with the required resource-tier argument.
pub fn launch_game(exe: &Path) -> bool {
    if !exe.is_file() {
        log(format!("[ERROR] Game executable was not found: {}", exe.display()));
        log("[ERROR] Open the Misc tab, press \"Forget\" under Game install, then restart APR to choose the game again.");
        return false;
    }
    if is_game_running() {
        log("[WuWa] Game process is already running; launch skipped.");
        return true;
    }

    let source = installs::get_game_source(exe);
    let Some(tier) = installs::highest_resource_tier(exe) else {
        log("[WuWa] Could not determine installed resource tier; launch aborted.");
        if let Some(dir) = exe.parent() {
            log(format!("[WuWa] Checked relative to: {}", dir.display()));
        }
        return false;
    };

    // -krqlv is required by current tiered-client builds.
    let arg = format!("-krqlv={tier}");
    log(format!("[WuWa] Install source: {}", source.unwrap_or("Unknown")));
    log(format!("[WuWa] Launching: {}", exe.display()));
    log(format!("[WuWa] Resource tier: {}", tier.to_uppercase()));
    log(format!("[WuWa] Arguments: {arg}"));

    let mut cmd = Command::new(exe);
    cmd.arg(&arg);
    // APR may run at below-normal priority and a child would inherit that: force NORMAL for the game.
    cmd.creation_flags(NORMAL_PRIORITY_CLASS.0);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    match cmd.spawn() {
        Ok(child) => {
            log(format!("[WuWa] Launcher process PID: {}", child.id()));
            log("[WuWa] Launch command accepted by Windows.");
            true
        }
        Err(e) => {
            log(format!("[WuWa] Launch failed: {e}"));
            false
        }
    }
}

pub fn wait_for_game_process(timeout: Duration) -> bool {
    log("[WuWa] Waiting for game process...");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if stopping() {
            return false;
        }
        if is_game_running() {
            log("[WuWa] Game process detected.");
            return true;
        }
        if !sleep_interruptible(Duration::from_millis(500)) {
            return false;
        }
    }
    log("[WuWa] Timed out waiting for the game process.");
    false
}
