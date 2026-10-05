//! Putting a font *into* a document, so words can be typed that the document's
//! own fonts cannot spell.
//!
//! # The problem this solves
//!
//! A PDF producer embeds a **subset** of each font — only the glyphs the file
//! already uses. Measured on a real document: four fonts on one page, carrying
//! between eleven and thirty-four characters each. One of them could draw
//! nothing but `,01234579h–`.
//!
//! Editing text inside that means the letters available are the letters already
//! on the page in that font. Typing "Sample" where the font has no `S` cannot
//! be done by rearranging what is there; a glyph has to come from somewhere
//! else. This module is that somewhere else.
//!
//! # What it writes
//!
//! A simple `/TrueType` font with `/WinAnsiEncoding` — one byte a character,
//! which is what the rest of the editing path already speaks — with the font
//! program embedded whole in a `/FontFile2` stream, and the widths read out of
//! the font's own tables so a reader spaces the text the way the font intends.
//!
//! **Whole, not subset.** Subsetting a TrueType means rebuilding `glyf`, `loca`
//! and `cmap` and renumbering every glyph, and getting it subtly wrong produces
//! a file that opens fine and draws the wrong letters. A whole font is a few
//! hundred kilobytes, written once per document however many runs are edited.
//! The trade is size against a class of bug that would be very hard to see.
//!
//! # What it does not do
//!
//! It does not make the text match its surroundings. A face that is not the
//! document's own face is a different face, and the caller has to say so —
//! which is why [`Embedded::name`] comes back rather than being applied
//! silently.

use crate::error::{PdfError, Result};
use crate::pdf::{write_stream, Dict, Object};

/// A font ready to be written into a document.
#[derive(Debug, Clone)]
pub struct Embedded {
    /// The objects to add, as `(number, bytes)`.
    pub objects: Vec<(u32, Vec<u8>)>,
    /// The object number of the font dictionary, for a resource entry.
    pub font: u32,
    /// What the face calls itself, for telling somebody what they now have.
    pub name: String,
}

/// The metrics a `/FontDescriptor` needs, in 1000ths of an em.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub ascent: i32,
    pub descent: i32,
    pub cap_height: i32,
    pub italic_angle: f32,
    pub bbox: (i32, i32, i32, i32),
    /// Whether the face declares itself bold, so a caller can pick a weight.
    pub bold: bool,
}

/// Read a TrueType or OpenType face's metrics.
///
/// `None` for anything this cannot parse, which the caller must treat as "not a
/// font I can use" rather than substituting defaults — a descriptor of invented
/// numbers produces text that draws in the wrong place.
pub fn metrics(bytes: &[u8]) -> Option<Metrics> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let per_em = f32::from(face.units_per_em());
    if per_em <= 0.0 {
        return None;
    }
    let scale = |value: f32| (value * 1000.0 / per_em).round() as i32;
    let bbox = face.global_bounding_box();

    Some(Metrics {
        ascent: scale(f32::from(face.ascender())),
        descent: scale(f32::from(face.descender())),
        // Not every face declares one; the ascender is the honest stand-in and
        // is what readers fall back to anyway.
        cap_height: scale(f32::from(face.capital_height().unwrap_or(face.ascender()))),
        italic_angle: face.italic_angle(),
        bbox: (
            scale(f32::from(bbox.x_min)),
            scale(f32::from(bbox.y_min)),
            scale(f32::from(bbox.x_max)),
            scale(f32::from(bbox.y_max)),
        ),
        bold: face.is_bold(),
    })
}

/// What this face calls itself, if it says.
pub fn face_name(bytes: &[u8]) -> Option<String> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    face.names()
        .into_iter()
        .filter(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .find_map(|name| name.to_string())
        .or_else(|| {
            face.names()
                .into_iter()
                .filter(|name| name.name_id == ttf_parser::name_id::FULL_NAME)
                .find_map(|name| name.to_string())
        })
}

/// Whether this face can draw every character of some words.
///
/// Asked before anything is written, so a font that cannot help is passed over
/// rather than embedded and then found wanting.
pub fn can_spell(bytes: &[u8], text: &str) -> bool {
    let Ok(face) = ttf_parser::Face::parse(bytes, 0) else { return false };
    text.chars().all(|c| {
        c.is_whitespace() || (win_ansi_code(c).is_some() && face.glyph_index(c).is_some())
    })
}

