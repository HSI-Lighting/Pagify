//! A plain word list, checked and suggested against — not a real spell
//! checker's affix rules, just "is this exact spelling in the dictionary"
//! and "which known words are one or two edits away."
//!
//! See `third_party/dict/README.md` for where the list came from and why a
//! flat `HashSet` was chosen over pulling in `hunspell-rs`/`symspell`.
//!
//! **Languages.** English is the flat list above. Arabic is a Hunspell
//! dictionary (words plus affix rules), read by `spellbook`. Chinese cannot be
//! checked by a word list — there are no spaces between words and any real
//! character is a real character — so it is checked for *unusual characters*
//! only: those outside the few thousand in everyday use, which is what OCR and
//! broken font encodings produce. Persian, Urdu and every other script are not
//! judged, and a page that is mostly in one is reported as skipped.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const WORDLIST: &str = include_str!("../../../third_party/dict/en-US.txt");
const ARABIC_DIC: &str = include_str!("../../../third_party/dict/ar/ar.dic");
const ARABIC_AFF: &str = include_str!("../../../third_party/dict/ar/ar.aff");
const HAN_COMMON: &str = include_str!("../../../third_party/dict/han-common.txt");

/// The Arabic dictionary, read the first time it is needed — half a million
/// entries, so on the scan's own thread and not at start-up. `None` if it
/// could not be read, in which case Arabic is simply not judged.
fn arabic() -> Option<&'static spellbook::Dictionary> {
    static ARABIC: OnceLock<Option<spellbook::Dictionary>> = OnceLock::new();
    ARABIC.get_or_init(|| spellbook::Dictionary::new(ARABIC_AFF, ARABIC_DIC).ok()).as_ref()
}

/// The ranges of everyday Chinese characters, as code points, sorted.
fn han_common() -> &'static [(u32, u32)] {
    static RANGES: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let hex = |s: &str| u32::from_str_radix(s.trim(), 16).ok();
        let mut ranges: Vec<(u32, u32)> = HAN_COMMON
            .lines()
            .filter_map(|line| match line.split_once('-') {
                Some((a, b)) => Some((hex(a)?, hex(b)?)),
                None => hex(line).map(|c| (c, c)),
            })
            .collect();
        ranges.sort_unstable();
        ranges
    })
}

/// Whether `c` is a Chinese character in everyday use — simplified, traditional
/// or Japanese kanji. Anything else Han is "unusual".
fn is_common_han(c: char) -> bool {
    let code = c as u32;
    let ranges = han_common();
    match ranges.binary_search_by(|&(first, _)| first.cmp(&code)) {
        Ok(_) => true,
        Err(0) => false,
        Err(at) => ranges[at - 1].1 >= code,
    }
}

fn is_han(c: char) -> bool {
    use pdf_core::ocr::script::Script;
    Script::of(c) == Some(Script::Han)
}

/// The plain Arabic alphabet, plus alef wasla — what a dictionary of ordinary
/// Arabic can judge. Persian and Urdu letters, the shaped presentation forms
/// and digits are not in it.
fn is_arabic_letter(c: char) -> bool {
    matches!(c as u32, 0x0621..=0x063A | 0x0641..=0x064A | 0x0671)
}

/// Marks that are written over or between the letters and are not part of the
/// spelling the dictionary holds: short vowels and other diacritics, the
/// dagger alef, the tatweel stretch, and the joiners.
fn is_arabic_mark(c: char) -> bool {
    matches!(c as u32, 0x064B..=0x0652 | 0x0670 | 0x0640 | 0x200C | 0x200D)
}

/// `word` with its marks taken off, if it is nothing but plain Arabic letters
/// and marks; `None` for anything else — a Persian word, a shaped form, a
/// letter of another script.
fn plain_arabic(word: &str) -> Option<String> {
    let mut plain = String::new();
    for c in word.chars() {
        if is_arabic_mark(c) {
            continue;
        }
        if !is_arabic_letter(c) {
            return None;
        }
        plain.push(c);
    }
    Some(plain)
}

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
    if words().contains(lower.as_str()) || is_custom(&lower) {
        return true;
    }
    regular_inflection_roots(&lower)
        .iter()
        .any(|root| words().contains(root.as_str()) || is_custom(root))
}

