//! A text run's style identity: which font program draws it, how thick that
//! font's strokes are and which way it runs — read for a whole page at once.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test run_styles -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::HashMap;

use pdfium_render::prelude::PdfiumLibraryBindingsAccessor;
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, RunStyle, TextRun};

/// The datasheet this was built against: three A4 pages from Illustrator,
/// Montserrat in five weights stored under one name. Not in the repository
/// (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

/// A one-page PDF whose content stream is `content`, with Helvetica as `/F1`,
/// Helvetica-Bold as `/F2` and Courier as `/F3`.
fn page_drawing(content: &str) -> Vec<u8> {
    pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> >> >>"
            .to_string(),
        stream(content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_string(),
    ])
}

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

/// The object index of the first of `runs` whose words contain `needle`.
fn object_containing(runs: &[TextRun], needle: &str) -> usize {
    runs.iter()
        .find(|r| r.text.contains(needle))
        .unwrap_or_else(|| panic!("no run contains {needle:?}"))
        .object
}

fn style_of(styles: &HashMap<usize, RunStyle>, object: usize) -> RunStyle {
    *styles
        .get(&object)
        .unwrap_or_else(|| panic!("object {object} has no style ({} objects have)", styles.len()))
}

/// **One id per font, however many runs use it, numbered from 0 in the order
/// the page first reaches them.**
///
/// The page draws Helvetica, Helvetica-Bold, Helvetica again, Courier, then
/// Helvetica-Bold again, all at 12 pt: so the runs differ by font and by
/// nothing else. A later block detector reads "same id" as "same font program".
#[test]
fn one_dense_id_per_font_in_order_of_first_use() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 12 Tf 72 700 Td (Alpha) Tj ET \
                   BT /F2 12 Tf 72 680 Td (Bravo) Tj ET \
                   BT /F1 12 Tf 72 660 Td (Charlie) Tj ET \
                   BT /F3 12 Tf 72 640 Td (Delta) Tj ET \
                   BT /F2 12 Tf 72 620 Td (Echo) Tj ET";
    let doc = PdfiumDocument::open_bytes(page_drawing(content), None).expect("open");
    let styles = doc.run_styles(0).expect("styles");
    let by_word = |word: &str| {
        let run = doc
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.text.trim() == word)
            .unwrap_or_else(|| panic!("no run reads {word:?}"));
        style_of(&styles, run.object)
    };
    let (alpha, bravo, charlie, delta, echo) =
        (by_word("Alpha"), by_word("Bravo"), by_word("Charlie"), by_word("Delta"), by_word("Echo"));
    println!(
        "font ids  Alpha {} Bravo {} Charlie {} Delta {} Echo {}   stems (milli-em)  Helvetica {:?} Helvetica-Bold {:?} Courier {:?}",
        alpha.font, bravo.font, charlie.font, delta.font, echo.font,
        alpha.stem_milli_em, bravo.stem_milli_em, delta.stem_milli_em
    );

    assert_eq!(styles.len(), 5, "every text object should have a style");
    assert_eq!(alpha.font, charlie.font, "the same font must be the same id");
    assert_eq!(bravo.font, echo.font, "the same font must be the same id");
    assert_ne!(alpha.font, bravo.font, "Helvetica and Helvetica-Bold are different fonts");
    assert_ne!(alpha.font, delta.font, "Helvetica and Courier are different fonts");
    assert_ne!(bravo.font, delta.font, "Helvetica-Bold and Courier are different fonts");
    assert_eq!(
        (alpha.font, bravo.font, delta.font),
        (0, 1, 2),
        "ids are dense from 0, in the order the page first uses its fonts"
    );

    // **None of the three is embedded, so none has a stem.** PDFium draws a
    // stand-in from this computer for each (Arial's 95 and 145 for Helvetica
    // and its Bold; whatever happens to be installed elsewhere), and a number
    // about this machine is not a measurement of the file's font. Their names
    // say which is bold; `font_stems.rs` holds the weights of embedded fonts.
    assert_eq!(
        (alpha.stem_milli_em, bravo.stem_milli_em, delta.stem_milli_em),
        (None, None, None),
        "a font that is not embedded has no stem"
    );
}