/// Which characters a face can actually **draw** — not merely map to a glyph.
///
/// See [`outlined_chars`].
#[derive(Debug, Clone)]
pub struct OutlineCoverage {
    drawn: std::collections::HashSet<char>,
}

impl OutlineCoverage {
    /// Whether the face has ink for this character: a glyph for it, with an
    /// outline. `false` for a character the face does not map at all, and for
    /// one it maps to an empty glyph — which is also what a space is, so a
    /// caller laying out text treats whitespace as drawable itself.
    pub fn has(&self, c: char) -> bool {
        self.drawn.contains(&c)
    }
}

/// Which characters this face can really draw, read once.
///
/// **Not [`can_spell`], which is fooled by exactly the fonts this exists for.**
/// A producer embeds a *subset* — only the glyphs the file already uses — and
/// many subsetters keep the whole `cmap` and every advance width while
/// emptying the outlines of the glyphs nothing drew. Ask such a face about a
/// letter it never drew and it says yes: there is a glyph, it has a width. A
/// renderer that trusts that draws nothing, and — because the character was
/// *found* — never tries the next font in its fallback list. Measured on a
/// real datasheet: a bold heading's subset has no ink for b, f, j, k, q or z.
///
/// So this asks the next question: does that glyph have an outline. `None` for
/// anything that is not a font this can read, and for a face with no `cmap` at
/// all — a CID-keyed subset, whose glyphs the page reaches by number, never by
/// character. Nothing can be looked up in such a face by character, so a
/// renderer already falls back for every one and there is nothing to correct.
///
/// Computed over the characters the face's own Unicode `cmap` lists, so the
/// cost follows the size of the face's character set (a subset: dozens; a full
/// Latin face: a couple of thousand), not the size of Unicode.
///
/// ponytail: eager over the whole `cmap`. Measured: about 0.1 to 0.2 ms for a
/// PDF's embedded subset, about 1 ms for all of Montserrat; a fully embedded CJK
/// face (60,000-odd codes) would be tens of milliseconds, once per face. Make
/// `has` lazy per character if that is ever felt.
pub fn outlined_chars(bytes: &[u8]) -> Option<OutlineCoverage> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let cmap = face.tables().cmap?;
    let mut drawn = std::collections::HashSet::new();
    for subtable in cmap.subtables {
        // Unicode subtables — and a Windows *symbol* one, which a font that
        // draws its letters at U+F020..U+F0FF carries instead. `Face` does not
        // look in it, but the renderer the editor draws with does, and prefers
        // it over any other: leaving it out would call every letter of such a
        // face undrawable and send the whole line to the fallback.
        let symbol = subtable.platform_id == ttf_parser::PlatformId::Windows && subtable.encoding_id == 0;
        if !subtable.is_unicode() && !symbol {
            continue;
        }
        subtable.codepoints(|code| {
            let Some(c) = char::from_u32(code) else { return };
            // A Unicode code point is resolved the way a lookup resolves it —
            // through `Face`, over every Unicode subtable in order — so one two
            // subtables disagree about is judged by the glyph a renderer would
            // actually be given. `glyph_bounding_box` is `None` for a glyph
            // with no outline: an empty `glyf` entry, or a CFF charstring that
            // draws nothing.
            let glyph = if symbol { subtable.glyph_index(code) } else { face.glyph_index(c) };
            if glyph.is_some_and(|glyph| face.glyph_bounding_box(glyph).is_some()) {
                drawn.insert(c);
                if symbol {
                    drawn.extend(symbol_alias(code));
                }
            }
        });
    }
    Some(OutlineCoverage { drawn })
}

/// The ordinary character a symbol face's private-use code stands for.
///
/// A symbol font keeps its letters at U+F020..U+F0FF, and the renderer reads
/// the same glyphs at U+0020..U+00FF (the duplication HarfBuzz and Windows
/// both make). So a glyph found at U+F061 is also what `a` draws.
fn symbol_alias(code: u32) -> Option<char> {
    (0xF000..=0xF0FF).contains(&code).then(|| char::from_u32(code - 0xF000)).flatten()
}

