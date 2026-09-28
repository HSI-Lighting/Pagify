//! A plain word list, checked and suggested against — not a real spell
//! checker's affix rules, just "is this exact spelling in the dictionary"
//! and "which known words are one or two edits away."
//!
//! See `third_party/dict/README.md` for where the list came from and why a
//! flat `HashSet` was chosen over pulling in `hunspell-rs`/`symspell`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const WORDLIST: &str = include_str!("../../../third_party/dict/en-US.txt");

fn words() -> &'static HashSet<&'static str> {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| WORDLIST.lines().filter(|l| !l.is_empty()).collect())
}

/// Every dictionary word, bucketed by its own length — checked once, so a
/// suggestion search only ever scans words within two letters of the target
/// instead of all 370,000.
fn by_length() -> &'static HashMap<usize, Vec<&'static str>> {
    static BY_LEN: OnceLock<HashMap<usize, Vec<&'static str>>> = OnceLock::new();
    BY_LEN.get_or_init(|| {
        let mut map: HashMap<usize, Vec<&'static str>> = HashMap::new();
        for word in WORDLIST.lines().filter(|l| !l.is_empty()) {
            map.entry(word.chars().count()).or_default().push(word);
        }
        map
    })
}

/// Whether the dictionary recognises `word`, case-insensitively — "Lamp"
/// and "LAMP" both read as "lamp" the same way a person would judge them —
/// or recognises it as a regular inflection of a word it does know.
///
/// **The word list is a set of headwords, not every form of one.**
/// "prioritize" is in it; "prioritizes" and "prioritizing" are not, and
/// ordinary prose conjugates and pluralises words constantly — without
/// this, a plain plural or a verb in the wrong tense would flag as a
/// misspelling as often as a real one did, on every page of real text this
/// was measured against.
///
/// Peeling a suffix off can occasionally call a genuine typo correct
/// instead — "occured" strips its own "ed" to "occur", a real word, when
/// the actual mistake was the missing second "r". Accepted on purpose: a
/// checker that stays quiet on a handful of typos like that is far less
/// grating than one that flags "walks" or "jumping" throughout a document.
pub fn is_known(word: &str) -> bool {
    let lower = word.to_lowercase();
    if words().contains(lower.as_str()) {
        return true;
    }
    regular_inflection_roots(&lower).iter().any(|root| words().contains(root.as_str()))
}

/// Every root `word` could be a common regular inflection of, English's own
/// couple of spelling adjustments included (`y` before `-ies`, a dropped
/// `e` before `-ing`/`-ed`) — not a real stemmer, just the handful of
/// patterns regular verbs and plurals actually follow.
fn regular_inflection_roots(word: &str) -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(stem) = word.strip_suffix("ies") {
        roots.push(format!("{stem}y")); // flies -> fly
    }
    if let Some(stem) = word.strip_suffix("es") {
        roots.push(stem.to_string()); // boxes -> box
    }
    if let Some(stem) = word.strip_suffix('s') {
        if !word.ends_with("ss") {
            roots.push(stem.to_string()); // lamps -> lamp
        }
    }
    if let Some(stem) = word.strip_suffix("ied") {
        roots.push(format!("{stem}y")); // tried -> try
    }
    if let Some(stem) = word.strip_suffix("ing") {
        roots.push(stem.to_string()); // jumping -> jump
        roots.push(format!("{stem}e")); // prioritizing -> prioritize
    }
    if let Some(stem) = word.strip_suffix("ed") {
        roots.push(stem.to_string()); // jumped -> jump
        roots.push(format!("{stem}e")); // prioritized -> prioritize
    }
    roots
}