/// **`axis` is the direction the text runs, in the top-left page space the rest
/// of a run's geometry uses — whether the turn came from the text matrix or
/// from the graphics state's CTM, and whatever the font size.**
#[test]
fn axis_is_the_text_direction_in_top_left_page_space() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 12 Tf 72 700 Td (flat) Tj ET \
                   BT /F1 12 Tf 0 1 -1 0 300 300 Tm (rising) Tj ET \
                   q 0 -1 1 0 100 500 cm BT /F1 12 Tf 0 0 Td (falling) Tj ET Q \
                   BT /F1 1 Tf 8 0 0 8 72 600 Tm (scaled) Tj ET";
    let doc = PdfiumDocument::open_bytes(page_drawing(content), None).expect("open");
    let styles = doc.run_styles(0).expect("styles");
    let runs = doc.text_runs(0).expect("runs");
    for (word, expected) in [
        ("flat", (1.0f32, 0.0f32)),     // upright
        ("rising", (0.0, -1.0)),         // Tm turned a quarter anticlockwise: up the page
        ("falling", (0.0, 1.0)),         // the same turn made by cm, the other way: down the page
        ("scaled", (1.0, 0.0)),          // `1 Tf` with the size in the matrix: still a unit vector
    ] {
        let object = object_containing(&runs, word);
        let (x, y) = style_of(&styles, object).axis;
        println!("axis of {word:>8}: ({x:+.4}, {y:+.4})  expected {expected:?}");
        assert!(
            (x - expected.0).abs() < 1e-4 && (y - expected.1).abs() < 1e-4,
            "{word}: axis ({x}, {y}) should be {expected:?}"
        );
    }
}

/// **A font with no outline to measure still gets a style, its stem unknown —
/// and asking does not take the process down.**
///
/// The stem probe asks PDFium for glyph outlines from whatever fonts a file
/// carries, and a file can carry fonts that have none (a Type 3 font draws from
/// content streams), fonts nobody embedded, or junk where the font program
/// should be. A crash inside PDFium cannot be caught from Rust, and the file
/// is not ours.
#[test]
fn fonts_without_outlines_still_get_a_style() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 12 Tf 72 700 Td (Hello) Tj ET \
                   BT /F4 12 Tf 72 660 Td (aaa) Tj ET \
                   BT /F5 12 Tf 72 620 Td <00240025> Tj ET \
                   BT /F6 12 Tf 72 580 Td (junk) Tj ET";
    let descriptor = |name: &str, extra: &str| {
        format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [0 0 1000 1000] \
             /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 {extra} >>"
        )
    };
    let pdf = pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R /F4 6 0 R /F5 8 0 R /F6 11 0 R >> >> >>"
            .to_string(),
        stream(content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        // 6: Type 3, whose one glyph is drawn by the content stream in 7.
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
         /CharProcs << /sq 7 0 R >> /Encoding << /Type /Encoding /Differences [97 /sq] >> \
         /FirstChar 97 /LastChar 97 /Widths [1000] >>"
            .to_string(),
        stream("1000 0 0 0 800 800 d1 0 0 800 800 re f"),
        // 8-10: a composite font whose program nobody embedded.
        "<< /Type /Font /Subtype /Type0 /BaseFont /Arial /Encoding /Identity-H /DescendantFonts [9 0 R] >>"
            .to_string(),
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Arial \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor 10 0 R /DW 1000 >>"
            .to_string(),
        descriptor("Arial", ""),
        // 11-13: a TrueType font whose embedded "program" is text.
        "<< /Type /Font /Subtype /TrueType /BaseFont /JunkSans /FontDescriptor 12 0 R >>".to_string(),
        descriptor("JunkSans", "/FontFile2 13 0 R"),
        stream("JUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNKJUNK"),
    ]);
    let doc = PdfiumDocument::open_bytes(pdf, None).expect("open");
    let styles = doc.run_styles(0).expect("styles");
    let mut objects: Vec<usize> = styles.keys().copied().collect();
    objects.sort_unstable();
    for object in &objects {
        let s = styles[object];
        println!("object {object}: font {} stem {:?} axis {:?}", s.font, s.stem_milli_em, s.axis);
    }

    assert_eq!(objects, [0, 1, 2, 3], "every one of the four text objects should have a style");
    let ids: std::collections::BTreeSet<u32> = styles.values().map(|s| s.font).collect();
    assert_eq!(ids, (0..4).collect(), "four different fonts, numbered from 0");
    // Helvetica is not embedded: PDFium would draw (and measure) a stand-in, so
    // it has no stem of its own either — an embedded font that can be measured
    // is in `font_stems.rs`.
    assert_eq!(styles[&0].stem_milli_em, None, "a font that is not embedded has no stem");
    assert_eq!(styles[&1].stem_milli_em, None, "a Type 3 font has no outline to measure");
}

