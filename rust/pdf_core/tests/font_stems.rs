//! What a font's **stem** ([`pdf_core::document::RunStyle::stem_milli_em`]) is a
//! measurement of — and when PDFium's answer is not one.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test font_stems -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or a font prints why and returns, and still counts as
//! passed. Look for the numbers each test prints.
//!
//! **Why these tests exist.** The detector splits a paragraph where two lines'
//! fonts differ by more than 12/1000 em of stem, so a stem that is wrong splits
//! a paragraph. A real document (a Smallpdf "TEST PDF") set its body in two
//! copies of one face — a simple TrueType font and a CID Identity-H one — and
//! the CID copy read 476 against the other's 134: three of its four paragraphs
//! were cut where the resource changed. The cause is not in the CID font. PDFium
//! answers an outline question about a letter the font does not have with the
//! font's `.notdef`, and the engine took that for a letter. The first tests build
//! that failure from nothing, out of one Montserrat, with the real parts of a
//! subset CID font a producer writes (renumbered glyphs, no `cmap`, a
//! `/ToUnicode` or none); the later ones keep what must not move.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::{BTreeMap, BTreeSet};

use pdf_core::document::pdfium_doc::{pdfium, PdfiumDocument};
use pdf_core::document::Document;
use pdfium_render::prelude::*;

// ======================================================================= pages ==

/// The Montserrat the app ships with (`desktop/third_party/fonts`): the family
/// the datasheet is set in, so its stems are the datasheet's (Regular 74).
fn montserrat(name: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../desktop/third_party/fonts").join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skipped: {} is not here", path.display());
            None
        }
    }
}

/// A PDF made of these numbered objects (the first is object 1, the catalogue),
/// byte-safe, with a classic cross-reference table.
fn pdf_of_bytes(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes(),
    );
    out
}

fn stream_of(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// What a composite font's `/ToUnicode` says.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Map {
    /// every letter the font carries
    All,
    /// every letter but this one
    Without(char),
    /// there is none
    Nothing,
}

/// A CID font made of a subset of a program: its glyphs renumbered from 1 (0 is
/// `.notdef`), its `cmap` gone — as every subsetter that writes a CID font does —
/// and the codes that spell each character.
struct Cid {
    font: usize,
    code_of: BTreeMap<char, u16>,
}

impl Cid {
    /// `text` shown as this font's codes.
    fn show(&self, text: &str) -> String {
        let hex: String = text.chars().map(|c| format!("{:04X}", self.code_of[&c])).collect();
        format!("<{hex}> Tj")
    }
}

/// A one-page PDF being built: objects 1 to 4 are the catalogue, the page tree,
/// the page and its content; fonts and their parts follow. Everything drawn is a
/// line of text in a font of its own, so the *n*th line is the page's *n*th object.
struct Page {
    objects: Vec<Vec<u8>>,
    drawn: Vec<(usize, String)>,
}

