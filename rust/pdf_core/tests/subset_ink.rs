//! A code is not a glyph: retyping into a font subset must not write a
//! character the subset has no outline for.
//!
//! **Reported from use.** Retyping the `600` of a `600mm` dimension to `500`
//! drew an empty box where the `5` should be. The font was a Type1C subset of
//! `space zero six m` under `/Encoding /WinAnsiEncoding` with no `/ToUnicode`,
//! so every printable ASCII code was assumed spellable — and code `0x35` is a
//! perfectly good code. Nothing was wrong with it except that the font had
//! nothing to draw for it, and no check along the way asked.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test subset_ink
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;
use pdf_core::pdf::{subset, File, Object};

/// A stream object holding `data`, which must be ASCII.
fn stream(data: &str) -> String {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// A PDF made of these numbered objects (the first is object 1, and must be
/// the catalogue), with a classic cross-reference table.
fn pdf_of(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// One page drawing `600mm` in `/F1`, a single-byte font with no `/ToUnicode`
/// whose descriptor says what `descriptor_extra` says.
fn page_with(encoding: &str, descriptor_extra: &str) -> Vec<u8> {
    pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        stream("BT /F1 24 Tf 20 100 Td (600mm) Tj ET"),
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Helvetica \
             /Encoding {encoding} /FontDescriptor 6 0 R >>"
        ),
        format!(
            "<< /Type /FontDescriptor /FontName /ABCDEF+Helvetica /Flags 32 \
             /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -200 \
             /CapHeight 700 /StemV 80 {descriptor_extra} >>"
        ),
    ])
}

/// The drawable codes of `/F1` in `bytes`, by the same route the editor takes.
fn drawable(bytes: &[u8]) -> Option<std::collections::BTreeSet<u32>> {
    let file = File::parse(bytes).expect("parse");
    // The page's font dictionary, as the editor reads it: `/F1` is object 5.
    let fonts = pdf_core::pdf::Dict(vec![(b"F1".to_vec(), Object::Reference(5, 0))]);
    subset::drawable_ascii(&file, &fonts, b"F1")
}

#[test]
fn a_charset_says_which_codes_have_an_outline() {
    let bytes = page_with("/WinAnsiEncoding", "/CharSet (/space/zero/six/m)");
    let ink = drawable(&bytes).expect("a CharSet this can read");
    for kept in ['0', '6', 'm', ' '] {
        assert!(ink.contains(&(kept as u32)), "{kept:?} is in the subset but was left out");
    }
    for gone in ['5', '1', 'a', 'M'] {
        assert!(!ink.contains(&(gone as u32)), "{gone:?} is not in the subset but was kept");
    }
}

#[test]
fn a_differences_array_renames_what_a_code_means() {
    // Code 0x35 is told to be `six` (which the subset has) and code 0x30 to be
    // `five` (which it has not): the encoding's own names win over ASCII's.
    let bytes = page_with(
        "<< /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [53 /six 48 /five] >>",
        "/CharSet (/space/zero/six/m)",
    );
    let ink = drawable(&bytes).expect("readable");
    assert!(ink.contains(&0x35), "code 0x35 is renamed `six`, which is in the subset");
    assert!(!ink.contains(&0x30), "code 0x30 is renamed `five`, which is not");
}

#[test]
fn what_cannot_be_read_is_no_evidence_not_nothing() {
    // No `/CharSet` at all.
    assert!(drawable(&page_with("/WinAnsiEncoding", "")).is_none());
    // Glyphs renamed `g1 g2 …`: none of these is the name of an ASCII letter,
    // and calling every letter missing would refuse words the font draws.
    assert!(drawable(&page_with("/WinAnsiEncoding", "/CharSet (/g1/g2/g3)")).is_none());
}

fn open(bytes: Vec<u8>) -> PdfiumDocument {
    PdfiumDocument::open_bytes(bytes, None).expect("open")
}