/// **On the real datasheet the heading and the body are different fonts, with
/// stems that say which is which.**
///
/// Page 1's ExtraBold heading "The Light Source - COB" and the Light paragraph
/// under it carry the same face name, the same declared weight and the same
/// size; only the font resource and the drawn stroke differ. Object numbers on
/// page 1 are the ones the paragraph investigation measured; pages 2 and 3
/// carry the same blocks at other numbers, so those are found by their words.
#[test]
fn on_the_datasheet_the_heading_and_the_body_are_told_apart() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let doc = PdfiumDocument::open_path(DATASHEET, None).expect("open the datasheet");

    // ---- page 1
    let styles = doc.run_styles(0).expect("page 1 styles");
    let heading: Vec<RunStyle> = (980..=984).map(|o| style_of(&styles, o)).collect();
    assert!(
        heading.iter().all(|s| s.font == heading[0].font),
        "objects 980..=984 (\"The Light Source - COB\") should share one font: {heading:?}"
    );
    let heading = heading[0];

    // 985..=1043 is the paragraph under it. Two of those numbers (1026, 1035)
    // are words Illustrator converted to outlines — not text objects at all —
    // and 987 is the three-letter "HSI " in the Medium weight, another resource.
    let (mut body_objects, mut body): (usize, Option<RunStyle>) = (0, None);
    for object in 985..=1043 {
        if object == 1026 || object == 1035 {
            assert!(!styles.contains_key(&object), "object {object} is an outline, not text");
            continue;
        }
        if object == 987 {
            continue;
        }
        let style = style_of(&styles, object);
        let first = *body.get_or_insert(style);
        assert_eq!(style.font, first.font, "object {object} left the body's font");
        body_objects += 1;
    }
    let body = body.expect("a body");
    assert_eq!(body_objects, 56, "59 numbers, less two outlines and the one \"HSI \"");
    // The claim this API exists for, first: same face name, same declared
    // weight, same size, and still two different fonts.
    assert_ne!(heading.font, body.font, "the heading and the body are different fonts");
    let hsi = style_of(&styles, 987);
    assert_ne!(hsi.font, body.font, "\"HSI \" is drawn from another font resource");
    assert_ne!(hsi.font, heading.font, "\"HSI \" is not the heading's font either");

    let (h, b) = (heading.stem_milli_em.expect("heading stem"), body.stem_milli_em.expect("body stem"));
    println!(
        "page 1: heading font {} stem {h}   body font {} stem {b}   \"HSI \" font {} stem {:?}   {} text objects, {} fonts",
        heading.font, body.font, hsi.font, hsi.stem_milli_em, styles.len(),
        styles.values().map(|s| s.font).max().map_or(0, |m| m + 1)
    );
    assert!(h.abs_diff(198) <= 12, "ExtraBold heading stem {h} should be about 198");
    assert!(b.abs_diff(51) <= 12, "Light body stem {b} should be about 51");

    // A one-letter-wide stroke is not a font nobody can measure: the single
    // Myriad object on page 1 has an embedded copy with no `I`, `l` or `i`, and
    // PDFium answers every letter with its 0.5 x 0.7 em "missing glyph" box. A
    // stem of 500 would be that box mistaken for a letter.
    let myriad = style_of(&styles, 335);
    println!("page 1 object 335 (Myriad, no usable letter): stem {:?}", myriad.stem_milli_em);
    assert_eq!(myriad.stem_milli_em, None, "the missing-glyph box is not a measurement");

    // The fonts the file does not embed have no stem of their own: PDFium
    // measures a stand-in from this computer — a Thin (20) for page 1's two
    // non-embedded `Montserrat-Thin` objects, which would have told the detector
    // that a Light line was a different weight from its neighbours, and Arial's
    // 95 for its ten non-embedded `Helvetica` ones.
    for (object, what, was) in [(1148usize, "a non-embedded Montserrat-Thin", 20u16), (1131, "a non-embedded Helvetica", 95)] {
        let stem = style_of(&styles, object).stem_milli_em;
        println!("page 1 object {object} ({what}): stem {stem:?} (it read {was} before)");
        assert_eq!(stem, None, "object {object}, {what}, has a stem of its own");
    }

    // ---- pages 2 and 3: the heading and the body, found by their words.
    for (page, heading_words, body_words) in [(1usize, "Photometr", "Light output ratio"), (2, "Photometr", "Fenix series")] {
        let styles = doc.run_styles(page).expect("styles");
        let runs = doc.text_runs(page).expect("runs");
        let (ho, bo) = (object_containing(&runs, heading_words), object_containing(&runs, body_words));
        let (heading, body) = (style_of(&styles, ho), style_of(&styles, bo));
        println!(
            "page {}: {heading_words:?} is object {ho} (font {}, stem {:?})   {body_words:?} is object {bo} (font {}, stem {:?})   {} text objects",
            page + 1, heading.font, heading.stem_milli_em, body.font, body.stem_milli_em, styles.len()
        );
        assert_ne!(heading.font, body.font, "page {}: the heading and the body are different fonts", page + 1);
        let (h, b) = (heading.stem_milli_em.expect("heading stem"), body.stem_milli_em.expect("body stem"));
        assert!(h.abs_diff(198) <= 12, "page {}: ExtraBold heading stem {h} should be about 198", page + 1);
        assert!(b.abs_diff(51) <= 12, "page {}: Light body stem {b} should be about 51", page + 1);

        // The Myriad subset on these pages has no `I` and no `l`, only `i`, so
        // its stem comes from the letter with the dot. Measured whole, `i`
        // reads 110 here; the lower half, the stem alone, reads 88 — the way
        // `i`'s lower half equalled `l` in each of the 22 font resources on
        // the three pages that have a usable outline for both letters
        // (counted from a probe of every font's I, l and i outlines).
        let myriad = style_of(&styles, object_containing(&runs, "Size may change"));
        println!("page {}: Myriad (only `i` to measure) stem {:?}", page + 1, myriad.stem_milli_em);
        assert!(
            myriad.stem_milli_em.is_some_and(|s| s.abs_diff(88) <= 2),
            "page {}: Myriad's stem {:?} should be 88, not the dot's 110",
            page + 1, myriad.stem_milli_em
        );
    }

    // ---- every page: one id per font resource, and the turned text.
    //
    // 9, 11 and 9 are the `/Font` names the three content streams use (TT0.., a
    // Type 1 `T1_0`, `FXF..`); checked against the stream itself, the ids and
    // the names were in one-to-one correspondence over every text object — none
    // merged, none split. The turned objects are the dimension labels ("166m",
    // "64m", "120 m"), a quarter turn one way or the other.
    for (page, fonts, turned) in [(0usize, 9u32, 0usize), (1, 11, 4), (2, 9, 3)] {
        let styles = doc.run_styles(page).expect("styles");
        let used: std::collections::BTreeSet<u32> = styles.values().map(|s| s.font).collect();
        assert_eq!(used, (0..fonts).collect(), "page {}: ids should be exactly 0..{fonts}", page + 1);
        let rotated = styles.values().filter(|s| (s.axis.0 - 1.0).abs() > 1e-3 || s.axis.1.abs() > 1e-3).count();
        println!("page {}: {} fonts, {rotated} turned text objects", page + 1, used.len());
        assert_eq!(rotated, turned, "page {}: turned text objects", page + 1);
    }
    let styles = doc.run_styles(1).expect("page 2 styles");
    // Page 2 has the datasheet's one CID font, `FHCJYT+Montserrat-Light`
    // (Identity-H, 2 objects): it has a `/ToUnicode`, so its `I` is found
    // through its own map and its stem is the Light 51 — a composite font is not
    // `None` for being composite. Its non-embedded namesakes are.
    let cid_light = style_of(&styles, 2838).stem_milli_em;
    println!("page 2 object 2838 (CID Montserrat-Light): stem {cid_light:?}");
    assert!(cid_light.is_some_and(|s| s.abs_diff(51) <= 12), "the CID Light's stem {cid_light:?} should be about 51");
    for object in [2839usize, 2845] {
        assert_eq!(style_of(&styles, object).stem_milli_em, None, "object {object}: a non-embedded Montserrat-Light (20 before)");
    }
    let runs = doc.text_runs(1).expect("page 2 runs");
    for (label, expected) in [("166m", (0.0f32, -1.0f32)), ("64m", (0.0, 1.0))] {
        let (x, y) = style_of(&styles, object_containing(&runs, label)).axis;
        println!("page 2 label {label:?}: axis ({x:+.4}, {y:+.4})");
        assert!((x - expected.0).abs() < 1e-4 && (y - expected.1).abs() < 1e-4, "{label}: axis ({x}, {y}) should be {expected:?}");
    }
}