impl Page {
    fn new() -> Self {
        Page { objects: vec![Vec::new(); 4], drawn: Vec::new() }
    }

    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len()
    }

    /// Draw `show` (a show-text operator with its operand) in the font object
    /// `font`; the page object that results is numbered as the lines are.
    fn draw(&mut self, font: usize, show: &str) -> usize {
        self.drawn.push((font, show.to_string()));
        self.drawn.len() - 1
    }

    /// The program embedded whole, and a descriptor naming it.
    fn descriptor(&mut self, program: &[u8], flags: u32) -> usize {
        let file = self.add(stream_of(&format!("/Length1 {}", program.len()), program));
        self.add(
            format!(
                "<< /Type /FontDescriptor /FontName /ABCDEF+Probe /Flags {flags} /FontBBox [-300 -300 1500 1100] \
                 /ItalicAngle 0 /Ascent 968 /Descent -251 /CapHeight 700 /StemV 80 /FontFile2 {file} 0 R >>"
            )
            .into_bytes(),
        )
    }

    /// A simple TrueType font with the whole program embedded and WinAnsi codes.
    fn simple_font(&mut self, program: &[u8]) -> usize {
        let descriptor = self.descriptor(program, 32);
        self.add(
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Probe /Encoding /WinAnsiEncoding \
                 /FontDescriptor {descriptor} 0 R >>"
            )
            .into_bytes(),
        )
    }

    /// A CID font (Identity-H, `CIDToGIDMap /Identity`) of a subset of `program`
    /// holding the glyphs of `chars`, with the `/ToUnicode` that `map` names.
    fn cid_font(&mut self, program: &[u8], chars: &str, map: Map) -> Cid {
        let face = ttf_parser::Face::parse(program, 0).expect("a font");
        let mut remapper = subsetter::GlyphRemapper::new();
        let mut code_of = BTreeMap::new();
        for c in chars.chars().collect::<BTreeSet<_>>() {
            let gid = face.glyph_index(c).unwrap_or_else(|| panic!("no glyph for {c:?}")).0;
            code_of.insert(c, remapper.remap(gid));
        }
        let subset = subsetter::subset(program, 0, &remapper).expect("subset");
        let descriptor = self.descriptor(&subset, 4);
        let cid_font = self.add(
            format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Probe \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                 /FontDescriptor {descriptor} 0 R /DW 600 /CIDToGIDMap /Identity >>"
            )
            .into_bytes(),
        );
        let to_unicode = match map {
            Map::Nothing => String::new(),
            Map::All | Map::Without(_) => {
                let entries: Vec<(&char, &u16)> = code_of.iter().filter(|(c, _)| map != Map::Without(**c)).collect();
                let mut body = String::from(
                    "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
                     /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
                     1 begincodespacerange <0000> <FFFF> endcodespacerange\n",
                );
                body.push_str(&format!("{} beginbfchar\n", entries.len()));
                for (c, code) in entries {
                    body.push_str(&format!("<{code:04X}> <{:04X}>\n", *c as u32));
                }
                body.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end");
                let object = self.add(stream_of("", body.as_bytes()));
                format!(" /ToUnicode {object} 0 R")
            }
        };
        let font = self.add(
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Probe /Encoding /Identity-H \
                 /DescendantFonts [{cid_font} 0 R]{to_unicode} >>"
            )
            .into_bytes(),
        );
        Cid { font, code_of }
    }

    fn finish(mut self) -> Vec<u8> {
        let fonts: String = self.drawn.iter().enumerate().map(|(n, (font, _))| format!("/F{n} {font} 0 R ")).collect();
        let content: String = self
            .drawn
            .iter()
            .enumerate()
            .map(|(n, (_, show))| format!("BT /F{n} 12 Tf 72 {} Td {show} ET \n", 740 - 30 * n))
            .collect();
        self.objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
        self.objects[1] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec();
        self.objects[2] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << {fonts}>> >> >>"
        )
        .into_bytes();
        self.objects[3] = stream_of("", content.as_bytes());
        pdf_of_bytes(&self.objects)
    }
}

/// The stem of each text object of the one page, in page order.
fn stems(doc: &PdfiumDocument) -> Vec<Option<u16>> {
    let styles = doc.run_styles(0).expect("styles");
    let mut objects: Vec<usize> = styles.keys().copied().collect();
    objects.sort_unstable();
    objects.iter().map(|o| styles[o].stem_milli_em).collect()
}

