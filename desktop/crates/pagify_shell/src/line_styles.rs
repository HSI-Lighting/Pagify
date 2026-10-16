//! The looks of a line of the page's own words, read in and written back — the glue
//! between what a piece of the page is drawn with (`RunTextLook`, a font, a size, a
//! colour) and what the Edit Text box shows per character (`CharLook`), and between
//! the box and the `SpanStyle`s that Apply sends.
//!
//! Pure: no page, no window. The numbers a superscript is drawn with are the ones
//! the new text box writes, so a script read from the page is written back the same.

use std::collections::HashMap;
use std::ops::Range;

use pdf_core::document::{Color, RunTextLook, SpanFont, SpanStyle};

use crate::rich_text::{CharLook, RichText, Script};

/// A superscript or subscript is drawn at this fraction of its size.
const SCRIPT_SIZE: f32 = 0.58;
/// Raised this far (a fraction of its size) for a superscript, lowered this far for a subscript.
const SUPER_RISE: f32 = 0.33;
const SUB_RISE: f32 = -0.14;

/// How a piece of the page looks as one character look.
///
/// `line_size` is the size of the line's first piece: a piece raised a third of it
/// and drawn at 58% is that line's superscript, and the rise it has beyond that is a
/// baseline shift on top. Anything else is a baseline shift at the size it has.
pub fn piece_look(
    face: String,
    page_font: u32,
    size: f32,
    color: [u8; 4],
    look: &RunTextLook,
    line_size: f32,
) -> CharLook {
    // `wanted` and `tolerance` are fractions of the line's size.
    let near = |value: f32, wanted: f32, tolerance: f32| (value - wanted * line_size).abs() <= tolerance * line_size;
    let small = near(size, SCRIPT_SIZE, 0.06);
    let (script, size, baseline_pt) = if line_size > 0.0 && small && near(look.rise, SUPER_RISE, 0.05) {
        (Script::Super, line_size, look.rise - SUPER_RISE * line_size)
    } else if line_size > 0.0 && small && near(look.rise, SUB_RISE, 0.05) {
        (Script::Sub, line_size, look.rise - SUB_RISE * line_size)
    } else {
        (Script::Normal, size, look.rise)
    };
    CharLook {
        face,
        page_font: Some(page_font),
        size,
        color,
        underline: look.underline,
        strike: look.strike,
        script,
        letter_spacing_pt: look.char_spacing,
        width_pct: look.width_pct,
        baseline_pt,
    }
}

/// A control code a font has no glyph for: what the box drops or turns into a hyphen.
fn is_code(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

/// Which raw character each character of one line of the box came from.
///
/// `raw` is the line's characters as the pieces spell them, each with something to
/// find it again by (`P`); `fixed` is the line as the box shows it. A control code
/// the box dropped is skipped, one it turned into "-" keeps its source, and a "-"
/// the box added where the page draws a hyphen after the line has none.
pub fn align_to_box<P: Copy>(raw: &[(char, P)], fixed: &str) -> Vec<Option<P>> {
    let mut at = 0;
    fixed
        .chars()
        .map(|c| {
            // The character itself, past any codes the box dropped before it.
            let past_codes = at + raw[at..].iter().take_while(|(r, _)| is_code(*r)).count();
            match raw.get(past_codes) {
                Some((r, source)) if *r == c => {
                    at = past_codes + 1;
                    Some(*source)
                }
                // A code that became a hyphen.
                _ if c == '-' && past_codes > at => {
                    at += 1;
                    Some(raw[at - 1].1)
                }
                // A hyphen the page draws and its text does not carry.
                _ => None,
            }
        })
        .collect()
}

/// The character ranges of the lines of `text` (split at `\n`), in characters.
pub fn line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    text.split('\n')
        .map(|line| {
            let range = start..start + line.chars().count();
            start = range.end + 1;
            range
        })
        .collect()
}

/// Whether `a` and `b` — the same words — look different anywhere. Different words
/// are not a difference of look: they are judged by the words.
pub fn looks_differ(a: &RichText, b: &RichText) -> bool {
    a.text() == b.text() && a.runs() != b.runs()
}