/// What `run_styles` costs a page, beside what `run_font_names` does and beside
/// a bare page open. Not asserted: a timing assertion on a shared machine fails
/// for reasons that are not the code. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test run_styles -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_a_page_costs() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let doc = PdfiumDocument::open_path(DATASHEET, None).expect("open the datasheet");

    fn median(values: impl IntoIterator<Item = f64>) -> f64 {
        let mut v: Vec<f64> = values.into_iter().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }
    let ms = |f: &dyn Fn()| {
        let start = std::time::Instant::now();
        f();
        start.elapsed().as_secs_f64() * 1000.0
    };
    // A bare FPDF_LoadPage + FPDF_ClosePage: the floor under any call that opens
    // the page itself, which `run_font_names` and `run_styles` both do.
    let bindings = pdf_core::document::pdfium_doc::pdfium().expect("pdfium").bindings();
    let document = doc.backend_handle().expect("a PDFium document") as pdfium_render::prelude::FPDF_DOCUMENT;
    for page in 0..3usize {
        let objects = doc.run_styles(page).expect("styles").len();
        let calls: [&dyn Fn(); 3] = [
            &|| {
                std::hint::black_box(doc.run_styles(page).expect("styles"));
            },
            &|| {
                std::hint::black_box(doc.run_font_names(page).expect("names"));
            },
            &|| unsafe {
                let raw = bindings.FPDF_LoadPage(document, page as i32);
                assert!(!raw.is_null(), "page {page} did not load");
                bindings.FPDF_ClosePage(raw);
            },
        ];
        // Warm: the first call of a process pays for PDFium's own caches.
        for call in calls {
            call();
        }
        // Interleaved, in a rotating order, so a drift in machine load lands on
        // all three alike and the paired differences below stay meaningful.
        let mut samples: [Vec<f64>; 3] = Default::default();
        for round in 0..15 {
            for k in 0..3 {
                let which = (round + k) % 3;
                samples[which].push(ms(calls[which]));
            }
        }
        let [styles, names, open] = &samples;
        let paired = |a: &[f64], b: &[f64]| median(a.iter().zip(b).map(|(x, y)| x - y));
        println!(
            "page {}: {objects:>4} text objects   run_styles {:.3} ms   run_font_names {:.3} ms   bare page open {:.3} ms   | run_styles minus bare open {:+.3} ms, minus run_font_names {:+.3} ms   (medians of 15)",
            page + 1,
            median(styles.iter().copied()),
            median(names.iter().copied()),
            median(open.iter().copied()),
            paired(styles, open),
            paired(styles, names),
        );
    }
}
