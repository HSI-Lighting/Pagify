//! Applying an edited paragraph to a page: a batch of retyped lines written in
//! one transaction.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test paragraph_apply -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **Every edit here is made to a copy of the datasheet**, written to the
//! temporary directory and removed afterwards. The original is only ever read.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::path::PathBuf;
use std::time::Instant;

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, TextRun, TextStyle};

/// The datasheet this was built against: three A4 pages from Illustrator. Not in
/// the repository (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

/// The paragraph under page 1's "The Light Source - COB" heading: objects
/// 985..=1043, less 1026 and 1035, which Illustrator converted to outlines. 57
/// text objects.
fn paragraph() -> Vec<usize> {
    (985..=1043).filter(|o| *o != 1026 && *o != 1035).collect()
}

/// The 33 of those 57 whose own show-text operator sits exactly where the object
/// does. The other 24 are pieces that continue a line (`(Th) Tj (e COB in ) Tj`):
/// the operator that draws one has no position of its own. Until the edits found
/// an operator by count as well as by position, none of those could be edited and
/// asking for all 57 was refused ("that text is drawn in a way this cannot edit"),
/// so this is the subset the batch could apply. It stays, because the page that
/// results from it is a fixed point — the fingerprints printed below were
/// recorded before the count was used and have not moved — and a change in how
/// runs are found is proved harmless to batches that were already resolvable by
/// their not moving. All 57 are applied in `tests/continuation_edit.rs`.
fn addressable() -> Vec<usize> {
    vec![
        985, 987, 988, 989, 991, 993, 995, 997, 999, 1001, 1003, 1005, 1007, 1008, 1010, 1012, 1014, 1016, 1018,
        1019, 1021, 1023, 1025, 1027, 1028, 1030, 1031, 1033, 1036, 1037, 1039, 1041, 1042,
    ]
}

/// A copy of the datasheet this test is free to edit, removed when dropped.
struct Copy(PathBuf);

impl Copy {
    fn of_the_datasheet(tag: &str) -> Option<Self> {
        skip_without_pdfium()?;
        if !std::path::Path::new(DATASHEET).exists() {
            eprintln!("skipped: the real datasheet is not at {DATASHEET}");
            return None;
        }
        let path = std::env::temp_dir().join(format!("pagify-paragraph-apply-{}-{tag}.pdf", std::process::id()));
        std::fs::copy(DATASHEET, &path).expect("copy the datasheet");
        Some(Copy(path))
    }

