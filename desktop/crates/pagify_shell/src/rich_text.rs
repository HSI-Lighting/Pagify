//! A text box's words and the look of each of their characters — what the Text
//! Style panel changes and what both the box's preview and its writer read.
//!
//! **Indices are characters**, as egui's text cursor counts them, never bytes.
//! The looks are kept as runs: each the end of a stretch of characters (counted
//! from the start) and the look the whole stretch has. Typing takes the look of
//! the character before the cursor — the first character's, at the very start —
//! the way a word processor does, and an empty text still keeps one look: the one
//! the next character typed will take.

use std::ops::Range;

/// Superscript, subscript, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// How a character looks.
#[derive(Debug, Clone, PartialEq)]
pub struct CharLook {
    /// The font, by the name `pdf_core::text` knows it by.
    pub face: String,
    /// The page's own font this character is drawn in (`RunStyle::font`), when it
    /// is one the page already has: what lets a writer say "like that piece"
    /// instead of embedding a copy. `None` for a face typed in, and cleared by any
    /// change of face.
    pub page_font: Option<u32>,
    /// Points, before any superscript or subscript makes it smaller.
    pub size: f32,
    pub color: [u8; 4],
    pub underline: bool,
    pub strike: bool,
    pub script: Script,
    /// Points added after the character.
    pub letter_spacing_pt: f32,
    /// The character's width, in percent of its own.
    pub width_pct: f32,
    /// Points up (+) or down (−) from where it would sit.
    pub baseline_pt: f32,
}

impl CharLook {
    /// A face at a size and colour, with nothing else done to it.
    pub fn plain(face: impl Into<String>, size: f32, color: [u8; 4]) -> CharLook {
        CharLook {
            face: face.into(),
            page_font: None,
            size,
            color,
            underline: false,
            strike: false,
            script: Script::Normal,
            letter_spacing_pt: 0.0,
            width_pct: 100.0,
            baseline_pt: 0.0,
        }
    }
}

/// What to change in a look; `None` leaves that part as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CharChange {
    pub face: Option<String>,
    /// Which page font the characters are drawn in — set after `face`, so a change
    /// can put both back (`Some(None)` for none).
    pub page_font: Option<Option<u32>>,
    pub size: Option<f32>,
    pub color: Option<[u8; 4]>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub script: Option<Script>,
    pub letter_spacing_pt: Option<f32>,
    pub width_pct: Option<f32>,
    pub baseline_pt: Option<f32>,
}

impl CharChange {
    fn apply(&self, look: &mut CharLook) {
        if let Some(face) = &self.face {
            look.face = face.clone();
            look.page_font = None;
        }
        if let Some(page_font) = self.page_font {
            look.page_font = page_font;
        }
        if let Some(v) = self.size {
            look.size = v;
        }
        if let Some(v) = self.color {
            look.color = v;
        }
        if let Some(v) = self.underline {
            look.underline = v;
        }
        if let Some(v) = self.strike {
            look.strike = v;
        }
        if let Some(v) = self.script {
            look.script = v;
        }
        if let Some(v) = self.letter_spacing_pt {
            look.letter_spacing_pt = v;
        }
        if let Some(v) = self.width_pct {
            look.width_pct = v;
        }
        if let Some(v) = self.baseline_pt {
            look.baseline_pt = v;
        }
    }
}

/// One value for the whole of a range, or not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mixed<T> {
    One(T),
    Mixed,
}

impl<T: PartialEq + Clone> Mixed<T> {
    fn of<'a>(mut values: impl Iterator<Item = &'a T>) -> Mixed<T>
    where
        T: 'a,
    {
        match values.next() {
            None => Mixed::Mixed,
            Some(first) => {
                if values.all(|v| v == first) {
                    Mixed::One(first.clone())
                } else {
                    Mixed::Mixed
                }
            }
        }
    }
}

/// A range's look, field by field — what the panel shows for a selection.
#[derive(Debug, Clone, PartialEq)]
pub struct LookOf {
    pub face: Mixed<String>,
    pub size: Mixed<f32>,
    pub color: Mixed<[u8; 4]>,
    pub underline: Mixed<bool>,
    pub strike: Mixed<bool>,
    pub script: Mixed<Script>,
    pub letter_spacing_pt: Mixed<f32>,
    pub width_pct: Mixed<f32>,
    pub baseline_pt: Mixed<f32>,
}