/// What PDFium hands back when the font of text object `object` on page `page`
/// is asked for a character's outline: the number of points of the path and its
/// width in em, or `None` for no path.
fn raw_glyph_on(doc: &PdfiumDocument, page: usize, object: usize, letter: char) -> Option<(usize, f32)> {
    let bindings = pdfium().expect("pdfium").bindings();
    let handle = doc.backend_handle().expect("a PDFium document") as FPDF_DOCUMENT;
    let raw = unsafe { bindings.FPDF_LoadPage(handle, page as i32) };
    let font = unsafe { bindings.FPDFTextObj_GetFont(bindings.FPDFPage_GetObject(raw, object as i32)) };
    let path = unsafe { bindings.FPDFFont_GetGlyphPath(font, letter as u32, 1000.0) };
    let found = if path.is_null() {
        None
    } else {
        let segments = unsafe { bindings.FPDFGlyphPath_CountGlyphSegments(path) };
        let (mut left, mut right) = (f32::MAX, f32::MIN);
        let mut points = 0;
        for index in 0..segments.max(0) {
            let segment = unsafe { bindings.FPDFGlyphPath_GetGlyphPathSegment(path, index) };
            let (mut x, mut y) = (0.0f32, 0.0f32);
            if !segment.is_null() && unsafe { bindings.FPDFPathSegment_GetPoint(segment, &mut x, &mut y) } != 0 {
                left = left.min(x);
                right = right.max(x);
                points += 1;
            }
        }
        (points > 0).then_some((points, right - left))
    };
    unsafe { bindings.FPDF_ClosePage(raw) };
    found
}

fn raw_glyph(doc: &PdfiumDocument, object: usize, letter: char) -> Option<(usize, f32)> {
    raw_glyph_on(doc, 0, object, letter)
}

const TEXT: &str = "Hill Illinois lilt";

// ============================================================ the measurement ==

/// **A bold face measures thicker than a regular one**, embedded whole as simple
/// TrueType fonts: the datasheet's Regular reads 74, and this Bold clearly more.
/// What the stem exists for, with real outlines in place of the stand-ins the two
/// Helvetica tests in `run_styles.rs` no longer measure.
#[test]
fn the_weights_of_embedded_fonts_come_out_in_order() {
    let Some(_) = skip_without_pdfium() else { return };
    let (Some(regular), Some(bold)) = (montserrat("Montserrat-Regular.ttf"), montserrat("Montserrat-Bold.ttf")) else {
        return;
    };
    let _lock = serial();
    let mut page = Page::new();
    let (r, b) = (page.simple_font(&regular), page.simple_font(&bold));
    page.draw(r, &format!("({TEXT}) Tj"));
    page.draw(b, &format!("({TEXT}) Tj"));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    let found = stems(&doc);
    println!("Montserrat Regular {:?}, Bold {:?}", found[0], found[1]);
    let (r, b) = (found[0].expect("Regular's stem"), found[1].expect("Bold's stem"));
    assert!(r.abs_diff(74) <= 2, "Regular's stem {r} should be the datasheet's 74");
    assert!(b > r + 20, "Bold's stem {b} should be clearly thicker than Regular's {r}");
}

/// **A font that is not embedded has no stem**: PDFium draws, and measures, a
/// stand-in from this computer. A TrueType and a Type 1, both without a program.
#[test]
fn a_font_that_is_not_embedded_has_no_stem() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut page = Page::new();
    let not_embedded =
        page.add(b"<< /Type /Font /Subtype /TrueType /BaseFont /Montserrat-Light /Encoding /WinAnsiEncoding >>".to_vec());
    let helvetica = page.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_vec());
    page.draw(not_embedded, &format!("({TEXT}) Tj"));
    page.draw(helvetica, &format!("({TEXT}) Tj"));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    // The trap, so this cannot pass by the stand-in having no `I`: PDFium does
    // answer, with an outline from this computer.
    let (points, width) = raw_glyph(&doc, 1, 'I').expect("PDFium answers for Helvetica-Bold's I");
    println!("Helvetica-Bold, not embedded: PDFium's I is {points} points, {width:.3} em wide");
    assert!(points >= 4 && width > 0.0);
    assert_eq!(stems(&doc), [None, None], "a font that is not embedded has no stem");
}