    fn open(&self) -> PdfiumDocument {
        PdfiumDocument::open_path(self.0.to_str().expect("path"), None).expect("open the copy")
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// Every text object of page 1 as the page says it now: words, box, origin,
/// size and colour. What a half-applied batch could not leave alone.
fn page_one_runs(doc: &PdfiumDocument) -> Vec<TextRun> {
    doc.text_runs_unfiltered(0).expect("runs")
}

/// Page 1's decoded content stream, as the document would write it — the thing
/// an apply edits, so two applies that leave identical bytes here have done the
/// same thing to the page. (Not the whole saved file: a saved copy carries
/// whatever the writer stamps on it.)
fn page_one_content(doc: &mut PdfiumDocument) -> Vec<u8> {
    use pdf_core::pdf::{content, Object};
    let mut bytes = Vec::new();
    DocumentMut::save_full_copy(doc, &mut bytes).expect("save");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file.resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages")).expect("tree");
    fn first_page(file: &pdf_core::pdf::File<'_>, node: &Object) -> Option<Object> {
        let dict = node.as_dict()?;
        if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
            return Some(node.clone());
        }
        if let Ok(Object::Array(kids)) = file.resolve(dict.get(b"Kids")?) {
            for kid in kids {
                if let Some(page) = file.resolve(&kid).ok().and_then(|kid| first_page(file, &kid)) {
                    return Some(page);
                }
            }
        }
        None
    }
    let page = first_page(&file, &pages).expect("page 1");
    let contents = page.as_dict().and_then(|d| d.get(b"Contents")).expect("contents");
    let mut stream = Vec::new();
    match file.resolve(contents).expect("contents") {
        Object::Stream(d, r) => stream.extend_from_slice(&content::decode(&d, &bytes[r]).expect("decode")),
        Object::Array(parts) => {
            for part in parts {
                if let Ok(Object::Stream(d, r)) = file.resolve(&part) {
                    stream.extend_from_slice(&content::decode(&d, &bytes[r]).expect("decode"));
                    stream.push(b'\n');
                }
            }
        }
        _ => panic!("odd contents"),
    }
    stream
}

/// The words the page has for each of these objects, in order.
fn words_of(runs: &[TextRun], objects: &[usize]) -> Vec<String> {
    objects
        .iter()
        .map(|o| runs.iter().find(|r| r.object == *o).unwrap_or_else(|| panic!("object {o} is gone")).text.clone())
        .collect()
}

/// Words as the editor compares them. PDFium reports a hyphen drawn at a line's
/// end as U+0002 (the app turns it into "-" before it ever reaches an editor,
/// and a font has no glyph for the marker), and adds a space of its own to what
/// it reads back, which an edit drops again where the font has no glyph for it.
fn clean(words: &str) -> String {
    words.replace('\u{2}', "-").trim_end().to_string()
}

/// The edits an apply sends: each line one character shorter — a real change
/// that needs no glyph the font does not already have, so it takes the batch's
/// fast path and not the embedding one — or, for a line already shortened, back
/// to what it first said. So any number of rounds stay real changes and the
/// text never wears away.
fn next_edits(objects: &[usize], original: &[String], now: &[String]) -> Vec<(usize, String, TextStyle)> {
    objects
        .iter()
        .zip(original.iter().zip(now))
        .map(|(object, (was, is))| {
            let was = clean(was);
            let text = if clean(is) == was {
                // Never down to a lone letter: an `l` in a light face at 8 pt is
                // 0.4 pt wide, under the 0.5 pt of ink `text_runs` and
                // `text_run_at` need before they call something a run, and the
                // line would stop being one.
                let mut chars: Vec<char> = was.chars().collect();
                if chars.len() > 2 {
                    chars.pop();
                }
                chars.into_iter().collect()
            } else {
                was
            };
            (*object, text, TextStyle::default())
        })
        .collect()
}

fn timing_line(doc: &mut PdfiumDocument) -> String {
    doc.take_last_batch_timing()
        .iter()
        .map(|(what, took)| format!("{} {:.1} ms", what.trim(), took.as_secs_f64() * 1000.0))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// **Applying the paragraph writes every line, and says what each one was.**
#[test]
fn a_paragraph_apply_writes_every_line_and_reports_what_each_was() {
    let Some(copy) = Copy::of_the_datasheet("writes") else { return };
    let _lock = serial();
    let mut doc = copy.open();
    let objects = addressable();

    let original = words_of(&page_one_runs(&doc), &objects);
    let edits = next_edits(&objects, &original, &original);
    assert!(edits.iter().zip(&original).any(|((_, new, _), old)| clean(new) != clean(old)), "the edits change something");
    let start = Instant::now();
    let results = doc.set_text_runs_styled(0, &edits).expect("apply the paragraph");
    let took = start.elapsed();

    assert_eq!(results.len(), objects.len());
    for ((object, previous), (got, _)) in objects.iter().zip(&original).zip(&results) {
        assert_eq!(got, previous, "object {object}: the words it reports having replaced");
    }
    let after = words_of(&page_one_runs(&doc), &objects);
    for ((object, want, _), got) in edits.iter().map(|e| (&e.0, &e.1, &e.2)).zip(&after) {
        assert_eq!(clean(got), clean(want), "object {object}: the words the page now has");
    }
    let gone: Vec<(usize, String)> = objects
        .iter()
        .zip(&after)
        .filter(|(o, _)| doc.text_run_at(0, **o).expect("text_run_at").is_none())
        .map(|(o, w)| (*o, w.clone()))
        .collect();
    assert!(gone.is_empty(), "lines that stopped being text runs after the apply: {gone:?}");

    // And back again: applying the first words restores the first page.
    let restore = next_edits(&objects, &original, &after);
    doc.set_text_runs_styled(0, &restore).expect("restore the paragraph");
    let restored = words_of(&page_one_runs(&doc), &objects);
    for ((object, was), is) in objects.iter().zip(&original).zip(&restored) {
        assert_eq!(clean(is), clean(was), "object {object}: not restored");
    }

    let content = page_one_content(&mut doc);
    println!(
        "{} lines applied in {:.0} ms; every line reads back as written, and again as restored",
        objects.len(),
        took.as_secs_f64() * 1000.0
    );
    println!("batch timing (the restore): {}", timing_line(&mut doc));
    println!("page 1 content after apply and restore: {} bytes, fingerprint {:016x}", content.len(), fnv(&content));
}

/// A fingerprint of the page after one particular apply, for saying two builds
/// of the engine write the same page: every run's words and geometry, and the
/// content stream. Printed, so two runs can be compared; asserted equal to
/// itself so a run that is not repeatable says so.
#[test]
fn an_apply_leaves_a_repeatable_page() {
    let Some(copy) = Copy::of_the_datasheet("repeat") else { return };
    let _lock = serial();
    let objects = addressable();

    let mut seen = Vec::new();
    for _ in 0..2 {
        let mut doc = copy.open();
        let original = words_of(&page_one_runs(&doc), &objects);
        let edits = next_edits(&objects, &original, &original);
        doc.set_text_runs_styled(0, &edits).expect("apply the paragraph");
        let runs = format!("{:?}", page_one_runs(&doc));
        let content = page_one_content(&mut doc);
        seen.push((fnv(runs.as_bytes()), fnv(&content), content.len()));
    }
    println!(
        "after the apply: runs fingerprint {:016x}, page 1 content {} bytes, fingerprint {:016x}",
        seen[0].0, seen[0].2, seen[0].1
    );
    assert_eq!(seen[0], seen[1], "the same apply to the same copy twice gave two different pages");
}

/// **A batch with one bad request changes nothing.**
///
/// The good lines come first and the last request names an object that is not
/// text at all; nothing may have been written when the error comes back, or the
/// paragraph would be left half changed with its undo record spent.
#[test]
fn a_paragraph_apply_with_one_bad_request_changes_nothing() {
    let Some(copy) = Copy::of_the_datasheet("atomic") else { return };
    let _lock = serial();
    let mut doc = copy.open();
    let objects = addressable();

    let runs_before = page_one_runs(&doc);
    let content_before = page_one_content(&mut doc);
    let original = words_of(&runs_before, &objects);
    let mut edits = next_edits(&objects, &original, &original);
    edits.push((1026, "anything".to_string(), TextStyle::default())); // an outline, not a text run

    let error = doc.set_text_runs_styled(0, &edits).err().expect("the batch must fail");
    println!("refused: {error}");

    assert!(page_one_runs(&doc) == runs_before, "a refused batch changed a run on the page");
    assert!(page_one_content(&mut doc) == content_before, "a refused batch changed the page's content stream");
}

/// A one-page PDF drawing `content`, with Helvetica as `/F1`.
fn page_drawing(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
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
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes(),
    );
    out
}

/// **A batch can retype the pieces of a line that continue one another**, however
/// many of them it names.
///
/// `(Alpha one) Tj (alpha two) Tj` draws two objects, and the second has no
/// position of its own in the stream: the operator that draws it reports the
/// line's origin, a long way from the object, so the lookup by position finds
/// nothing and the batch finds it by counting instead — which needs a read of
/// every run on the page. That read is the same for every piece, so a batch
/// makes it once.
#[test]
fn a_batch_can_retype_pieces_that_continue_a_line() {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 12 Tf 72 700 Td (Alpha one) Tj (alpha two) Tj ET \n\
                   BT /F1 12 Tf 72 680 Td (Bravo one) Tj (bravo two) Tj ET \n\
                   BT /F1 12 Tf 72 660 Td (Charlie one) Tj (charlie two) Tj ET \n\
                   BT /F1 12 Tf 72 640 Td (Delta one) Tj (delta two) Tj ET";
    let mut doc = PdfiumDocument::open_bytes(page_drawing(content), None).expect("open");
    let words = |doc: &PdfiumDocument| -> Vec<String> {
        doc.text_runs_unfiltered(0).expect("runs").iter().map(|r| r.text.trim_end().to_string()).collect()
    };
    assert_eq!(words(&doc).len(), 8, "eight text objects, two to a line");

    // Each line's second piece, one character shorter: found by counting.
    let edits = vec![
        (1, "alpha tw".to_string(), TextStyle::default()),
        (3, "bravo tw".to_string(), TextStyle::default()),
        (5, "charlie tw".to_string(), TextStyle::default()),
        (7, "delta tw".to_string(), TextStyle::default()),
    ];
    let results = doc.set_text_runs_styled(0, &edits).expect("apply the pieces");
    let replaced: Vec<String> = results.iter().map(|(was, _)| was.trim_end().to_string()).collect();
    assert_eq!(replaced, ["alpha two", "bravo two", "charlie two", "delta two"]);
    assert_eq!(
        words(&doc),
        ["Alpha one", "alpha tw", "Bravo one", "bravo tw", "Charlie one", "charlie tw", "Delta one", "delta tw"],
        "each second piece changed and each first piece did not"
    );
}

/// What one apply costs and where it goes: the batch's own timing breakdown
/// (`take_last_batch_timing`), as the app logs it, for batches of growing size
/// so the cost of a batch can be told from the cost of a line. Also prints what
/// asking for the whole paragraph does. Not asserted. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test paragraph_apply -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_a_paragraph_apply_costs() {
    let Some(copy) = Copy::of_the_datasheet("cost") else { return };
    let _lock = serial();
    let mut doc = copy.open();

    // The whole paragraph, as the app's 57 objects: said, not asserted.
    {
        let objects = paragraph();
        let original = words_of(&page_one_runs(&doc), &objects);
        match doc.set_text_runs_styled(0, &next_edits(&objects, &original, &original)) {
            Ok(done) => println!("all {} objects of the paragraph: applied ({} results)", objects.len(), done.len()),
            Err(error) => println!("all {} objects of the paragraph: refused, nothing written: {error}", objects.len()),
        }
    }

    // Growing batches of the lines that can be found; every batch is a real
    // change, alternating between shortened and restored.
    let all = addressable();
    let original = words_of(&page_one_runs(&doc), &all);
    for size in [2usize, 8, 16, 33, 33, 33, 33] {
        let objects = &all[..size];
        let now = words_of(&page_one_runs(&doc), objects);
        let edits = next_edits(objects, &original[..size], &now);
        let start = Instant::now();
        doc.set_text_runs_styled(0, &edits).expect("apply the paragraph");
        let took = start.elapsed().as_secs_f64() * 1000.0;
        println!("{size:>2} lines in {took:7.1} ms ({:5.1} ms a line)   {}", took / size as f64, timing_line(&mut doc));
    }
}
