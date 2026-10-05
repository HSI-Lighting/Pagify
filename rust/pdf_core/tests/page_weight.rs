//! What a page weighs, and what it costs to read it: [`Document::page_scale`],
//! the one-pass read of a large page's words, and the shapes-only walk.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test page_weight -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **The problem.** Reading the words of a page took time quadratic in its text
//! objects (`FPDFTextObj_GetText` walks every character of the page to find an
//! object's own, and is called once per object), and the list of what a page
//! draws read the words of every text object a second time just to label them:
//! 4,700 objects took 1.5 s, 10,000 took 9 s, 39,000 took five minutes — on the
//! first click on the page, on the interface's thread. `page_scale` says how big
//! a page is for a few milliseconds, so a caller can refuse before reading; and
//! what a large page's read costs is linear now. The second half of this file is
//! the evidence that it is the *same read*.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DrawnKind, PageScale};

const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

// ============================================================ synthetic pages ==

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

/// A page with Helvetica as `/F1`, a one-pixel picture as `/Im1` and a form as
/// `/Fm1` (a word and a rule inside it), drawing `content`, `height` points tall.
fn page_of(content: &str, height: u32) -> Vec<u8> {
    pdf_of_bytes(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 {height}] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R /Fm1 7 0 R >> >> >>"
        )
        .into_bytes(),
        stream_of("", content.as_bytes()),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream_of(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[0x80],
        ),
        stream_of(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 100] /Resources << /Font << /F1 5 0 R >> >>",
            b"BT /F1 10 Tf 5 50 Td (inside) Tj ET 5 20 100 4 re f",
        ),
    ])
}

/// What the hand count below is made of: three words, two paths, a picture and a
/// form.
const COUNTED: &str = "\
    BT /F1 12 Tf 72 700 Td (one) Tj ET \n\
    BT /F1 12 Tf 72 680 Td (two) Tj ET \n\
    BT /F1 12 Tf 72 660 Td (three) Tj ET \n\
    72 600 100 20 re f \n\
    72 580 m 172 580 l S \n\
    q 50 0 0 50 300 600 cm /Im1 Do Q \n\
    q 1 0 0 1 300 500 cm /Fm1 Do Q";

/// `n` words, one text object each, 24 to a line, with a rule under every line:
/// a page of tables and prose at its densest. Its height grows with `n`.
fn dense_page(n: usize) -> Vec<u8> {
    let mut content = String::with_capacity(n * 60);
    let lines = n.div_ceil(24);
    let height = (lines * 12 + 80) as u32;
    for i in 0..n {
        let (line, column) = (i / 24, i % 24);
        let word = format!("w{:05}{}", i, ["", ",", ".", "-"][i % 4]);
        content.push_str(&format!(
            "BT /F1 9 Tf {} {} Td ({word}) Tj ET\n",
            20 + column * 24,
            height as usize - 40 - line * 12
        ));
        if column == 23 {
            content.push_str(&format!("20 {} 570 0.5 re f\n", height as usize - 42 - line * 12));
        }
    }
    page_of(&content, height)
}

// ================================================================ page_scale ==

/// **The count is the page's own, by hand.** Three words, two paths, a picture and
/// a form make seven objects, three of them text — and the word inside the form is
/// not one of the three (it is numbered inside the form), nor is the form's rule
/// one more of seven. A page with nothing on it is `0, 0`.
#[test]
fn a_page_is_counted_by_hand() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = PdfiumDocument::open_bytes(page_of(COUNTED, 792), None).expect("open");
    let scale = doc.page_scale(0).expect("page_scale");
    println!("{scale:?}");
    assert_eq!(scale, PageScale { text_objects: 3, page_objects: 7 });
    // The same page's other reads say the same: three text runs, and seven things
    // drawn at the top level (the form's two are listed beside, one level in).
    assert_eq!(doc.text_runs_unfiltered(0).expect("runs").len(), 3);
    let drawn = doc.drawn_objects(0).expect("drawn");
    assert_eq!(drawn.iter().filter(|d| d.depth == 0).count(), 7);
    assert_eq!(drawn.iter().filter(|d| d.kind == DrawnKind::Words && d.depth == 0).count(), 3);

    let empty = PdfiumDocument::open_bytes(page_of("", 792), None).expect("open");
    assert_eq!(empty.page_scale(0).expect("page_scale"), PageScale::default());

    // A page that is not there is an error, not a zero.
    assert!(doc.page_scale(1).is_err());
}