/// **A composite font is measured through its own `/ToUnicode`** — and gives the
/// stem of its own `I`, which is the same as the simple font's. The CID subset's
/// glyphs are renumbered (nothing about its codes is ASCII) and it has no `cmap`:
/// that PDFium finds the `I` at all is its `/ToUnicode` and `CIDToGIDMap` at work,
/// and it is what keeps the datasheet's CID Light (`FHCJYT+Montserrat-Light`) at 51.
#[test]
fn a_composite_font_is_measured_through_its_own_to_unicode_map() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(program) = montserrat("Montserrat-Regular.ttf") else { return };
    let _lock = serial();
    let mut page = Page::new();
    let simple = page.simple_font(&program);
    let cid = page.cid_font(&program, TEXT, Map::All);
    page.draw(simple, &format!("({TEXT}) Tj"));
    page.draw(cid.font, &cid.show(TEXT));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    let found = stems(&doc);
    println!("simple {:?}, composite with a /ToUnicode {:?}", found[0], found[1]);
    assert_eq!(found[0], found[1], "the same face, whichever way it is written");
    assert!(found[1].is_some_and(|s| s.abs_diff(74) <= 2));
}

/// **A composite font with no `/ToUnicode` has no stem — not the `.notdef` box.**
/// This is the bug, from nothing. With no map, PDFium cannot find the code for an
/// `I` and answers with the font's glyph 0, a box 0.507 em wide in Montserrat:
/// read as a letter, a stem of 507, and a different "weight" from the same face's
/// 74 that split a paragraph in two. The precondition below is the trap itself;
/// without it this test could pass by PDFium having stopped doing that.
#[test]
fn a_composite_font_with_no_to_unicode_map_has_no_stem() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(program) = montserrat("Montserrat-Regular.ttf") else { return };
    let _lock = serial();
    let mut page = Page::new();
    let simple = page.simple_font(&program);
    let cid = page.cid_font(&program, TEXT, Map::Nothing);
    page.draw(simple, &format!("({TEXT}) Tj"));
    page.draw(cid.font, &cid.show(TEXT));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");

    let (points, width) = raw_glyph(&doc, 1, 'I').expect("PDFium answers for the CID font's I");
    println!("with no /ToUnicode, PDFium's I is {points} points, {width:.3} em wide (a letter I is 5 and 0.074)");
    assert!(width > 0.4 && points > 5, "the trap is gone: PDFium no longer answers .notdef for an I it cannot find");

    let found = stems(&doc);
    println!("simple {:?}, composite with no /ToUnicode {:?}", found[0], found[1]);
    assert_eq!(found[1], None, "a .notdef box is not a stem");
    assert!(found[0].is_some(), "the simple copy of the font is measured as ever");
}

/// **A font that lacks the capital I falls back to its l** — and reads the same
/// stem within a few thousandths. The two copies of one face on the TEST PDF:
/// whichever letter a copy happens to carry, the stems agree to within the
/// detector's tolerance (12), so neither splits a paragraph.
#[test]
fn two_copies_of_one_face_agree_whichever_letters_each_carries() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(program) = montserrat("Montserrat-Regular.ttf") else { return };
    let _lock = serial();
    let mut page = Page::new();
    let simple = page.simple_font(&program);
    let no_capital_i = page.cid_font(&program, TEXT, Map::Without('I'));
    let nothing = page.cid_font(&program, TEXT, Map::Nothing);
    page.draw(simple, &format!("({TEXT}) Tj"));
    page.draw(no_capital_i.font, &no_capital_i.show(TEXT));
    page.draw(nothing.font, &nothing.show(TEXT));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    let found = stems(&doc);
    println!(
        "with every letter {:?}, without the I in its /ToUnicode {:?}, with no /ToUnicode {:?}",
        found[0], found[1], found[2]
    );
    let (a, b) = (found[0].expect("the whole font"), found[1].expect("falls back to l"));
    assert!(a.abs_diff(b) <= 12, "the two copies read {a} and {b}: the detector would call them two weights");
    assert_eq!(found[2], None);
}

