//! What a font **subset** can really draw, read from the font itself.
//!
//! A simple font with no `/ToUnicode` is read as ASCII, one code to a
//! character — the one thing every standard encoding agrees on. That says which
//! character a *code* means. It says nothing about whether the font program
//! has an *outline* behind it, and a producer's subset keeps only the glyphs
//! the file drew.
//!
//! Measured on a real datasheet: `MyriadPro-Regular` embedded as a Type1C
//! subset of `space zero six m`, under `/Encoding /WinAnsiEncoding`. Every
//! digit is a perfectly good *code* — and typing `5` wrote code `0x35`, which
//! the reader drew as an empty box, with no error anywhere because every check
//! along the way was about codes. The page's TrueType subsets are the same
//! trap without even a `/CharSet` to say so.
//!
//! So the font program is asked: has this code's glyph any ink? Where the
//! program cannot be read (a Type 1 `FontFile`), the descriptor's `/CharSet` —
//! the producer's own list of the glyphs it kept — is read instead. Anything
//! neither can judge comes back `None`, "no evidence", so the caller keeps
//! doing what it did before rather than guessing.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use super::{content, Dict, File, Object};

/// The codes in `0x20..=0x7E` whose glyph the subset kept, for the font
/// `name` in `fonts`.
///
/// `None` when this cannot tell: nothing readable to ask, or nothing in what
/// it read that matches an ASCII character (a subset that renamed its glyphs
/// `g1 g2 …` is not one this can read, and treating every letter as missing
/// would refuse words the font draws perfectly well).
pub fn drawable_ascii(file: &File<'_>, fonts: &Dict, name: &[u8]) -> Option<BTreeSet<u32>> {
    let font = file.resolve(fonts.get(name)?).ok()?;
    let font = font.as_dict()?;
    let descriptor = file.resolve(font.get(b"FontDescriptor")?).ok()?;
    let descriptor = descriptor.as_dict()?;
    let differences = differences(file, font);

    let mut drawable = program_ink(file, descriptor, &differences)
        .or_else(|| charset_ink(file, descriptor, &differences))?;
    // A space draws nothing, and plenty of subsets list none.
    drawable.insert(0x20);
    Some(drawable)
}

/// Put a font file's table directory in the order the format requires: sorted by
/// tag, ascending.
///
/// **Why this exists.** Every reader here — `subsetter`, `ttf-parser` — finds a
/// table by binary search over that directory, which is only correct when it is
/// sorted. Microsoft Office's PDF export (Excel, Word) writes the directory in
/// its own order (`glyf, cmap, head, hhea, hmtx, loca, maxp, name, OS/2, ...`), so
/// the search walks past `glyf`, finds nothing, and the subsetter reports
/// `UnknownKind` for a perfectly good TrueType font. That is what stopped a new
/// line being added to any paragraph in a PDF printed from Excel — and
/// `ttf-parser` fails the same way, quietly, with `None`.
///
/// Only the 16-byte directory records move. Offsets are absolute, so the tables
/// themselves stay where they are and stay valid. A collection (`ttcf`), or data
/// too short to hold the directory it declares, is left exactly as it was.
pub fn sort_table_directory(font: &mut [u8]) {
    if font.len() < 12 || &font[..4] == b"ttcf" {
        return;
    }
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let Some(directory) = font.get_mut(12..12 + 16 * count) else { return };
    let mut records: Vec<[u8; 16]> = directory
        .chunks_exact(16)
        .map(|record| record.try_into().expect("16 bytes"))
        .collect();
    if records.windows(2).all(|pair| pair[0][..4] <= pair[1][..4]) {
        return;
    }
    records.sort_by(|a, b| a[..4].cmp(&b[..4]));
    for (slot, record) in directory.chunks_exact_mut(16).zip(&records) {
        slot.copy_from_slice(record);
    }
}