/// **The count agrees with what the page reads as, on real pages**: the
/// datasheet's three pages have 883, 1,488 and 1,394 text objects.
#[test]
fn the_datasheet_s_pages_are_counted() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let doc = PdfiumDocument::open_path(DATASHEET, None).expect("open");
    for (page, text) in [(0usize, 883usize), (1, 1488), (2, 1394)] {
        let scale = doc.page_scale(page).expect("page_scale");
        let runs = doc.text_runs_unfiltered(page).expect("runs").len();
        println!("page {}: {scale:?}; text_runs_unfiltered {runs}", page + 1);
        assert_eq!(scale.text_objects, text);
        assert_eq!(scale.text_objects, runs, "the count is what a read returns");
        assert!(scale.page_objects >= scale.text_objects);
    }
}

// ======================================================== the same words, read ==

/// The oracle: PDFium's own call, one text object at a time (`text_runs_some` is
/// that and only that), for every text object of the page.
fn words_by_object(doc: &PdfiumDocument, page: usize) -> HashMap<usize, String> {
    let objects: HashSet<usize> = doc.text_runs_unfiltered(page).expect("runs").iter().map(|r| r.object).collect();
    doc.text_runs_some(page, &objects).expect("text_runs_some").into_iter().map(|r| (r.object, r.text)).collect()
}

/// Every page of `doc` (up to `pages`): the words read in one pass equal PDFium's
/// own for every object. Returns (objects compared, objects whose words end in a
/// space PDFium added).
fn same_words(doc: &PdfiumDocument, pages: usize, what: &str) -> (usize, usize) {
    let (mut objects, mut spaced) = (0, 0);
    for page in 0..doc.page_count().min(pages) {
        let by_object = words_by_object(doc, page);
        let by_characters = doc.text_words_by_characters(page).expect("by characters");
        assert_eq!(by_characters.len(), by_object.len(), "{what} page {}: not the same objects", page + 1);
        for (object, words) in &by_object {
            let mine = by_characters.get(object).unwrap_or_else(|| panic!("{what} page {}: object {object} missing", page + 1));
            assert_eq!(mine, words, "{what} page {}, object {object}: one pass read {mine:?}, PDFium's own call {words:?}", page + 1);
            objects += 1;
            if words.ends_with(' ') {
                spaced += 1;
            }
        }
    }
    (objects, spaced)
}

/// **One pass reads the datasheet's words exactly as PDFium's own call does** —
/// 3,765 text objects on its three pages, 1,035 of them ending in a space (their
/// own or the one PDFium adds), hyphens reported as U+0002, one pass per page
/// against one call per object.
#[test]
fn one_pass_reads_the_datasheet_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let doc = PdfiumDocument::open_path(DATASHEET, None).expect("open");
    let (objects, spaced) = same_words(&doc, 3, "the datasheet");
    println!("the datasheet: {objects} text objects read the same, {spaced} of them with PDFium's trailing space");
    assert_eq!(objects, 883 + 1488 + 1394);
    assert!(spaced > 400, "the trailing space rule was not exercised");
}

/// **And every committed fixture's**: the repository's own PDFs, every page, none
/// of them a page of prose — a two-column page, mixed sizes, pictures, forms, text
/// in a form, text drawn in a clip, an encrypted page left out.
#[test]
fn one_pass_reads_every_fixture_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut total = 0;
    let mut files = 0;
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures").filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for path in names {
        if path.extension().and_then(|e| e.to_str()) != Some("pdf") {
            continue;
        }
        let Ok(doc) = PdfiumDocument::open_path(path.to_str().expect("path"), None) else { continue };
        let (objects, _) = same_words(&doc, 50, &path.display().to_string());
        total += objects;
        files += 1;
    }
    println!("{files} fixtures, {total} text objects read the same");
    assert!(files >= 10 && total > 100);
}