/// The bytes that spell these words under `/WinAnsiEncoding`.
///
/// `None` for a character the encoding has no code for — most of Unicode. A
/// caller must refuse rather than substitute: a wrong code draws a wrong letter
/// silently, which is worse than not writing it at all.
pub fn win_ansi(text: &str) -> Option<Vec<u8>> {
    text.chars().map(win_ansi_code).collect()
}

/// Build everything a document needs in order to draw with this font.
///
/// `first` is the first object number free in the file; three are used.
pub fn truetype(bytes: &[u8], first: u32) -> Result<Embedded> {
    let face = ttf_parser::Face::parse(bytes, 0)
        .map_err(|_| PdfError::InvalidArgument("that file is not a font this can read".into()))?;
    let metrics = metrics(bytes)
        .ok_or_else(|| PdfError::InvalidArgument("that font declares no usable metrics".into()))?;
    let per_em = f32::from(face.units_per_em());

    // A PostScript name has no spaces in it, and a `/Name` in a PDF cannot hold
    // one either without escaping. Both problems, one answer.
    let readable = face_name(bytes).unwrap_or_else(|| "PagifyTyping".into());
    let pdf_name: String = readable
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '+')
        .collect();
    let pdf_name = if pdf_name.is_empty() { "PagifyTyping".to_string() } else { pdf_name };

    let overflow = || PdfError::Unsupported("this file already uses the highest object number a PDF can hold");
    let (font, descriptor, file) = (
        first,
        first.checked_add(1).ok_or_else(overflow)?,
        first.checked_add(2).ok_or_else(overflow)?,
    );

    // Widths for every code the encoding can carry, in 1000ths of an em. A code
    // the face has no glyph for is zero, which is what a reader expects for a
    // character it will not draw.
    let mut widths = Vec::with_capacity(224);
    for code in 32u8..=255 {
        let advance = win_ansi_char(code)
            .and_then(|c| face.glyph_index(c))
            .and_then(|glyph| face.glyph_hor_advance(glyph))
            .map(|units| (f32::from(units) * 1000.0 / per_em).round() as i32)
            .unwrap_or(0);
        widths.push(Object::Number(advance.to_string().into_bytes()));
    }

    let mut font_dict = Dict(Vec::new());
    font_dict.set(b"Type", Object::Name(b"Font".to_vec()));
    font_dict.set(b"Subtype", Object::Name(b"TrueType".to_vec()));
    font_dict.set(b"BaseFont", Object::Name(pdf_name.clone().into_bytes()));
    font_dict.set(b"FirstChar", Object::Number(b"32".to_vec()));
    font_dict.set(b"LastChar", Object::Number(b"255".to_vec()));
    font_dict.set(b"Widths", Object::Array(widths));
    font_dict.set(b"Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    font_dict.set(b"FontDescriptor", Object::Reference(descriptor, 0));

    let mut descriptor_dict = Dict(Vec::new());
    descriptor_dict.set(b"Type", Object::Name(b"FontDescriptor".to_vec()));
    descriptor_dict.set(b"FontName", Object::Name(pdf_name.clone().into_bytes()));
    // Bit 6, nonsymbolic: the font uses the standard Latin character set, which
    // is what `/WinAnsiEncoding` says it does. Bit 19 as well where the face
    // says it is bold, which is what a reader uses to fake one if it must.
    let flags = 32 | if metrics.bold { 1 << 18 } else { 0 };
    descriptor_dict.set(b"Flags", Object::Number(flags.to_string().into_bytes()));
    descriptor_dict.set(
        b"FontBBox",
        Object::Array(
            [metrics.bbox.0, metrics.bbox.1, metrics.bbox.2, metrics.bbox.3]
                .iter()
                .map(|n| Object::Number(n.to_string().into_bytes()))
                .collect(),
        ),
    );
    descriptor_dict.set(
        b"ItalicAngle",
        Object::Number(format!("{:.1}", metrics.italic_angle).into_bytes()),
    );
    descriptor_dict.set(b"Ascent", Object::Number(metrics.ascent.to_string().into_bytes()));
    descriptor_dict.set(b"Descent", Object::Number(metrics.descent.to_string().into_bytes()));
    descriptor_dict.set(b"CapHeight", Object::Number(metrics.cap_height.to_string().into_bytes()));
    // Required, and nothing reads it for a font whose program is embedded — the
    // reader has the outlines and can see the stems for itself.
    descriptor_dict.set(b"StemV", Object::Number(b"80".to_vec()));
    descriptor_dict.set(b"FontFile2", Object::Reference(file, 0));

    let packed = crate::pdf::content::encode(bytes)?;
    let mut file_dict = Dict(Vec::new());
    // The *uncompressed* length, which is how a reader knows it has the whole
    // font program and not just the part that fitted.
    file_dict.set(b"Length1", Object::Number(bytes.len().to_string().into_bytes()));
    file_dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));

    let mut font_bytes = Vec::new();
    crate::pdf::write_object(&mut font_bytes, &Object::Dict(font_dict));
    let mut descriptor_bytes = Vec::new();
    crate::pdf::write_object(&mut descriptor_bytes, &Object::Dict(descriptor_dict));

    Ok(Embedded {
        objects: vec![
            (font, font_bytes),
            (descriptor, descriptor_bytes),
            (file, write_stream(&file_dict, &packed)),
        ],
        font,
        name: readable,
    })
}

