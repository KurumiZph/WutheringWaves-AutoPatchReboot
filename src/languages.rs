//! Game languages: what the app looks for on screen in each language (login text, patch notice, the
//! "user id" label), which Windows OCR language reads it, and how it is matched.
//!
//! Ten languages are compiled in. `languages.json` in the data folder is loaded at start-up (and on
//! "Reload languages"): it can change those texts after a game update, or add new languages, without
//! rebuilding the app. English is always available: if it is missing from the file, or the file is
//! broken, or the selected language was deleted, the app falls back to the built-in English.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::config;
use crate::state::lock;

/// How phrases are compared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchMode {
    /// Latin-style text: compare word by word (OCR returns separate words).
    Words,
    /// Chinese / Japanese / Korean / Thai: no reliable word breaks, so spaces are ignored and the phrase
    /// is searched as a run of characters.
    Chars,
}

#[derive(Clone, Debug)]
pub struct Language {
    pub id: String,
    pub name: String,
    /// Windows OCR language tag, e.g. "ja-JP"
    pub ocr: String,
    pub mode: MatchMode,
    /// the login screen text (any ONE of these)
    pub login: Vec<String>,
    /// the patch notice: ALL of these must be present
    pub patch: Vec<String>,
    /// labels in front of the user id number ("User ID", "UID", ...)
    pub uid: Vec<String>,
    /// optional per-language match strictness (0.0 - 1.0); the preset's value is used when absent
    pub login_threshold: Option<f64>,
    pub patch_threshold: Option<f64>,
}

impl Language {
    pub fn english() -> Language {
        builtin().into_iter().next().expect("built-in English")
    }
}

fn lang(
    id: &str,
    name: &str,
    ocr: &str,
    mode: MatchMode,
    login: &[&str],
    patch: &[&str],
    uid: &[&str],
    login_threshold: Option<f64>,
) -> Language {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Language {
        id: id.to_string(),
        name: name.to_string(),
        ocr: ocr.to_string(),
        mode,
        login: v(login),
        patch: v(patch),
        uid: v(uid),
        login_threshold,
        patch_threshold: None,
    }
}

/// The built-in languages. Only English has patch-notice text so far (it only shows up on real patches).
pub fn builtin() -> Vec<Language> {
    use MatchMode::{Chars, Words};
    vec![
        lang("en", "English", "en-US", Words, &["Tap to land in Solaris-3"],
            &["Patching complete", "Please restart the game"], &["User ID", "UID"], None),
        lang("fr", "Français (French)", "fr-FR", Words, &["Cliquer pour se connecter"], &[], &["Identifiant", "UID"], None),
        lang("de", "Deutsch (German)", "de-DE", Words, &["Tippe, um auf Solaris-3 zu landen"], &[], &["Benutzer-ID", "UID"], None),
        lang("es", "Español (Spanish)", "es-ES", Words, &["Toca para aterrizar en Solaris-3"], &[], &["UID"], None),
        lang("pt", "Português (Portuguese)", "pt-BR", Words, &["Toque para pousar em Solaris-3"], &[], &["UID"], None),
        lang("ja", "日本語 (Japanese)", "ja-JP", Chars, &["クリックして接続"], &[], &["ユーザーID", "UID"], Some(0.75)),
        lang("ko", "한국어 (Korean)", "ko-KR", Chars, &["솔라리스 연결"], &[], &["UID"], Some(0.75)),
        lang("zh-hans", "简体中文 (Chinese, Simplified)", "zh-Hans-CN", Chars, &["点击连接"], &[], &["特征码", "UID"], Some(0.75)),
        lang("zh-hant", "繁體中文 (Chinese, Traditional)", "zh-Hant-TW", Chars, &["點擊連線"], &[], &["特徵碼", "UID"], Some(0.75)),
        lang("th", "ภาษาไทย (Thai)", "th-TH", Chars, &["แตะเพื่อเข้าสู่โซลาริส-3"], &[], &["UID"], Some(0.75)),
    ]
}

// ---------------- the loaded list ----------------
static LANGS: Mutex<Vec<Language>> = Mutex::new(Vec::new());
/// (OCR tag in lowercase, installed?) for every language, filled in by a background check.
static PACKS: Mutex<Vec<(String, bool)>> = Mutex::new(Vec::new());
static PACKS_CHANGED: AtomicBool = AtomicBool::new(false);

pub fn list() -> Vec<Language> {
    let l = lock(&LANGS);
    if l.is_empty() {
        builtin()
    } else {
        l.clone()
    }
}

/// The language with this id, or English (+ true) if it does not exist (any more).
pub fn resolve(id: &str) -> (Language, bool) {
    let wanted = id.trim().to_lowercase();
    match list().into_iter().find(|l| l.id == wanted) {
        Some(l) => (l, false),
        None => (Language::english(), true),
    }
}