/// **The same check on any documents**: every page (up to `PAGIFY_WORDS_PAGES`, 4
/// by default) of every PDF named in `PAGIFY_WORDS_DOCS` (separated by `;`), one pass
/// against PDFium's own call for every text object. Not part of the regular run —
/// it is the tool for holding the one-pass read to a new kind of document:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> PAGIFY_WORDS_DOCS="a.pdf;b.pdf" \
///   cargo test --release --test page_weight -- --ignored --nocapture one_pass_reads_the_documents_named
/// ```
///
/// The corpus it was written against, run on 2026-10-05: 105 documents — the
/// owner's real quotations, purchase orders, invoices, receipts (one of them
/// Arabic), datasheets and CAD plots, and some synthetic pages — the first four
/// pages of each: 28,692 text objects, **not one read differently**.
#[test]
#[ignore = "a tool: run with PAGIFY_WORDS_DOCS and --ignored --nocapture"]
fn one_pass_reads_the_documents_named_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let list = std::env::var("PAGIFY_WORDS_DOCS").unwrap_or_default();
    let pages: usize = std::env::var("PAGIFY_WORDS_PAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    let (mut documents, mut compared) = (0, 0);
    for path in list.split(';').filter(|p| !p.is_empty()) {
        let Ok(doc) = PdfiumDocument::open_path(path, None) else {
            println!("{path}: does not open");
            continue;
        };
        let (objects, spaced) = same_words(&doc, pages, path);
        println!("{path}: {objects} text objects the same ({spaced} ending in a space)");
        documents += 1;
        compared += objects;
    }
    println!("{documents} documents, {compared} text objects read exactly as PDFium reads them");
    assert!(documents > 0, "no document named in PAGIFY_WORDS_DOCS opened");
}

/// **A page built to be awkward**: the trailing-space rule's edges. Words that
/// touch (no gap, so nothing is added between them), a space drawn by a Tj of its
/// own, a space inside a word's own string, a word that ends in a space, text
/// drawn out of reading order (so the character after a word is another line's),
/// an empty string, a hyphen, a character outside the Latin range written as two
/// code units, and the same word drawn twice on the same spot.
#[test]
fn one_pass_reads_an_awkward_page_exactly() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let content = "\
        BT /F1 12 Tf 72 700 Td (touch) Tj (ing) Tj ET \n\
        BT /F1 12 Tf 72 680 Td (alone) Tj 40 0 Td ( ) Tj 5 0 Td (after) Tj ET \n\
        BT /F1 12 Tf 72 660 Td (two words) Tj 100 0 Td (and a space ) Tj ET \n\
        BT /F1 12 Tf 72 640 Td (far) Tj ET \n\
        BT /F1 12 Tf 300 640 Td (right) Tj ET \n\
        BT /F1 12 Tf 72 620 Td () Tj ET \n\
        BT /F1 12 Tf 72 600 Td (hyphen-) Tj ET \n\
        BT /F1 12 Tf 72 580 Td (ation) Tj ET \n\
        BT /F1 12 Tf 72 560 Td (twice) Tj ET \n\
        BT /F1 12 Tf 72 560 Td (twice) Tj ET \n\
        BT /F1 12 Tf 72 540 Td (\\344\\366\\374) Tj ET \n\
        BT /F1 12 Tf 72 520 Td (\\000nul) Tj ET \n\
        BT /F1 12 Tf 72 510 Td (\\000) Tj ET \n\
        BT /F1 12 Tf 72 500 Td (last) Tj ET";
    let doc = PdfiumDocument::open_bytes(page_of(content, 792), None).expect("open");
    let (objects, spaced) = same_words(&doc, 1, "the awkward page");
    println!("{objects} text objects, {spaced} of them end in a space PDFium added");
    assert!(objects >= 16, "the page was meant to hold seventeen text objects, it holds {objects}");
}