/// The 32 characters `/WinAnsiEncoding` puts where Latin-1 has controls.
///
/// Everything else in the encoding is ASCII below 127 and Latin-1 above 159, so
/// only this stretch needs a table. Getting one of these wrong writes a
/// different punctuation mark, which is exactly the kind of error nobody sees
/// until it is printed.
const HIGH: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
    '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}',
    '\u{017D}', '\u{FFFD}', '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
    '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
    '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// What a code spells, or `None` where the encoding leaves a gap.
fn win_ansi_char(code: u8) -> Option<char> {
    match code {
        0x20..=0x7E => Some(code as char),
        0x80..=0x9F => {
            let found = HIGH[(code - 0x80) as usize];
            (found != '\u{FFFD}').then_some(found)
        }
        0xA0..=0xFF => Some(code as char),
        _ => None,
    }
}

/// The code that spells a character, or `None` where there is not one.
fn win_ansi_code(character: char) -> Option<u8> {
    match character {
        ' '..='~' => Some(character as u8),
        '\u{A0}'..='\u{FF}' => Some(character as u8),
        _ => HIGH
            .iter()
            .position(|c| *c == character && *c != '\u{FFFD}')
            .map(|at| 0x80 + at as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_its_own_encoding() {
        assert_eq!(win_ansi("Hello, world!"), Some(b"Hello, world!".to_vec()));
        assert_eq!(win_ansi_char(b'A'), Some('A'));
    }

    /// The stretch where the encoding is not Latin-1, and the reason the table
    /// exists at all.
    #[test]
    fn the_punctuation_windows_moved_is_where_windows_put_it() {
        assert_eq!(win_ansi("\u{2019}"), Some(vec![0x92]), "a right single quote");
        assert_eq!(win_ansi("\u{2014}"), Some(vec![0x97]), "an em dash");
        assert_eq!(win_ansi("\u{20AC}"), Some(vec![0x80]), "a euro sign");
        assert_eq!(win_ansi_char(0x92), Some('\u{2019}'));
        assert_eq!(win_ansi_char(0x97), Some('\u{2014}'));
    }

    #[test]
    fn latin_one_above_the_gap_is_itself() {
        assert_eq!(win_ansi("é"), Some(vec![0xE9]));
        assert_eq!(win_ansi_char(0xE9), Some('é'));
    }

    /// **A character with no code refuses rather than substituting.** A wrong
    /// code draws a wrong letter silently.
    #[test]
    fn a_character_the_encoding_cannot_carry_is_refused() {
        assert_eq!(win_ansi("日本語"), None);
        assert_eq!(win_ansi("Hello 日"), None);
        // The holes inside the moved stretch are holes.
        assert_eq!(win_ansi_char(0x81), None);
        assert_eq!(win_ansi_char(0x8D), None);
    }

    #[test]
    fn nonsense_is_not_a_font() {
        assert!(metrics(b"not a font at all").is_none());
        assert!(!can_spell(b"not a font at all", "abc"));
        assert!(truetype(b"not a font at all", 10).is_err());
        assert!(outlined_chars(b"not a font at all").is_none());
    }

    // -- which characters a face can really draw ---------------------------

    /// A real face, with a real `cmap` and real outlines: the Montserrat
    /// Regular the app bundles for outlined-text matching. Fails, rather than
    /// skips, where it is not found — a wrong path must not look like a pass.
    fn montserrat() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../desktop/third_party/fonts/Montserrat-Regular.ttf"
        );
        std::fs::read(path).unwrap_or_else(|e| panic!("the bundled Montserrat was not found at {path}: {e}"))
    }

    /// Where a table starts in an sfnt file: the directory is 12 bytes of
    /// header, then 16 per table (tag, checksum, offset, length).
    fn sfnt_table(font: &[u8], tag: &[u8; 4]) -> usize {
        let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
        let u32_at = |at: usize| u32::from_be_bytes([font[at], font[at + 1], font[at + 2], font[at + 3]]) as usize;
        (0..u16_at(4))
            .map(|i| 12 + 16 * i)
            .find(|&at| &font[at..at + 4] == tag)
            .map(|at| u32_at(at + 8))
            .unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)))
    }

    /// The same face with every `cmap` record rewritten as a Windows *symbol*
    /// subtable (platform 3, encoding 0). The codes are untouched — `a` is
    /// still at 0x61 — but nothing in the face is a Unicode subtable any more,
    /// which is how a symbol-encoded face looks to `ttf_parser::Face`.
    fn as_a_symbol_face(font: &[u8]) -> Vec<u8> {
        let cmap = sfnt_table(font, b"cmap");
        let records = u16::from_be_bytes([font[cmap + 2], font[cmap + 3]]) as usize;
        let mut out = font.to_vec();
        for i in 0..records {
            let at = cmap + 4 + 8 * i;
            out[at..at + 2].copy_from_slice(&3u16.to_be_bytes());
            out[at + 2..at + 4].copy_from_slice(&0u16.to_be_bytes());
        }
        out
    }

    /// The same face with one letter's outline taken out the way a subsetter
    /// leaves a glyph nothing drew: its `loca` entry made equal to the next
    /// one, so the glyph has no data at all. `cmap` and `hmtx` are untouched —
    /// the face still maps the letter and still gives it a width.
    fn without_outline_for(font: &[u8], c: char) -> Vec<u8> {
        let glyph = ttf_parser::Face::parse(font, 0)
            .expect("a face")
            .glyph_index(c)
            .expect("the face maps this letter")
            .0 as usize;
        let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
        let (head, loca) = (sfnt_table(font, b"head"), sfnt_table(font, b"loca"));
        let long_offsets = u16_at(head + 50) != 0;
        let width = if long_offsets { 4 } else { 2 };
        let mut out = font.to_vec();
        let (from, to) = (loca + width * (glyph + 1), loca + width * glyph);
        let next = out[from..from + width].to_vec();
        out[to..to + width].copy_from_slice(&next);
        out
    }

    /// **The reason this function exists.** A face that maps a letter and
    /// gives it a width but has no outline for it — what a subsetted font
    /// embedded in a PDF is — answers yes to `can_spell` and draws nothing.
    /// `outlined_chars` says no, and still says yes for the letters beside it.
    #[test]
    fn a_letter_the_face_maps_but_never_drew_is_not_covered() {
        let full = montserrat();
        let gutted = without_outline_for(&full, 'b');

        // The setup is what it claims to be: mapped, with a width...
        let face = ttf_parser::Face::parse(&gutted, 0).expect("still a face");
        let glyph = face.glyph_index('b').expect("the cmap still maps b");
        assert!(face.glyph_hor_advance(glyph).unwrap_or(0) > 0, "and still gives it a width");
        assert!(can_spell(&gutted, "b"), "which is all `can_spell` asks");
        // ...and with nothing to draw.
        assert!(face.glyph_bounding_box(glyph).is_none(), "the glyph really is empty now");

        let coverage = outlined_chars(&gutted).expect("a face");
        assert!(!coverage.has('b'), "b has no ink");
        // Its neighbours in glyph order — `a` shares the loca entry that was
        // changed — are untouched.
        for c in ['a', 'c', 'B', 'A', 'z', '0'] {
            assert!(coverage.has(c), "{c:?} was not touched and must still be covered");
        }
        // And the face it was made from covers b, so the change is the cause.
        assert!(outlined_chars(&full).expect("a face").has('b'));
    }

    /// **A symbol-encoded face is read the way the renderer reads it.** Its
    /// letters are in a Windows symbol subtable, which `ttf_parser::Face`
    /// never looks in; the editor's renderer prefers it over every other. A
    /// coverage that ignored it would call every letter of such a face blank
    /// and send the whole line to the fallback font.
    #[test]
    fn a_symbol_encoded_face_is_covered_where_the_renderer_would_draw_it() {
        let full = montserrat();
        let symbol = as_a_symbol_face(&full);
        // The setup is what it claims to be: the plain lookup finds nothing...
        let face = ttf_parser::Face::parse(&symbol, 0).expect("still a face");
        assert!(face.glyph_index('a').is_none(), "setup: no Unicode subtable is left to find `a` in");
        assert!(!can_spell(&symbol, "a"));
        // ...and the coverage still covers it.
        let coverage = outlined_chars(&symbol).expect("a face");
        for c in ['a', 'b', 'Z', '0', '-'] {
            assert!(coverage.has(c), "{c:?} is drawn by a symbol face and was not covered");
        }
        // Emptied letters are still emptied.
        let gutted = as_a_symbol_face(&without_outline_for(&full, 'b'));
        let coverage = outlined_chars(&gutted).expect("a face");
        assert!(!coverage.has('b'), "b has no ink in a symbol face either");
        assert!(coverage.has('a'));
    }

    /// **A face with no `cmap` has no coverage to read** — the CID-keyed
    /// subsets Illustrator embeds (measured on a real datasheet: both the
    /// Light and the ExtraBold of one page) reach their glyphs by number, so
    /// nothing can be looked up in them by character. `None`, not an empty
    /// set: the renderer falls back for every character already, and calling
    /// them all blank would only chop each line into sections for nothing.
    #[test]
    fn a_face_with_no_cmap_has_no_coverage_to_read() {
        let mut font = montserrat();
        let directory = (0..u16::from_be_bytes([font[4], font[5]]) as usize)
            .map(|i| 12 + 16 * i)
            .find(|&at| &font[at..at + 4] == b"cmap")
            .expect("the face has a cmap");
        font[directory..directory + 4].copy_from_slice(b"cmaX");
        // Still a face — metrics read — just not one that maps characters.
        assert!(metrics(&font).is_some());
        assert!(outlined_chars(&font).is_none());
    }

    /// A symbol font keeps its letters at U+F020..U+F0FF and is read at
    /// U+0020..U+00FF too; nothing else gets an alias.
    #[test]
    fn a_symbol_codes_alias_is_the_ordinary_character() {
        assert_eq!(symbol_alias(0xF061), Some('a'));
        assert_eq!(symbol_alias(0xF020), Some(' '));
        assert_eq!(symbol_alias(0xF0FF), Some('\u{FF}'));
        assert_eq!(symbol_alias(0x0061), None, "an ordinary code has no alias");
        assert_eq!(symbol_alias(0xF100), None, "past the symbol range");
        assert_eq!(symbol_alias(0xEFFF), None, "before it");
    }

    /// A character the face does not map at all has nothing to draw either —
    /// which is what lets a caller send it to a fallback font — and a space,
    /// which every face maps to an empty glyph, is not "ink": whitespace is
    /// the caller's to treat as drawable.
    #[test]
    fn unmapped_characters_and_whitespace_are_not_ink() {
        let coverage = outlined_chars(&montserrat()).expect("a face");
        assert!(!coverage.has('\u{4e00}'), "a CJK ideograph is not in Montserrat");
        assert!(!coverage.has(' '), "a space has no outline");
        assert!(coverage.has('é'), "an accented Latin letter is");
    }

    /// Read once and answered from memory: a full Latin face is a few thousand
    /// characters, and the editor asks per character on every frame.
    #[test]
    fn reading_a_whole_face_is_cheap_enough_to_do_when_it_is_picked() {
        let font = montserrat();
        let started = std::time::Instant::now();
        let coverage = outlined_chars(&font).expect("a face");
        let took = started.elapsed();
        eprintln!("outlined_chars over Montserrat-Regular ({} bytes): {took:?}", font.len());
        assert!(coverage.drawn.len() > 500, "a Latin face covers hundreds of characters");
        // Typically a few milliseconds; the bound only catches a quadratic slip.
        assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    }
}