/// The language chosen in Settings.
pub fn selected() -> Language {
    resolve(&crate::settings::language_id()).0
}

// ---------------- languages.json ----------------
fn lang_to_json(l: &Language) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("id".into(), json!(l.id));
    o.insert("name".into(), json!(l.name));
    o.insert("ocr".into(), json!(l.ocr));
    o.insert("match".into(), json!(if l.mode == MatchMode::Chars { "chars" } else { "words" }));
    o.insert("login".into(), json!(l.login));
    o.insert("patch".into(), json!(l.patch));
    o.insert("uid".into(), json!(l.uid));
    if let Some(t) = l.login_threshold {
        o.insert("login_threshold".into(), json!(t));
    }
    if let Some(t) = l.patch_threshold {
        o.insert("patch_threshold".into(), json!(t));
    }
    Value::Object(o)
}

/// The text of the default languages.json (with a short explanation at the top).
pub fn default_file_text() -> String {
    let readme = [
        "WuWa APR language texts. Edit this file to fix a text after a game update or to add a language, then use Misc > Reload languages (or restart the app).",
        "id: short unique name (lowercase). name: shown in Settings. ocr: Windows OCR language tag (needs that OCR language pack installed in Windows).",
        "match: \"words\" for Latin-style text, \"chars\" for Chinese / Japanese / Korean / Thai (spaces are ignored).",
        "login: the login screen text - any ONE of the listed texts counts. patch: the patch notice - ALL listed texts must be on screen at once; leave it [] if you do not have the text yet (patch detection is then off for that language).",
        "uid: the label(s) in front of the user id number in the bottom-right corner, e.g. \"UID\" or \"User ID\".",
        "login_threshold / patch_threshold (optional, 0.5 - 1.0): how closely the screen must match; higher = stricter. Short CJK phrases need ~0.75.",
        "English is built in: if it is missing here, or this file is broken, or the selected language was removed, the app uses the built-in English.",
    ];
    let data = json!({
        "_readme": readme,
        "version": 1,
        "languages": builtin().iter().map(lang_to_json).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&data).unwrap_or_default()
}

fn str_list(v: Option<&Value>) -> Result<Vec<String>, String> {
    match v {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for it in items {
                match it.as_str() {
                    Some(s) if !s.trim().is_empty() => out.push(s.trim().to_string()),
                    Some(_) => {}
                    None => return Err("a list must contain only text".to_string()),
                }
            }
            Ok(out)
        }
        Some(_) => Err("expected a list of texts".to_string()),
    }
}

fn threshold(v: Option<&Value>, what: &str, notes: &mut Vec<String>, n: usize) -> Option<f64> {
    let t = v?.as_f64()?;
    if (0.5..=1.0).contains(&t) {
        Some(t)
    } else {
        notes.push(format!("[LANG] languages.json entry #{n}: {what} {t} is outside 0.5 - 1.0 and was ignored."));
        None
    }
}

/// Parses languages.json. Broken entries are skipped (with a note); a broken file is an error.
pub fn parse(text: &str) -> Result<(Vec<Language>, Vec<String>), String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let items = root.get("languages").and_then(|v| v.as_array()).ok_or("there is no \"languages\" list")?;
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let mut seen = HashSet::new();
    for (i, e) in items.iter().enumerate() {
        let n = i + 1;
        let skip = |why: &str, notes: &mut Vec<String>| notes.push(format!("[LANG] languages.json entry #{n} skipped: {why}."));
        let id = e.get("id").and_then(|v| v.as_str()).map(|s| s.trim().to_lowercase()).unwrap_or_default();
        if id.is_empty() {
            skip("it has no \"id\"", &mut notes);
            continue;
        }
        if !seen.insert(id.clone()) {
            skip(&format!("the id \"{id}\" is used twice"), &mut notes);
            continue;
        }
        let ocr = e.get("ocr").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).unwrap_or_default();
        if ocr.is_empty() {
            skip(&format!("\"{id}\" has no \"ocr\" language tag"), &mut notes);
            continue;
        }
        let login = match str_list(e.get("login")) {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => {
                skip(&format!("\"{id}\" has no \"login\" text"), &mut notes);
                continue;
            }
            Err(why) => {
                skip(&format!("\"{id}\": login: {why}"), &mut notes);
                continue;
            }
        };
        let (patch, uid) = match (str_list(e.get("patch")), str_list(e.get("uid"))) {
            (Ok(p), Ok(u)) => (p, u),
            (Err(why), _) | (_, Err(why)) => {
                skip(&format!("\"{id}\": {why}"), &mut notes);
                continue;
            }
        };
        let mode = match e.get("match").and_then(|v| v.as_str()).unwrap_or("words").to_lowercase().as_str() {
            "chars" => MatchMode::Chars,
            "words" => MatchMode::Words,
            other => {
                notes.push(format!("[LANG] languages.json \"{id}\": unknown match \"{other}\" - using \"words\"."));
                MatchMode::Words
            }
        };
        let name = e.get("name").and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
        out.push(Language {
            id,
            name,
            ocr,
            mode,
            login,
            patch,
            uid,
            login_threshold: threshold(e.get("login_threshold"), "login_threshold", &mut notes, n),
            patch_threshold: threshold(e.get("patch_threshold"), "patch_threshold", &mut notes, n),
        });
    }
    Ok((out, notes))
}

