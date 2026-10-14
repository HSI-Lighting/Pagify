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

/// Give a font program that has **no `cmap` table** one, built from what the
/// PDF says each glyph spells (`glyphs`: character → glyph id).
///
/// **Why this exists.** An `/Identity-H` font is addressed by glyph id and read
/// through `/ToUnicode`, so the page never needs the program's own `cmap`, and
/// producers (InDesign) leave it out. Every shaper finds a letter's glyph through
/// `cmap`: without one each letter shapes to glyph 0, the empty box, and nothing
/// says so. Measured on a datasheet: words copied in its body font (a
/// cmap-less `Montserrat-Light` subset) pasted as a row of boxes.
///
/// Writes a Windows Unicode subtable — `(3,1)` format 4, plus `(3,10)` format 12
/// when a character is past U+FFFF — and rebuilds the table directory: sorted
/// tags, 4-byte aligned tables, their checksums and `head.checkSumAdjustment`.
/// `None` when the font already has a `cmap`, cannot be read, or nothing in
/// `glyphs` names a glyph it has.
pub fn add_unicode_cmap(font: &[u8], glyphs: &BTreeMap<char, u16>) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(font, 0).ok()?;
    if face.tables().cmap.is_some() {
        return None;
    }
    let count = face.number_of_glyphs();
    let glyphs: Vec<(u32, u16)> =
        glyphs.iter().filter(|(_, &g)| g != 0 && g < count).map(|(&c, &g)| (c as u32, g)).collect();
    if glyphs.is_empty() {
        return None;
    }
    let mut tables = tables_of(font)?;
    tables.push((*b"cmap", unicode_cmap(&glyphs)?));
    Some(sfnt(&font[..4], tables))
}