/// **A subset that carries none of the three letters has no stem.** All it can
/// say for `I`, `l` and `i` is the one glyph it draws for a letter it has not got.
#[test]
fn a_subset_with_none_of_the_three_letters_has_no_stem() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(program) = montserrat("Montserrat-Regular.ttf") else { return };
    let _lock = serial();
    let mut page = Page::new();
    let cid = page.cid_font(&program, "Hexo Hex", Map::All);
    page.draw(cid.font, &cid.show("Hexo Hex"));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    // All three letters are the same glyph: that is how `.notdef` is told from a letter.
    let glyphs: Vec<_> = ['I', 'l', 'i'].iter().map(|c| raw_glyph(&doc, 0, *c)).collect();
    println!("the three letters of a subset that has none: {glyphs:?}");
    assert!(glyphs[0].is_some() && glyphs[0] == glyphs[1] && glyphs[1] == glyphs[2], "the trap: one glyph for all three");
    assert_eq!(stems(&doc), [None]);
}

/// The program with its glyph 0 — the `.notdef` — redrawn as a plain rectangle
/// `width_em` wide and `height_em` tall: what a font whose missing-glyph box is
/// narrow looks like (ArialNarrow's is 0.190 wide, which is how every invoice of the
/// owner's that uses it came to have a stem of 190). The glyph's own bytes are
/// rewritten in place, padded to the length they had, so no offset in the file moves.
fn with_a_narrow_notdef(mut font: Vec<u8>, width_em: f32, height_em: f32) -> Option<Vec<u8>> {
    let u16_at = |f: &[u8], at: usize| u16::from_be_bytes([f[at], f[at + 1]]);
    let u32_at = |f: &[u8], at: usize| u32::from_be_bytes([f[at], f[at + 1], f[at + 2], f[at + 3]]);
    let tables = u16_at(&font, 4) as usize;
    let table = |font: &[u8], tag: &[u8; 4]| {
        (0..tables).find_map(|i| {
            let record = 12 + 16 * i;
            (&font[record..record + 4] == tag).then(|| u32_at(font, record + 8) as usize)
        })
    };
    let (head, loca, glyf) = (table(&font, b"head")?, table(&font, b"loca")?, table(&font, b"glyf")?);
    let upem = f32::from(u16_at(&font, head + 18));
    let long_offsets = u16_at(&font, head + 50) != 0;
    let (from, to) = if long_offsets {
        (u32_at(&font, loca) as usize, u32_at(&font, loca + 4) as usize)
    } else {
        (u16_at(&font, loca) as usize * 2, u16_at(&font, loca + 2) as usize * 2)
    };
    let (w, h) = ((width_em * upem) as i16, (height_em * upem) as i16);
    let mut glyph: Vec<u8> = Vec::new();
    for v in [1i16, 0, 0, w, h] {
        glyph.extend_from_slice(&v.to_be_bytes()); // one contour; xMin, yMin, xMax, yMax
    }
    glyph.extend_from_slice(&3u16.to_be_bytes()); // the last point of the contour
    glyph.extend_from_slice(&0u16.to_be_bytes()); // no instructions
    glyph.extend_from_slice(&[1, 1, 1, 1]); // four points on the curve, 16-bit coordinates
    for dx in [0, w, 0, -w] {
        glyph.extend_from_slice(&dx.to_be_bytes());
    }
    for dy in [0, 0, h, 0] {
        glyph.extend_from_slice(&dy.to_be_bytes());
    }
    if glyph.len() > to.checked_sub(from)? {
        return None;
    }
    glyph.resize(to - from, 0);
    font[glyf + from..glyf + to].copy_from_slice(&glyph);
    Some(font)
}