/// Whether the characters of `range_a` of `a` look any different from those of
/// `range_b` of `b`, run by run from each range's start.
pub fn range_looks_differ(a: &RichText, range_a: Range<usize>, b: &RichText, range_b: Range<usize>) -> bool {
    let runs = |rich: &RichText, range: &Range<usize>| -> Vec<(usize, CharLook)> {
        rich.spans(range.clone()).into_iter().map(|(span, look)| (span.end - range.start, look.clone())).collect()
    };
    runs(a, &range_a) != runs(b, &range_b)
}

/// The pieces of a line whose words are the page's own and whose looks changed, each
/// with the spans that write it again in place: `(object, its exact words, spans)`.
///
/// `objects` are the line's pieces; `orig` the line's characters in `picked` (the box
/// as it opened) and `typed_start` where the same line starts in `rich` (the box as it is
/// now); `box_chars` says which piece draws each character of `picked`. A character
/// the box does not show (a dropped control code) takes the look of the one before it.
/// A piece whose characters all look as they did is not mentioned; one that only lost
/// its line is given a span that asks for nothing, so that it is still written again
/// (and its old rule cut) with the words it had.
pub fn in_place_pieces(
    objects: &[usize],
    rich: &RichText,
    typed_start: usize,
    picked: &RichText,
    orig: Range<usize>,
    box_chars: &[Option<(usize, usize)>],
    piece_texts: &HashMap<usize, String>,
    font_objects: &HashMap<u32, usize>,
) -> Vec<(usize, String, Vec<SpanStyle>)> {
    let mut out = Vec::new();
    for &object in objects {
        let Some(text) = piece_texts.get(&object) else { continue };
        let count = text.chars().count();
        // Where in the line each character of the piece is shown, if it is.
        let mut shown: Vec<Option<usize>> = vec![None; count];
        for at in orig.clone() {
            if let Some(Some((o, index))) = box_chars.get(at) {
                if *o == object && *index < count {
                    shown[*index] = Some(at - orig.start);
                }
            }
        }
        let Some(first) = shown.iter().flatten().next().copied() else { continue };
        let now = |at: usize| rich.look_at(typed_start + at + 1).clone();
        let was = |at: usize| picked.look_at(orig.start + at + 1).clone();
        let changed = shown.iter().flatten().any(|at| now(*at) != was(*at));
        if !changed {
            continue;
        }
        // Every character of the piece has a look: the one shown, else the one before.
        let mut looks: Vec<CharLook> = Vec::with_capacity(count);
        for at in &shown {
            looks.push(match at {
                Some(at) => now(*at),
                None => looks.last().cloned().unwrap_or_else(|| now(first)),
            });
        }
        let own = was(first);
        let piece = RichText::from_looks(text, &looks);
        let mut spans = line_spans(&piece, 0..count, &own, font_objects);
        if spans.is_empty() {
            spans.push(SpanStyle {
                chars: 0..count,
                font: None,
                size: None,
                color: None,
                char_spacing: None,
                width_pct: None,
                rise: None,
                underline: false,
                strike: false,
            });
        }
        out.push((object, text.clone(), spans));
    }
    out
}

/// The spans that write `chars` of `rich` (one line of the box) over a line whose own
/// look is `base`: the characters that are not drawn as the line is, and every
/// character that is underlined or struck — a rule belongs to the span that asks
/// for it, so a line drawn under keeps its rule only by saying so.
///
/// `font_objects` says, for each page font, an object that is drawn in it.
/// `chars` counts characters of the box; the spans count from its start.
pub fn line_spans(
    rich: &RichText,
    chars: Range<usize>,
    base: &CharLook,
    font_objects: &HashMap<u32, usize>,
) -> Vec<SpanStyle> {
    let mut out: Vec<SpanStyle> = Vec::new();
    for (range, look) in rich.spans(chars.clone()) {
        let Some(mut span) = span_of(look, base, font_objects) else { continue };
        span.chars = range.start - chars.start..range.end - chars.start;
        match out.last_mut() {
            Some(last) if last.chars.end == span.chars.start && same_look(last, &span) => last.chars.end = span.chars.end,
            _ => out.push(span),
        }
    }
    out
}

