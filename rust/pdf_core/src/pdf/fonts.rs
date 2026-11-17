//! Fonts a page draws with, but the file does not carry.
//!
//! # Why
//!
//! A page edited in another program can name a font without embedding its
//! program: `/BaseFont /Montserrat-Light` with a descriptor, widths and an
//! encoding, and no `/FontFile2`. Pagify's own fallback, Helvetica through
//! `FPDFText_LoadStandardFont`, is not embedded either. Every reader then
//! substitutes a face of its own — the 2026-11-16 report: retyped words bold
//! black in WhatsApp and thin grey on the PC that wrote them.
//!
//! # What this does
//!
//! Reads the pages' and their forms' resources, finds each font with no
//! program, asks a **finder** the app supplies for the face by PostScript name,
//! and writes the program into the file: `/FontFile2` on a simple font's
//! descriptor, or `/FontFile3` for an OpenType face with CFF outlines. A base-14
//! `/Type1` dictionary in WinAnsi becomes a `/TrueType` one with the face's own
//! widths.
//!
//! **Left exactly as they were:**
//! - **Type0 (CID) fonts.** Their codes are CIDs, and a CID is a glyph number
//!   only in the one program the producer used; a Chinese or Japanese font
//!   named by its collection (`Adobe-GB1`, `Adobe-Japan1`) numbers characters
//!   by that collection, not by any font's glyphs. Storing this PC's face
//!   under them can draw the wrong characters, and no file in the 2026-11-16
//!   report had one without its program.
//! - **A base-14 font in any encoding but WinAnsi** (StandardEncoding,
//!   `/Differences`): rewriting it as a WinAnsi TrueType font would change what
//!   its codes mean. Pagify's own fallback writes WinAnsi.
//!
//! **Only the glyphs the codes can reach.** A simple font's codes are at most
//! 256, and in WinAnsi (every such font in the 2026-11-16 report) they reach a
//! known set of characters; the program is stored with only those outlines,
//! glyph numbers unchanged (`keep_win_ansi_glyphs`) — about a tenth of a face.
//! Whole programs cost a one-page quotation 320 KB → 1.2 MB. A font in any
//! other encoding, a symbolic one, or a CFF face is stored whole.
//!
//! **Variable fonts are refused.** Their default instance is not a weight this
//! code can choose correctly (Google's Montserrat defaults to Thin, not
//! Regular), so a file found only as a variable font is reported as missing
//! rather than embedded as the wrong weight.
//!
//! Nothing here reads an OS font: `find_face` is the app's, so the engine's
//! tests stay hermetic.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::{content, embed, write_object, write_stream, Dict, File, Object};
use crate::error::{PdfError, Result};

/// What embedding did, and the file it produced.
#[derive(Debug, Default, Clone)]
pub struct FontsEmbedded {
    /// The whole file; byte for byte the input when no font was embedded.
    pub bytes: Vec<u8>,
    /// Each face stored, by the program's own PostScript name, and its size.
    pub stored: Vec<(String, usize)>,
    /// Faces the finder could not supply, or supplied only as a variable font.
    pub missing: Vec<String>,
    /// Simple fonts whose own `/Widths` disagree with the face's by more than
    /// 2% — kept as they were, because they position the text.
    pub width_mismatches: Vec<String>,
}

/// Where a font's program should be written.
#[derive(Clone, Copy, PartialEq)]
enum Program {
    TrueType,
    OpenTypeCff,
}

/// The face to ask the finder for, or that this one is deliberately left alone.
enum Face {
    Leave,
    Find(String),
}