/// **A `.notdef` that looks like a stem is still not one.** Montserrat's missing-
/// glyph box is 0.507 wide, so a stem read from it (507) is turned away by the
/// ceiling on what a stem can be; a face whose box is narrow has no such luck
/// (ArialNarrow's is 0.190 wide — the stem of 190 that every one of the owner's
/// invoices set in it carried). Here Montserrat's glyph 0 is redrawn 0.2 wide, a
/// perfectly good stem as far as its size goes, and three CID copies of the font
/// are drawn: one with a `/ToUnicode` for every letter (it reads its `I`, 74), one
/// with none (its `I`, `l` and `i` are all that rectangle: no stem) and one that
/// lacks only the capital I (it falls back to its `l`, 71).
#[test]
fn a_notdef_that_looks_like_a_stem_is_still_not_one() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(program) = montserrat("Montserrat-Regular.ttf") else { return };
    let Some(narrow) = with_a_narrow_notdef(program.clone(), 0.2, 0.7) else {
        eprintln!("skipped: Montserrat's glyph 0 has no room for a rectangle");
        return;
    };
    let _lock = serial();
    let mut page = Page::new();
    let whole = page.cid_font(&narrow, TEXT, Map::All);
    let nothing = page.cid_font(&narrow, TEXT, Map::Nothing);
    let no_capital_i = page.cid_font(&narrow, TEXT, Map::Without('I'));
    page.draw(whole.font, &whole.show(TEXT));
    page.draw(nothing.font, &nothing.show(TEXT));
    page.draw(no_capital_i.font, &no_capital_i.show(TEXT));
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");

    // The trap: for the font with no map, PDFium's `I` is the 0.2-wide rectangle.
    let (points, width) = raw_glyph(&doc, 1, 'I').expect("PDFium answers for the I");
    println!("with no /ToUnicode, PDFium's I is {points} points, {width:.3} em wide (a rectangle: the redrawn .notdef)");
    assert!((width - 0.2).abs() < 0.01 && points >= 4, "the redrawn .notdef is not what PDFium answers: {points} points, {width}");

    let found = stems(&doc);
    println!("every letter {:?}, no /ToUnicode {:?}, none for I {:?}", found[0], found[1], found[2]);
    assert!(found[0].is_some_and(|s| s.abs_diff(74) <= 2), "the font with every letter reads its I");
    assert_eq!(found[1], None, "a .notdef of 0.2 em is not a stem of 200");
    assert!(found[2].is_some_and(|s| s.abs_diff(74) <= 12), "without its I the font reads its l, not the box: {:?}", found[2]);
}

/// **A Type 3 font has no stem, whatever it says about itself.** It reports
/// itself embedded and carries no font program — its glyphs are drawings in the
/// page — and PDFium answers outline questions about it from a stand-in: here
/// this computer's Arial, 95. (A Chrome-printed StockHub page had two such fonts
/// and every cell of it read 95.)
#[test]
fn a_type_3_font_has_no_stem_not_the_arial_standing_in_for_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut page = Page::new();
    let glyph = |outline: &str| stream_of("", format!("1000 0 0 0 250 740 d1 {outline}").as_bytes());
    let (g_i, g_l, g_dot) = (
        page.add(glyph("100 0 80 700 re f")),
        page.add(glyph("100 0 70 740 re f")),
        page.add(glyph("100 0 80 540 re f 100 600 80 80 re f")),
    );
    // A `/ToUnicode`, as a producer writes one for every font it prints text in:
    // it is what lets PDFium find the code for the character it is asked for.
    let to_unicode = page.add(stream_of(
        "",
        b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Adobe-Identity-UCS def \
          /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 3 beginbfchar \
          <49> <0049> <6C> <006C> <69> <0069> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
    ));
    let font = page.add(
        format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
             /CharProcs << /I {g_i} 0 R /l {g_l} 0 R /i {g_dot} 0 R >> \
             /Encoding << /Type /Encoding /Differences [73 /I 105 /i 108 /l] >> \
             /FirstChar 73 /LastChar 108 /Widths [250 {} 250 0 0 250] /ToUnicode {to_unicode} 0 R >>",
            "0 ".repeat(31).trim_end()
        )
        .into_bytes(),
    );
    page.draw(font, "(Ili) Tj");
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    let (points, width) = raw_glyph(&doc, 0, 'I').expect("PDFium answers for the Type 3 font's I");
    println!("a Type 3 font's I, as PDFium answers it: {points} points, {width:.3} em wide (the stand-in; the page draws a 0.08 box)");
    assert!(points >= 4 && width > 0.0, "the trap is gone: PDFium no longer answers for a Type 3 font");
    assert_eq!(stems(&doc), [None]);
}

