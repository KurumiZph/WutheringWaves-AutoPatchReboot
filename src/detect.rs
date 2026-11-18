//! OCR result types and the login / patch / user-id detection logic (pure, no Win32).
//! Everything language-specific (texts, matching style, labels) comes from a `Language` profile.

use crate::config::*;
use crate::languages::{Language, MatchMode};
use crate::textmatch::{normalize_text, similarity};

/// A word exactly as the OCR engine returned it.
#[derive(Clone, Debug)]
pub struct RawWord {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// A normalised single token. One OCR word like "Solaris-3" becomes two tokens
/// ("solaris", "3") that share the same box.
#[derive(Clone, Debug, Default)]
pub struct Word {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// OCR output; `width`/`height` are the size of the image that was OCR'd.
#[derive(Clone, Debug, Default)]
pub struct Ocr {
    pub words: Vec<Word>,
    pub width: i32,
    pub height: i32,
}

impl Ocr {
    pub fn empty(width: i32, height: i32) -> Ocr {
        Ocr { words: Vec::new(), width, height }
    }

    pub fn from_raw(raw: &[RawWord], width: i32, height: i32) -> Ocr {
        let mut words = Vec::new();
        for r in raw {
            for token in normalize_text(&r.text).split_whitespace() {
                words.push(Word {
                    text: token.to_string(),
                    left: r.left,
                    top: r.top,
                    width: r.width,
                    height: r.height,
                });
            }
        }
        Ocr { words, width, height }
    }

    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

// ---------------- phrase search ----------------

pub struct FuzzyHit {
    pub score: f64,
    pub candidate: String,
    /// index of the first matching word and how many words the match covers
    pub start: usize,
    pub size: usize,
}

/// Words mode: sliding window over the OCR words.
fn find_words(words: &[Word], target: &str, threshold: f64) -> Option<FuzzyHit> {
    let target = normalize_text(target);
    let target_count = target.split_whitespace().count();
    if target_count == 0 {
        return None;
    }
    let first_size = target_count.saturating_sub(1).max(1);
    for size in first_size..(target_count + 3) {
        if size > words.len() {
            break;
        }
        for start in 0..=(words.len() - size) {
            let candidate = words[start..start + size].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            let score = similarity(&target, &candidate);
            if score >= threshold {
                return Some(FuzzyHit { score, candidate, start, size });
            }
        }
    }
    None
}

/// All OCR characters of all words as one stream (spaces ignored) + the word each character came from.
fn char_stream(words: &[Word]) -> (Vec<char>, Vec<usize>) {
    let mut chars = Vec::new();
    let mut owner = Vec::new();
    for (i, w) in words.iter().enumerate() {
        for c in w.text.chars().filter(|c| *c != ' ') {
            chars.push(c);
            owner.push(i);
        }
    }
    (chars, owner)
}

fn target_chars(target: &str) -> String {
    normalize_text(target).chars().filter(|c| *c != ' ').collect()
}

/// Chars mode: the phrase is searched as a run of characters; the best match wins.
fn find_chars(words: &[Word], target: &str, threshold: f64) -> Option<FuzzyHit> {
    let target = target_chars(target);
    let tl = target.chars().count();
    if tl == 0 {
        return None;
    }
    let (chars, owner) = char_stream(words);
    let mut best: Option<FuzzyHit> = None;
    for size in tl.saturating_sub(1).max(1)..(tl + 3) {
        if size > chars.len() {
            break;
        }
        for start in 0..=(chars.len() - size) {
            let candidate: String = chars[start..start + size].iter().collect();
            let score = similarity(&target, &candidate);
            if score >= threshold && best.as_ref().map(|b| score > b.score).unwrap_or(true) {
                let (first, last) = (owner[start], owner[start + size - 1]);
                best = Some(FuzzyHit { score, candidate, start: first, size: last - first + 1 });
            }
        }
    }
    best
}

/// Looks for `target` in the OCR words using the language's matching style.
pub fn find_phrase(words: &[Word], target: &str, threshold: f64, mode: MatchMode) -> Option<FuzzyHit> {
    match mode {
        MatchMode::Words => find_words(words, target, threshold),
        MatchMode::Chars => find_chars(words, target, threshold),
    }
}

/// Best similarity any window of the OCR text reaches against `target` (no threshold).
/// Windows may be shorter than the target (down to half of it), so a partly read phrase still counts.
pub fn best_phrase_score(words: &[Word], target: &str, mode: MatchMode) -> f64 {
    let mut best = 0.0f64;
    match mode {
        MatchMode::Words => {
            let target = normalize_text(target);
            let tc = target.split_whitespace().count();
            if tc == 0 || words.is_empty() {
                return 0.0;
            }
            for size in tc.div_ceil(2).max(1)..(tc + 3) {
                if size > words.len() {
                    break;
                }
                for start in 0..=(words.len() - size) {
                    let cand = words[start..start + size].iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                    best = best.max(similarity(&target, &cand));
                }
            }
        }
        MatchMode::Chars => {
            let target = target_chars(target);
            let tl = target.chars().count();
            let (chars, _) = char_stream(words);
            if tl == 0 || chars.is_empty() {
                return 0.0;
            }
            for size in tl.div_ceil(2).max(1)..(tl + 3) {
                if size > chars.len() {
                    break;
                }
                for start in 0..=(chars.len() - size) {
                    let cand: String = chars[start..start + size].iter().collect();
                    best = best.max(similarity(&target, &cand));
                }
            }
        }
    }
    best
}

/// True if the screen is partway to the login text or one of the patch phrases.
pub fn near_miss(ocr: &Ocr, lang: &Language) -> bool {
    lang.login.iter().chain(lang.patch.iter()).any(|t| best_phrase_score(&ocr.words, t, lang.mode) >= NEAR_MISS_THRESHOLD)
}

// ---------------- user id ----------------

pub struct UidResult {
    /// boxes of everything that was masked (the screenshot code blacks them out)
    pub masked: Vec<Word>,
    /// a "<label> <number>" pair was seen: the user id line of the game
    pub label_found: bool,
}

/// How many digits end this token (a user id is 9 digits; 6-12 are accepted).
fn trailing_digits(text: &str) -> usize {
    text.chars().rev().take_while(|c| c.is_ascii_digit()).count()
}

fn mask_trailing(word: &mut Word, n: usize, masked: &mut Vec<Word>) {
    masked.push(word.clone());
    let keep = word.text.chars().count() - n;
    word.text = word.text.chars().take(keep).chain(std::iter::repeat('*').take(n)).collect();
}

/// Masks the user id in place (so it can never reach a log or a screenshot).
///  * after any of the language's labels ("User ID", "UID", "特征码", ...): a 6-12 digit number
///  * anywhere: a standalone 9-digit number (the label may have been misread)
/// Only the first kind sets `label_found` (that is what ends APR when the game is already running).
pub fn censor_uid(ocr: &mut Ocr, lang: &Language) -> UidResult {
    let mut masked = Vec::new();
    let mut label_found = false;

    for label in &lang.uid {
        let Some(hit) = find_phrase(&ocr.words, label, UID_LABEL_THRESHOLD, lang.mode) else { continue };
        let to = (hit.start + hit.size + 2).min(ocr.words.len());
        for i in hit.start..to {
            let n = trailing_digits(&ocr.words[i].text);
            if (6..=12).contains(&n) && !ocr.words[i].text.contains('*') {
                mask_trailing(&mut ocr.words[i], n, &mut masked);
                label_found = true;
            }
        }
    }

    for w in ocr.words.iter_mut() {
        if w.text.len() == 9 && w.text.bytes().all(|b| b.is_ascii_digit()) {
            mask_trailing(w, 9, &mut masked);
        }
    }
    UidResult { masked, label_found }
}

// ---------------- user id: close-up of the bottom-right corner ----------------

/// The digit an OCR engine may have meant for a look-alike letter ("O" for 0, "l" for 1, ...).
/// Tokens are already lowercase. Only used on the small corner crop, never on the whole screen.
fn digit_of(c: char) -> Option<char> {
    match c {
        '0'..='9' => Some(c),
        'o' => Some('0'),
        'i' | 'l' => Some('1'),
        'z' => Some('2'),
        's' => Some('5'),
        'b' => Some('8'),
        'g' => Some('9'),
        _ => None,
    }
}

/// The digits a token ENDS with, look-alikes fixed: (digits, number of chars they cover, is the whole token).
/// "uid904630761" -> ("904630761", 9, false); "9o463o761" -> ("904630761", 9, true).
fn digit_tail(text: &str) -> (String, usize, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut n = 0;
    while n < chars.len() && digit_of(chars[chars.len() - 1 - n]).is_some() {
        n += 1;
    }
    let tail: String = chars[chars.len() - n..].iter().filter_map(|c| digit_of(*c)).collect();
    // a few real digits must be in there: "is", "bless" and friends are words, not numbers
    let real = chars[chars.len() - n..].iter().filter(|c| c.is_ascii_digit()).count();
    if n < 2 || real * 2 < n {
        return (String::new(), 0, false);
    }
    (tail, n, n == chars.len())
}

/// Like `censor_uid`, but for the OCR of a close-up of the bottom-right corner, where the only thing that can be
/// is the user id. More forgiving, because here a false alarm is very unlikely:
///  * look-alike characters are read as digits ("9O463O76l")
///  * a number split into groups ("904 630 761") is joined
///  * a label glued to the number ("UID904630761") is fine
///  * a bare 9-digit number counts even if the label was not read
/// Everything it recognises is masked, and `label_found` is set (that ends APR: the game is already running).
pub fn censor_uid_corner(ocr: &mut Ocr, lang: &Language) -> UidResult {
    let mut r = censor_uid(ocr, lang);
    if !r.masked.is_empty() {
        // a label + number, or (in this corner) a bare 9-digit number: either way it is the user id
        r.label_found = true;
        return r;
    }
    let label_seen = lang.uid.iter().any(|l| find_phrase(&ocr.words, l, 0.7, lang.mode).is_some());

    let mut i = 0;
    while i < ocr.words.len() {
        if ocr.words[i].text.contains('*') {
            i += 1;
            continue;
        }
        let (mut digits, first_n, first_whole) = digit_tail(&ocr.words[i].text);
        if digits.is_empty() {
            i += 1;
            continue;
        }
        // join the following groups on the same line, as long as they are whole digit tokens
        let mut j = i;
        let whole = first_whole;
        let line_h = ocr.words[i].height.max(1.0);
        while whole && j + 1 < ocr.words.len() {
            let next = &ocr.words[j + 1];
            let (nd, _, nw) = digit_tail(&next.text);
            if !nw || nd.is_empty() || (next.top - ocr.words[i].top).abs() > line_h * 0.6 {
                break;
            }
            digits.push_str(&nd);
            j += 1;
        }
        let ok = digits.len() == 9 || (label_seen && (6..=12).contains(&digits.len()));
        if !ok {
            i = j + 1;
            continue;
        }
        let first_chars = ocr.words[i].text.chars().count();
        mask_trailing(&mut ocr.words[i], first_n.min(first_chars), &mut r.masked);
        for k in i + 1..=j {
            let n = ocr.words[k].text.chars().count();
            mask_trailing(&mut ocr.words[k], n, &mut r.masked);
        }
        r.label_found = true;
        i = j + 1;
    }
    r
}

// ---------------- detection ----------------

pub struct LoginHit {
    pub score: f64,
    pub candidate: String,
}

/// The login screen: any one of the language's login texts.
pub fn detect_login(ocr: &Ocr, lang: &Language, threshold: f64) -> Option<LoginHit> {
    let mut best: Option<(FuzzyHit, &String)> = None;
    for phrase in &lang.login {
        if let Some(hit) = find_phrase(&ocr.words, phrase, threshold, lang.mode) {
            if best.as_ref().map(|(b, _)| hit.score > b.score).unwrap_or(true) {
                best = Some((hit, phrase));
            }
        }
    }
    let (hit, _) = best?;
    // Position is not required, just a confidence boost (login text sits low).
    let matched = &ocr.words[hit.start..hit.start + hit.size];
    let avg_top = matched.iter().map(|w| w.top).sum::<f32>() / matched.len() as f32;
    let relative_y = avg_top / (ocr.height.max(1) as f32);
    let bottom = relative_y >= 0.65;
    let score = (hit.score + if bottom { 0.05 } else { 0.0 }).min(1.0);
    Some(LoginHit { score, candidate: hit.candidate })
}

pub struct PatchHit {
    pub score: f64,
    pub candidate: String,
}

/// The patch notice: ALL of the language's patch phrases, roughly in the centre of the screen.
/// A language without patch phrases never triggers.
pub fn detect_patch(ocr: &Ocr, lang: &Language, threshold: f64) -> Option<PatchHit> {
    if lang.patch.is_empty() {
        return None;
    }
    let mut hits = Vec::new();
    for phrase in &lang.patch {
        hits.push(find_phrase(&ocr.words, phrase, threshold, lang.mode)?);
    }

    let mut matched: Vec<&Word> = Vec::new();
    for h in &hits {
        matched.extend(ocr.words[h.start..h.start + h.size].iter());
    }
    if matched.is_empty() {
        return None;
    }
    let n = matched.len() as f32;
    let avg_left = matched.iter().map(|w| w.left).sum::<f32>() / n;
    let avg_top = matched.iter().map(|w| w.top).sum::<f32>() / n;
    let rel_x = avg_left / (ocr.width.max(1) as f32);
    let rel_y = avg_top / (ocr.height.max(1) as f32);

    let (x_lo, x_hi) = PATCH_CENTER_X_RANGE;
    let (y_lo, y_hi) = PATCH_CENTER_Y_RANGE;
    if !(x_lo..=x_hi).contains(&rel_x) || !(y_lo..=y_hi).contains(&rel_y) {
        return None; // text found, but not where the popup actually appears
    }

    let score = hits.iter().map(|h| h.score).fold(f64::INFINITY, f64::min);
    let candidate = hits.iter().map(|h| h.candidate.as_str()).collect::<Vec<_>>().join(" | ");
    Some(PatchHit { score, candidate })
}

/// Test mode: a synthetic centred patch notice made from the language's patch phrases (English if it has none).
pub fn make_fake_patch(ocr: &Ocr, lang: &Language) -> Ocr {
    let source = if lang.patch.is_empty() { Language::english().patch } else { lang.patch.clone() };
    let tokens: Vec<String> = source.iter().flat_map(|p| normalize_text(p).split_whitespace().map(String::from).collect::<Vec<_>>()).collect();
    let x = (ocr.width as f32 * 0.35).floor();
    let y = (ocr.height as f32 * 0.42).floor();
    let words = tokens
        .into_iter()
        .enumerate()
        .map(|(i, t)| Word { text: t, left: x + i as f32 * 90.0, top: y, width: 80.0, height: 32.0 })
        .collect();
    Ocr { words, width: ocr.width, height: ocr.height }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::builtin;

    fn ocr_of(words: &[(&str, f32, f32)], w: i32, h: i32) -> Ocr {
        let raw: Vec<RawWord> = words
            .iter()
            .map(|(t, l, tp)| RawWord { text: t.to_string(), left: *l, top: *tp, width: 50.0, height: 20.0 })
            .collect();
        Ocr::from_raw(&raw, w, h)
    }

    fn en() -> Language {
        Language::english()
    }

    fn by_id(id: &str) -> Language {
        builtin().into_iter().find(|l| l.id == id).unwrap()
    }

    #[test]
    fn login_detected_even_with_hyphen_and_noise() {
        let ocr = ocr_of(
            &[("Exit", 10.0, 10.0), ("Tap", 800.0, 900.0), ("to", 860.0, 900.0), ("land", 900.0, 900.0),
              ("in", 960.0, 900.0), ("Solaris-3", 1000.0, 900.0), ("Login", 10.0, 1000.0)],
            1920, 1080,
        );
        let hit = detect_login(&ocr, &en(), 0.72).expect("login");
        assert!(hit.score > 0.95);
    }

    #[test]
    fn every_builtin_language_finds_its_own_login_text() {
        for l in builtin() {
            let phrase = &l.login[0];
            // Latin text arrives as words; CJK / Thai often arrives one character per word
            let raw: Vec<(String, f32, f32)> = if l.mode == MatchMode::Chars {
                phrase.chars().filter(|c| !c.is_whitespace()).enumerate().map(|(i, c)| (c.to_string(), 800.0 + i as f32 * 30.0, 990.0)).collect()
            } else {
                phrase.split_whitespace().enumerate().map(|(i, w)| (w.to_string(), 700.0 + i as f32 * 90.0, 990.0)).collect()
            };
            let refs: Vec<(&str, f32, f32)> = raw.iter().map(|(t, x, y)| (t.as_str(), *x, *y)).collect();
            let thr = l.login_threshold.unwrap_or(0.72);
            let ocr = ocr_of(&refs, 1920, 1080);
            assert!(detect_login(&ocr, &l, thr).is_some(), "{} login not found", l.id);
        }
    }

    #[test]
    fn short_cjk_phrase_tolerates_one_wrong_character_but_not_a_different_prompt() {
        let zh = by_id("zh-hans");
        let one_wrong = ocr_of(&[("点击连按", 800.0, 990.0)], 1920, 1080);
        assert!(detect_login(&one_wrong, &zh, 0.75).is_some());
        let other_prompt = ocr_of(&[("点击确认", 800.0, 990.0)], 1920, 1080); // "tap to confirm"
        assert!(detect_login(&other_prompt, &zh, 0.75).is_none());
    }

    #[test]
    fn a_language_without_patch_text_never_triggers_patch() {
        let fr = by_id("fr");
        let words = [("Patching", 800.0, 450.0), ("complete", 900.0, 450.0), ("Please", 800.0, 500.0),
                     ("restart", 880.0, 500.0), ("the", 960.0, 500.0), ("game", 1000.0, 500.0)];
        assert!(detect_patch(&ocr_of(&words, 1920, 1080), &fr, 0.69).is_none());
        assert!(detect_patch(&ocr_of(&words, 1920, 1080), &en(), 0.69).is_some());
    }

    #[test]
    fn patch_needs_both_phrases_in_centre() {
        let words = [("Patching", 800.0, 450.0), ("complete", 900.0, 450.0), ("Please", 800.0, 500.0),
                     ("restart", 880.0, 500.0), ("the", 960.0, 500.0), ("game", 1000.0, 500.0)];
        assert!(detect_patch(&ocr_of(&words, 1920, 1080), &en(), 0.69).is_some());
        // same text in the top-left corner must not count
        let corner = [("Patching", 5.0, 5.0), ("complete", 90.0, 5.0), ("Please", 5.0, 30.0),
                      ("restart", 80.0, 30.0), ("the", 160.0, 30.0), ("game", 200.0, 30.0)];
        assert!(detect_patch(&ocr_of(&corner, 1920, 1080), &en(), 0.69).is_none());
    }

    #[test]
    fn partly_read_phrase_is_a_near_miss_but_unrelated_text_is_not() {
        let partial = ocr_of(&[("Tap", 800.0, 900.0), ("to", 860.0, 900.0), ("land", 900.0, 900.0)], 1920, 1080);
        assert!(near_miss(&partial, &en()));
        assert!(detect_login(&partial, &en(), 0.72).is_none());
        let unrelated = ocr_of(&[("Settings", 10.0, 10.0), ("Account", 90.0, 10.0), ("Exit", 170.0, 10.0)], 1920, 1080);
        assert!(!near_miss(&unrelated, &en()));
        assert!(!near_miss(&Ocr::empty(1920, 1080), &en()));
    }

    #[test]
    fn uid_is_masked_with_the_label_of_every_language() {
        // (language, OCR words as the engine might return them)
        let cases: [(&str, Vec<&str>); 10] = [
            ("en", vec!["User", "ID:", "904630761"]),
            ("fr", vec!["Identifiant", ":", "904630761"]),
            ("de", vec!["Benutzer-ID:", "904630761"]),
            ("es", vec!["UID:", "904630761"]),
            ("pt", vec!["UID:", "904630761"]),
            ("ja", vec!["ユーザーID：904630761"]),
            ("ko", vec!["UID:", "904630761"]),
            ("zh-hans", vec!["特征码:", "904630761"]),
            ("zh-hant", vec!["特徵碼:904630761"]),
            ("th", vec!["UID:", "904630761"]),
        ];
        for (id, words) in cases {
            let l = by_id(id);
            let raw: Vec<(&str, f32, f32)> = words.iter().enumerate().map(|(i, w)| (*w, 1700.0 + i as f32 * 80.0, 1060.0)).collect();
            let mut ocr = ocr_of(&raw, 1920, 1080);
            let r = censor_uid(&mut ocr, &l);
            assert!(r.label_found, "{id}: label + number not recognised");
            assert!(!ocr.text().contains("904630761"), "{id}: user id leaked: {}", ocr.text());
            assert!(ocr.text().contains("*********"), "{id}");
        }
    }

    #[test]
    fn a_standalone_nine_digit_number_is_masked_but_does_not_count_as_the_uid_line() {
        let mut ocr = ocr_of(&[("Version", 10.0, 10.0), ("904630761", 90.0, 10.0), ("3.7.0_9052414", 200.0, 10.0)], 1920, 1080);
        let r = censor_uid(&mut ocr, &en());
        assert_eq!(r.masked.len(), 1);
        assert!(!r.label_found);
        assert!(ocr.text().contains("9052414")); // a 7-digit build number is left alone
    }
    #[test]
    fn corner_reads_look_alikes_glued_labels_split_groups_and_bare_numbers() {
        let cases: [(&str, Vec<&str>); 6] = [
            ("look-alike digits", vec!["User", "ID:", "9O463O76l"]),
            ("glued label", vec!["UID904630761"]),
            ("glued word label", vec!["UserID:904630761"]),
            ("split groups", vec!["User", "ID:", "904", "630", "761"]),
            ("bare number", vec!["904630761"]),
            ("label misread", vec!["Usor", "lD:", "904630761"]),
        ];
        for (name, words) in cases {
            let raw: Vec<(&str, f32, f32)> = words.iter().enumerate().map(|(i, w)| (*w, 1700.0 + i as f32 * 80.0, 60.0)).collect();
            let mut ocr = ocr_of(&raw, 2000, 300);
            let r = censor_uid_corner(&mut ocr, &en());
            assert!(r.label_found, "{name}: not recognised ({})", ocr.text());
            assert!(!r.masked.is_empty(), "{name}: nothing masked");
            let t = ocr.text();
            for part in ["9046", "0761", "630", "463"] {
                assert!(!t.contains(part), "{name}: leaked {part}: {t}");
            }
        }
    }

    #[test]
    fn corner_does_not_mistake_a_version_or_words_for_a_user_id() {
        for words in [vec!["Ver", "3.7.0_9052414"], vec!["Settings", "Account"], vec!["bless", "is", "sizes"], vec!["1234"]] {
            let raw: Vec<(&str, f32, f32)> = words.iter().enumerate().map(|(i, w)| (*w, 1700.0 + i as f32 * 80.0, 60.0)).collect();
            let mut ocr = ocr_of(&raw, 2000, 300);
            let r = censor_uid_corner(&mut ocr, &en());
            assert!(!r.label_found, "false alarm on {words:?}");
        }
    }
}