/// **A large page goes through the one-pass read on its own and the words are
/// PDFium's**: above [`LINEAR_TEXT_READ_FROM`] text objects `text_runs_unfiltered`
/// does not call PDFium per object, and this holds the list it returns to the
/// per-object oracle, object by object, on pages of 2,000 and 6,000 words.
#[test]
fn a_large_page_reads_the_same_words_through_text_runs() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    for n in [2_000usize, 6_000] {
        let doc = PdfiumDocument::open_bytes(dense_page(n), None).expect("open");
        assert_eq!(doc.page_scale(0).expect("scale").text_objects, n);
        let runs = doc.text_runs_unfiltered(0).expect("runs");
        assert_eq!(runs.len(), n);
        let oracle = words_by_object(&doc, 0);
        let wrong: Vec<_> = runs.iter().filter(|r| oracle.get(&r.object) != Some(&r.text)).take(3).collect();
        assert!(wrong.is_empty(), "{n} words: text_runs_unfiltered differs from PDFium's own call: {wrong:?}");
        assert!(runs.iter().filter(|r| r.text.ends_with(' ')).count() > n / 2, "{n} words: the space rule was not exercised");
        println!("{n} words: text_runs_unfiltered = PDFium's own call on every run");

        // The list of what the page draws labels each word with its words; a large
        // page has those read in one pass as well, and they are PDFium's.
        let label_of = |words: &str| {
            let words = words.trim();
            let short: String = words.chars().take(40).collect();
            if words.chars().count() > 40 {
                format!("{short}…")
            } else if short.is_empty() {
                "(blank)".to_string()
            } else {
                short
            }
        };
        let drawn = doc.drawn_objects(0).expect("drawn");
        let words: Vec<_> = drawn.iter().filter(|d| d.kind == DrawnKind::Words).collect();
        assert_eq!(words.len(), n);
        let wrong: Vec<_> = words.iter().filter(|d| Some(label_of(oracle.get(&d.object).map_or("", |s| s.as_str()))) != Some(d.label.clone())).take(3).collect();
        assert!(wrong.is_empty(), "{n} words: drawn_objects labels differ from PDFium's words: {wrong:?}");
        println!("{n} words: drawn_objects labels = PDFium's words on every one");
    }
}

// =================================================================== shapes ==

/// **`drawn_shapes` is exactly the shapes of `drawn_objects`**, entry for entry,
/// on a page with everything that can change a path's entry: a form with a path
/// inside, a grey box standing under a picture it backs (the placeholder rule moves
/// it), a picture of its own, text, and a path at the end.
#[test]
fn the_shapes_of_a_page_are_the_shapes_of_its_drawing_list() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let content = "\
        BT /F1 12 Tf 72 700 Td (heading) Tj ET \n\
        0.8 g 300 600 50 50 re f \n\
        q 50 0 0 50 300 600 cm /Im1 Do Q \n\
        0 g 72 600 100 20 re f \n\
        q 1 0 0 1 100 300 cm /Fm1 Do Q \n\
        BT /F1 9 Tf 72 100 Td (footer) Tj ET \n\
        72 80 m 300 80 l S \n\
        q 0.5 0 0 0.5 400 100 cm /Fm1 Do Q";
    let doc = PdfiumDocument::open_bytes(page_of(content, 792), None).expect("open");
    let all = doc.drawn_objects(0).expect("drawn");
    let shapes_of_all: Vec<_> = all.iter().filter(|d| d.kind == DrawnKind::Shape).cloned().collect();
    let shapes = doc.drawn_shapes(0).expect("shapes");
    println!("{} drawn objects, {} shapes: {:?}", all.len(), shapes.len(), shapes.iter().map(|s| (s.object, s.depth, s.label.as_str())).collect::<Vec<_>>());
    assert!(shapes.len() >= 5, "the page was meant to have at least five paths (it has {})", shapes.len());
    assert!(shapes.iter().any(|s| s.label == "placeholder"), "the grey box under the picture was not turned into a placeholder");
    assert!(shapes.iter().any(|s| s.depth > 0 && s.label != "placeholder"), "no path inside a form");
    assert_eq!(shapes, shapes_of_all);
}

/// The same on the datasheet's three pages, where there are 20 or more rules and 16
/// or more word-sized paths on each.
#[test]
fn the_datasheet_s_shapes_are_the_same_either_way() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let doc = PdfiumDocument::open_path(DATASHEET, None).expect("open");
    for page in 0..3 {
        let all: Vec<_> =
            doc.drawn_objects(page).expect("drawn").into_iter().filter(|d| d.kind == DrawnKind::Shape).collect();
        let shapes = doc.drawn_shapes(page).expect("shapes");
        println!("page {}: {} shapes", page + 1, shapes.len());
        assert!(shapes.len() > 100);
        assert_eq!(shapes, all, "page {}", page + 1);
    }
}