/// A `cmap` table for `glyphs` (character → glyph, in character order).
fn unicode_cmap(glyphs: &[(u32, u16)]) -> Option<Vec<u8>> {
    // Runs where character and glyph both step by one: a format 4 segment, a
    // format 12 group.
    let mut runs: Vec<(u32, u32, u16)> = Vec::new();
    for &(c, g) in glyphs {
        match runs.last_mut() {
            Some((start, end, first)) if *end + 1 == c && u32::from(*first) + (c - *start) == u32::from(g) => *end = c,
            _ => runs.push((c, c, g)),
        }
    }

    // Format 4: the Basic Multilingual Plane, ending in the 0xFFFF segment the
    // format requires. A run crossing 0xFFFF is cut there.
    let mut segments: Vec<(u16, u16, u16)> = runs
        .iter()
        .filter(|(start, ..)| *start < 0xFFFF)
        .map(|&(start, end, first)| (start as u16, end.min(0xFFFE) as u16, first.wrapping_sub(start as u16)))
        .collect();
    segments.push((0xFFFF, 0xFFFF, 1));
    let n = segments.len();
    let search = 2 * (1usize << n.ilog2());
    let mut format4: Vec<u8> = Vec::new();
    for value in [4, 16 + 8 * n, 0, 2 * n, search, (search / 2).ilog2() as usize, 2 * n - search] {
        format4.extend(u16::try_from(value).ok()?.to_be_bytes());
    }
    let column = |pick: fn(&(u16, u16, u16)) -> u16| segments.iter().flat_map(move |s| pick(s).to_be_bytes());
    format4.extend(column(|s| s.1));
    format4.extend([0, 0]);
    format4.extend(column(|s| s.0));
    format4.extend(column(|s| s.2));
    format4.extend(std::iter::repeat_n(0u8, 2 * n));

    // Format 12, only when a character is past the BMP; it then holds every run.
    let format12 = runs.iter().any(|(_, end, _)| *end > 0xFFFF).then(|| {
        let mut table: Vec<u8> = vec![0, 12, 0, 0];
        for value in [16 + 12 * runs.len(), 0, runs.len()] {
            table.extend((value as u32).to_be_bytes());
        }
        for &(start, end, first) in &runs {
            for value in [start, end, u32::from(first)] {
                table.extend(value.to_be_bytes());
            }
        }
        table
    });

    let subtables: Vec<(u16, Vec<u8>)> = std::iter::once((1, format4)).chain(format12.map(|t| (10, t))).collect();
    let mut out: Vec<u8> = vec![0, 0];
    out.extend((subtables.len() as u16).to_be_bytes());
    let mut offset = 4 + 8 * subtables.len();
    for (encoding, table) in &subtables {
        out.extend(3u16.to_be_bytes());
        out.extend(encoding.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        offset += table.len();
    }
    for (_, table) in subtables {
        out.extend(table);
    }
    Some(out)
}

/// A font's tables, by tag, as they sit in the file. `None` for a collection or
/// a directory pointing outside the data.
fn tables_of(font: &[u8]) -> Option<Vec<([u8; 4], Vec<u8>)>> {
    if font.get(..4)? == b"ttcf" {
        return None;
    }
    let count = u16::from_be_bytes(font.get(4..6)?.try_into().ok()?) as usize;
    (0..count)
        .map(|i| {
            let record = font.get(12 + 16 * i..28 + 16 * i)?;
            let word = |at: usize| u32::from_be_bytes(record[at..at + 4].try_into().expect("4 bytes")) as usize;
            let data = font.get(word(8)..word(8).checked_add(word(12))?)?;
            Some((record[..4].try_into().expect("4 bytes"), data.to_vec()))
        })
        .collect()
}

/// A font file holding `tables`, laid out as the format requires.
fn sfnt(version: &[u8], mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    for (tag, data) in &mut tables {
        // Summed as zero, then set from the whole file below.
        if tag == b"head" && data.len() >= 12 {
            data[8..12].fill(0);
        }
    }
    let n = tables.len();
    let power = if n == 0 { 0 } else { 1usize << n.ilog2() };
    let mut out = version.to_vec();
    for value in [n, power * 16, power.max(1).ilog2() as usize, (n - power) * 16] {
        out.extend((value as u16).to_be_bytes());
    }
    let mut offset = 12 + 16 * n;
    let mut head = None;
    for (tag, data) in &tables {
        out.extend(tag);
        out.extend(checksum(data).to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        if tag == b"head" && data.len() >= 12 {
            head = Some(offset);
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if let Some(at) = head {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

/// The format's table checksum: big-endian words summed, the last zero-padded.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONTSERRAT: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../desktop/third_party/fonts/Montserrat-Regular.ttf");

    /// The font with its `cmap` taken out, as an `/Identity-H` subset is embedded.
    fn without_cmap(font: &[u8]) -> Vec<u8> {
        let tables = tables_of(font).expect("tables").into_iter().filter(|(tag, _)| tag != b"cmap").collect();
        sfnt(&font[..4], tables)
    }

    #[test]
    fn a_font_without_a_cmap_is_given_one_that_finds_the_same_glyphs() {
        let original = std::fs::read(MONTSERRAT).expect("the bundled Montserrat");
        let face = ttf_parser::Face::parse(&original, 0).expect("parses");
        let text = "Reflector Anodized Aluminium";
        let glyphs: BTreeMap<char, u16> =
            text.chars().map(|c| (c, face.glyph_index(c).expect("Montserrat has it").0)).collect();

        let stripped = without_cmap(&original);
        let bare = ttf_parser::Face::parse(&stripped, 0).expect("still a font");
        assert!(bare.tables().cmap.is_none(), "the test font must really have lost its cmap");
        assert_eq!(bare.glyph_index('R'), None);

        let mended = add_unicode_cmap(&stripped, &glyphs).expect("a cmap is added");
        let mended = ttf_parser::Face::parse(&mended, 0).expect("the rewritten font parses");
        for c in text.chars() {
            assert_eq!(mended.glyph_index(c), face.glyph_index(c), "{c:?}");
        }
        assert_eq!(mended.glyph_index('Q'), None, "Q was not in the map");
    }

    #[test]
    fn a_character_past_the_basic_plane_is_found_too() {
        let stripped = without_cmap(&std::fs::read(MONTSERRAT).expect("the bundled Montserrat"));
        let glyphs = BTreeMap::from([('A', 36), ('\u{1D400}', 37), ('\u{1D401}', 38)]);
        let mended = add_unicode_cmap(&stripped, &glyphs).expect("a cmap is added");
        let mended = ttf_parser::Face::parse(&mended, 0).expect("parses");
        assert_eq!(mended.glyph_index('A').map(|g| g.0), Some(36));
        assert_eq!(mended.glyph_index('\u{1D400}').map(|g| g.0), Some(37));
        assert_eq!(mended.glyph_index('\u{1D401}').map(|g| g.0), Some(38));
    }

    #[test]
    fn a_font_that_has_a_cmap_is_left_alone() {
        let original = std::fs::read(MONTSERRAT).expect("the bundled Montserrat");
        assert_eq!(add_unicode_cmap(&original, &BTreeMap::from([('A', 36)])), None);
    }

    #[test]
    fn the_rewritten_file_has_the_checksums_the_format_requires() {
        let original = std::fs::read(MONTSERRAT).expect("the bundled Montserrat");
        let rebuilt = without_cmap(&original);
        // A whole file sums to the magic number once `checkSumAdjustment` is in.
        assert_eq!(checksum(&rebuilt), 0xB1B0_AFBA);
        let count = u16::from_be_bytes([rebuilt[4], rebuilt[5]]) as usize;
        for i in 0..count {
            let record = &rebuilt[12 + 16 * i..28 + 16 * i];
            let word = |at: usize| u32::from_be_bytes(record[at..at + 4].try_into().unwrap());
            let (offset, length) = (word(8) as usize, word(12) as usize);
            assert_eq!(offset % 4, 0);
            let mut data = rebuilt[offset..offset + length].to_vec();
            if &record[..4] == b"head" {
                data[8..12].fill(0);
            }
            assert_eq!(checksum(&data), word(4), "{}", String::from_utf8_lossy(&record[..4]));
        }
    }
}
