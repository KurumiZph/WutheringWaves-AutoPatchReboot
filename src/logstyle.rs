//! Parsing and colour-classification of log lines (pure, unit-tested).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Error,
    Warn,
    Success,
    Patch,
    Login,
    System,
    Dim,
    Normal,
}

pub struct Parsed {
    /// "HH:MM:SS" (empty if the line had no timestamp)
    pub time: String,
    pub message: String,
    pub kind: Kind,
}

/// "[2026-11-02 12:34:56] [TAG] text"  ->  time "12:34:56", message "[TAG] text"
pub fn parse_line(line: &str) -> Parsed {
    let line = line.trim_end_matches(['\r', '\n']);
    let (time, rest) = split_timestamp(line);
    let message = rest.replace('\t', "    ");
    let kind = classify(&message);
    Parsed { time, message, kind }
}

fn split_timestamp(line: &str) -> (String, &str) {
    if let Some(inner) = line.strip_prefix('[') {
        if let Some(end) = inner.find(']') {
            let ts = &inner[..end];
            let b = ts.as_bytes();
            if b.len() == 19 && b[4] == b'-' && b[7] == b'-' && b[10] == b' ' {
                if let Some(time) = ts.get(11..) {
                    return (time.to_string(), inner[end + 1..].trim_start());
                }
            }
        }
    }
    (String::new(), line)
}

/// First matching rule wins, so errors outrank everything else.
pub fn classify(msg: &str) -> Kind {
    let l = msg.to_lowercase();
    let has = |needle: &str| l.contains(needle);
    if has("error") || has("fatal") || has("fail") || has("crashed") {
        Kind::Error
    } else if has("warning") || has("timed out") || has("frame is blank") || has("minimized") {
        Kind::Warn
    } else if has("[success]")
        || has("login detected")
        || has("user id detected")
        || has("fully stopped")
        || has("restart launched")
        || (has("[test]") && has("pass"))
    {
        Kind::Success
    } else if has("[patch]") {
        Kind::Patch
    } else if has("[login]") {
        Kind::Login
    } else if has("[system]") {
        Kind::System
    } else if ["[ocr", "[window", "[capture", "[config", "[debug", "[test"].iter().any(|t| l.starts_with(t)) {
        Kind::Dim
    } else {
        Kind::Normal
    }
}

/// Splits `msg` into visual lines of at most `cols` characters, breaking at a space when there is one.
/// (The log font is monospaced, so this needs no text measuring.)
pub fn wrap_text(msg: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    if msg.is_empty() {
        return vec![String::new()];
    }
    let chars: Vec<char> = msg.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= cols {
            lines.push(chars[start..].iter().collect());
            break;
        }
        let end = start + cols; // chars[end] exists because more than `cols` characters remain
        match (start + 1..=end).rev().find(|&i| chars[i] == ' ') {
            Some(i) => {
                lines.push(chars[start..i].iter().collect());
                start = i + 1;
                while start < chars.len() && chars[start] == ' ' {
                    start += 1;
                }
            }
            None => {
                lines.push(chars[start..end].iter().collect());
                start = end;
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_timestamp() {
        let p = parse_line("[2026-11-02 12:34:56] [WuWa] Launching: C:\\x");
        assert_eq!(p.time, "12:34:56");
        assert_eq!(p.message, "[WuWa] Launching: C:\\x");
        assert_eq!(p.kind, Kind::Normal);
        let bare = parse_line("no timestamp here");
        assert_eq!(bare.time, "");
        assert_eq!(bare.message, "no timestamp here");
        assert_eq!(parse_line("[2026-11-02 12:34:56]").message, "");
    }

    #[test]
    fn classifies_real_log_lines() {
        assert_eq!(classify("[OCR ERROR] boom"), Kind::Error);
        assert_eq!(classify("[WuWa] Restart failed."), Kind::Error);
        assert_eq!(classify("[WuWa] WARNING: WuWa process may still be running."), Kind::Warn);
        assert_eq!(classify("[CAPTURE] Game window is minimized - scanning paused"), Kind::Warn);
        assert_eq!(classify("==== [SUCCESS] LOGIN SCREEN CONFIRMED ===="), Kind::Success);
        assert_eq!(classify("[SYSTEM] Login detected - APR finished. This app will close"), Kind::Success);
        assert_eq!(classify("==== [PATCH] RESTART MESSAGE CONFIRMED ===="), Kind::Patch);
        assert_eq!(classify("[LOGIN] Possible detection 1/2 | score=0.91"), Kind::Login);
        assert_eq!(classify("[SYSTEM] Detected 1 valid installation(s)."), Kind::System);
        assert_eq!(classify("[OCR] MONITORING STARTED"), Kind::Dim);
        assert_eq!(classify("[TEST] Simulated patch screen: PASS (detected as expected)"), Kind::Success);
        assert_eq!(classify("[TEST] Simulated login screen: FAIL (not detected)"), Kind::Error);
        assert_eq!(classify("[TEST] OCR read 12 words"), Kind::Dim);
        assert_eq!(classify("[WuWa] Game fully stopped after 2.3s. Relaunching."), Kind::Success);
        assert_eq!(classify("[WuWa] Closing Wuthering Waves..."), Kind::Normal);
    }

    #[test]
    fn wraps_at_spaces_and_hard_breaks_long_words() {
        assert_eq!(wrap_text("hello world foo", 11), vec!["hello world", "foo"]);
        assert_eq!(wrap_text("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("", 10), vec![""]);
        assert_eq!(wrap_text("short", 10), vec!["short"]);
    }
}