/// Ask the embedded font program, if it is one this can read: a TrueType or
/// OpenType font, or a bare CFF (`/FontFile3 /Type1C`, which is not wrapped in
/// the container the first two are).
///
/// `Some` of an **empty** set is an answer — a font that is readable and has
/// no ink for any ASCII character. That is not rare: one page's `ArialMT` was
/// 4,547 glyphs, a complete `cmap`, and no outline for any of them but the one
/// symbol it was embedded to draw. `None` is for a program this could not
/// read, or whose glyphs it could not find by any name or code.
fn program_ink(
    file: &File<'_>,
    descriptor: &Dict,
    differences: &BTreeMap<u32, String>,
) -> Option<BTreeSet<u32>> {
    let mut program = [&b"FontFile2"[..], b"FontFile3"].into_iter().find_map(|key| {
        let Object::Stream(dict, range) = file.resolve(descriptor.get(key)?).ok()? else {
            return None;
        };
        content::decode(&dict, file.bytes().get(range)?)
    })?;
    // Office exports an unsorted directory, which `ttf-parser` cannot search —
    // see `sort_table_directory`.
    sort_table_directory(&mut program);

    let mut out = BTreeSet::new();
    if let Ok(face) = ttf_parser::Face::parse(&program, 0) {
        let subtables: Vec<ttf_parser::cmap::Subtable<'_>> =
            face.tables().cmap?.subtables.into_iter().collect();
        // The order a reader tries them in for a plain (non-symbolic) font:
        // Unicode, then the symbolic block, then Mac Roman.
        let find = |code: u32| {
            let unicode = subtables.iter().filter(|s| s.is_unicode());
            let symbolic = subtables
                .iter()
                .filter(|s| s.platform_id == ttf_parser::PlatformId::Windows && s.encoding_id == 0);
            let mac = subtables
                .iter()
                .filter(|s| s.platform_id == ttf_parser::PlatformId::Macintosh && s.encoding_id == 0);
            unicode
                .filter_map(|s| s.glyph_index(code))
                .chain(symbolic.filter_map(|s| s.glyph_index(0xF000 + code).or_else(|| s.glyph_index(code))))
                .chain(mac.filter_map(|s| s.glyph_index(code)))
                .find(|g| g.0 != 0)
        };
        for code in 0x21u32..=0x7E {
            let glyph = differences.get(&code).and_then(|n| face.glyph_index_by_name(n)).or_else(|| find(code));
            if glyph.is_some_and(|glyph| face.glyph_bounding_box(glyph).is_some()) {
                out.insert(code);
            }
        }
        return Some(out);
    }
    if let Some(cff) = ttf_parser::cff::Table::parse(&program) {
        struct Ink;
        impl ttf_parser::OutlineBuilder for Ink {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {}
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        let mut found = 0usize;
        for code in 0x21u32..=0x7E {
            let names = match differences.get(&code) {
                Some(name) => vec![name.clone()],
                None => candidates(code),
            };
            let Some(glyph) = names.iter().find_map(|n| cff.glyph_index_by_name(n)) else { continue };
            found += 1;
            // Only a glyph with no ink at all is left out. Any other failure to
            // draw one is this reader's, not the font's.
            if !matches!(cff.outline(glyph, &mut Ink), Err(ttf_parser::CFFError::ZeroBBox)) {
                out.insert(code);
            }
        }
        // Glyphs named by none of the names an ASCII character goes by: a
        // font this cannot read, and not a font with nothing in it.
        return (found > 0).then_some(out);
    }
    None
}

/// Ask the descriptor's `/CharSet`, the producer's own list of what it kept.
fn charset_ink(
    file: &File<'_>,
    descriptor: &Dict,
    differences: &BTreeMap<u32, String>,
) -> Option<BTreeSet<u32>> {
    let listed = listed_glyphs(file, descriptor)?;
    let mut out = BTreeSet::new();
    for code in 0x21u32..=0x7E {
        let names = match differences.get(&code) {
            Some(name) => vec![name.clone()],
            None => candidates(code),
        };
        if names.iter().any(|n| listed.contains(n)) {
            out.insert(code);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The glyph names the descriptor's `/CharSet` string lists.
fn listed_glyphs(file: &File<'_>, descriptor: &Dict) -> Option<HashSet<String>> {
    let text: Vec<u8> = match file.resolve(descriptor.get(b"CharSet")?).ok()? {
        Object::LiteralString(bytes) => bytes,
        Object::HexString(hex) => {
            let digits: Vec<u8> = hex.iter().copied().filter(|b| b.is_ascii_hexdigit()).collect();
            digits
                .chunks(2)
                .filter_map(|pair| {
                    let pair = std::str::from_utf8(pair).ok()?;
                    u8::from_str_radix(pair, 16).ok()
                })
                .collect()
        }
        _ => return None,
    };
    let names: HashSet<String> = text
        .split(|b| *b == b'/')
        .map(|n| String::from_utf8_lossy(n).trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    (!names.is_empty()).then_some(names)
}

/// The names `/Encoding`'s `/Differences` gives to codes, if it has any.
fn differences(file: &File<'_>, font: &Dict) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let Some(encoding) = font.get(b"Encoding").and_then(|e| file.resolve(e).ok()) else {
        return out;
    };
    let Some(list) = encoding
        .as_dict()
        .and_then(|d| d.get(b"Differences"))
        .and_then(|d| file.resolve(d).ok())
    else {
        return out;
    };
    let Object::Array(items) = list else { return out };

    let mut code = 0u32;
    for item in items {
        let item = file.resolve(&item).unwrap_or(item);
        if let Some(n) = item.as_i64() {
            code = u32::try_from(n).unwrap_or(0);
        } else if let Some(name) = item.as_name() {
            out.insert(code, String::from_utf8_lossy(name).to_string());
            code += 1;
        }
    }
    out
}

/// Every name an ASCII code may go by in a glyph list: the encoding's own, the
/// Standard encoding's where it differs, and the `uniXXXX` form some producers
/// write instead.
fn candidates(code: u32) -> Vec<String> {
    const DIGITS: [&str; 10] =
        ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    let names: Vec<&str> = match code {
        0x20 => vec!["space"],
        0x21 => vec!["exclam"],
        0x22 => vec!["quotedbl"],
        0x23 => vec!["numbersign"],
        0x24 => vec!["dollar"],
        0x25 => vec!["percent"],
        0x26 => vec!["ampersand"],
        0x27 => vec!["quotesingle", "quoteright"],
        0x28 => vec!["parenleft"],
        0x29 => vec!["parenright"],
        0x2A => vec!["asterisk"],
        0x2B => vec!["plus"],
        0x2C => vec!["comma"],
        0x2D => vec!["hyphen", "minus", "sfthyphen"],
        0x2E => vec!["period"],
        0x2F => vec!["slash"],
        0x30..=0x39 => vec![DIGITS[(code - 0x30) as usize]],
        0x3A => vec!["colon"],
        0x3B => vec!["semicolon"],
        0x3C => vec!["less"],
        0x3D => vec!["equal"],
        0x3E => vec!["greater"],
        0x3F => vec!["question"],
        0x40 => vec!["at"],
        0x41..=0x5A => {
            let at = (code - 0x41) as usize;
            vec![&LETTERS[at..at + 1]]
        }
        0x5B => vec!["bracketleft"],
        0x5C => vec!["backslash"],
        0x5D => vec!["bracketright"],
        0x5E => vec!["asciicircum"],
        0x5F => vec!["underscore"],
        0x60 => vec!["grave", "quoteleft"],
        0x61..=0x7A => {
            let at = (code - 0x61) as usize + 26;
            vec![&LETTERS[at..at + 1]]
        }
        0x7B => vec!["braceleft"],
        0x7C => vec!["bar"],
        0x7D => vec!["braceright"],
        0x7E => vec!["asciitilde"],
        _ => Vec::new(),
    };
    let mut out: Vec<String> = names.into_iter().map(str::to_string).collect();
    out.push(format!("uni{code:04X}"));
    out
}