/// And on every committed fixture.
#[test]
fn every_fixture_s_shapes_are_the_same_either_way() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut names: Vec<_> = std::fs::read_dir(&dir).expect("fixtures").filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    let (mut files, mut shapes_seen) = (0, 0);
    for path in names {
        if path.extension().and_then(|e| e.to_str()) != Some("pdf") {
            continue;
        }
        let Ok(doc) = PdfiumDocument::open_path(path.to_str().expect("path"), None) else { continue };
        for page in 0..doc.page_count().min(50) {
            let Ok(all) = doc.drawn_objects(page) else { continue };
            let all: Vec<_> = all.into_iter().filter(|d| d.kind == DrawnKind::Shape).collect();
            let shapes = doc.drawn_shapes(page).expect("shapes");
            assert_eq!(shapes, all, "{} page {}", path.display(), page + 1);
            shapes_seen += shapes.len();
        }
        files += 1;
    }
    println!("{files} fixtures, {shapes_seen} shapes the same either way");
    assert!(files >= 10 && shapes_seen > 50);
}

// ================================================================ the weight ==

/// **A page of 50,000 words is counted and read in time**: a generous bound that a
/// quadratic read cannot meet (the same page took more than eight minutes before).
/// 50,000 text objects, 2,084 rules.
#[test]
fn a_page_of_fifty_thousand_objects_is_counted_and_read_in_time() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let n = 50_000;
    let doc = PdfiumDocument::open_bytes(dense_page(n), None).expect("open");

    let start = Instant::now();
    let scale = doc.page_scale(0).expect("scale");
    let counted = start.elapsed();
    println!("page_scale: {scale:?} in {:.1} ms", counted.as_secs_f64() * 1000.0);
    assert_eq!(scale.text_objects, n);
    // A rule under every full line of 24 words.
    assert_eq!(scale.page_objects, n + n / 24);
    assert!(counted.as_secs_f64() < 5.0, "counting 50,000 objects took {counted:?}");

    let start = Instant::now();
    let runs = doc.text_runs_unfiltered(0).expect("runs");
    let read = start.elapsed();
    println!("text_runs_unfiltered: {} runs in {:.2} s", runs.len(), read.as_secs_f64());
    assert_eq!(runs.len(), n);
    assert!(runs[0].text.starts_with("w00000") && runs[n - 1].text.starts_with("w49999"));
    assert!(read.as_secs_f64() < 30.0, "reading the words of 50,000 objects took {read:?}");

    let start = Instant::now();
    let shapes = doc.drawn_shapes(0).expect("shapes");
    let listed = start.elapsed();
    println!("drawn_shapes: {} shapes in {:.2} s", shapes.len(), listed.as_secs_f64());
    assert_eq!(shapes.len(), n / 24);
    assert!(listed.as_secs_f64() < 30.0, "listing the shapes of 50,000 objects took {listed:?}");
}

