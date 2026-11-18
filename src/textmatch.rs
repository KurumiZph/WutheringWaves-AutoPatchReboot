//! Text normalisation + a port of Python's `difflib.SequenceMatcher(None, a, b).ratio()`,
//! so the match thresholds tuned in the Python version keep their meaning.

/// "é" -> "e", "ß" -> "ss", ... (input is already lowercase). Other characters: None.
fn fold_latin(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => "i",
        'ł' => "l",
        'ñ' | 'ń' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'ŕ' | 'ř' => "r",
        'ś' | 'š' | 'ş' => "s",
        'ť' | 'ţ' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        _ => return None,
    })
}

/// Thai vowel / tone marks are not "alphanumeric" for Rust, but they are part of the words.
fn is_thai(c: char) -> bool {
    ('\u{0E00}'..='\u{0E7F}').contains(&c)
}

/// Lowercase and keep the letters / digits of ANY script (Latin accents are folded: "é" -> "e");
/// everything else - punctuation of every kind, full-width ones included - becomes a space.
/// Full-width forms ("ＩＤ：") are converted to plain ASCII first.
pub fn normalize_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for original in text.chars() {
        let c = if ('\u{FF01}'..='\u{FF5E}').contains(&original) {
            char::from_u32(original as u32 - 0xFEE0).unwrap_or(original)
        } else {
            original
        };
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue; // combining accent: drop
        }
        for lc in c.to_lowercase() {
            if let Some(folded) = fold_latin(lc) {
                cleaned.push_str(folded);
            } else if lc.is_alphanumeric() || is_thai(lc) {
                cleaned.push(lc);
            } else {
                cleaned.push(' ');
            }
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Longest common block of a[alo..ahi] / b[blo..bhi]; ties go to the earliest `i`, then earliest `j`
/// (exactly difflib's rule when no junk is configured).
fn longest_match(
    a: &[char],
    b: &[char],
    alo: usize,
    ahi: usize,
    blo: usize,
    bhi: usize,
) -> (usize, usize, usize) {
    let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0usize);
    // prev[j + 1] = length of the common run ending at a[i-1], b[j]
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for i in alo..ahi {
        for slot in cur.iter_mut() {
            *slot = 0;
        }
        for j in blo..bhi {
            if a[i] == b[j] {
                let k = prev[j] + 1;
                cur[j + 1] = k;
                if k > best_size {
                    best_i = i + 1 - k;
                    best_j = j + 1 - k;
                    best_size = k;
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    (best_i, best_j, best_size)
}

fn matching_chars(a: &[char], b: &[char], alo: usize, ahi: usize, blo: usize, bhi: usize) -> usize {
    if alo >= ahi || blo >= bhi {
        return 0;
    }
    let (i, j, k) = longest_match(a, b, alo, ahi, blo, bhi);
    if k == 0 {
        return 0;
    }
    k + matching_chars(a, b, alo, i, blo, j) + matching_chars(a, b, i + k, ahi, j + k, bhi)
}

/// 2*M / T, like difflib's `ratio()`.
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let m = matching_chars(&a, &b, 0, a.len(), 0, b.len());
    2.0 * m as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize_text("Solaris-3"), "solaris 3");
        assert_eq!(normalize_text("  Tap  to LAND! "), "tap to land");
        assert_eq!(normalize_text("User ID: 123"), "user id 123");
    }

    #[test]
    fn normalizes_every_script() {
        assert_eq!(normalize_text("Tippe, um auf Solaris-3 zu landen"), "tippe um auf solaris 3 zu landen");
        assert_eq!(normalize_text("Identifiant : 904630761"), "identifiant 904630761");
        assert_eq!(normalize_text("Português - Español"), "portugues espanol");
        assert_eq!(normalize_text("Stra\u{00DF}e"), "strasse");
        assert_eq!(normalize_text("ユーザーID：904630761"), "ユーザーid 904630761"); // full-width colon + katakana long mark
        assert_eq!(normalize_text("特征码: 904630761"), "特征码 904630761");
        assert_eq!(normalize_text("솔라리스 연결"), "솔라리스 연결");
        assert_eq!(normalize_text("แตะเพื่อเข้าสู่โซลาริส-3"), "แตะเพื่อเข้าสู่โซลาริส 3"); // Thai marks kept
        assert_eq!(normalize_text("\u{FF35}\u{FF29}\u{FF24}"), "uid"); // full-width UID
    }

    #[test]
    fn similarity_basics() {
        assert_eq!(similarity("abc", "abc"), 1.0);
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("abc", "xyz"), 0.0);
        // difflib: SequenceMatcher(None, "abcd", "bcde").ratio() == 0.75
        assert!((similarity("abcd", "bcde") - 0.75).abs() < 1e-9);
    }
}
