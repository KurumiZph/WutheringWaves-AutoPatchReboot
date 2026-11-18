//! Finding Wuthering Waves installations (Steam / standalone launcher), the saved
//! selection, and the picker prompts.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use windows::Win32::Storage::FileSystem::GetLogicalDrives;
use windows::Win32::UI::WindowsAndMessaging::{
    IDNO, IDOK, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_OKCANCEL, MB_YESNO, MB_YESNOCANCEL,
};

use crate::config::{self, EXE_SEARCH_NAMES};
use crate::log::log;
use crate::ui;

#[derive(Clone, Debug)]
pub struct Installation {
    pub path: String,
    pub source: String, // "steam" | "launcher"
}

// ---------------- small path helpers ----------------

fn abspath(p: &str) -> String {
    std::path::absolute(p).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
}

fn norm_key(p: &str) -> String {
    abspath(p).replace('/', "\\").to_lowercase()
}

/// Bounded top-down search for any of `names` (like os.walk with a depth limit).
fn find_exe_in(base: &Path, names: &[&str], max_depth: usize) -> Option<PathBuf> {
    fn walk(dir: &Path, names: &[&str], depth: usize, max: usize) -> Option<PathBuf> {
        let read = std::fs::read_dir(dir).ok()?;
        let mut files: Vec<String> = Vec::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for entry in read.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(entry.path()),
                Ok(_) => files.push(entry.file_name().to_string_lossy().into_owned()),
                Err(_) => {}
            }
        }
        for name in names {
            if let Some(real) = files.iter().find(|f| f.eq_ignore_ascii_case(name)) {
                return Some(dir.join(real));
            }
        }
        if depth < max {
            for d in dirs {
                if let Some(found) = walk(&d, names, depth + 1, max) {
                    return Some(found);
                }
            }
        }
        None
    }
    if !base.is_dir() {
        return None;
    }
    walk(base, names, 0, max_depth)
}

// ---------------- Steam ----------------