/// Fill in a font program for every font a page uses that has none.
pub fn embed_missing(bytes: &[u8], find_face: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<FontsEmbedded> {
    let file = File::parse(bytes)?;
    let mut fonts: BTreeMap<u32, Dict> = BTreeMap::new();
    collect_page_fonts(&file, &mut fonts)?;
    if fonts.is_empty() {
        return Ok(FontsEmbedded { bytes: bytes.to_vec(), ..Default::default() });
    }

    let mut next = file.next_object_number()?;
    let mut replacements: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut extra: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut stored = Vec::new();
    let mut missing = Vec::new();
    let mut width_mismatches = Vec::new();
    // One program object per face, however many pages name it. A catalogue
    // names the same face in a font object on every page, and writing the
    // program once per object multiplied a 6 MB file by four (measured on
    // SUHAIL, 2026-11-16).
    let mut programs: HashMap<(String, String, bool), u32> = HashMap::new();

    for (number, font) in fonts {
        let subtype = font.get(b"Subtype").and_then(Object::as_name).map(|n| n.to_vec());
        // Type3 draws with procedures of its own, not a font program; Type0 is
        // left alone — see the module's doc.
        if matches!(subtype.as_deref(), Some(b"Type3") | Some(b"Type0")) {
            continue;
        }
        let Some(base) = font.get(b"BaseFont").and_then(Object::as_name) else { continue };
        let shown = String::from_utf8_lossy(strip_subset(base)).to_string();
        let Face::Find(wanted) = face_to_find(strip_subset(base)) else { continue };
        let win_ansi = font.get(b"Encoding").map(|e| file.resolve(e)).transpose()?
            == Some(Object::Name(b"WinAnsiEncoding".to_vec()));
        if is_base14(strip_subset(base)) && !win_ansi {
            continue;
        }

        if has_program(&file, &font)? {
            continue;
        }

        let Some(program) = find_face(&wanted) else {
            push_unique(&mut missing, shown);
            continue;
        };
        if is_variable(&program) {
            // The default instance is not this face's weight. Refused, not
            // embedded — see the module's doc.
            push_unique(&mut missing, shown);
            continue;
        }
        let Some(program_kind) = program_kind(&program) else {
            push_unique(&mut missing, shown);
            continue;
        };
        if embed::metrics(&program).is_none() {
            push_unique(&mut missing, shown);
            continue;
        }

        let program_name = embed::face_name(&program).unwrap_or_else(|| wanted.clone());
        // Only the glyphs the codes can reach, where that is known: a
        // nonsymbolic font in WinAnsi. Anything else is stored whole.
        let symbolic = descriptor_of(&file, &font)?
            .0
            .get(b"Flags")
            .and_then(Object::as_i64)
            .is_some_and(|flags| flags & 4 != 0);
        let kept = if win_ansi && !symbolic && program_kind == Program::TrueType {
            keep_win_ansi_glyphs(&program)
        } else {
            None
        };
        let key = (wanted.clone(), embed::program_hash(&program), kept.is_some());
        let program_number = match programs.get(&key) {
            Some(number) => *number,
            None => {
                let written = kept.as_deref().unwrap_or(&program);
                let (number, body) = font_program(next, written, program_kind)?;
                next += 1;
                extra.push((number, body));
                programs.insert(key, number);
                stored.push((program_name.clone(), written.len()));
                number
            }
        };
        let program_ref = Object::Reference(program_number, 0);

        if subtype.as_deref() == Some(b"Type1") || subtype.as_deref() == Some(b"MMType1") {
            let base14 = is_base14(strip_subset(base));
            let (descriptor_number, descriptor_body) =
                embed_descriptor(next, &program, &program_name, &program_ref, program_kind)?;
            next += 1;
            extra.push((descriptor_number, descriptor_body));

            // A base-14 face has no widths or encoding of its own to keep: the
            // dictionary is built from the face, exactly as a new font is. Any
            // other Type1 font keeps its codes' widths and encoding, which
            // position its text, and only the program and subtype change.
            let mut dict = if base14 {
                Dict(Vec::new())
            } else {
                if widths_differ(&file, &font, &program) {
                    push_unique(&mut width_mismatches, shown.clone());
                }
                font.clone()
            };
            dict.set(b"Type", Object::Name(b"Font".to_vec()));
            dict.set(b"Subtype", Object::Name(b"TrueType".to_vec()));
            dict.set(b"BaseFont", Object::Name(sanitise(&program_name).into_bytes()));
            if base14 || dict.get(b"Widths").is_none() {
                dict.set(b"FirstChar", Object::Number(b"32".to_vec()));
                dict.set(b"LastChar", Object::Number(b"255".to_vec()));
                dict.set(
                    b"Widths",
                    Object::Array(
                        embed::win_ansi_widths(&program)
                            .unwrap_or_else(|| vec![0; 224])
                            .into_iter()
                            .map(|w| Object::Number(w.to_string().into_bytes()))
                            .collect(),
                    ),
                );
                dict.set(b"Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
            }
            dict.set(b"FontDescriptor", Object::Reference(descriptor_number, 0));
            let mut body = Vec::new();
            write_object(&mut body, &Object::Dict(dict));
            replacements.push((number, body));
            continue;
        }

        // A simple TrueType font: keep its dictionary, names and widths, and
        // give its descriptor the program.
        let (mut descriptor, descriptor_number) = descriptor_of(&file, &font)?;
        let descriptor_number = descriptor_number
            .filter(|n| !replacements.iter().any(|(m, _)| m == n));
        let file_key: &[u8] = match program_kind {
            Program::TrueType => b"FontFile2",
            Program::OpenTypeCff => b"FontFile3",
        };
        descriptor.set(file_key, program_ref);
        if descriptor.get(b"FontName").is_none() {
            descriptor.set(b"FontName", Object::Name(sanitise(&program_name).into_bytes()));
        }
        if widths_differ(&file, &font, &program) {
            push_unique(&mut width_mismatches, shown.clone());
        }
        let mut font = font;
        if let Some(n) = descriptor_number {
            let mut body = Vec::new();
            write_object(&mut body, &Object::Dict(descriptor));
            replacements.push((n, body));
        } else {
            font.set(b"FontDescriptor", Object::Dict(descriptor));
            let mut body = Vec::new();
            write_object(&mut body, &Object::Dict(font));
            replacements.push((number, body));
        }
    }

    if replacements.is_empty() && extra.is_empty() {
        return Ok(FontsEmbedded { bytes: bytes.to_vec(), stored, missing, width_mismatches });
    }
    let bytes = file.rewrite_adding(&replacements, &extra, &Dict::default())?;
    Ok(FontsEmbedded { bytes, stored, missing, width_mismatches })
}

/// Add a name once, however many font objects name it. A catalogue has one
/// font object per page, and the report is about the face, not the object.
fn push_unique(list: &mut Vec<String>, name: String) {
    if !list.contains(&name) {
        list.push(name);
    }
}

/// A font program's object: the stream, and its `/Length1`.
fn font_program(number: u32, program: &[u8], kind: Program) -> Result<(u32, Vec<u8>)> {
    let packed = content::encode(program)?;
    let mut dict = Dict(Vec::new());
    dict.set(b"Length1", Object::Number(program.len().to_string().into_bytes()));
    dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
    if kind == Program::OpenTypeCff {
        dict.set(b"Subtype", Object::Name(b"OpenType".to_vec()));
    }
    Ok((number, write_stream(&dict, &packed)))
}

/// A descriptor built for a face, for the base-14 replacement.
fn embed_descriptor(
    number: u32,
    program: &[u8],
    name: &str,
    file_ref: &Object,
    kind: Program,
) -> Result<(u32, Vec<u8>)> {
    let metrics = embed::metrics(program)
        .ok_or_else(|| PdfError::InvalidArgument("that font declares no usable metrics".into()))?;
    let mut descriptor = Dict(Vec::new());
    descriptor.set(b"Type", Object::Name(b"FontDescriptor".to_vec()));
    descriptor.set(b"FontName", Object::Name(sanitise(name).into_bytes()));
    // Bit 6, nonsymbolic; bit 19 where the face says it is bold.
    let flags = 32 | if metrics.bold { 1 << 18 } else { 0 };
    descriptor.set(b"Flags", Object::Number(flags.to_string().into_bytes()));
    descriptor.set(
        b"FontBBox",
        Object::Array(
            [metrics.bbox.0, metrics.bbox.1, metrics.bbox.2, metrics.bbox.3]
                .iter()
                .map(|n| Object::Number(n.to_string().into_bytes()))
                .collect(),
        ),
    );
    descriptor.set(
        b"ItalicAngle",
        Object::Number(format!("{:.1}", metrics.italic_angle).into_bytes()),
    );
    descriptor.set(b"Ascent", Object::Number(metrics.ascent.to_string().into_bytes()));
    descriptor.set(b"Descent", Object::Number(metrics.descent.to_string().into_bytes()));
    descriptor.set(b"CapHeight", Object::Number(metrics.cap_height.to_string().into_bytes()));
    descriptor.set(b"StemV", Object::Number(b"80".to_vec()));
    let key: &[u8] = match kind {
        Program::TrueType => b"FontFile2",
        Program::OpenTypeCff => b"FontFile3",
    };
    descriptor.set(key, file_ref.clone());
    let mut body = Vec::new();
    write_object(&mut body, &Object::Dict(descriptor));
    Ok((number, body))
}

/// A font's descriptor, and the object to write it back to (when it is its own
/// object; a direct descriptor is written with its font instead).
fn descriptor_of(file: &File<'_>, font: &Dict) -> Result<(Dict, Option<u32>)> {
    match font.get(b"FontDescriptor") {
        Some(Object::Reference(number, _)) => match file.object(*number) {
            Ok(Object::Dict(dict)) => Ok((dict, Some(*number))),
            _ => Ok((Dict(Vec::new()), None)),
        },
        Some(Object::Dict(dict)) => Ok((dict.clone(), None)),
        _ => Ok((Dict(Vec::new()), None)),
    }
}

/// Whether this simple font carries a program already.
fn has_program(file: &File<'_>, font: &Dict) -> Result<bool> {
    let descriptor = descriptor_of(file, font)?.0;
    Ok([b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
        .iter()
        .any(|key| descriptor.get(*key).is_some()))
}

/// The face to ask the finder for: the `/BaseFont` name without its subset
/// prefix, with the metric twin for a base-14 name.
fn face_to_find(base: &[u8]) -> Face {
    if base == b"Symbol" || base == b"ZapfDingbats" {
        return Face::Leave;
    }
    let wanted = match base {
        // The base-14 Type1 names Windows' metric-compatible faces answer to.
        b"Helvetica" => "ArialMT",
        b"Helvetica-Bold" => "Arial-BoldMT",
        b"Helvetica-Oblique" => "Arial-ItalicMT",
        b"Helvetica-BoldOblique" => "Arial-BoldItalicMT",
        b"Times-Roman" => "TimesNewRomanPSMT",
        b"Times-Bold" => "TimesNewRomanPS-BoldMT",
        b"Times-Italic" => "TimesNewRomanPS-ItalicMT",
        b"Times-BoldItalic" => "TimesNewRomanPS-BoldItalicMT",
        b"Courier" => "CourierNewPSMT",
        b"Courier-Bold" => "CourierNewPS-BoldMT",
        b"Courier-Oblique" => "CourierNewPS-ItalicMT",
        b"Courier-BoldOblique" => "CourierNewPS-BoldItalicMT",
        // Windows' own spellings of the same families.
        b"Arial" => "ArialMT",
        b"Arial,Bold" => "Arial-BoldMT",
        b"Arial,Italic" => "Arial-ItalicMT",
        b"Arial,BoldItalic" => "Arial-BoldItalicMT",
        b"TimesNewRoman" => "TimesNewRomanPSMT",
        b"TimesNewRoman,Bold" => "TimesNewRomanPS-BoldMT",
        b"TimesNewRoman,Italic" => "TimesNewRomanPS-ItalicMT",
        b"TimesNewRoman,BoldItalic" => "TimesNewRomanPS-BoldItalicMT",
        b"CourierNew" => "CourierNewPSMT",
        b"CourierNew,Bold" => "CourierNewPS-BoldMT",
        b"CourierNew,Italic" => "CourierNewPS-ItalicMT",
        b"CourierNew,BoldItalic" => "CourierNewPS-BoldItalicMT",
        other => return match std::str::from_utf8(other) {
            Ok(name) => Face::Find(name.to_string()),
            Err(_) => Face::Leave,
        },
    };
    Face::Find(wanted.to_string())
}

fn is_base14(base: &[u8]) -> bool {
    matches!(
        base,
        b"Helvetica"
            | b"Helvetica-Bold"
            | b"Helvetica-Oblique"
            | b"Helvetica-BoldOblique"
            | b"Times-Roman"
            | b"Times-Bold"
            | b"Times-Italic"
            | b"Times-BoldItalic"
            | b"Courier"
            | b"Courier-Bold"
            | b"Courier-Oblique"
            | b"Courier-BoldOblique"
    )
}

/// `/BaseFont` without an `ABCDEF+` subset prefix.
fn strip_subset(name: &[u8]) -> &[u8] {
    if name.len() > 7
        && name[6] == b'+'
        && name[..6].iter().all(|b| b.is_ascii_uppercase())
    {
        &name[7..]
    } else {
        name
    }
}

/// A PostScript name as a PDF `/Name`: no spaces, no punctuation a name cannot
/// hold.
fn sanitise(name: &str) -> String {
    let cleaned: String =
        name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '+').collect();
    if cleaned.is_empty() { "PagifyEmbedded".into() } else { cleaned }
}

/// Which program a face's bytes are, or `None` for something this cannot write.
fn program_kind(program: &[u8]) -> Option<Program> {
    match program.get(..4)? {
        b"OTTO" => Some(Program::OpenTypeCff),
        [0x00, 0x01, 0x00, 0x00] | b"true" | b"typ1" => Some(Program::TrueType),
        _ => None,
    }
}

/// Whether a face declares variation axes — a variable font, whose default
/// instance is not a weight that can be chosen safely.
fn is_variable(program: &[u8]) -> bool {
    if program.len() < 12 {
        return false;
    }
    let count = usize::from(u16::from_be_bytes([program[4], program[5]]));
    let mut at = 12;
    for _ in 0..count {
        if at + 16 > program.len() {
            return false;
        }
        if &program[at..at + 4] == b"fvar" {
            return true;
        }
        at += 16;
    }
    false
}

/// Whether the file's own `/Widths` disagree with the face's by more than 2%.
fn widths_differ(file: &File<'_>, font: &Dict, program: &[u8]) -> bool {
    let Some(widths) = font.get(b"Widths") else { return false };
    let Ok(Object::Array(items)) = file.resolve(widths) else { return false };
    let first = font.get(b"FirstChar").and_then(Object::as_i64).unwrap_or(0).max(0) as usize;
    let Some(face) = embed::win_ansi_widths(program) else { return false };
    items.iter().enumerate().any(|(index, item)| {
        let code = first + index;
        let (Some(was), Some(&now)) = (item.as_f64(), face.get(code.wrapping_sub(32))) else {
            return false;
        };
        // A producer writes 0 for a code it never draws: not a disagreement.
        if was == 0.0 {
            return false;
        }
        let now = f64::from(now);
        let scale = was.abs().max(now.abs());
        scale > 0.0 && (was - now).abs() / scale > 0.02
    })
}

// ------------------------------------------------------------------ subset --

/// The tables a reader draws a simple TrueType font with. The rest — layout
/// (`GSUB`, `GPOS`, `kern`), bitmaps, `DSIG` — shapes or positions text the
/// page has already positioned, and is left out.
const KEPT_TABLES: [&[u8; 4]; 14] = [
    b"OS/2", b"cmap", b"cvt ", b"fpgm", b"gasp", b"glyf", b"head", b"hhea", b"hmtx", b"loca", b"maxp",
    b"name", b"post", b"prep",
];

/// `program` with only the outlines a WinAnsi font's codes can reach: glyph 0,
/// the glyph `cmap` gives each WinAnsi character, and the parts of any
/// composite among them (an accented letter draws its letter and its accent).
///
/// **Glyph numbers do not move.** Every other glyph stays, as an empty
/// outline, so `cmap`, `hmtx` and the composites' references stay exactly
/// right and nothing is renumbered — the risk a renumbering subsetter carries
/// for codes already on the page. `post` becomes version 3 (no glyph names):
/// a simple font's codes reach glyphs through `cmap`.
///
/// `None` — store the face whole — when the program is not one this can cut
/// (no `glyf`, no Unicode `cmap`) or the cut does not read back as the same font.
fn keep_win_ansi_glyphs(program: &[u8]) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(program, 0).ok()?;
    face.glyph_index('A')?;
    let tables = table_records(program)?;
    let table = |tag: &[u8; 4]| tables.iter().find(|(t, _)| t == tag).map(|(_, data)| *data);
    let (head, loca, glyf, maxp) = (table(b"head")?, table(b"loca")?, table(b"glyf")?, table(b"maxp")?);
    let glyphs = usize::from(u16::from_be_bytes([*maxp.get(4)?, *maxp.get(5)?]));
    let long = i16::from_be_bytes([*head.get(50)?, *head.get(51)?]) == 1;
    let offset = |glyph: usize| -> Option<usize> {
        if long {
            Some(u32::from_be_bytes(loca.get(glyph * 4..glyph * 4 + 4)?.try_into().ok()?) as usize)
        } else {
            Some(usize::from(u16::from_be_bytes(loca.get(glyph * 2..glyph * 2 + 2)?.try_into().ok()?)) * 2)
        }
    };
    let outline = |glyph: usize| -> Option<&[u8]> { glyf.get(offset(glyph)?..offset(glyph + 1)?) };

    let mut keep: std::collections::BTreeSet<usize> = std::iter::once(0)
        .chain(
            (32u8..=255)
                .filter_map(embed::win_ansi_char)
                .filter_map(|c| face.glyph_index(c))
                .map(|glyph| usize::from(glyph.0)),
        )
        .collect();
    let mut queue: Vec<usize> = keep.iter().copied().collect();
    while let Some(glyph) = queue.pop() {
        for part in components(outline(glyph)?) {
            if part < glyphs && keep.insert(part) {
                queue.push(part);
            }
        }
    }

    let mut new_glyf = Vec::new();
    let mut new_loca = Vec::with_capacity((glyphs + 1) * 4);
    for glyph in 0..glyphs {
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        if keep.contains(&glyph) {
            new_glyf.extend_from_slice(outline(glyph)?);
            new_glyf.resize(new_glyf.len().next_multiple_of(4), 0);
        }
    }
    new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
    let mut new_head = head.to_vec();
    new_head.get_mut(8..12)?.fill(0); // checkSumAdjustment, set once the whole font is written
    new_head.get_mut(50..52)?.copy_from_slice(&1i16.to_be_bytes()); // long loca

    let mut kept = Vec::new();
    for (tag, data) in &tables {
        let data = match tag {
            b"glyf" => std::mem::take(&mut new_glyf),
            b"loca" => std::mem::take(&mut new_loca),
            b"head" => std::mem::take(&mut new_head),
            b"post" => {
                let mut post = data.get(..32)?.to_vec();
                post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
                post
            }
            tag if KEPT_TABLES.contains(&tag) => data.to_vec(),
            _ => continue,
        };
        kept.push((*tag, data));
    }
    let cut = write_sfnt(program.get(..4)?, kept);
    let check = ttf_parser::Face::parse(&cut, 0).ok()?;
    (check.number_of_glyphs() == face.number_of_glyphs() && cut.len() < program.len()).then_some(cut)
}

/// A font file's tables, by tag, as slices of it. `None` for anything that is
/// not one font's table directory (a collection, a truncated file).
fn table_records(program: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let count = usize::from(u16::from_be_bytes([*program.get(4)?, *program.get(5)?]));
    (0..count)
        .map(|i| {
            let record = program.get(12 + 16 * i..28 + 16 * i)?;
            let offset = u32::from_be_bytes(record[8..12].try_into().ok()?) as usize;
            let length = u32::from_be_bytes(record[12..16].try_into().ok()?) as usize;
            Some((record[..4].try_into().ok()?, program.get(offset..offset.checked_add(length)?)?))
        })
        .collect()
}

/// The glyphs a composite glyph draws; none for a simple glyph.
fn components(outline: &[u8]) -> Vec<usize> {
    let mut parts = Vec::new();
    if outline.len() < 10 || i16::from_be_bytes([outline[0], outline[1]]) >= 0 {
        return parts;
    }
    let mut at = 10;
    while let Some(record) = outline.get(at..at + 4) {
        let flags = u16::from_be_bytes([record[0], record[1]]);
        parts.push(usize::from(u16::from_be_bytes([record[2], record[3]])));
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 }; // ARG_1_AND_2_ARE_WORDS
        at += if flags & 0x0008 != 0 {
            2 // WE_HAVE_A_SCALE
        } else if flags & 0x0040 != 0 {
            4 // WE_HAVE_AN_X_AND_Y_SCALE
        } else if flags & 0x0080 != 0 {
            8 // WE_HAVE_A_TWO_BY_TWO
        } else {
            0
        };
        if flags & 0x0020 == 0 {
            break; // no MORE_COMPONENTS
        }
    }
    parts
}

