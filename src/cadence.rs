//! Decides how long to wait before the next scan, based on what the OCR just read.
//!
//! Every tick still captures and OCRs - only the *pace* changes, with a hard maximum gap
//! (the idle interval). The numbers come from the settings (see settings.rs):
//!  * detection being confirmed            -> fast interval
//!  * "near miss" (partial match, capped)  -> fast interval
//!  * text unchanged for a few scans       -> idle interval
//!  * anything else / text changed         -> normal scan interval

use std::time::Duration;

use crate::config::TEXT_UNCHANGED_SIMILARITY;
use crate::settings::Values;
use crate::textmatch::similarity;

pub struct Cadence {
    last_signature: Option<String>,
    unchanged: u32,
    burst_left: u32,
}

impl Default for Cadence {
    fn default() -> Self {
        Self::new()
    }
}

impl Cadence {
    pub fn new() -> Self {
        Cadence { last_signature: None, unchanged: 0, burst_left: 0 }
    }

    /// Forget the history (new game window after a restart).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// `signature` = OCR text of the scan that just finished,
    /// `candidate` = a login/patch detection is waiting for its confirmations,
    /// `near_miss` = no detection, but the text is partway to a target.
    pub fn next_interval(&mut self, signature: &str, near_miss: bool, candidate: bool, v: &Values) -> Duration {
        let changed = match &self.last_signature {
            Some(prev) => similarity(prev, signature) < TEXT_UNCHANGED_SIMILARITY,
            None => true,
        };
        if changed {
            self.unchanged = 0;
            self.burst_left = if near_miss { v.near_miss_bursts() } else { 0 };
            self.last_signature = Some(signature.to_string());
        } else {
            self.unchanged = self.unchanged.saturating_add(1);
        }

        if candidate {
            return v.burst_interval();
        }
        if near_miss && self.burst_left > 0 {
            self.burst_left -= 1;
            return v.burst_interval();
        }
        if self.unchanged >= v.idle_after() {
            v.idle_interval()
        } else {
            v.scan_interval()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_when_text_is_static_and_recovers_on_change() {
        let v = Values::balanced();
        let mut c = Cadence::new();
        for _ in 0..v.idle_after() {
            assert_eq!(c.next_interval("main menu", false, false, &v), v.scan_interval());
        }
        assert_eq!(c.next_interval("main menu", false, false, &v), v.idle_interval());
        assert_eq!(c.next_interval("completely different words here", false, false, &v), v.scan_interval());
    }

    #[test]
    fn near_miss_burst_is_capped() {
        let v = Values::balanced();
        let mut c = Cadence::new();
        let bursts = (0..30).filter(|_| c.next_interval("tap to land", true, false, &v) == v.burst_interval()).count();
        assert_eq!(bursts as u32, v.near_miss_bursts());
    }

    #[test]
    fn candidate_always_bursts() {
        let v = Values::balanced();
        let mut c = Cadence::new();
        for _ in 0..10 {
            assert_eq!(c.next_interval("patching complete", false, true, &v), v.burst_interval());
        }
    }
}