/// The size a character is drawn at, a script's reduction in.
fn drawn_size(look: &CharLook) -> f32 {
    if look.script == Script::Normal { look.size } else { look.size * SCRIPT_SIZE }
}

/// How far up from the baseline it is drawn: its script's raise and its own shift.
fn drawn_rise(look: &CharLook) -> f32 {
    let script = match look.script {
        Script::Normal => 0.0,
        Script::Super => SUPER_RISE * look.size,
        Script::Sub => SUB_RISE * look.size,
    };
    script + look.baseline_pt
}

/// What `look` asks of the page beyond `base`, or nothing where it asks for none.
fn span_of(look: &CharLook, base: &CharLook, font_objects: &HashMap<u32, usize>) -> Option<SpanStyle> {
    let differs = |a: f32, b: f32| (a - b).abs() > 1e-3;
    let font = if look.face == base.face && look.page_font == base.page_font {
        None
    } else {
        match look.page_font.and_then(|font| font_objects.get(&font)) {
            Some(object) => Some(SpanFont::LikeObject(*object)),
            None => Some(SpanFont::Face(look.face.clone())),
        }
    };
    let span = SpanStyle {
        chars: 0..0,
        font,
        size: differs(drawn_size(look), drawn_size(base)).then(|| drawn_size(look)),
        color: (look.color != base.color).then(|| Color { r: look.color[0], g: look.color[1], b: look.color[2], a: look.color[3] }),
        char_spacing: differs(look.letter_spacing_pt, base.letter_spacing_pt).then_some(look.letter_spacing_pt),
        width_pct: differs(look.width_pct, base.width_pct).then_some(look.width_pct),
        rise: differs(drawn_rise(look), drawn_rise(base)).then(|| drawn_rise(look)),
        underline: look.underline,
        strike: look.strike,
    };
    let written = span.font.is_some()
        || span.size.is_some()
        || span.color.is_some()
        || span.char_spacing.is_some()
        || span.width_pct.is_some()
        || span.rise.is_some();
    (written || span.underline || span.strike).then_some(span)
}

/// Whether two spans write the same, wherever they lie.
fn same_look(a: &SpanStyle, b: &SpanStyle) -> bool {
    SpanStyle { chars: 0..0, ..a.clone() } == SpanStyle { chars: 0..0, ..b.clone() }
}

/// One piece of a line as the page has it: what it says, and what it is drawn with.
#[derive(Debug, Clone)]
pub struct Piece {
    pub object: usize,
    /// The words as the page draws them (`Document::run_exact_texts`).
    pub text: String,
    pub face: String,
    /// The page's own id for its font (`RunStyle::font`), where that is known.
    pub page_font: Option<u32>,
    pub size: f32,
    pub color: [u8; 4],
    pub look: RunTextLook,
}

/// What the Edit Text box starts with: the page's looks, character by character.
#[derive(Debug, Clone)]
pub struct OpenedLooks {
    pub rich: RichText,
    /// Each line's first piece's look: what a line written again is drawn in
    /// where its characters say nothing else. The paragraph's own look for a
    /// line with no piece.
    pub line_base: Vec<CharLook>,
    /// For each character of the box (newlines included): its piece and its
    /// index in that piece's text. `None` for a newline, an added hyphen, and a
    /// line with no piece.
    pub box_chars: Vec<Option<(usize, usize)>>,
    pub piece_texts: HashMap<usize, String>,
    /// For each page font in the paragraph, the first piece drawn in it.
    pub font_objects: HashMap<u32, usize>,
}