/// The words a person has added to their own dictionary — a company name, a
/// product code that reads as a word, a place — kept in one text file, a word
/// to a line, beside the rest of Pagify's settings.
///
/// **Reported from use: "no custom dictionary".** Ignore All only lasted as
/// long as the panel did, so the same names were flagged again on every check.
/// A word added here is known to every later check, in every window, and a
/// regular inflection of it is known too (`Pagifys`), as for any other word.
#[derive(Debug, Default)]
pub struct Custom {
    words: HashSet<String>,
    path: Option<std::path::PathBuf>,
}

impl Custom {
    /// The words in `path`, one to a line; a missing or unreadable file is an
    /// empty dictionary, and the next added word creates it.
    pub fn load(path: std::path::PathBuf) -> Custom {
        let words = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        Custom { words, path: Some(path) }
    }

    pub fn contains(&self, lower: &str) -> bool {
        self.words.contains(lower)
    }

    /// Add one word and write the file again. `Ok(false)` when it was already
    /// there. The word stays added for this session even if the file cannot be
    /// written, and the error says so.
    pub fn add(&mut self, word: &str) -> Result<bool, String> {
        let word = word.trim().to_lowercase();
        if word.is_empty() || word.chars().any(char::is_whitespace) {
            return Err("a dictionary entry is one word.".into());
        }
        if !self.words.insert(word) {
            return Ok(false);
        }
        let Some(path) = &self.path else { return Ok(true) };
        let mut sorted: Vec<&str> = self.words.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut text = sorted.join("\n");
        text.push('\n');
        pagify_shell::state::write_own(path, text.as_bytes())
            .map(|()| true)
            .map_err(|e| format!("added for now, but {} could not be written: {e}", path.display()))
    }
}

fn custom() -> &'static std::sync::RwLock<Custom> {
    static CUSTOM: OnceLock<std::sync::RwLock<Custom>> = OnceLock::new();
    CUSTOM.get_or_init(Default::default)
}

fn is_custom(lower: &str) -> bool {
    custom().read().unwrap_or_else(|e| e.into_inner()).contains(lower)
}

/// Read the person's dictionary from Pagify's settings folder and keep adding
/// to it there. Not called under test, so a test run never touches the real one.
pub fn use_dictionary_file() {
    if let Some(dir) = pagify_shell::state::state_dir() {
        *custom().write().unwrap_or_else(|e| e.into_inner()) = Custom::load(dir.join("dictionary.txt"));
    }
}

/// Add `word` to the dictionary every check consults.
pub fn add_word(word: &str) -> Result<bool, String> {
    custom().write().unwrap_or_else(|e| e.into_inner()).add(word)
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

/// [`edit_distance`], or `None` when it is more than `limit`.
///
/// Gives up as soon as a whole row of the table is past `limit`: every path to
/// the corner goes through each row and never gets cheaper, so nothing later
/// can come back under it. Almost every one of the candidates a suggestion
/// search looks at is nowhere near, and is dropped after a few letters.
fn edit_distance_within(
    a: &[char],
    b: &[char],
    limit: usize,
    previous: &mut Vec<usize>,
    current: &mut Vec<usize>,
) -> Option<usize> {
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    previous.clear();
    previous.extend(0..=b.len());
    current.clear();
    current.resize(b.len() + 1, 0);
    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_least = current[0];
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let here = (previous[j + 1] + 1).min(current[j] + 1).min(previous[j] + cost);
            current[j + 1] = here;
            row_least = row_least.min(here);
        }
        if row_least > limit {
            return None;
        }
        std::mem::swap(previous, current);
    }
    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

