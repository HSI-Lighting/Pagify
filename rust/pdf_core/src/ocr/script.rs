//! Which script a page is written in, and how sure we are.
//!
//! The rule this module exists to enforce: **never default silently to Latin.**
//! A Latin recogniser run over Arabic does not fail — it returns confident
//! nonsense, which is the worst failure available here. A page that says it
//! could not decide is worth more than one that quietly returns a plausible
//! wrong part number.

use serde::{Deserialize, Serialize};

/// A script family, which is the unit a recognition model covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Devanagari,
    Thai,
    Han,
    Kana,
    Hangul,
}

impl Script {
    pub const ALL: [Script; 10] = [
        Script::Latin,
        Script::Cyrillic,
        Script::Greek,
        Script::Arabic,
        Script::Hebrew,
        Script::Devanagari,
        Script::Thai,
        Script::Han,
        Script::Kana,
        Script::Hangul,
    ];

    /// The script a character belongs to, if it belongs to one that matters.
    ///
    /// Digits, punctuation and spaces return `None` on purpose: they appear in
    /// every script and voting on them would make every page look Latin.
    pub fn of(ch: char) -> Option<Script> {
        let code = ch as u32;
        Some(match code {
            0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x024F => Script::Latin,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
            0x0400..=0x04FF | 0x0500..=0x052F => Script::Cyrillic,
            0x0590..=0x05FF | 0xFB1D..=0xFB4F => Script::Hebrew,
            0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
            0x0900..=0x097F => Script::Devanagari,
            0x0E00..=0x0E7F => Script::Thai,
            0x3040..=0x309F | 0x30A0..=0x30FF => Script::Kana,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Script::Han,
            0xAC00..=0xD7AF | 0x1100..=0x11FF => Script::Hangul,
            _ => return None,
        })
    }

    pub fn is_rtl(self) -> bool {
        matches!(self, Script::Arabic | Script::Hebrew)
    }

    /// Whether this script needs a dictionary to find word boundaries.
    pub fn needs_segmentation(self) -> bool {
        matches!(self, Script::Han | Script::Kana | Script::Thai | Script::Hangul)
    }
}

/// What the evidence says about a page's script.
#[derive(Debug, Clone, PartialEq)]
pub enum Guess {
    /// One script clearly dominates.
    Certain(Script),
    /// Several scripts in play, with the leader first. **Ask, do not pick.**
    Ambiguous(Vec<Script>),
    /// Nothing to go on.
    Unknown,
}

impl Guess {
    /// The script to recognise with, or `None` if the user must be asked.
    ///
    /// Deliberately not "the best guess". An ambiguous page returns `None`, and
    /// the caller has to do something about it, because guessing here produces
    /// confident nonsense rather than an error anybody notices.
    pub fn settled(&self) -> Option<Script> {
        match self {
            Guess::Certain(script) => Some(*script),
            Guess::Ambiguous(_) | Guess::Unknown => None,
        }
    }
}

/// How much of a page one script has to account for before it is the answer.
///
/// A high bar: a document is usually in one script, and the exceptions —
/// an English part number in an Arabic catalogue, a Latin brand name in
/// Japanese — are exactly the pages where guessing wrong is worst.
const DOMINANT: f32 = 0.80;

/// Guess a page's script from text already extracted from it.
pub fn detect(text: &str) -> Guess {
    let mut counts: std::collections::HashMap<Script, usize> = std::collections::HashMap::new();
    let mut total = 0usize;

    for ch in text.chars() {
        if let Some(script) = Script::of(ch) {
            *counts.entry(script).or_default() += 1;
            total += 1;
        }
    }

    if total == 0 {
        return Guess::Unknown;
    }

    let mut ranked: Vec<(Script, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(format!("{:?}", a.0).cmp(&format!("{:?}", b.0))));

    let (leader, count) = ranked[0];
    if count as f32 / total as f32 >= DOMINANT {
        // Japanese mixes kana and han as a matter of course, so neither one
        // dominating is not ambiguity — it is Japanese.
        return Guess::Certain(leader);
    }

    if japanese(&ranked, total) {
        return Guess::Certain(Script::Kana);
    }

    Guess::Ambiguous(ranked.into_iter().map(|(script, _)| script).collect())
}

/// Kana and Han together, which is ordinary Japanese rather than two scripts
/// fighting. Without this, every Japanese page is "ambiguous" and every user is
/// asked a question with an obvious answer.
fn japanese(ranked: &[(Script, usize)], total: usize) -> bool {
    let share = |wanted: Script| {
        ranked
            .iter()
            .find(|(script, _)| *script == wanted)
            .map(|(_, n)| *n as f32 / total as f32)
            .unwrap_or(0.0)
    };
    let kana = share(Script::Kana);
    let han = share(Script::Han);
    kana > 0.05 && kana + han >= DOMINANT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_in_one_script_is_certain() {
        assert_eq!(detect("Ordinary English prose about luminaires"), Guess::Certain(Script::Latin));
        assert_eq!(detect("مرحبا بكم في الكتالوج"), Guess::Certain(Script::Arabic));
        assert_eq!(detect("Обычный русский текст"), Guess::Certain(Script::Cyrillic));
    }

    #[test]
    fn digits_and_punctuation_do_not_vote() {
        // Otherwise a page of part numbers looks Latin whatever it is in.
        assert_eq!(Script::of('7'), None);
        assert_eq!(Script::of('-'), None);
        assert_eq!(Script::of(' '), None);
        assert_eq!(detect("12345 -- 67.89"), Guess::Unknown);
    }

    /// The rule the module exists for.
    #[test]
    fn a_genuinely_mixed_page_refuses_to_pick() {
        // Half Latin, half Arabic. Running either recogniser over the other
        // half returns confident nonsense, so the honest answer is a question.
        let mixed = "specification مواصفات lighting إضاءة catalogue كتالوج";
        match detect(mixed) {
            Guess::Ambiguous(scripts) => {
                assert!(scripts.contains(&Script::Latin));
                assert!(scripts.contains(&Script::Arabic));
            }
            other => panic!("a mixed page was decided rather than questioned: {other:?}"),
        }
        assert_eq!(detect(mixed).settled(), None, "settled() handed back a guess");
    }

    #[test]
    fn a_stray_latin_word_does_not_unsettle_a_page() {
        // A brand name in an Arabic catalogue is normal and must not turn into
        // a question on every page.
        let arabic = "هذا كتالوج الإضاءة الخاص بشركة HSI ويحتوي على مواصفات كاملة للمنتجات";
        assert_eq!(detect(arabic), Guess::Certain(Script::Arabic));
    }

    #[test]
    fn japanese_mixing_kana_and_han_is_not_ambiguous() {
        // Written Japanese is both by definition. Calling it ambiguous would
        // ask the user a question with only one sensible answer.
        let japanese = "東京都の照明カタログです。製品の仕様はこちらをご覧ください。";
        assert!(
            matches!(detect(japanese), Guess::Certain(_)),
            "ordinary Japanese was treated as a mixed-script page"
        );
    }

    #[test]
    fn an_empty_page_is_unknown_not_latin() {
        assert_eq!(detect(""), Guess::Unknown);
        assert_eq!(detect("").settled(), None, "an empty page defaulted to a script");
    }

    #[test]
    fn direction_and_segmentation_come_from_the_script() {
        assert!(Script::Arabic.is_rtl());
        assert!(Script::Hebrew.is_rtl());
        assert!(!Script::Latin.is_rtl());

        assert!(Script::Han.needs_segmentation());
        assert!(Script::Thai.needs_segmentation());
        assert!(!Script::Latin.needs_segmentation());
    }
}
