//! Finding text, and deciding where words begin.
//!
//! ## Store raw, index normalised
//!
//! A search for "find" has to match a page containing the `ﬁ` ligature, and a
//! search for an Arabic or Devanagari word has to match regardless of how its
//! diacritics happen to be composed. That wants NFKC.
//!
//! What it must **not** do is normalise the stored text. NFKC is lossy — it
//! folds ligatures apart, rewrites compatibility characters, and collapses
//! distinctions the file deliberately made. Copy-out would then hand back
//! something the document does not contain. So the extracted text is kept
//! exactly as the file has it, and a separate normalised copy exists for
//! searching, with a map back to the original offsets.
//!
//! That map is the whole difficulty: `ﬁ` is one character raw and two
//! normalised, so a hit at normalised offset 5 is not at raw offset 5.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;

/// The NFKC form of a string, for comparison only. Never store this in place of
/// the original.
pub fn normalise(text: &str) -> String {
    text.nfkc().collect::<String>().to_lowercase()
}

/// A page's text, searchable without being altered.
#[derive(Debug, Clone)]
pub struct SearchIndex {
    raw: String,
    normalised: String,
    /// For each byte offset in `normalised`, the byte offset in `raw` it came
    /// from. One entry per normalised char, plus a terminator.
    offsets: Vec<(usize, usize)>,
}

impl SearchIndex {
    pub fn new(raw: &str) -> Self {
        let mut normalised = String::with_capacity(raw.len());
        let mut offsets = Vec::new();

        // Normalised a cluster at a time — a base character together with any
        // combining marks that follow it — so every produced character knows
        // which original it came from.
        //
        // Not per *character*: composing "e" followed by U+0301 into "é" needs
        // both of them at once, and a per-character pass can only ever see one,
        // so decomposed text would never match its composed form. Normalising
        // the whole string in one go composes correctly and throws the
        // correspondence away instead. The cluster is the unit that keeps both.
        let chars: Vec<(usize, char)> = raw.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            let (start, _) = chars[i];
            let mut end_index = i + 1;
            while end_index < chars.len()
                && unicode_normalization::char::is_combining_mark(chars[end_index].1)
            {
                end_index += 1;
            }
            let end = chars.get(end_index).map(|(o, _)| *o).unwrap_or(raw.len());

            let folded: String = raw[start..end].nfkc().collect::<String>().to_lowercase();
            for produced in folded.chars() {
                offsets.push((normalised.len(), start));
                normalised.push(produced);
            }
            i = end_index;
        }
        offsets.push((normalised.len(), raw.len()));

        SearchIndex { raw: raw.to_string(), normalised, offsets }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Every match, as a range into the **raw** text.
    pub fn find(&self, needle: &str) -> Vec<Range<usize>> {
        let needle = normalise(needle);
        if needle.is_empty() {
            return Vec::new();
        }

        let mut hits = Vec::new();
        let mut from = 0usize;
        while let Some(found) = self.normalised[from..].find(&needle) {
            let start = from + found;
            let end = start + needle.len();
            if let (Some(a), Some(b)) = (self.to_raw(start), self.to_raw(end)) {
                if a < b {
                    hits.push(a..b);
                }
            }
            from = start + needle.len().max(1);
            if from >= self.normalised.len() {
                break;
            }
        }
        hits
    }

    fn to_raw(&self, normalised_offset: usize) -> Option<usize> {
        match self.offsets.binary_search_by_key(&normalised_offset, |(n, _)| *n) {
            Ok(i) => Some(self.offsets[i].1),
            // Landing inside a character that normalisation produced: report
            // the start of the raw character it came from, which is the only
            // offset that exists in the original.
            Err(i) => self.offsets.get(i.saturating_sub(1)).map(|(_, raw)| *raw),
        }
    }
}

// ---------------------------------------------------------------------------
// Word boundaries
// ---------------------------------------------------------------------------

/// Where a double-click should select from and to.
///
/// ## What this does and does not do for CJK and Thai
///
/// Those scripts have no word spaces. Extraction is *correct* leaving them
/// unsegmented — the characters are all there and in order — but
/// double-click-to-select-word is not: UAX#29 treats a run of ideographs as one
/// word, so a double click selects the whole sentence.
///
/// Real segmentation needs a dictionary (`lindera`, `jieba-rs`) and a model per
/// language, which is a selection feature dressed as an extraction one and does
/// not belong in the critical path. What happens instead: **each ideograph is
/// its own word.** That is not linguistically right — 東京 is one word — but it
/// is far better than selecting a paragraph, and it degrades honestly.
///
/// Thai has no such fallback: its characters are not individually meaningful,
/// so a run of Thai is returned whole, and a dictionary is the only fix.
pub fn word_at(text: &str, byte_offset: usize) -> Range<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return 0..0;
    }

    let position = chars
        .iter()
        .position(|(offset, ch)| *offset <= byte_offset && byte_offset < offset + ch.len_utf8())
        .unwrap_or(chars.len() - 1);
    let (start_offset, ch) = chars[position];

    if is_ideograph(ch) {
        // One character, one word. A whole-run selection is worse.
        return start_offset..start_offset + ch.len_utf8();
    }

    if ch.is_whitespace() {
        return start_offset..start_offset + ch.len_utf8();
    }

    let same_class = |c: char| {
        if is_thai(ch) {
            is_thai(c)
        } else if ch.is_alphanumeric() {
            c.is_alphanumeric() || c == '\'' || c == '-'
        } else {
            !c.is_alphanumeric() && !c.is_whitespace() && !is_ideograph(c)
        }
    };

    let mut start = position;
    while start > 0 && same_class(chars[start - 1].1) && !is_ideograph(chars[start - 1].1) {
        start -= 1;
    }
    let mut end = position;
    while end + 1 < chars.len() && same_class(chars[end + 1].1) && !is_ideograph(chars[end + 1].1) {
        end += 1;
    }

    let (from, _) = chars[start];
    let (to, last) = chars[end];
    from..to + last.len_utf8()
}