#[cfg(test)]
thread_local! {
    /// How many times [`suggest`] has run on this thread — a search is
    /// hundreds of thousands of edit distances, so a test has to be able to
    /// say "once per word, not once per frame". Per thread, not global: the
    /// tests run in parallel and each of them calls `suggest`.
    pub(crate) static SUGGEST_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Known words that read close to `word`, nearest first — candidates are
/// only ever drawn from within two letters of its own length, which is
/// what keeps this a length-bucketed scan rather than all 370,000 words
/// run through `edit_distance` on every call.
pub fn suggest(word: &str, limit: usize) -> Vec<String> {
    #[cfg(test)]
    SUGGEST_CALLS.with(|calls| calls.set(calls.get() + 1));
    // Arabic comes from its own dictionary; a Chinese character has nothing to
    // be suggested from.
    if let Some(plain) = plain_arabic(word).filter(|p| !p.is_empty()) {
        let mut out = Vec::new();
        if let Some(dictionary) = arabic() {
            dictionary.suggest(&plain, &mut out);
        }
        out.truncate(limit);
        return out;
    }
    if word.chars().any(is_han) {
        return Vec::new();
    }
    let lower = word.to_lowercase();
    let letters: Vec<char> = lower.chars().collect();
    let len = letters.len();
    let by_len = by_length();

    let mut scored: Vec<(usize, &str)> = Vec::new();
    // Three buffers for the whole search rather than three allocations per
    // candidate: there are a couple of hundred thousand candidates.
    let (mut chars, mut previous, mut current) = (Vec::new(), Vec::new(), Vec::new());
    for candidate_len in len.saturating_sub(2).max(1)..=len + 2 {
        let Some(candidates) = by_len.get(&candidate_len) else { continue };
        for &candidate in candidates {
            chars.clear();
            chars.extend(candidate.chars());
            if let Some(distance) =
                edit_distance_within(&letters, &chars, 2, &mut previous, &mut current)
            {
                scored.push((distance, candidate));
            }
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(limit).map(|(_, w)| w.to_string()).collect()
}

/// Every run of letters and digits in `text`, as byte ranges into it — the
/// unit a spelling check treats as one word. A run with no letter in it
/// (`2024`) is not a word and is left out.
///
/// **Digits belong to the word they touch**: `300dpi` and `v2` are one word
/// each, not `dpi` and `v`, so the checker can see that a "word" has a number
/// in it and leave it alone — a model code or a unit is not a misspelling.
///
/// **A zero-width non-joiner or joiner (U+200C, U+200D) between two letters
/// keeps them in one word.** Persian writes its plurals and clitics that way
/// (the half-space), and cutting there left a stem and a suffix, each flagged
/// on its own. One left at the end of a word is not part of it.
///
/// Every other mark is a boundary, including the apostrophe inside a
/// contraction: `don't` reads as `don` and `t`. Wrong for a contraction
/// specifically, but not worth a second rule for how rarely a real
/// contraction is also a real misspelling — `words_in_have_no_contraction_handling`
/// pins this down so the day it does matter, it fails on purpose rather than
/// by surprise.
pub fn words_in(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut out = Vec::new();
    // Where the word being read began, where its last letter or digit ended,
    // and whether it has a letter in it at all.
    let mut open: Option<(usize, usize, bool)> = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            let (start, _, letter) = open.unwrap_or((i, i, false));
            open = Some((start, i + c.len_utf8(), letter || c.is_alphabetic()));
        } else if open.is_some() && matches!(c, '\u{200C}' | '\u{200D}') {
            // Joins what is either side of it; never ends the word, and never
            // counts as part of its end.
        } else if let Some((start, end, letter)) = open.take() {
            if letter {
                out.push((start..end, &text[start..end]));
            }
        }
    }
    if let Some((start, end, true)) = open {
        out.push((start..end, &text[start..end]));
    }
    out
}

/// Whether this dictionary can say anything about `word`. It holds plain a-z
/// English and nothing else, so a word with an accent, a digit, a half-space
/// or a letter of another script is unknown to it by construction — never
/// because it is misspelled. Flagging those is wrong every time (Arabic and
/// Persian were flagged word for word), so they are not judged.
fn can_judge(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_alphabetic())
}

/// Whether `word` has a letter of a script other than Latin — Arabic,
/// Cyrillic, Han and so on. An accented Latin word is not one of these: the
/// language is another, the script is the same.
fn in_another_script(word: &str) -> bool {
    use pdf_core::ocr::script::Script;
    word.chars().any(|c| Script::of(c).is_some_and(|script| script != Script::Latin))
}