/// **A stem above 300 is skipped**: a serif foot (Noto Serif Bold's `I`) is not a
/// weight. The next letter is tried, and where none is plausible the font has no
/// stem; what is reported is never above what a weight can be.
#[test]
fn a_stem_above_300_is_not_a_weight() {
    let Some(_) = skip_without_pdfium() else { return };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src/main/assets/fonts/NotoSerif-Bold.ttf");
    let Ok(program) = std::fs::read(&path) else {
        eprintln!("skipped: {} is not here", path.display());
        return;
    };
    let _lock = serial();
    let mut page = Page::new();
    let font = page.simple_font(&program);
    page.draw(font, "(I) Tj");
    page.draw(font, "(l) Tj");
    page.draw(font, "(i) Tj");
    let doc = PdfiumDocument::open_bytes(page.finish(), None).expect("open");
    let found = stems(&doc);
    // One font, three objects: they share it, so they share its stem.
    println!("Noto Serif Bold: stem {:?}; its capital I is {:?}", found[0], raw_glyph(&doc, 0, 'I'));
    assert!(found.iter().all(|s| *s == found[0]));
    assert!(found[0].is_none_or(|s| s <= 300), "a stem of {:?} is above what a weight can be", found[0]);
}

// ================================================================ real files ==

/// The datasheet's pages, CAMINO and the DQ-8 quotation, where they are on this
/// machine. Each is only read.
fn real(path: &str) -> Option<PdfiumDocument> {
    if !std::path::Path::new(path).exists() {
        eprintln!("skipped: {path} is not on this machine");
        return None;
    }
    Some(PdfiumDocument::open_path(path, None).expect("open"))
}

/// **CAMINO's two CID fonts**: the ExtraBold heading's read 507 (Montserrat's
/// `.notdef` box, for an `I` and an `l` its subset does not have) and now reads
/// its `i`, about the same as the simple ExtraBold's 198; the Light body's, which
/// has a `/ToUnicode` for its `I`, keeps its 51. Its non-embedded `ArialMT` has none.
#[test]
fn camino_s_cid_fonts() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(doc) = real(r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf") else { return };
    let _lock = serial();
    let styles = doc.run_styles(0).expect("styles");
    let at = |o: usize| styles.get(&o).unwrap_or_else(|| panic!("object {o} has no style")).stem_milli_em;
    // First objects of the fonts, as the page numbers them.
    let (heading_cid, body_cid, arial, heading_simple) = (at(495), at(5), at(6), at(2));
    println!(
        "CAMINO: ExtraBold CID {heading_cid:?} (507 before), Light CID {body_cid:?}, ArialMT (not embedded) {arial:?} \
         (95 before), ExtraBold simple {heading_simple:?}"
    );
    assert!(heading_cid.is_some_and(|s| s.abs_diff(198) <= 12), "the CID ExtraBold reads {heading_cid:?}");
    assert!(heading_simple.is_some_and(|s| s.abs_diff(198) <= 12));
    assert!(body_cid.is_some_and(|s| s.abs_diff(51) <= 12));
    assert_eq!(arial, None);
}