/// A box's words and their looks.
#[derive(Debug, Clone, PartialEq)]
pub struct RichText {
    text: String,
    /// Each run's end, in characters, ascending; the last is the character count.
    /// One `(0, look)` when the text is empty: the typing look.
    runs: Vec<(usize, CharLook)>,
}

impl RichText {
    /// No words yet; the first one typed takes `look`.
    pub fn new(look: CharLook) -> RichText {
        RichText { text: String::new(), runs: vec![(0, look)] }
    }

    /// `text` with one look per character, neighbours that look the same joined.
    /// A list that is short carries on in its last look; an empty one, in a
    /// plain look, which only a text with no characters should be given.
    pub fn from_looks(text: &str, looks: &[CharLook]) -> RichText {
        let typing = looks.first().cloned().unwrap_or_else(|| CharLook::plain("", 12.0, [0, 0, 0, 255]));
        let segments =
            (0..text.chars().count()).map(|i| (1, looks.get(i).or(looks.last()).unwrap_or(&typing).clone())).collect();
        RichText { text: text.to_string(), runs: ends(segments, typing) }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn char_count(&self) -> usize {
        self.runs.last().map_or(0, |(end, _)| *end)
    }

    pub fn runs(&self) -> &[(usize, CharLook)] {
        &self.runs
    }

    /// The runs, cut to `range`; nothing for an empty range.
    pub fn spans(&self, range: Range<usize>) -> Vec<(Range<usize>, &CharLook)> {
        let mut start = 0;
        let mut out = Vec::new();
        for (end, look) in &self.runs {
            let from = start.max(range.start);
            let to = (*end).min(range.end);
            if from < to {
                out.push((from..to, look));
            }
            start = *end;
        }
        out
    }

    /// The look typing at `index` takes: the character before it, or the first
    /// at the very start.
    pub fn look_at(&self, index: usize) -> &CharLook {
        let at = index.saturating_sub(1).min(self.char_count().saturating_sub(1));
        self.runs.iter().find(|(end, _)| at < *end).map_or(&self.runs[0].1, |(_, look)| look)
    }

    /// The words are now `new_text`: the change is the stretch between what the
    /// old and new text have in common at their start and at their end.
    pub fn edit(&mut self, new_text: &str) {
        if new_text == self.text {
            return;
        }
        let old: Vec<char> = self.text.chars().collect();
        let new: Vec<char> = new_text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let room = (old.len() - prefix).min(new.len() - prefix);
        let suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
        let removed = old.len() - prefix - suffix;
        let inserted = new.len() - prefix - suffix;
        let typing = self.look_at(prefix).clone();

        let mut segments = self.segments();
        split_at(&mut segments, prefix);
        split_at(&mut segments, prefix + removed);
        let mut at = 0;
        let mut kept = Vec::with_capacity(segments.len() + 1);
        let mut placed = false;
        for (len, look) in segments {
            if at == prefix && !placed {
                kept.push((inserted, typing.clone()));
                placed = true;
            }
            if at < prefix || at >= prefix + removed {
                kept.push((len, look));
            }
            at += len;
        }
        if !placed {
            kept.push((inserted, typing.clone()));
        }
        self.text = new_text.to_string();
        self.runs = ends(kept, typing);
    }

    /// Change the look of `range` — of every character, and of the typing look,
    /// when the range is empty.
    pub fn set_char(&mut self, range: Range<usize>, change: &CharChange) {
        let count = self.char_count();
        if count == 0 {
            change.apply(&mut self.runs[0].1);
            return;
        }
        let range = if range.is_empty() { 0..count } else { range.start.min(count)..range.end.min(count) };
        let mut segments = self.segments();
        split_at(&mut segments, range.start);
        split_at(&mut segments, range.end);
        let mut at = 0;
        for (len, look) in segments.iter_mut() {
            if at >= range.start && at + *len <= range.end {
                change.apply(look);
            }
            at += *len;
        }
        let typing = self.runs[0].1.clone();
        self.runs = ends(segments, typing);
    }

    /// `range`'s look, field by field — every character's when the range is empty,
    /// and the typing look when the text is.
    pub fn look_of(&self, range: Range<usize>) -> LookOf {
        let count = self.char_count();
        let looks: Vec<&CharLook> = if count == 0 {
            vec![&self.runs[0].1]
        } else {
            let range = if range.is_empty() { 0..count } else { range };
            self.spans(range).into_iter().map(|(_, look)| look).collect()
        };
        LookOf {
            face: Mixed::of(looks.iter().map(|l| &l.face)),
            size: Mixed::of(looks.iter().map(|l| &l.size)),
            color: Mixed::of(looks.iter().map(|l| &l.color)),
            underline: Mixed::of(looks.iter().map(|l| &l.underline)),
            strike: Mixed::of(looks.iter().map(|l| &l.strike)),
            script: Mixed::of(looks.iter().map(|l| &l.script)),
            letter_spacing_pt: Mixed::of(looks.iter().map(|l| &l.letter_spacing_pt)),
            width_pct: Mixed::of(looks.iter().map(|l| &l.width_pct)),
            baseline_pt: Mixed::of(looks.iter().map(|l| &l.baseline_pt)),
        }
    }

    /// The runs as lengths.
    fn segments(&self) -> Vec<(usize, CharLook)> {
        let mut start = 0;
        self.runs
            .iter()
            .map(|(end, look)| {
                let len = end - start;
                start = *end;
                (len, look.clone())
            })
            .collect()
    }
}

/// Make `at` a boundary between segments.
fn split_at(segments: &mut Vec<(usize, CharLook)>, at: usize) {
    let mut start = 0;
    for i in 0..segments.len() {
        let len = segments[i].0;
        if at > start && at < start + len {
            let look = segments[i].1.clone();
            segments[i].0 = at - start;
            segments.insert(i + 1, (start + len - at, look));
            return;
        }
        start += len;
    }
}

/// Lengths back to ends: empty segments dropped, neighbours that look the same
/// joined, and `typing` kept as the one look of an empty text.
fn ends(segments: Vec<(usize, CharLook)>, typing: CharLook) -> Vec<(usize, CharLook)> {
    let mut runs: Vec<(usize, CharLook)> = Vec::new();
    let mut at = 0;
    for (len, look) in segments.into_iter().filter(|(len, _)| *len > 0) {
        at += len;
        match runs.last_mut() {
            Some((end, last)) if *last == look => *end = at,
            _ => runs.push((at, look)),
        }
    }
    if runs.is_empty() {
        runs.push((0, typing));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> CharLook {
        CharLook::plain("Face", 14.0, [0, 0, 0, 255])
    }

    fn rich(text: &str) -> RichText {
        let mut r = RichText::new(look());
        r.edit(text);
        r
    }

    fn underline(on: bool) -> CharChange {
        CharChange { underline: Some(on), ..Default::default() }
    }

    fn size(pt: f32) -> CharChange {
        CharChange { size: Some(pt), ..Default::default() }
    }

    fn underlines(r: &RichText) -> Vec<(Range<usize>, bool)> {
        r.spans(0..r.char_count()).into_iter().map(|(range, look)| (range, look.underline)).collect()
    }

    fn sizes(r: &RichText) -> Vec<(usize, f32)> {
        r.runs().iter().map(|(end, look)| (*end, look.size)).collect()
    }

    #[test]
    fn typing_after_a_word_takes_its_look() {
        let mut r = rich("ab");
        r.set_char(1..2, &underline(true));
        r.edit("abc");
        assert_eq!(underlines(&r), vec![(0..1, false), (1..3, true)]);
    }

    #[test]
    fn typing_at_the_start_takes_the_first_characters_look() {
        let mut r = rich("ab");
        r.set_char(0..2, &underline(true));
        r.edit("xab");
        assert_eq!(underlines(&r), vec![(0..3, true)]);
    }

    #[test]
    fn deleting_across_runs_keeps_the_rest() {
        let mut r = rich("aaBBcc");
        r.set_char(2..4, &size(20.0));
        r.edit("aBcc");
        assert_eq!(sizes(&r), vec![(1, 14.0), (2, 20.0), (4, 14.0)]);
    }

    #[test]
    fn a_paste_over_a_range_takes_the_look_before_it() {
        let mut r = rich("aaBBcc");
        r.set_char(2..4, &size(20.0));
        r.edit("aaxyzcc");
        assert_eq!(sizes(&r), vec![(7, 14.0)], "xyz should take the look of the a before it");
    }

    #[test]
    fn setting_a_look_and_taking_it_off_again_leaves_one_run() {
        let mut r = rich("abcdef");
        r.set_char(2..4, &underline(true));
        assert_eq!(r.runs().len(), 3);
        r.set_char(2..4, &underline(false));
        assert_eq!(r.runs().len(), 1);
    }

    #[test]
    fn an_empty_range_changes_every_character_and_the_typing_look() {
        let mut r = RichText::new(look());
        r.set_char(0..0, &size(20.0));
        r.edit("x");
        assert_eq!(sizes(&r), vec![(1, 20.0)]);

        let mut r = rich("ab");
        r.set_char(1..1, &size(18.0));
        assert_eq!(sizes(&r), vec![(2, 18.0)]);
    }

    #[test]
    fn look_of_a_range_says_mixed_where_it_varies() {
        let mut r = rich("ab");
        r.set_char(1..2, &underline(true));
        let both = r.look_of(0..2);
        assert_eq!(both.underline, Mixed::Mixed);
        assert_eq!(both.size, Mixed::One(14.0));
        assert_eq!(r.look_of(0..0).underline, Mixed::Mixed, "an empty range is every character");
        assert_eq!(r.look_of(1..2).underline, Mixed::One(true));
        assert_eq!(RichText::new(look()).look_of(0..0).size, Mixed::One(14.0), "an empty text shows its typing look");
    }

    #[test]
    fn from_looks_merges_equal_neighbours() {
        let (a, b) = (look(), CharLook { size: 20.0, ..look() });
        let r = RichText::from_looks("abcd", &[a.clone(), a.clone(), b.clone(), b.clone()]);
        assert_eq!(r.text(), "abcd");
        assert_eq!(r.runs(), &[(2, a.clone()), (4, b.clone())]);
        let one = RichText::from_looks("é日", &[a.clone(), a.clone()]);
        assert_eq!(one.runs(), &[(2, a.clone())], "characters, not bytes");
        // Taken apart and put together again, a text is as it was.
        let mut edited = rich("aaBBcc");
        edited.set_char(2..4, &size(20.0));
        let looks: Vec<CharLook> = (0..6).map(|i| edited.look_at(i + 1).clone()).collect();
        assert_eq!(RichText::from_looks("aaBBcc", &looks), edited);
        let empty = RichText::from_looks("", &[a.clone()]);
        assert_eq!((empty.char_count(), empty.look_at(0)), (0, &a), "an empty text keeps the look the next character takes");
    }

    #[test]
    fn a_face_change_forgets_the_page_font() {
        let mut r = RichText::from_looks("ab", &[CharLook { page_font: Some(7), ..look() }, CharLook { page_font: Some(7), ..look() }]);
        r.set_char(1..2, &size(20.0));
        assert_eq!(r.look_of(0..2).size, Mixed::Mixed);
        assert_eq!(r.look_at(2).page_font, Some(7), "a change of size leaves the page's font alone");
        r.set_char(1..2, &CharChange { face: Some("Other".into()), ..Default::default() });
        assert_eq!(r.look_at(1).page_font, Some(7));
        assert_eq!((r.look_at(2).face.as_str(), r.look_at(2).page_font), ("Other", None));
    }

    #[test]
    fn a_change_can_put_the_page_font_back() {
        let page = CharLook { page_font: Some(7), ..look() };
        let mut r = RichText::from_looks("ab", &[page.clone(), page.clone()]);
        r.set_char(0..2, &CharChange { face: Some("Typed".into()), ..Default::default() });
        assert_eq!(r.look_at(1).page_font, None);
        r.set_char(0..2, &CharChange { face: Some(page.face.clone()), page_font: Some(Some(7)), ..Default::default() });
        assert_eq!(r.runs(), &[(2, page)]);
    }

    #[test]
    fn indices_are_characters_not_bytes() {
        let mut r = rich("é日b");
        r.set_char(1..2, &size(20.0));
        let spans: Vec<(Range<usize>, f32)> = r.spans(0..3).into_iter().map(|(range, look)| (range, look.size)).collect();
        assert_eq!(spans, vec![(0..1, 14.0), (1..2, 20.0), (2..3, 14.0)]);
    }
}