/// CJK ideographs, kana, and Hangul syllables.
pub fn is_ideograph(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF        // Hiragana, Katakana
        | 0x3400..=0x4DBF      // CJK Extension A
        | 0x4E00..=0x9FFF      // CJK Unified
        | 0xF900..=0xFAFF      // Compatibility ideographs
        | 0xAC00..=0xD7AF      // Hangul syllables
        | 0x20000..=0x2A6DF)   // Extension B
}

pub fn is_thai(ch: char) -> bool {
    matches!(ch as u32, 0x0E00..=0x0E7F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ligature_is_found_by_its_letters() {
        let index = SearchIndex::new("the ﬁnal draft");
        let hits = index.find("final");
        assert_eq!(hits.len(), 1, "the ligature was not matched");

        // And the range points back into the *raw* text, where "ﬁnal" is
        // shorter than "final" by a byte count that would otherwise be wrong.
        let hit = &index.raw()[hits[0].clone()];
        assert!(hit.starts_with('ﬁ'), "range landed in the wrong place: {hit:?}");
    }

    #[test]
    fn the_stored_text_is_never_normalised() {
        // NFKC is lossy. Copy-out must hand back what the document contains.
        let index = SearchIndex::new("the ﬁnal draft");
        assert_eq!(index.raw(), "the ﬁnal draft");
        assert!(index.raw().contains('ﬁ'));
    }

    #[test]
    fn search_is_case_insensitive_and_finds_every_hit() {
        let index = SearchIndex::new("Lamp lamp LAMP");
        assert_eq!(index.find("lamp").len(), 3);
        assert_eq!(index.find("LAMP").len(), 3);
    }

    #[test]
    fn decomposed_and_composed_accents_match_each_other() {
        let composed = SearchIndex::new("café");
        let decomposed = SearchIndex::new("cafe\u{0301}");
        assert_eq!(composed.find("café").len(), 1);
        assert_eq!(decomposed.find("café").len(), 1, "decomposed text did not match");
    }

    #[test]
    fn an_empty_search_finds_nothing_rather_than_everything() {
        let index = SearchIndex::new("some text");
        assert!(index.find("").is_empty());
        assert!(SearchIndex::new("").find("anything").is_empty());
    }

    #[test]
    fn a_repeated_needle_does_not_loop_forever() {
        let index = SearchIndex::new("aaaaaa");
        assert!(!index.find("aa").is_empty());
    }

    // -- word boundaries ----------------------------------------------------

    #[test]
    fn a_latin_word_selects_whole() {
        let text = "the quick brown fox";
        assert_eq!(&text[word_at(text, 6)], "quick");
        assert_eq!(&text[word_at(text, 0)], "the");
        assert_eq!(&text[word_at(text, 18)], "fox");
    }

    #[test]
    fn hyphens_and_apostrophes_stay_inside_a_word() {
        let text = "a well-known can't";
        assert_eq!(&text[word_at(text, 4)], "well-known");
        assert_eq!(&text[word_at(text, 15)], "can't");
    }

    #[test]
    fn one_ideograph_is_one_word_rather_than_the_whole_sentence() {
        // Not linguistically right — 東京 is one word — but selecting the entire
        // run, which is what UAX#29 does here, is far worse.
        let text = "東京都に行きます";
        let selected = &text[word_at(text, 0)];
        assert_eq!(selected, "東");
        assert!(selected.chars().count() == 1);
    }

    #[test]
    fn latin_inside_cjk_still_selects_as_a_latin_word() {
        let text = "型番ABC123です";
        let offset = text.find('A').unwrap();
        assert_eq!(&text[word_at(text, offset)], "ABC123");
    }

    #[test]
    fn thai_returns_the_run_because_a_dictionary_is_the_only_honest_fix() {
        let text = "สวัสดีครับ";
        let selected = &text[word_at(text, 0)];
        assert_eq!(selected, text, "Thai was split without a dictionary, which cannot be right");
    }

    #[test]
    fn an_offset_past_the_end_does_not_panic() {
        let text = "short";
        let _ = word_at(text, 9_999);
        let _ = word_at("", 0);
    }
}