/// **The DQ-8 quotation has no stems**: every one of its four fonts is a
/// non-embedded Helvetica (95, 145, 96 and 147 before, all Arial's). The detector
/// tells its bold part names from the rest by their names.
#[test]
fn the_dq_8_quotation_s_fonts_have_no_stem() {
    let Some(_) = skip_without_pdfium() else { return };
    let Some(doc) = real(r"C:\Users\hsili\Desktop\DQ-8-ALYASRAFASHION-ALFAHIM-HS-2026-SF-8-REV-00.pdf") else { return };
    let _lock = serial();
    let styles = doc.run_styles(0).expect("styles");
    let faces = doc.run_font_names(0).expect("faces");
    let mut seen = BTreeSet::new();
    for (object, style) in &styles {
        assert_eq!(style.stem_milli_em, None, "object {object} ({:?}) has a stem", faces.get(object));
        seen.insert(faces.get(object).cloned().unwrap_or_default());
    }
    println!("the quotation's fonts: {seen:?}");
    assert!(seen.len() >= 2, "a quotation set in one font");
}

// ============================================================ the verdict table ==

/// What a font answers for I, l and i: the points of the path and its width in
/// em, or `-`.
fn letters_line(doc: &PdfiumDocument, page: usize, object: usize) -> String {
    ['I', 'l', 'i']
        .iter()
        .map(|c| match raw_glyph_on(doc, page, object, *c) {
            Some((points, width)) => format!("{c}:{points}pt/{width:.3}"),
            None => format!("{c}:-"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The verdict table: every font of the first pages of the documents named in
/// `PAGIFY_STEM_DOCS` (separated by `;`), with what PDFium answers for `I`, `l`
/// and `i` and the stem the engine reports. Not asserted; run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> PAGIFY_STEM_DOCS="a.pdf;b.pdf" PAGIFY_STEM_PAGES=3 \
///   cargo test --release --test font_stems -- --ignored --nocapture what_every_font_says
/// ```
#[test]
#[ignore = "a table, not a check; run with --ignored --nocapture"]
fn what_every_font_says() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let list = std::env::var("PAGIFY_STEM_DOCS").unwrap_or_default();
    let max_pages: usize = std::env::var("PAGIFY_STEM_PAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    let bindings = pdfium().expect("pdfium").bindings();
    for path in list.split(';').filter(|p| !p.is_empty()) {
        println!("\n==================== {path}");
        let Ok(doc) = PdfiumDocument::open_path(path, None) else {
            println!("  does not open");
            continue;
        };
        for page in 0..doc.page_count().min(max_pages) {
            let styles = doc.run_styles(page).unwrap_or_default();
            let faces = doc.run_font_names(page).unwrap_or_default();
            let mut objects: Vec<usize> = styles.keys().copied().collect();
            objects.sort_unstable();
            // The first object of each font and how many objects use it.
            let mut by_font: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
            for object in objects {
                by_font.entry(styles[&object].font).or_insert((object, 0)).1 += 1;
            }
            println!("--- page {}", page + 1);
            let handle = doc.backend_handle().expect("a PDFium document") as FPDF_DOCUMENT;
            let raw = unsafe { bindings.FPDF_LoadPage(handle, page as i32) };
            for (first, count) in by_font.values() {
                let font = unsafe { bindings.FPDFTextObj_GetFont(bindings.FPDFPage_GetObject(raw, *first as i32)) };
                let embedded = unsafe { bindings.FPDFFont_GetIsEmbedded(font) };
                let mut program: usize = 0;
                unsafe { bindings.FPDFFont_GetFontData(font, std::ptr::null_mut(), 0, &mut program) };
                println!(
                    "  #{first:<6} x{count:<5} {:<40} embedded={embedded:<2} program={program:<8} {}  ->  stem {:?}",
                    faces.get(first).cloned().unwrap_or_default(),
                    letters_line(&doc, page, *first),
                    styles[first].stem_milli_em
                );
            }
            unsafe { bindings.FPDF_ClosePage(raw) };
        }
    }
}