/// What counting a page costs, beside a bare page open (the floor under it) and
/// beside a count that does not look at kinds — on the datasheet's pages and on a
/// synthetic page of 40,000 words. Medians of 15, interleaved. Not asserted; run on
/// demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test page_weight -- --ignored --nocapture what_counting_costs
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_counting_costs() {
    use pdfium_render::prelude::PdfiumLibraryBindingsAccessor;
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let bindings = pdf_core::document::pdfium_doc::pdfium().expect("pdfium").bindings();
    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }
    let mut docs: Vec<(String, PdfiumDocument, usize)> = Vec::new();
    if std::path::Path::new(DATASHEET).exists() {
        for page in 0..3 {
            docs.push((format!("datasheet page {}", page + 1), PdfiumDocument::open_path(DATASHEET, None).expect("open"), page));
        }
    }
    docs.push(("40,000 words".to_string(), PdfiumDocument::open_bytes(dense_page(40_000), None).expect("open"), 0));
    for (name, doc, page) in docs {
        let handle = doc.backend_handle().expect("a PDFium document") as pdfium_render::prelude::FPDF_DOCUMENT;
        let open = || {
            let start = Instant::now();
            let raw = unsafe { bindings.FPDF_LoadPage(handle, page as i32) };
            unsafe { bindings.FPDF_ClosePage(raw) };
            start.elapsed().as_secs_f64() * 1000.0
        };
        let count_only = || {
            let start = Instant::now();
            let raw = unsafe { bindings.FPDF_LoadPage(handle, page as i32) };
            let n = unsafe { bindings.FPDFPage_CountObjects(raw) };
            unsafe { bindings.FPDF_ClosePage(raw) };
            std::hint::black_box(n);
            start.elapsed().as_secs_f64() * 1000.0
        };
        let scale = || {
            let start = Instant::now();
            std::hint::black_box(doc.page_scale(page).expect("page_scale"));
            start.elapsed().as_secs_f64() * 1000.0
        };
        for _ in 0..3 {
            open();
            count_only();
            scale();
        }
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..15 {
            a.push(open());
            b.push(count_only());
            c.push(scale());
        }
        println!(
            "{name:<18}: bare page open {:7.2} ms | open + CountObjects {:7.2} ms | page_scale (open + count + a kind per object) {:7.2} ms",
            median(a),
            median(b),
            median(c)
        );
    }
}

/// The cost of each read at 1k, 5k, 10k and 40k text objects, beside what it was.
/// Not asserted (a timing assertion on a shared machine fails for reasons that are
/// not the code); run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test page_weight -- --ignored --nocapture what_a_page_costs
/// ```
///
/// "per object" is PDFium's own call, one object at a time — the read this crate
/// did before and still does for a page of 1,500 text objects or fewer;
/// "one pass" is what a larger page gets now. `drawn_objects` labels every text
/// object with its words (the same quadratic read, again); `drawn_shapes` does not.
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_a_page_costs() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let sizes: Vec<usize> = std::env::var("PAGIFY_SIZES")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1_000, 5_000, 10_000, 40_000]);
    let ms = |start: Instant| start.elapsed().as_secs_f64() * 1000.0;
    // The per-object reads of a page of 40,000 objects take minutes each: left out
    // unless asked for with `PAGIFY_SLOW=1`.
    let slow = std::env::var("PAGIFY_SLOW").is_ok();
    for n in sizes {
        let doc = PdfiumDocument::open_bytes(dense_page(n), None).expect("open");
        let start = Instant::now();
        let scale = doc.page_scale(0).expect("scale");
        let counted = ms(start);

        let start = Instant::now();
        let runs = doc.text_runs_unfiltered(0).expect("runs");
        let now = ms(start);

        // The per-object read of every object: what `text_runs_unfiltered` was.
        let start = Instant::now();
        let objects: HashSet<usize> = runs.iter().map(|r| r.object).collect();
        let before = if n <= 12_000 || slow {
            let by_object = doc.text_runs_some(0, &objects).expect("some");
            assert_eq!(by_object.len(), runs.len());
            Some(ms(start))
        } else {
            None
        };

        let start = Instant::now();
        let shapes = doc.drawn_shapes(0).expect("shapes");
        let shapes_ms = ms(start);
        let start = Instant::now();
        let drawn = if n <= 12_000 || slow { doc.drawn_objects(0).ok().map(|d| (d.len(), ms(start))) } else { None };

        let start = Instant::now();
        let styles = doc.run_styles(0).expect("styles").len();
        let styles_ms = ms(start);

        println!(
            "{n:>6} text objects ({} objects): page_scale {counted:6.1} ms | words: one pass {now:8.1} ms, per object {} | \
             shapes {}: drawn_shapes {shapes_ms:7.1} ms, drawn_objects {} | run_styles {styles} in {styles_ms:.1} ms",
            scale.page_objects,
            before.map(|b| format!("{b:9.1} ms")).unwrap_or_else(|| "     (skipped)".into()),
            shapes.len(),
            drawn.map(|(_, t)| format!("{t:8.1} ms")).unwrap_or_else(|| "(skipped)".into()),
        );
    }
}