/// A font file from its tables: directory sorted by tag, every table 4-byte
/// aligned with its checksum, and `head`'s checksum adjustment set.
fn write_sfnt(version: &[u8], mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    let count = tables.len() as u16;
    let power = 1u16 << (15 - count.max(1).leading_zeros());
    let mut out = version.to_vec();
    for value in [count, power * 16, power.trailing_zeros() as u16, count * 16 - power * 16] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    let start = 12 + 16 * tables.len();
    let mut body = Vec::new();
    let mut head_at = None;
    for (tag, data) in &tables {
        if tag == b"head" {
            head_at = Some(start + body.len());
        }
        out.extend_from_slice(tag);
        out.extend_from_slice(&checksum(data).to_be_bytes());
        out.extend_from_slice(&((start + body.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        body.resize(body.len().next_multiple_of(4), 0);
    }
    out.extend_from_slice(&body);
    if let Some(at) = head_at {
        let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjust.to_be_bytes());
    }
    out
}

/// The OpenType table checksum: the sum of its big-endian 32-bit words.
fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

// ---------------------------------------------------------------- page walk --

/// Every font any page uses: its resources, its forms' resources, and the
/// appearances of the annotations that stay.
fn collect_page_fonts(file: &File<'_>, fonts: &mut BTreeMap<u32, Dict>) -> Result<()> {
    let root = file
        .resolve(file.trailer().get(b"Root").ok_or_else(|| {
            PdfError::InvalidArgument("the file has no catalogue".into())
        })?)?;
    let pages = root
        .as_dict()
        .and_then(|d| d.get(b"Pages"))
        .cloned()
        .ok_or_else(|| PdfError::InvalidArgument("the file has no page tree".into()))?;
    let mut pages_seen = HashSet::new();
    let mut forms_seen = HashSet::new();
    collect_pages(file, &pages, None, 0, &mut pages_seen, &mut forms_seen, fonts)
}

#[allow(clippy::too_many_arguments)]
fn collect_pages(
    file: &File<'_>,
    node: &Object,
    inherited: Option<Object>,
    depth: usize,
    seen: &mut HashSet<u32>,
    forms: &mut HashSet<u32>,
    fonts: &mut BTreeMap<u32, Dict>,
) -> Result<()> {
    let Some((number, _)) = node.as_reference() else { return Ok(()) };
    if depth > 64 || !seen.insert(number) {
        return Err(PdfError::InvalidArgument("the page tree loops or is too deep".into()));
    }
    let Object::Dict(dict) = file.object(number)? else { return Ok(()) };
    let resources = dict.get(b"Resources").cloned().or(inherited);
    match dict.get(b"Kids") {
        Some(Object::Array(kids)) => {
            for kid in kids {
                collect_pages(file, kid, resources.clone(), depth + 1, seen, forms, fonts)?;
            }
        }
        _ => {
            if let Some(resources) = &resources {
                collect_resources(file, resources, 0, forms, fonts)?;
            }
            if let Some(annots) = dict.get(b"Annots") {
                collect_annotation_fonts(file, annots, 0, forms, fonts)?;
            }
        }
    }
    Ok(())
}

fn collect_resources(
    file: &File<'_>,
    resources: &Object,
    depth: usize,
    forms: &mut HashSet<u32>,
    fonts: &mut BTreeMap<u32, Dict>,
) -> Result<()> {
    if depth > 32 {
        return Ok(());
    }
    let Ok(Object::Dict(resources)) = file.resolve(resources) else { return Ok(()) };
    if let Some(font) = resources.get(b"Font") {
        if let Ok(Object::Dict(dicts)) = file.resolve(font) {
            for (_, value) in dicts.0 {
                let Some((number, _)) = value.as_reference() else { continue };
                if let Ok(Object::Dict(dict)) = file.object(number) {
                    fonts.entry(number).or_insert(dict);
                }
            }
        }
    }
    if let Some(xobjects) = resources.get(b"XObject") {
        if let Ok(Object::Dict(dicts)) = file.resolve(xobjects) {
            for (_, value) in dicts.0 {
                let Some((number, _)) = value.as_reference() else { continue };
                if !forms.insert(number) {
                    continue;
                }
                let Ok(Object::Stream(dict, _)) = file.object(number) else { continue };
                if dict.get(b"Subtype").and_then(Object::as_name) != Some(b"Form") {
                    continue;
                }
                if let Some(inner) = dict.get(b"Resources") {
                    collect_resources(file, inner, depth + 1, forms, fonts)?;
                }
            }
        }
    }
    Ok(())
}

fn collect_annotation_fonts(
    file: &File<'_>,
    annots: &Object,
    depth: usize,
    forms: &mut HashSet<u32>,
    fonts: &mut BTreeMap<u32, Dict>,
) -> Result<()> {
    if depth > 32 {
        return Ok(());
    }
    let Ok(Object::Array(items)) = file.resolve(annots) else { return Ok(()) };
    for item in items {
        let Ok(Object::Dict(annot)) = file.resolve(&item) else { continue };
        let Some(appearance) = annot.get(b"AP") else { continue };
        let Ok(Object::Dict(appearance)) = file.resolve(appearance) else { continue };
        let Some(normal) = appearance.get(b"N") else { continue };
        match file.resolve(normal)? {
            Object::Stream(dict, _) => {
                if let Some(resources) = dict.get(b"Resources") {
                    collect_resources(file, resources, depth + 1, forms, fonts)?;
                }
            }
            // A dictionary of states, of which every one an appearance.
            Object::Dict(states) => {
                for (_, value) in states.0 {
                    let Ok(Object::Stream(dict, _)) = file.resolve(&value) else { continue };
                    if let Some(resources) = dict.get(b"Resources") {
                        collect_resources(file, resources, depth + 1, forms, fonts)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::File;

    /// A classic-xref PDF whose object `n` is `objects[n - 1]`.
    fn build(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
        );
        out
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nendstream");
        out
    }

    /// One page whose resources name `/F1` as object 5, in Helvetica with a
    /// descriptor but no program.
    fn helvetica_without_program() -> Vec<u8> {
        build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td (Hello) Tj ET"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        ])
    }

    /// **The break:** a Chinese font named by its character collection, with no
    /// program. Its codes are Adobe-GB1 CIDs, not glyph numbers in any face
    /// this PC has; storing one under it would draw the wrong characters.
    #[test]
    fn a_cid_font_without_a_program_is_left_exactly_as_it_was() {
        let input = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td <4E2D6587> Tj ET"),
            b"<< /Type /Font /Subtype /Type0 /BaseFont /SimSun /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>".to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SimSun /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> /FontDescriptor 7 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /SimSun /Flags 6 >>".to_vec(),
        ]);
        let program = fixture_font();
        let out = embed_missing(&input, &|_| Some(program.clone())).expect("embeds");
        assert_eq!((out.stored.len(), out.missing.len()), (0, 0), "{:?} {:?}", out.stored, out.missing);
        assert_eq!(out.bytes, input, "the CID font was changed");
    }

    /// **The break:** a base-14 font in StandardEncoding, or with its own
    /// `/Differences`, rewritten as a WinAnsi TrueType font — its codes would
    /// then mean other characters.
    #[test]
    fn a_base14_font_in_another_encoding_is_left_exactly_as_it_was() {
        for encoding in ["", "/Encoding << /Differences [39 /quoteright 96 /quoteleft] >>"] {
            let font = format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {encoding} >>");
            let input = build(&[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
                stream("", b"BT /F1 24 Tf 72 700 Td (It's) Tj ET"),
                font.into_bytes(),
            ]);
            let out = embed_missing(&input, &finder(fixture_font())).expect("embeds");
            assert!(out.stored.is_empty(), "Helvetica {encoding:?} was rewritten");
            assert_eq!(out.bytes, input, "Helvetica {encoding:?} was changed");
        }
    }

    /// A fake face finder that records what it was asked and answers with a
    /// small, valid TrueType program.
    fn finder(program: Vec<u8>) -> impl Fn(&str) -> Option<Vec<u8>> {
        move |name| (name == "ArialMT").then(|| program.clone())
    }

    #[test]
    fn a_base14_helvetica_becomes_a_truetype_font_with_a_program() {
        let input = helvetica_without_program();
        let out = embed_missing(&input, &finder(fixture_font())).expect("embeds");
        assert_eq!(out.stored.len(), 1, "{:?}", out.missing);
        let file = File::parse(&out.bytes).expect("a readable file");
        let Object::Dict(font) = file.object(5).expect("font") else { panic!("not a dict") };
        assert_eq!(font.get(b"Subtype"), Some(&Object::Name(b"TrueType".to_vec())));
        assert_eq!(font.get(b"Encoding"), Some(&Object::Name(b"WinAnsiEncoding".to_vec())));
        let Some(Object::Reference(descriptor_number, _)) = font.get(b"FontDescriptor") else {
            panic!("no descriptor")
        };
        let Object::Dict(descriptor) = file.object(*descriptor_number).expect("descriptor") else {
            panic!("not a dict")
        };
        assert!(descriptor.get(b"FontFile2").is_some(), "no program on the descriptor");
    }

    #[test]
    fn a_font_with_a_program_already_is_left_alone() {
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td (Hello) Tj ET"),
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Carried /FirstChar 32 /LastChar 255 /Widths [250] /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Carried /FontFile2 7 0 R >>".to_vec(),
            stream("", b"not really a font"),
        ];
        objects[6] = stream("", b"program");
        let input = build(&objects);
        let out = embed_missing(&input, &|_| panic!("the finder must not be asked")).expect("embeds");
        assert!(out.stored.is_empty());
        assert_eq!(out.bytes, input);
    }

    /// A simple `/TrueType` with its own `/Widths` and a descriptor, and no
    /// program — the shape the datasheet's `Montserrat-*` fonts have.
    fn montserrat_without_program() -> Vec<u8> {
        build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td (HS90LC) Tj ET"),
            b"<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Montserrat-Regular /FirstChar 32 /LastChar 34 /Widths [250 300 350] /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Montserrat-Regular >>".to_vec(),
        ])
    }

    #[test]
    fn a_simple_font_with_no_program_gets_one_and_keeps_its_widths() {
        let input = montserrat_without_program();
        let program = fixture_font();
        let out = embed_missing(&input, &|name| (name == "Montserrat-Regular").then(|| program.clone()))
            .expect("embeds");
        assert_eq!(out.stored.len(), 1, "{:?}", out.missing);
        assert_eq!(out.stored[0].0, "Montserrat-Regular");
        assert_eq!(out.width_mismatches, vec!["Montserrat-Regular".to_string()]);

        let file = File::parse(&out.bytes).expect("a readable file");
        let Object::Dict(font) = file.object(5).expect("font") else { panic!("not a dict") };
        assert_eq!(font.get(b"BaseFont"), Some(&Object::Name(b"ABCDEF+Montserrat-Regular".to_vec())));
        assert_eq!(
            font.get(b"Widths"),
            Some(&Object::Array(vec![
                Object::Number(b"250".to_vec()),
                Object::Number(b"300".to_vec()),
                Object::Number(b"350".to_vec()),
            ])),
            "the file's widths position its text and must stay"
        );
        let Object::Dict(descriptor) = file.object(6).expect("descriptor") else { panic!("not a dict") };
        assert!(descriptor.get(b"FontFile2").is_some(), "no program on the descriptor");
    }

    /// A `/Type1` font that is not base-14 — a CAD drawing's Swiss or Arial
    /// Narrow. It keeps its widths and encoding; only its program and subtype
    /// change, so its text does not move.
    #[test]
    fn a_named_type1_font_becomes_truetype_and_keeps_its_codes() {
        let input = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td (AB) Tj ET"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Montserrat-Regular /FirstChar 32 /LastChar 33 /Widths [200 300] /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Montserrat-Regular >>".to_vec(),
        ]);
        let program = fixture_font();
        let out = embed_missing(&input, &|name| (name == "Montserrat-Regular").then(|| program.clone()))
            .expect("embeds");
        assert_eq!(out.stored.len(), 1, "{:?}", out.missing);

        let file = File::parse(&out.bytes).expect("a readable file");
        let Object::Dict(font) = file.object(5).expect("font") else { panic!("not a dict") };
        assert_eq!(font.get(b"Subtype"), Some(&Object::Name(b"TrueType".to_vec())));
        assert_eq!(
            font.get(b"Widths"),
            Some(&Object::Array(vec![Object::Number(b"200".to_vec()), Object::Number(b"300".to_vec())])),
            "the codes' widths position the text"
        );
        assert_eq!(font.get(b"Encoding"), Some(&Object::Name(b"WinAnsiEncoding".to_vec())));
        let Object::Reference(descriptor_number, _) = font.get(b"FontDescriptor").expect("descriptor") else {
            panic!("no descriptor")
        };
        let Object::Dict(descriptor) = file.object(*descriptor_number).expect("descriptor") else {
            panic!("not a dict")
        };
        assert!(descriptor.get(b"FontFile2").is_some(), "no program on the descriptor");
    }

    #[test]
    fn a_face_the_finder_does_not_have_is_reported_by_name() {
        let input = helvetica_without_program();
        let out = embed_missing(&input, &|_| None).expect("embeds");
        assert_eq!(out.missing, vec!["Helvetica".to_string()]);
        assert_eq!(out.bytes, input);
    }

    #[test]
    fn a_variable_font_is_refused_not_embedded() {
        let program = fixture_font();
        let mut variable = program.clone();
        // Rename a table tag to `fvar` and fix the table's checksum position
        // loosely: only the tag matters to the check.
        let at = table_directory(&variable).expect("a table directory");
        variable[at..at + 4].copy_from_slice(b"fvar");
        let input = helvetica_without_program();
        let out = embed_missing(&input, &finder(variable)).expect("embeds");
        assert!(out.stored.is_empty(), "a variable font was embedded");
        assert_eq!(out.missing, vec!["Helvetica".to_string()]);
    }

    /// The program a font's descriptor carries, decoded.
    fn stored_program(file: &File<'_>, font: u32) -> Vec<u8> {
        let Object::Dict(font) = file.object(font).expect("font") else { panic!("not a dict") };
        let descriptor = file.resolve(font.get(b"FontDescriptor").expect("a descriptor")).expect("descriptor");
        let Object::Dict(descriptor) = descriptor else { panic!("not a dict") };
        let Some(Object::Reference(number, _)) = descriptor.get(b"FontFile2") else { panic!("no program") };
        let Object::Stream(dict, range) = file.object(*number).expect("program") else { panic!("not a stream") };
        content::decode(&dict, &file.bytes()[range]).expect("decodes")
    }

    /// **The break:** the whole face stored for a font whose codes reach only
    /// WinAnsi's characters — a one-page quotation grew from 320 KB to 1.2 MB.
    /// What is stored keeps every glyph those codes reach, at the same glyph
    /// number with the same outline and width, and no other outline.
    #[test]
    fn a_win_ansi_font_is_stored_with_only_the_glyphs_its_codes_reach() {
        let program = fixture_font();
        let out = embed_missing(&helvetica_without_program(), &finder(program.clone())).expect("embeds");
        let stored = stored_program(&File::parse(&out.bytes).expect("a readable file"), 5);
        assert!(stored.len() * 3 < program.len(), "stored {} bytes of {}", stored.len(), program.len());
        assert_eq!(out.stored[0].1, stored.len(), "the report gives the size stored");

        let whole = ttf_parser::Face::parse(&program, 0).expect("the face");
        let cut = ttf_parser::Face::parse(&stored, 0).expect("the stored program is a font");
        assert_eq!(cut.number_of_glyphs(), whole.number_of_glyphs(), "glyph numbers must not move");
        for c in "AZaz09 .,é€–’ÿÆ".chars() {
            let glyph = whole.glyph_index(c).expect("in the face");
            assert_eq!(cut.glyph_index(c), Some(glyph), "{c} maps elsewhere");
            assert_eq!(cut.glyph_bounding_box(glyph), whole.glyph_bounding_box(glyph), "{c} lost its outline");
            assert_eq!(cut.glyph_hor_advance(glyph), whole.glyph_hor_advance(glyph), "{c} changed width");
        }
        let beyond = whole.glyph_index('Ж').expect("the fixture draws Cyrillic");
        assert!(whole.glyph_bounding_box(beyond).is_some());
        assert_eq!(cut.glyph_bounding_box(beyond), None, "a glyph no code reaches was kept");
    }

    /// A font in another encoding is stored whole: which glyphs its codes
    /// reach is not worked out, so nothing is cut that a code might need.
    #[test]
    fn a_font_in_another_encoding_is_stored_whole() {
        let input = build(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream("", b"BT /F1 24 Tf 72 700 Td (A) Tj ET"),
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Montserrat-Regular /FirstChar 65 /LastChar 65 /Widths [700] /Encoding /MacRomanEncoding /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Montserrat-Regular /Flags 32 >>".to_vec(),
        ]);
        let program = fixture_font();
        let out = embed_missing(&input, &|name| (name == "Montserrat-Regular").then(|| program.clone())).expect("embeds");
        let stored = stored_program(&File::parse(&out.bytes).expect("a readable file"), 5);
        assert_eq!(stored, program);
    }

    fn table_directory(font: &[u8]) -> Option<usize> {
        let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
        (count > 0 && font.len() >= 12 + 16).then_some(12)
    }

    /// A small real face the repository already carries.
    fn fixture_font() -> Vec<u8> {
        include_bytes!("../../../../desktop/third_party/fonts/Montserrat-Regular.ttf").to_vec()
    }
}