/// Loads languages.json (creating it from the built-in defaults if it does not exist). Returns notes to log.
pub fn load() -> Vec<String> {
    let path = config::languages_file();
    let mut notes = Vec::new();
    let mut langs = match std::fs::read_to_string(&path) {
        Ok(text) => match parse(&text) {
            Ok((l, n)) => {
                notes.extend(n);
                l
            }
            Err(e) => {
                notes.push(format!("[LANG] languages.json is not usable ({e}) - using the built-in languages. The file was left untouched."));
                builtin()
            }
        },
        Err(_) => {
            match std::fs::write(&path, default_file_text()) {
                Ok(()) => notes.push(format!("[LANG] Created {} with the built-in languages.", path.display())),
                Err(e) => notes.push(format!("[LANG] Could not create languages.json: {e}")),
            }
            builtin()
        }
    };
    if !langs.iter().any(|l| l.id == "en") {
        notes.push("[LANG] English is missing in languages.json - the built-in English is used.".to_string());
        langs.insert(0, Language::english());
    }
    notes.push(format!(
        "[LANG] {} language(s) loaded: {}",
        langs.len(),
        langs.iter().map(|l| l.id.as_str()).collect::<Vec<_>>().join(", ")
    ));
    *lock(&LANGS) = langs;
    notes
}

/// Overwrites languages.json with the built-in defaults (the old file is kept as languages.json.bak).
pub fn reset_to_defaults() -> Result<(), String> {
    let path = config::languages_file();
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("json.bak"));
    }
    std::fs::write(&path, default_file_text()).map_err(|e| e.to_string())
}

// ---------------- OCR language packs ----------------
/// Checks (in the background) which languages have their Windows OCR language installed.
pub fn check_packs() {
    std::thread::spawn(|| {
        let mut result = Vec::new();
        for l in list() {
            let installed = crate::ocr::language_supported(&l.ocr);
            result.push((l.ocr.to_lowercase(), installed));
        }
        *lock(&PACKS) = result;
        PACKS_CHANGED.store(true, Ordering::Relaxed);
        crate::state::wake();
    });
}

/// Some(true/false) once the background check has run, None before that.
pub fn pack_installed(tag: &str) -> Option<bool> {
    let key = tag.to_lowercase();
    lock(&PACKS).iter().find(|(t, _)| *t == key).map(|(_, ok)| *ok)
}

/// True once after the pack check finished (the UI repaints the Settings tab).
pub fn take_packs_changed() -> bool {
    PACKS_CHANGED.swap(false, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_round_trips_all_ten_languages() {
        let (langs, notes) = parse(&default_file_text()).expect("default file parses");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(langs.len(), 10);
        let en = &langs[0];
        assert_eq!(en.id, "en");
        assert_eq!(en.patch.len(), 2);
        assert_eq!(en.mode, MatchMode::Words);
        let ja = langs.iter().find(|l| l.id == "ja").unwrap();
        assert_eq!(ja.mode, MatchMode::Chars);
        assert_eq!(ja.login_threshold, Some(0.75));
    }

    #[test]
    fn broken_entries_are_skipped_not_fatal() {
        let text = r#"{ "languages": [
            { "id": "en", "ocr": "en-US", "login": ["Tap"] },
            { "ocr": "xx", "login": ["a"] },
            { "id": "bad", "ocr": "xx" },
            { "id": "en", "ocr": "en-US", "login": ["dup"] },
            { "id": "xx", "name": "Test", "ocr": "xx-XX", "match": "chars", "login": ["abc"], "login_threshold": 5 }
        ]}"#;
        let (langs, notes) = parse(text).unwrap();
        assert_eq!(langs.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), vec!["en", "xx"]);
        assert_eq!(notes.len(), 4); // 3 skipped entries + 1 ignored threshold
        assert_eq!(langs[1].login_threshold, None);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"hello": 1}"#).is_err());
    }
}
