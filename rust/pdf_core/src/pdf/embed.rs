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

    let (font, descriptor, file) = (first, first + 1, first + 2);

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
    }
}