fn find_steam_install_path() -> Option<String> {
    let keys = [
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ];
    for (hive, subkey, value_name) in keys {
        if let Ok(key) = RegKey::predef(hive).open_subkey(subkey) {
            if let Ok(value) = key.get_value::<String, _>(value_name) {
                if !value.is_empty() && Path::new(&value).is_dir() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Pulls every `"path"  "..."` value out of libraryfolders.vdf.
fn vdf_paths(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix("\"path\"") {
            if let Some(inner) = rest.trim().strip_prefix('"') {
                if let Some(end) = inner.find('"') {
                    out.push(inner[..end].replace("\\\\", "\\"));
                }
            }
        }
    }
    out
}

fn find_steam_library_folders() -> Vec<String> {
    let Some(steam) = find_steam_install_path() else {
        return Vec::new();
    };
    let mut libraries = vec![steam.clone()];
    let vdf = Path::new(&steam).join("steamapps").join("libraryfolders.vdf");
    if let Ok(content) = std::fs::read_to_string(vdf) {
        for p in vdf_paths(&content) {
            if !libraries.contains(&p) {
                libraries.push(p);
            }
        }
    }
    libraries
}

fn steam_manifest_is_wuwa(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines().any(|line| {
        let parts: Vec<&str> = line.split('"').collect(); // ["", key, " ", value, ""]
        parts.len() >= 4
            && parts[1].eq_ignore_ascii_case("installdir")
            && parts[3].eq_ignore_ascii_case("Wuthering Waves")
    })
}

// ---------------- other discovery methods ----------------

/// Non-Steam installs registered in "Uninstall".
fn find_game_exe_via_registry(names: &[&str]) -> Option<PathBuf> {
    let keys = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    for (hive, subkey) in keys {
        let Ok(key) = RegKey::predef(hive).open_subkey(subkey) else {
            continue;
        };
        for name in key.enum_keys().flatten() {
            let Ok(entry) = key.open_subkey(&name) else {
                continue;
            };
            let Ok(display) = entry.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if !display.to_lowercase().contains("wuthering waves") {
                continue;
            }
            let Ok(location) = entry.get_value::<String, _>("InstallLocation") else {
                continue;
            };
            if let Some(found) = find_exe_in(Path::new(&location), names, 3) {
                return Some(found);
            }
        }
    }
    None
}

/// Last-ditch guess: typical folder names on every drive letter.
fn find_game_exe_common_locations(names: &[&str]) -> Option<PathBuf> {
    let folders = [
        "Wuthering Waves",
        "Wuthering Waves Game",
        r"Wuthering Waves\Wuthering Waves Game",
        r"wuwa\Wuthering Waves Game",
        r"Games\Wuthering Waves",
        r"Program Files\Wuthering Waves",
    ];
    let bits = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if bits & (1 << i) == 0 {
            continue;
        }
        let drive = format!("{}:\\", (b'A' + i as u8) as char);
        for folder in folders {
            if let Some(found) = find_exe_in(&Path::new(&drive).join(folder), names, 4) {
                return Some(found);
            }
        }
    }
    None
}

fn dedupe_existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in paths {
        if !p.is_file() {
            continue;
        }
        if seen.insert(norm_key(&p.to_string_lossy())) {
            out.push(p);
        }
    }
    out
}

pub fn find_game_exe_candidates() -> Vec<PathBuf> {
    let names: &[&str] = &EXE_SEARCH_NAMES;
    let mut candidates = Vec::new();
    for library in find_steam_library_folders() {
        let dir = Path::new(&library).join("steamapps").join("common").join("Wuthering Waves");
        if let Some(found) = find_exe_in(&dir, names, 3) {
            candidates.push(found);
        }
    }
    if let Some(found) = find_game_exe_via_registry(names) {
        candidates.push(found);
    }
    if let Some(found) = find_game_exe_common_locations(names) {
        candidates.push(found);
    }
    dedupe_existing(candidates)
}

// ---------------- install classification ----------------

/// App id of the Steam appmanifest describing the Wuthering Waves install that contains `exe`
/// (read from the file name `appmanifest_<id>.acf`, so nothing is hard-coded).
pub fn steam_app_id(exe: &Path) -> Option<u32> {
    let game_dir = exe.parent()?;
    let game_dir = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    for parent in game_dir.ancestors() {
        let steamapps = parent.join("steamapps");
        if !steamapps.is_dir() {
            continue;
        }
        if let Ok(read) = std::fs::read_dir(&steamapps) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if let Some(id) = name.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) {
                    if steam_manifest_is_wuwa(&entry.path()) {
                        if let Ok(id) = id.parse::<u32>() {
                            return Some(id);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Does Steam currently consider this app running? (This is the state behind the Play / Stop button.)
/// `None` = Steam's registry values are not there, so only the process check can be used.
pub fn steam_app_running(app_id: u32) -> Option<bool> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut known = false;
    let mut running = false;
    if let Ok(key) = hkcu.open_subkey(format!(r"Software\Valve\Steam\Apps\{app_id}")) {
        if let Ok(v) = key.get_value::<u32, _>("Running") {
            known = true;
            running |= v != 0;
        }
    }
    if let Ok(key) = hkcu.open_subkey(r"Software\Valve\Steam") {
        if let Ok(v) = key.get_value::<u32, _>("RunningAppID") {
            known = true;
            running |= v == app_id;
        }
    }
    if known { Some(running) } else { None }
}

/// "steam" | "launcher" | None
pub fn get_game_source(exe: &Path) -> Option<&'static str> {
    // Check Steam FIRST: Steam installs can also contain launcherDownloadConfig.json.
    if steam_app_id(exe).is_some() {
        return Some("steam");
    }
    let game_dir = exe.parent()?;
    let game_dir = std::fs::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    for parent in game_dir.ancestors() {
        if parent.join("launcherDownloadConfig.json").is_file() {
            return Some("launcher");
        }
    }
    None
}

/// Highest installed resource tier (UHD > HD > SD), re-detected on every launch.
pub fn highest_resource_tier(exe: &Path) -> Option<&'static str> {
    let game_dir = exe.parent()?;
    for tier in ["uhd", "hd", "sd"] {
        if game_dir.join("launcherDownload").join(tier).join("OriginResource.json").is_file() {
            return Some(tier);
        }
    }
    let config_dir = game_dir.join("launcherDownloadConfig");
    for tier in ["uhd", "hd", "sd"] {
        if config_dir.join(format!("{tier}.json")).is_file() {
            return Some(tier);
        }
    }
    None
}

// ---------------- persisted selection ----------------

struct SavedConfig {
    installations: Vec<Installation>,
    last_used: Option<String>,
    remember_selection: bool,
}

fn load_installation_config() -> SavedConfig {
    let data = config::load_config();
    let mut installations = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(list) = data.get("installations").and_then(|v| v.as_array()) {
        for entry in list {
            // entries were plain strings in old configs, objects in newer ones
            let (path, source) = match entry {
                Value::String(s) => (Some(s.clone()), None),
                Value::Object(o) => (
                    ["path", "game_exe", "exe"].iter().find_map(|k| o.get(*k).and_then(|v| v.as_str()).map(String::from)),
                    o.get("source").and_then(|v| v.as_str()).map(String::from),
                ),
                _ => (None, None),
            };
            let Some(path) = path else { continue };
            if !Path::new(&path).is_file() || !seen.insert(norm_key(&path)) {
                continue;
            }
            let source = source
                .or_else(|| get_game_source(Path::new(&path)).map(String::from))
                .unwrap_or_default();
            installations.push(Installation { path: abspath(&path), source });
        }
    }

    // Existing configs pre-date the toggle, so default to remembering.
    SavedConfig {
        installations,
        last_used: data.get("last_used").and_then(|v| v.as_str()).map(abspath),
        remember_selection: data.get("remember_selection").and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

fn save_installation_config(installs: &[Installation], last_used: Option<&str>, remember: bool) {
    let mut seen = std::collections::HashSet::new();
    let clean: Vec<Value> = installs
        .iter()
        .filter(|i| seen.insert(norm_key(&i.path)))
        .map(|i| json!({ "path": abspath(&i.path), "source": i.source }))
        .collect();
    let mut data = json!({ "installations": clean, "remember_selection": remember });
    if let Some(last) = last_used {
        data["last_used"] = json!(abspath(last));
    }
    config::save_config(&data);
}

// ---------------- picker ----------------

fn source_label(source: &str) -> &'static str {
    match source {
        "steam" => "Steam",
        "launcher" => "Standalone launcher",
        _ => "Unknown",
    }
}

/// Shows each installation in turn (Yes = use it, No = next, Cancel = quit), then asks
/// whether to remember the choice. Returns (chosen path, remember).
fn prompt_select_installation(
    installs: &[Installation],
    remembered: Option<&str>,
    remember_default: bool,
) -> (Option<String>, bool) {
    // start with the remembered installation, if any
    let mut order: Vec<&Installation> = installs.iter().collect();
    if let Some(r) = remembered {
        let key = norm_key(r);
        if let Some(pos) = order.iter().position(|i| norm_key(&i.path) == key) {
            let first = order.remove(pos);
            order.insert(0, first);
        }
    }

    let mut idx = 0usize;
    let chosen = loop {
        let inst = order[idx];
        let multiple = order.len() > 1;
        let text = format!(
            "Run Wuthering Waves from this installation?\n\n[{}]\n{}\n\n{}",
            source_label(&inst.source),
            inst.path,
            if multiple { "Yes = use this one   No = show the next one   Cancel = exit" } else { "OK = use it   Cancel = exit" }
        );
        let buttons = if multiple { MB_YESNOCANCEL } else { MB_OKCANCEL };
        let style = buttons | MB_ICONQUESTION;
        let answer = ui::message_box(&text, "WuWa APR - Select Installation", style);
        if answer == IDYES || answer == IDOK {
            break Some(inst.path.clone());
        } else if answer == IDNO {
            idx = (idx + 1) % order.len();
        } else {
            break None; // IDCANCEL / closed
        }
    };

    let Some(path) = chosen else {
        return (None, remember_default);
    };
    let remember_answer = ui::message_box(
        "Remember this selection and start it automatically on the next runs?",
        "WuWa APR",
        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
    );
    (Some(path), remember_answer == IDYES)
}

// ---------------- main entry ----------------

pub fn resolve_game_exe() -> Option<PathBuf> {
    // Always run a fresh discovery first, so a stale remembered path can't hide a moved install.
    let mut unique: Vec<Installation> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for path in find_game_exe_candidates() {
        let Some(source) = get_game_source(&path) else { continue };
        let p = abspath(&path.to_string_lossy());
        if seen.insert(norm_key(&p)) {
            unique.push(Installation { path: p, source: source.to_string() });
        }
    }

    let saved = load_installation_config();
    // keep still-valid saved installs that discovery didn't return
    for entry in &saved.installations {
        let Some(source) = get_game_source(Path::new(&entry.path)) else { continue };
        if seen.insert(norm_key(&entry.path)) {
            unique.push(Installation { path: entry.path.clone(), source: source.to_string() });
        }
    }

    log(format!("[SYSTEM] Detected {} valid Wuthering Waves installation(s).", unique.len()));
    for i in &unique {
        log(format!("[SYSTEM] Installation: [{}] {}", i.source, i.path));
    }
    if unique.is_empty() {
        log("[SYSTEM] No Wuthering Waves installation was detected.");
        return None;
    }

    let remembered_valid = saved
        .last_used
        .as_ref()
        .map(|r| unique.iter().any(|i| norm_key(&i.path) == norm_key(r)))
        .unwrap_or(false);

    // A valid last-used path is auto-run when "remember" is on.
    if saved.remember_selection && remembered_valid {
        let last = saved.last_used.clone().unwrap_or_default();
        save_installation_config(&unique, Some(&last), true);
        log(format!("[SYSTEM] Using last-used installation: {last}"));
        return Some(PathBuf::from(last));
    }

    let (selected, remember) =
        prompt_select_installation(&unique, saved.last_used.as_deref(), saved.remember_selection);
    let Some(selected) = selected else {
        log("[SYSTEM] No installation selected. Exiting.");
        return None;
    };
    save_installation_config(&unique, Some(&selected), remember);
    if remember {
        log(format!("[SYSTEM] Selected and remembered installation: {selected}"));
    } else {
        log(format!("[SYSTEM] Selected installation for this run only: {selected}"));
    }
    Some(PathBuf::from(selected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vdf_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"E:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(vdf_paths(vdf), vec![r"C:\Program Files (x86)\Steam", r"E:\SteamLibrary"]);
    }
}