/// The looks of `buffer` — a paragraph's lines joined with `\n` — from the pieces
/// of each line (`lines[i]` is line `i`, left to right; missing or empty for a line
/// the page draws as shapes). `dominant` is the look of any character with no piece
/// of its own: a line with none, and the typing look of an empty box.
pub fn open_looks(lines: &[Vec<Piece>], buffer: &str, dominant: &CharLook) -> OpenedLooks {
    let mut looks: Vec<CharLook> = Vec::new();
    let mut box_chars = Vec::new();
    let mut line_base = Vec::new();
    let mut piece_texts = HashMap::new();
    let mut font_objects = HashMap::new();
    for (i, fixed) in buffer.split('\n').enumerate() {
        let pieces: &[Piece] = lines.get(i).map_or(&[], Vec::as_slice);
        // A piece raised a third of the line's first piece's size and drawn at 58% of it
        // is that line's superscript.
        let line_size = pieces.first().map_or(dominant.size, |p| p.size);
        let piece_looks: Vec<CharLook> = pieces
            .iter()
            .map(|p| {
                let mut look = piece_look(p.face.clone(), p.page_font.unwrap_or(0), p.size, p.color, &p.look, line_size);
                look.page_font = p.page_font;
                look
            })
            .collect();
        let base = piece_looks.first().cloned().unwrap_or_else(|| dominant.clone());
        for piece in pieces {
            piece_texts.insert(piece.object, piece.text.clone());
            if let Some(font) = piece.page_font {
                font_objects.entry(font).or_insert(piece.object);
            }
        }
        // The line's characters as the box was made from them: a newline inside a
        // piece is a space there.
        let raw: Vec<(char, (usize, usize))> = pieces
            .iter()
            .enumerate()
            .flat_map(|(n, p)| {
                p.text.chars().enumerate().map(move |(k, c)| (if matches!(c, '\r' | '\n') { ' ' } else { c }, (n, k)))
            })
            .collect();
        if i > 0 {
            let before = looks.last().cloned().unwrap_or_else(|| base.clone());
            looks.push(before);
            box_chars.push(None);
        }
        let mut last = base.clone();
        for source in align_to_box(&raw, fixed) {
            match source {
                Some((n, k)) => {
                    last = piece_looks[n].clone();
                    box_chars.push(Some((pieces[n].object, k)));
                }
                // An added hyphen takes the look of the character before it.
                None => box_chars.push(None),
            }
            looks.push(last.clone());
        }
        line_base.push(base);
    }
    let rich = if looks.is_empty() { RichText::new(dominant.clone()) } else { RichText::from_looks(buffer, &looks) };
    OpenedLooks { rich, line_base, box_chars, piece_texts, font_objects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_text::{CharChange, Mixed};

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const RED: [u8; 4] = [200, 0, 0, 255];

    fn base() -> CharLook {
        CharLook { page_font: Some(1), ..CharLook::plain("Base", 12.0, BLACK) }
    }

    fn rich_of(text: &str, looks: Vec<CharLook>) -> RichText {
        RichText::from_looks(text, &looks)
    }

    /// `n` characters in `look`.
    fn all(n: usize, look: &CharLook) -> Vec<CharLook> {
        vec![look.clone(); n]
    }

    fn no_spacing() -> RunTextLook {
        RunTextLook::default()
    }

    #[test]
    fn super_and_sub_read_back_from_size_and_rise() {
        let line = 12.0;
        let up = piece_look("F".into(), 3, 0.58 * line, BLACK, &RunTextLook { rise: 0.33 * line, ..no_spacing() }, line);
        assert_eq!((up.script, up.size, up.page_font), (Script::Super, line, Some(3)));
        assert!(up.baseline_pt.abs() < 1e-3, "{up:?}");
        // Raised a little more than a superscript is: the rest is a shift.
        let more = piece_look("F".into(), 3, 0.58 * line, BLACK, &RunTextLook { rise: 0.33 * line + 0.5, ..no_spacing() }, line);
        assert_eq!(more.script, Script::Super);
        assert!((more.baseline_pt - 0.5).abs() < 1e-3, "{more:?}");
        let down = piece_look("F".into(), 3, 0.58 * line, BLACK, &RunTextLook { rise: -0.14 * line, ..no_spacing() }, line);
        assert_eq!((down.script, down.size), (Script::Sub, line));
        assert!(down.baseline_pt.abs() < 1e-3, "{down:?}");
    }

    #[test]
    fn a_rise_that_fits_neither_is_a_baseline_shift() {
        let line = 12.0;
        let shifted = piece_look("F".into(), 3, line, BLACK, &RunTextLook { rise: 2.0, ..no_spacing() }, line);
        assert_eq!((shifted.script, shifted.size, shifted.baseline_pt), (Script::Normal, line, 2.0));
        // The raise of a superscript at a size that is not a superscript's.
        let big = piece_look("F".into(), 3, line, BLACK, &RunTextLook { rise: 0.33 * line, ..no_spacing() }, line);
        assert_eq!((big.script, big.size), (Script::Normal, line));
        assert!((big.baseline_pt - 0.33 * line).abs() < 1e-3);
        // The size of a superscript with no raise.
        let small = piece_look("F".into(), 3, 0.58 * line, BLACK, &no_spacing(), line);
        assert_eq!((small.script, small.size), (Script::Normal, 0.58 * line));
        // The rest of what the page says comes with it.
        let spaced = piece_look(
            "F".into(),
            3,
            line,
            RED,
            &RunTextLook { char_spacing: 1.5, width_pct: 80.0, underline: true, strike: true, ..no_spacing() },
            line,
        );
        assert_eq!((spaced.letter_spacing_pt, spaced.width_pct, spaced.color), (1.5, 80.0, RED));
        assert!(spaced.underline && spaced.strike);
    }

    #[test]
    fn align_to_box_skips_a_dropped_code_and_marks_an_added_hyphen() {
        let raw = |text: &str| -> Vec<(char, usize)> { text.chars().enumerate().map(|(i, c)| (c, i)).collect() };
        // A code at the start of the line is dropped, and the letters keep their places.
        assert_eq!(align_to_box(&raw("\u{2}ab"), "ab"), [Some(1), Some(2)]);
        // One between two letters is turned into a hyphen and keeps its source.
        assert_eq!(align_to_box(&raw("a\u{2}b"), "a-b"), [Some(0), Some(1), Some(2)]);
        // A hyphen the page draws and the text does not have is added, with no source.
        assert_eq!(align_to_box(&raw("ab"), "ab-"), [Some(0), Some(1), None]);
        // A real hyphen after a dropped code is still found.
        assert_eq!(align_to_box(&raw("\u{2}-x"), "-x"), [Some(1), Some(2)]);
        assert_eq!(align_to_box(&raw(""), ""), Vec::<Option<usize>>::new());
    }

    #[test]
    fn line_spans_is_empty_for_a_line_in_its_base_look() {
        let rich = rich_of("hello", all(5, &base()));
        assert_eq!(line_spans(&rich, 0..5, &base(), &HashMap::new()), []);
    }

    #[test]
    fn an_underlined_base_look_still_gets_its_span() {
        let under = CharLook { underline: true, ..base() };
        let rich = rich_of("hello", all(5, &under));
        let spans = line_spans(&rich, 0..5, &under, &HashMap::new());
        assert_eq!(spans.len(), 1, "{spans:?}");
        let span = &spans[0];
        assert_eq!(span.chars, 0..5);
        assert!(span.underline && !span.strike);
        assert_eq!((&span.font, span.size, span.color, span.char_spacing, span.width_pct, span.rise), (&None, None, None, None, None, None));
    }

    #[test]
    fn another_page_font_is_like_its_object() {
        let other_page_font = CharLook { page_font: Some(3), face: "Bold".into(), ..base() };
        let typed_in = CharLook { page_font: None, face: "Typed".into(), ..base() };
        let looks = [base(), other_page_font.clone(), typed_in.clone()];
        let rich = rich_of("abc", looks.to_vec());
        let spans = line_spans(&rich, 0..3, &base(), &HashMap::from([(3, 40)]));
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!((spans[0].chars.clone(), spans[0].font.clone()), (1..2, Some(SpanFont::LikeObject(40))));
        assert_eq!((spans[1].chars.clone(), spans[1].font.clone()), (2..3, Some(SpanFont::Face("Typed".into()))));
        // A page font with no piece to be like is written by its name.
        let named = line_spans(&rich_of("b", vec![other_page_font]), 0..1, &base(), &HashMap::new());
        assert_eq!(named[0].font, Some(SpanFont::Face("Bold".into())));
    }

    #[test]
    fn superscript_becomes_58_percent_and_a_third_of_an_em_up() {
        let up = CharLook { script: Script::Super, baseline_pt: 1.0, ..base() };
        let down = CharLook { script: Script::Sub, ..base() };
        let rich = rich_of("xy", vec![up, down]);
        let spans = line_spans(&rich, 0..2, &base(), &HashMap::new());
        assert_eq!(spans.len(), 2, "{spans:?}");
        let (size, rise) = (spans[0].size.expect("size"), spans[0].rise.expect("rise"));
        assert!((size - 0.58 * 12.0).abs() < 1e-3 && (rise - (0.33 * 12.0 + 1.0)).abs() < 1e-3, "{:?}", spans[0]);
        let (size, rise) = (spans[1].size.expect("size"), spans[1].rise.expect("rise"));
        assert!((size - 0.58 * 12.0).abs() < 1e-3 && (rise + 0.14 * 12.0).abs() < 1e-3, "{:?}", spans[1]);
    }

    fn piece(object: usize, text: &str, page_font: u32, size: f32) -> Piece {
        Piece {
            object,
            text: text.into(),
            face: format!("Font{page_font}"),
            page_font: Some(page_font),
            size,
            color: BLACK,
            look: no_spacing(),
        }
    }

    fn sizes(opened: &OpenedLooks) -> Vec<(usize, f32)> {
        opened.rich.runs().iter().map(|(end, look)| (*end, look.size)).collect()
    }

    #[test]
    fn a_line_of_two_pieces_opens_in_each_pieces_look() {
        let lines = vec![vec![piece(4, "Hello ", 1, 12.0), piece(5, "there", 2, 20.0)]];
        let opened = open_looks(&lines, "Hello there", &base());
        assert_eq!(sizes(&opened), [(6, 12.0), (11, 20.0)]);
        assert_eq!(opened.rich.look_of(6..11).size, Mixed::One(20.0));
        assert_eq!(opened.rich.look_at(11).page_font, Some(2));
        assert_eq!(opened.line_base.len(), 1);
        assert_eq!((opened.line_base[0].size, opened.line_base[0].page_font), (12.0, Some(1)));
        assert_eq!(opened.box_chars[0], Some((4, 0)));
        assert_eq!(opened.box_chars[6], Some((5, 0)));
        assert_eq!(opened.box_chars[10], Some((5, 4)));
        assert_eq!(opened.piece_texts[&5], "there");
        assert_eq!(opened.font_objects, HashMap::from([(1, 4), (2, 5)]));
    }

    #[test]
    fn a_line_of_one_look_opens_as_one_run() {
        let lines = vec![vec![piece(4, "one two", 1, 12.0)], vec![piece(5, "three", 1, 12.0)]];
        let opened = open_looks(&lines, "one two\nthree", &base());
        assert_eq!(opened.rich.runs().len(), 1, "{:?}", opened.rich.runs());
        assert_eq!(opened.rich.char_count(), 13);
        assert_eq!(opened.line_base.len(), 2);
        assert_eq!(opened.box_chars[7], None, "the newline has no piece");
        assert_eq!(opened.box_chars[8], Some((5, 0)));
    }

    #[test]
    fn a_code_turned_into_a_hyphen_keeps_its_piece_and_an_added_hyphen_has_none() {
        // "sup\u{2}" is "sup-" in the box; a second line "x" is given a hyphen the page
        // draws after it in the text of the line above.
        let lines = vec![vec![piece(1, "sup\u{2}", 1, 12.0)], vec![piece(2, "port", 2, 20.0)]];
        let opened = open_looks(&lines, "sup-\nport", &base());
        assert_eq!(opened.box_chars, [Some((1, 0)), Some((1, 1)), Some((1, 2)), Some((1, 3)), None, Some((2, 0)), Some((2, 1)), Some((2, 2)), Some((2, 3))]);
        let lines = vec![vec![piece(1, "ab", 1, 12.0)], vec![piece(2, "cd", 2, 20.0)]];
        let opened = open_looks(&lines, "ab-\ncd", &base());
        assert_eq!(opened.box_chars[2], None, "the added hyphen has no source");
        assert_eq!(opened.rich.look_at(3).size, 12.0, "and takes the look of the letter before it");
    }

    #[test]
    fn a_line_with_no_piece_opens_in_the_paragraphs_look() {
        let dominant = CharLook { size: 9.0, ..base() };
        let lines = vec![vec![piece(1, "abc", 1, 12.0)], vec![]];
        let opened = open_looks(&lines, "abc\n[drawn text]", &dominant);
        assert_eq!(opened.line_base[1], dominant);
        assert_eq!(opened.box_chars[4..].iter().filter(|c| c.is_some()).count(), 0);
        assert_eq!(opened.rich.look_of(4..opened.rich.char_count()).size, Mixed::One(9.0));
        // An empty box keeps the paragraph's look to type in.
        let empty = open_looks(&[], "", &dominant);
        assert_eq!((empty.rich.char_count(), empty.rich.look_at(0).size), (0, 9.0));
    }

    #[test]
    fn lines_are_counted_in_characters() {
        assert_eq!(line_ranges("é日\nb\n"), [0..2, 3..4, 5..5]);
        assert_eq!(line_ranges(""), [0..0]);
    }

    #[test]
    fn looks_differ_judges_the_same_words_only() {
        let (a, mut b) = (RichText::from_looks("abc", &all(3, &base())), RichText::from_looks("abc", &all(3, &base())));
        assert!(!looks_differ(&a, &b));
        b.set_char(1..2, &CharChange { underline: Some(true), ..Default::default() });
        assert!(looks_differ(&a, &b));
        // Other words are the words' business.
        let other = RichText::from_looks("abd", &all(3, &base()));
        assert!(!looks_differ(&a, &other));
        // The same run by run from each range's own start.
        assert!(range_looks_differ(&a, 1..3, &b, 1..3));
        assert!(!range_looks_differ(&a, 2..3, &b, 2..3));
        assert!(!range_looks_differ(&a, 0..1, &b, 0..1));
    }

    /// "Hello " and "there" opened as the page has them, with the box now as `edit` made it.
    fn two_pieces_edited(edit: impl Fn(&mut RichText)) -> (Vec<(usize, String, Vec<SpanStyle>)>, OpenedLooks) {
        let lines = vec![vec![piece(4, "Hello ", 1, 12.0), piece(5, "there", 1, 12.0)]];
        let opened = open_looks(&lines, "Hello there", &base());
        let mut now = opened.rich.clone();
        edit(&mut now);
        let found = in_place_pieces(
            &[4, 5],
            &now,
            0,
            &opened.rich,
            0..11,
            &opened.box_chars,
            &opened.piece_texts,
            &opened.font_objects,
        );
        (found, opened)
    }

    #[test]
    fn only_the_piece_whose_look_changed_is_written_again_with_its_own_words() {
        let (found, _) = two_pieces_edited(|r| {
            r.set_char(6..11, &CharChange { underline: Some(true), ..Default::default() });
        });
        assert_eq!(found.len(), 1, "{found:?}");
        let (object, text, spans) = &found[0];
        assert_eq!((*object, text.as_str()), (5, "there"));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].chars, 0..5, "counted in the piece's own words");
        assert!(spans[0].underline);
        // A look that crosses two pieces writes both, each over its own words.
        let (both, _) = two_pieces_edited(|r| {
            r.set_char(3..8, &CharChange { color: Some(RED), ..Default::default() });
        });
        let chars: Vec<_> = both.iter().map(|(o, _, spans)| (*o, spans[0].chars.clone())).collect();
        assert_eq!(chars, [(4, 3..6), (5, 0..2)]);
        // Nothing changed, nothing to write.
        assert!(two_pieces_edited(|_| {}).0.is_empty());
    }

    #[test]
    fn a_piece_that_lost_its_line_is_still_written_again_in_place() {
        let under = CharLook { underline: true, ..base() };
        let lines = vec![vec![piece(4, "Hello ", 1, 12.0), Piece { look: RunTextLook { underline: true, ..no_spacing() }, ..piece(5, "there", 1, 12.0) }]];
        let opened = open_looks(&lines, "Hello there", &base());
        assert_eq!(opened.rich.look_of(6..11).underline, Mixed::One(true), "test assumption: opened underlined");
        let mut now = opened.rich.clone();
        now.set_char(6..11, &CharChange { underline: Some(false), ..Default::default() });
        let found = in_place_pieces(&[4, 5], &now, 0, &opened.rich, 0..11, &opened.box_chars, &opened.piece_texts, &opened.font_objects);
        assert_eq!(found.len(), 1);
        let span = &found[0].2[0];
        assert_eq!((found[0].0, span.chars.clone(), span.underline), (5, 0..5, false));
        assert_eq!((&span.font, span.size, span.color, span.rise), (&None, None, None, None), "it asks for nothing but the words as they are");
        let _ = under;
    }

    #[test]
    fn a_dropped_code_takes_the_look_before_it() {
        let lines = vec![vec![piece(1, "sup\u{2}", 1, 12.0)]];
        let opened = open_looks(&lines, "sup-", &base());
        let mut now = opened.rich.clone();
        now.set_char(0..4, &CharChange { underline: Some(true), ..Default::default() });
        let found = in_place_pieces(&[1], &now, 0, &opened.rich, 0..4, &opened.box_chars, &opened.piece_texts, &opened.font_objects);
        assert_eq!(found[0].1, "sup\u{2}", "the page's own spelling");
        assert_eq!(found[0].2.len(), 1);
        assert_eq!(found[0].2[0].chars, 0..4);
        // A line further down the box counts from its own start.
        let lines = vec![vec![piece(1, "one", 1, 12.0)], vec![piece(2, "two", 1, 12.0)]];
        let opened = open_looks(&lines, "one\ntwo", &base());
        let mut now = opened.rich.clone();
        now.set_char(4..7, &CharChange { underline: Some(true), ..Default::default() });
        let found = in_place_pieces(&[2], &now, 4, &opened.rich, 4..7, &opened.box_chars, &opened.piece_texts, &opened.font_objects);
        assert_eq!((found[0].0, found[0].2[0].chars.clone()), (2, 0..3));
        // ... and the same line further down in the box as it is now than it was.
        let mut now = opened.rich.clone();
        now.edit("zz\none\ntwo");
        now.set_char(7..10, &CharChange { underline: Some(true), ..Default::default() });
        let found = in_place_pieces(&[2], &now, 7, &opened.rich, 4..7, &opened.box_chars, &opened.piece_texts, &opened.font_objects);
        assert_eq!((found[0].0, found[0].2[0].chars.clone()), (2, 0..3));
        assert!(
            in_place_pieces(&[1], &now, 3, &opened.rich, 0..3, &opened.box_chars, &opened.piece_texts, &opened.font_objects).is_empty(),
            "the line above it did not change"
        );
    }

    #[test]
    fn a_superscript_piece_opens_as_a_superscript() {
        let mut up = piece(2, "2", 1, 0.58 * 12.0);
        up.look.rise = 0.33 * 12.0;
        let lines = vec![vec![piece(1, "m", 1, 12.0), up]];
        let opened = open_looks(&lines, "m2", &base());
        let look = opened.rich.look_at(2);
        assert_eq!((look.script, look.size), (Script::Super, 12.0));
        assert_eq!(opened.rich.look_at(1).script, Script::Normal);
    }

    #[test]
    fn neighbouring_characters_with_one_look_make_one_span() {
        // Counted from the start of the line, not the box.
        let red = CharLook { color: RED, ..base() };
        let mut rich = rich_of("one two three", all(13, &base()));
        rich.set_char(4..13, &CharChange { color: Some(RED), ..Default::default() });
        let spans = line_spans(&rich, 4..13, &base(), &HashMap::new());
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(spans[0].chars, 0..9);
        assert_eq!(spans[0].color, Some(Color { r: 200, g: 0, b: 0, a: 255 }));
        // Different looks that write the same are one span as well: two page fonts
        // that are drawn like the same object.
        let (two, three) = (CharLook { page_font: Some(2), ..red.clone() }, CharLook { page_font: Some(3), ..red });
        let rich = rich_of("ab", vec![two, three]);
        let spans = line_spans(&rich, 0..2, &base(), &HashMap::from([(2, 40), (3, 40)]));
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(spans[0].chars, 0..2);
    }
}