/// How many single-character edits (insert, delete, substitute) turn `a`
/// into `b` — the standard Levenshtein table, kept to the two most recent
/// rows since nothing later needs the ones before them.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            current[j + 1] = (previous[j + 1] + 1).min(current[j] + 1).min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Known words that read close to `word`, nearest first — candidates are
/// only ever drawn from within two letters of its own length, which is
/// what keeps this a length-bucketed scan rather than all 370,000 words
/// run through `edit_distance` on every call.
pub fn suggest(word: &str, limit: usize) -> Vec<String> {
    let lower = word.to_lowercase();
    let letters: Vec<char> = lower.chars().collect();
    let len = letters.len();
    let by_len = by_length();

    let mut scored: Vec<(usize, &str)> = Vec::new();
    for candidate_len in len.saturating_sub(2).max(1)..=len + 2 {
        let Some(candidates) = by_len.get(&candidate_len) else { continue };
        for &candidate in candidates {
            let distance = edit_distance(&letters, &candidate.chars().collect::<Vec<_>>());
            if distance <= 2 {
                scored.push((distance, candidate));
            }
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(limit).map(|(_, w)| w.to_string()).collect()
}

/// Every run of letters in `text`, as byte ranges into it — the unit a
/// spelling check treats as one word.
///
/// **Digits and punctuation are always boundaries, never part of a word**,
/// including the apostrophe inside a contraction: `don't` reads as `don`
/// and `t`. Wrong for a contraction specifically, but not worth a second
/// rule for how rarely a real contraction is also a real misspelling —
/// `words_in_have_no_contraction_handling` pins this down so the day it
/// does matter, it fails on purpose rather than by surprise.
pub fn words_in(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_alphabetic() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push((s..i, &text[s..i]));
        }
    }
    if let Some(s) = start {
        out.push((s..text.len(), &text[s..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_word_is_known() {
        assert!(is_known("lighting"));
        assert!(is_known("Lighting"), "should fold case");
    }

    /// **Found from use, against a real document**: "prioritize" is a
    /// headword in the bundled list, but ordinary prose conjugates and
    /// pluralises constantly, and none of these inflected forms are
    /// separate entries in it.
    #[test]
    fn a_regular_inflection_of_a_known_word_is_known() {
        assert!(is_known("prioritizes"), "verb + s");
        assert!(is_known("prioritized"), "verb + ed, dropping the e");
        assert!(is_known("prioritizing"), "verb + ing, dropping the e");
        assert!(is_known("lamps"), "plural");
        assert!(is_known("boxes"), "plural + es");
        assert!(is_known("tried"), "y -> ied");
        assert!(is_known("jumped"), "regular past tense");
    }

    #[test]
    fn a_typo_is_still_a_typo_even_if_it_ends_in_s() {
        assert!(!is_known("lihgtings"));
    }

    #[test]
    fn a_typo_is_not_known() {
        assert!(!is_known("lihgting"));
    }

    #[test]
    fn a_typo_suggests_the_word_it_came_from() {
        let suggestions = suggest("definately", 5);
        assert!(
            suggestions.iter().any(|s| s == "definitely"),
            "expected \"definitely\" among {suggestions:?}"
        );
    }

    #[test]
    fn edit_distance_counts_the_smallest_number_of_changes() {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(edit_distance(&chars("cat"), &chars("cat")), 0);
        assert_eq!(edit_distance(&chars("cat"), &chars("cot")), 1, "one substitution");
        assert_eq!(edit_distance(&chars("cat"), &chars("cats")), 1, "one insertion");
        assert_eq!(edit_distance(&chars("cats"), &chars("cat")), 1, "one deletion");
        assert_eq!(edit_distance(&chars("kitten"), &chars("sitting")), 3, "the classic example");
    }

    #[test]
    fn suggestions_are_nearest_first() {
        // A single substitution ("a" for "i") from its real word, and
        // distinctive enough a length that nothing else in the dictionary
        // shares it at the same distance — unlike a transposed pair
        // ("lihgting" for "lighting"), which ties with several other real
        // words also one transposition away and says nothing about sort
        // order specifically.
        let suggestions = suggest("definately", 10);
        assert_eq!(
            suggestions.first().map(String::as_str),
            Some("definitely"),
            "the nearest match should sort first: {suggestions:?}"
        );
    }

    #[test]
    fn words_in_splits_on_anything_not_a_letter() {
        let found: Vec<&str> =
            words_in("Two-column, 300dpi: fixed_v2!").into_iter().map(|(_, w)| w).collect();
        assert_eq!(found, vec!["Two", "column", "dpi", "fixed", "v"]);
    }

    /// Documents, rather than silently accepts, the one shortcut this
    /// tokeniser takes: an apostrophe inside a contraction is not kept.
    #[test]
    fn words_in_have_no_contraction_handling() {
        let found: Vec<&str> = words_in("don't").into_iter().map(|(_, w)| w).collect();
        assert_eq!(found, vec!["don", "t"], "contractions are split, not joined");
    }

    #[test]
    fn ranges_point_back_at_the_exact_word() {
        let text = "see the lihgting plan";
        let (range, word) = words_in(text)
            .into_iter()
            .find(|(_, w)| *w == "lihgting")
            .expect("the typo should still be found as a word");
        assert_eq!(&text[range], word);
    }
}