/// **The reported bug.** `5` is not in the subset, so the run must not be
/// written with code `0x35` in it — with no other font to type with, the edit
/// is refused, and says what *can* be typed.
#[test]
fn a_character_the_subset_has_no_outline_for_is_refused_not_drawn_as_a_box() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open(page_with("/WinAnsiEncoding", "/CharSet (/space/zero/six/m)"));
    let run = doc.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("600")).expect("the run");

    let refused = doc.try_set_run_in_stream(0, run.object, "500mm");
    let why = format!("{:?}", refused.expect_err("`5` has no outline in this font, so this must not be written"));
    assert!(why.contains("06m"), "the refusal should list what can be typed: {why}");

    let after = doc.text_runs(0).expect("runs");
    assert!(after.iter().any(|r| r.text.contains("600mm")), "the refused edit changed the page: {after:?}");
}

/// **And the other direction**, which is what keeps the first from being
/// "refuse everything": words made only of what the subset kept still retype,
/// in the run's own font, with nothing swapped.
#[test]
fn words_made_of_what_the_subset_kept_still_retype_in_place() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open(page_with("/WinAnsiEncoding", "/CharSet (/space/zero/six/m)"));
    let run = doc.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("600")).expect("the run");

    doc.try_set_run_in_stream(0, run.object, "660mm").expect("6, 0 and m are all in the subset");
    assert!(doc.substituted_face().is_none(), "a font was swapped for words it could spell");
    let after = doc.text_runs(0).expect("runs");
    assert!(after.iter().any(|r| r.text.contains("660mm")), "the new words are not on the page: {after:?}");
}

/// **The same words, untouched.** Retyping a run with its own text must not
/// trip the check even when a `/CharSet` name could not be matched: the
/// characters are on the page, so they have outlines.
#[test]
fn a_runs_own_words_are_never_refused_for_a_name_that_did_not_match() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    // `m` is drawn, but the descriptor lists only the digits — the shape of a
    // `/CharSet` this could not match against the name an ASCII `m` goes by.
    let mut doc = open(page_with("/WinAnsiEncoding", "/CharSet (/space/zero/six)"));
    let run = doc.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("600")).expect("the run");

    doc.try_set_run_in_stream(0, run.object, "600mm").expect("its own words, in its own font");
    assert!(doc.substituted_face().is_none());
}

/// **A font whose table directory is out of order is still a font.**
///
/// Reported from use: no line could be added to a paragraph in a PDF printed from
/// Excel — `run-own-font-… could not be cut down: UnknownKind`. Office writes the
/// directory as `glyf, cmap, head, …`; `subsetter` and `ttf-parser` find a table by
/// binary search, so they walked past `glyf` and called the font unknown.
#[test]
fn a_font_with_an_unsorted_table_directory_can_be_registered_and_cut_down() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/src/main/assets/fonts/NotoSans-Bold.ttf");
    let Ok(sorted) = std::fs::read(path) else { return };

    // The same font with its directory written back to front: every table where
    // it was, only the records in the wrong order.
    let mut shuffled = sorted.clone();
    let count = u16::from_be_bytes([shuffled[4], shuffled[5]]) as usize;
    let mut records: Vec<Vec<u8>> = shuffled[12..12 + 16 * count].chunks(16).map(|c| c.to_vec()).collect();
    records.reverse();
    for (i, record) in records.iter().enumerate() {
        shuffled[12 + 16 * i..12 + 16 * (i + 1)].copy_from_slice(record);
    }
    assert_ne!(shuffled[12..12 + 16 * count], sorted[12..12 + 16 * count], "setup: the directory should differ");

    pdf_core::text::register("unsorted-directory-test", shuffled).expect("register");
    let cut = pdf_core::text::subset("unsorted-directory-test", &[36, 37, 38])
        .unwrap_or_else(|e| panic!("a font with an unsorted directory could not be cut down: {e}"));
    assert!(!cut.data.is_empty() && cut.data.len() < sorted.len() / 4, "the subset should be small");

    // And the helper leaves a directory that is already in order alone.
    let mut again = sorted.clone();
    subset::sort_table_directory(&mut again);
    assert_eq!(again, sorted);
}