/// Whether the Arabic dictionary knows `word` (marks already taken off). A
/// dictionary that failed to load knows everything: nothing is flagged on its
/// say-so.
fn arabic_known(plain: &str) -> bool {
    arabic().map_or(true, |dictionary| dictionary.check(plain))
}

/// The words on one page that the dictionaries do not know, each with the
/// index of the run it is in, in reading order — or `None` when the page is
/// mostly in a script or language none of them can judge, so nothing on it was
/// checked and the caller must say so rather than report it clean.
///
/// **Latin** words go to the English list. A word is skipped when it is a
/// single letter (essentially always either a real word or an initial, never
/// worth a prompt) or written in all capitals — an acronym or a model code such
/// as "DALI" or "CAMINO" that a plain English word list was never going to
/// know, and flagging every one of those would bury the real finds under noise
/// — and any word [`can_judge`] refuses.
///
/// **Arabic** words go to the Arabic dictionary, with their marks taken off.
/// **Chinese** is checked a character at a time and only for characters that
/// are not in everyday use; a returned "word" is that one character.
///
/// **Persian and Urdu are written in the same script and are not Arabic.** A
/// page where more than a tenth of the Arabic-script words carry letters Arabic
/// does not have (پ چ گ ک ی ...) is Persian or Urdu, and all its words count as
/// ones nothing can judge — the Arabic list would flag nearly every one.
pub fn misspelled_in_page<'a>(runs: &[&'a str]) -> Option<Vec<(usize, &'a str)>> {
    let mut words = 0usize;
    let mut foreign = 0usize;
    let mut unknown: Vec<(usize, &'a str)> = Vec::new();
    // Arabic-script words, judged after the whole page has been seen, and how
    // many of the page's were not plain Arabic.
    let mut arabic_words: Vec<(usize, &'a str, String)> = Vec::new();
    let mut arabic_other = 0usize;

    for (index, run) in runs.iter().enumerate() {
        for (_, word) in words_in(run) {
            words += 1;
            if word.chars().any(is_han) {
                for (at, c) in word.char_indices() {
                    if is_han(c) && !is_common_han(c) {
                        unknown.push((index, &word[at..at + c.len_utf8()]));
                    }
                }
                continue;
            }
            if word.chars().any(|c| pdf_core::ocr::script::Script::of(c) == Some(pdf_core::ocr::script::Script::Arabic)) {
                match plain_arabic(word) {
                    Some(plain) => arabic_words.push((index, word, plain)),
                    None => {
                        arabic_other += 1;
                        foreign += 1;
                    }
                }
                continue;
            }
            if in_another_script(word) {
                foreign += 1;
            }
            if !can_judge(word) || word.len() < 2 || word.chars().all(|c| c.is_uppercase()) {
                continue;
            }
            if !is_known(word) {
                unknown.push((index, word));
            }
        }
    }

    let arabic_total = arabic_words.len() + arabic_other;
    if arabic_other > 0 && arabic_other * 10 >= arabic_total {
        // Persian or Urdu: nothing on the page in this script is judged.
        foreign += arabic_words.len();
    } else {
        for (index, word, plain) in arabic_words {
            if plain.chars().count() >= 2 && !arabic_known(&plain) {
                unknown.push((index, word));
            }
        }
    }
    // Reading order across the languages: the stable sort keeps a run's own
    // order.
    unknown.sort_by_key(|&(index, _)| index);
    (foreign * 2 <= words).then_some(unknown)
}

/// Whether any run on a page holds Chinese — so the panel can say what the
/// check of it amounts to.
pub fn has_chinese(runs: &[&str]) -> bool {
    runs.iter().any(|run| run.chars().any(is_han))
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

    // ---- Arabic and Chinese -------------------------------------------------

    /// "the book is new", and a string of one letter that is not a word.
    const GOOD_ARABIC: &str = "\u{627}\u{644}\u{643}\u{62A}\u{627}\u{628} \u{62C}\u{62F}\u{64A}\u{62F}";
    const NONSENSE_ARABIC: &str = "\u{633}\u{633}\u{633}\u{633}\u{633}\u{633}";

    #[test]
    fn arabic_is_judged_by_the_arabic_dictionary() {
        assert_eq!(misspelled_in_page(&[GOOD_ARABIC]), Some(vec![]), "real words were flagged");
        assert_eq!(
            misspelled_in_page(&[GOOD_ARABIC, NONSENSE_ARABIC]),
            Some(vec![(1, NONSENSE_ARABIC)]),
            "a string that is not a word was not found"
        );
    }

    /// Short vowels, the stretch mark and the joiners are not part of the
    /// spelling the dictionary holds.
    #[test]
    fn arabic_marks_do_not_make_a_known_word_unknown() {
        // كِتَابٌ — "a book" with its short vowels written; and with a tatweel.
        let vowelled = "\u{643}\u{650}\u{62A}\u{64E}\u{627}\u{628}\u{64C}";
        let stretched = "\u{643}\u{640}\u{62A}\u{627}\u{628}";
        assert_eq!(misspelled_in_page(&[vowelled, stretched, GOOD_ARABIC]), Some(vec![]));
    }

    /// Persian and Urdu share the script and are not Arabic: the Arabic list
    /// would flag nearly every word, so a page that is Persian is not judged at
    /// all — and is reported as skipped, not as clean.
    #[test]
    fn a_persian_page_is_skipped_not_flagged() {
        // پچگ کیا — letters Arabic does not have, among ones it does.
        let persian = "\u{67E}\u{686}\u{6AF} \u{6A9}\u{6CC}\u{627} \u{628}\u{627}\u{628} \u{67E}\u{6CC}\u{62F}\u{627}";
        assert_eq!(misspelled_in_page(&[persian]), None);
        // A few Persian words in an English page do not skip it.
        assert_eq!(misspelled_in_page(&["a tpyo is here and fine today", persian]), Some(vec![(0, "tpyo")]));
    }

    #[test]
    fn a_shaped_presentation_form_is_not_judged() {
        // ﻛﺘﺎب — the same word as isolated and final shapes, which extraction
        // gives for a font with no Unicode map.
        let shaped = "\u{FEDB}\u{FE98}\u{FE8E}\u{FE91}";
        assert_eq!(misspelled_in_page(&["a tpyo is here and fine today", shaped]), Some(vec![(0, "tpyo")]));
    }

    #[test]
    fn arabic_suggestions_come_from_the_arabic_dictionary_and_do_not_take_long() {
        let started = std::time::Instant::now();
        let near = suggest("\u{643}\u{62A}\u{627}\u{628}\u{628}", 5);
        eprintln!("arabic suggestions: {near:?} in {:?}", started.elapsed());
        assert!(near.len() <= 5);
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "a suggestion took {:?}", started.elapsed());
    }

    /// Read fresh, not through the cached one, so the figure is the load itself.
    /// It happens once, on the scan's own thread, the first time Arabic is met.
    #[test]
    fn the_arabic_dictionary_loads_in_reasonable_time() {
        let started = std::time::Instant::now();
        let dictionary = spellbook::Dictionary::new(ARABIC_AFF, ARABIC_DIC).expect("the dictionary did not parse");
        eprintln!("arabic dictionary loaded in {:?}", started.elapsed());
        assert!(dictionary.check("\u{643}\u{62A}\u{627}\u{628}"), "كتاب is not known");
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "took {:?}", started.elapsed());
    }

    #[test]
    fn chinese_is_checked_for_unusual_characters_only() {
        // 这是一个测试 — all in everyday use, simplified.
        let plain = "\u{8FD9}\u{662F}\u{4E00}\u{4E2A}\u{6D4B}\u{8BD5}";
        // 測試 — traditional.
        let traditional = "\u{6E2C}\u{8A66}";
        assert_eq!(misspelled_in_page(&[plain, traditional]), Some(vec![]));
        // U+3400, a rare character from the first extension block: the one
        // character is what is returned, not the run it is in.
        let rare = "\u{6D4B}\u{8BD5}\u{3400}\u{6D4B}";
        assert_eq!(misspelled_in_page(&[rare]), Some(vec![(0, "\u{3400}")]));
        assert!(has_chinese(&[rare]) && !has_chinese(&["plain english"]));
    }

    #[test]
    fn chinese_beside_english_leaves_the_english_to_the_english_list() {
        let runs = ["LED \u{706F} a tpyo here", "\u{6D4B}\u{8BD5}\u{3400}"];
        assert_eq!(
            misspelled_in_page(&runs),
            Some(vec![(0, "tpyo"), (1, "\u{3400}")]),
            "reading order across the languages"
        );
    }

    #[test]
    fn the_common_character_ranges_hold_what_they_should() {
        assert!(is_common_han('\u{4E00}'), "一, the first");
        assert!(is_common_han('\u{9FA5}') || is_common_han('\u{9F9C}'));
        assert!(!is_common_han('\u{3400}'));
        assert!(!is_common_han('a'));
        assert!(han_common().windows(2).all(|w| w[0].1 < w[1].0), "ranges overlap or are out of order");
    }

    #[test]
    fn a_word_added_to_the_dictionary_is_known_and_kept_in_the_file() {
        let dir = std::env::temp_dir().join(format!("pagify-dict-{}", std::process::id()));
        let file = dir.join("dictionary.txt");
        let _ = std::fs::remove_dir_all(&dir);

        let mut mine = Custom::load(file.clone());
        assert!(!mine.contains("zqxhsi"), "a missing file is an empty dictionary");
        assert_eq!(mine.add("  ZqxHSI "), Ok(true), "trimmed, and kept in lower case");
        assert_eq!(mine.add("zqxhsi"), Ok(false), "already there");
        assert!(mine.add("two words").is_err(), "an entry is one word");
        assert!(mine.add("").is_err());

        // A new session reads it back, whatever case it was typed in.
        let again = Custom::load(file);
        assert!(again.contains("zqxhsi"));
        assert!(!again.contains("two words"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one dictionary the checks consult. A word nothing else uses, because
    /// the whole test run shares it.
    #[test]
    fn the_checker_knows_an_added_word_and_its_plain_inflections() {
        assert!(!is_known("zqxvlumen"), "the word was already known, so this proves nothing");
        assert_eq!(add_word("Zqxvlumen"), Ok(true));
        assert!(is_known("zqxvlumen"));
        assert!(is_known("ZQXVLUMEN"), "case is folded as for any word");
        assert!(is_known("zqxvlumens"), "a plural of it");
        assert!(!is_known("zqxvlumenx"), "a different word");
    }

    /// The shortcut must answer exactly what the full table answers, for every
    /// pair that is within the limit, and say "too far" for every pair that is
    /// not — over real words, both near and far.
    #[test]
    fn the_bounded_distance_agrees_with_the_full_one() {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        let sample: Vec<&str> = WORDLIST.lines().filter(|l| !l.is_empty()).step_by(997).take(250).collect();
        let (mut previous, mut current) = (Vec::new(), Vec::new());
        let mut within = 0;
        for &x in &sample {
            // The word itself, a typo of it, and every other sampled word.
            let mut typo = x.to_string();
            typo.pop();
            for other in std::iter::once(typo.as_str()).chain(sample.iter().copied()) {
                let (a, b) = (chars(x), chars(other));
                let full = edit_distance(&a, &b);
                let bounded = edit_distance_within(&a, &b, 2, &mut previous, &mut current);
                assert_eq!(bounded, (full <= 2).then_some(full), "{x} vs {other}");
                within += usize::from(full <= 2);
            }
        }
        assert!(within > 250, "the sample never came within the limit, so it proved nothing");
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

    /// Changed on purpose (it used to expect `dpi` and `v`): a digit now
    /// belongs to the word it touches, so the checker can see `300dpi` and
    /// `v2` have a number in them and not judge them. Punctuation still splits.
    #[test]
    fn words_in_splits_on_punctuation_and_keeps_digits_with_their_word() {
        let found: Vec<&str> =
            words_in("Two-column, 300dpi: fixed_v2! 2024").into_iter().map(|(_, w)| w).collect();
        assert_eq!(found, vec!["Two", "column", "300dpi", "fixed", "v2"]);
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
