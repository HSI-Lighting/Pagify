//! The words of a text object, read through the page's one text layer rather
//! than a fresh text layer per object.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test text_words -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.
//!
//! **The oracle is the old call.** `PdfPageTextObject::text()` loads the page's
//! text layer afresh for every object it is asked about (and leaves it open when
//! the object has no words), which is the cost this file exists to catch coming
//! back. It is still available here, through `pdfium-render` directly on a second
//! copy of the file, so "the words did not change" is checked against what the
//! engine used to say rather than against itself.

mod harness;
use harness::{serial, skip_without_pdfium};

use std::collections::HashSet;
use std::time::{Duration, Instant};

use pdf_core::document::pdfium_doc::{pdfium, PdfiumDocument};
use pdf_core::document::{Document, TextRun};
use pdfium_render::prelude::*;

/// The datasheet this was built against: three A4 pages from Illustrator. Not in
/// the repository (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

/// The engine's own copy of the datasheet, or `None` (after saying why) on a
/// machine that has no PDFium or no datasheet.
fn datasheet() -> Option<PdfiumDocument> {
    skip_without_pdfium()?;
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return None;
    }
    Some(PdfiumDocument::open_path(DATASHEET, None).expect("open the datasheet"))
}

/// Every text object's words on a page, read **the old way**: through
/// `PdfPageTextObject::text()`, on a second copy of the file open beside the
/// engine's own. Object index first, as everywhere else.
fn words_the_old_way(page: usize) -> Vec<(usize, String)> {
    let document = pdfium()
        .expect("pdfium")
        .load_pdf_from_file(DATASHEET, None)
        .expect("a second copy of the datasheet");
    let page = document.pages().get(i32::try_from(page).expect("page")).expect("page");
    page.objects()
        .iter()
        .enumerate()
        .filter_map(|(index, object)| object.as_text_object().map(|text| (index, text.text())))
        .collect()
}

/// Where a list of runs first stops matching the old call's words, said
/// briefly: a page has over a thousand objects and a message of all of them
/// says nothing.
fn first_mismatch(runs: &[TextRun], oracle: &[(usize, String)]) -> Option<String> {
    if let Some(i) = runs.iter().zip(oracle).position(|(r, (o, w))| r.object != *o || r.text != *w) {
        return Some(format!(
            "entry {i}: object {} says {:?}, the old call says object {} reads {:?}",
            runs[i].object, runs[i].text, oracle[i].0, oracle[i].1
        ));
    }
    (runs.len() != oracle.len())
        .then(|| format!("{} runs against {} text objects (the first {} agree)", runs.len(), oracle.len(), runs.len().min(oracle.len())))
}

/// The text objects a spot check asks `text_run_at` about: every ninth, and the
/// whole of page 1's heading and paragraph (980..=1043), where the paragraph
/// editor spends its time.
fn sampled(page: usize, oracle: &[(usize, String)]) -> Vec<usize> {
    oracle
        .iter()
        .map(|(object, _)| *object)
        .enumerate()
        .filter(|(n, object)| n % 9 == 0 || (page == 0 && (980..=1043).contains(object)))
        .map(|(_, object)| object)
        .collect()
}

/// **The words are byte-for-byte what the old call said, for every text object
/// on every page, through every route the engine reads them by.**
///
/// The old call is the oracle. The routes are the three that read words:
/// `text_runs_unfiltered` (which is `text_runs_all`, and so also what
/// `text_runs` filters), `text_runs_some` and `text_run_at`. Blank objects are
/// part of the claim — a lone space or a ligature drawn without words must still
/// read as it did — so the test counts them and refuses a page that has none.
#[test]
fn the_words_are_what_the_old_call_said_on_every_object_of_every_page() {
    let Some(doc) = datasheet() else { return };
    let _lock = serial();

    for page in 0..3usize {
        let oracle = words_the_old_way(page);
        let blank = oracle.iter().filter(|(_, words)| words.trim().is_empty()).count();
        assert!(blank > 0, "page {}: no blank text object, so the claim about them is vacuous", page + 1);

        // Every text object, none filtered.
        let all = doc.text_runs_unfiltered(page).expect("text_runs_unfiltered");
        if let Some(why) = first_mismatch(&all, &oracle) {
            panic!("page {}: text_runs_unfiltered: {why}", page + 1);
        }

        // The same objects asked for by number.
        let wanted: HashSet<usize> = oracle.iter().map(|(object, _)| *object).collect();
        let some = doc.text_runs_some(page, &wanted).expect("text_runs_some");
        if let Some(why) = first_mismatch(&some, &oracle) {
            panic!("page {}: text_runs_some: {why}", page + 1);
        }

        // `text_runs` is those with ink area, nothing else changed: the same
        // objects the unfiltered list passes through the filter, with the same
        // words.
        let ink = |r: &&TextRun| (r.rect.right - r.rect.left).abs() > 0.5 && (r.rect.top - r.rect.bottom).abs() > 0.5;
        let expected: Vec<&TextRun> = all.iter().filter(ink).collect();
        let filtered = doc.text_runs(page).expect("text_runs");
        assert_eq!(filtered.len(), expected.len(), "page {}: text_runs' filter moved", page + 1);
        for (got, want) in filtered.iter().zip(&expected) {
            assert!(got == *want, "page {}: text_runs object {} differs from the unfiltered list's", page + 1, got.object);
        }

        // One object at a time: present exactly when text_runs has it, and
        // reading what the old call read.
        let in_text_runs: HashSet<usize> = filtered.iter().map(|r| r.object).collect();
        let sample = sampled(page, &oracle);
        for &object in &sample {
            let one = doc.text_run_at(page, object).expect("text_run_at");
            assert_eq!(
                one.is_some(),
                in_text_runs.contains(&object),
                "page {}: text_run_at({object}) and text_runs disagree about whether it is a run",
                page + 1
            );
            if let Some(one) = one {
                let want = &oracle.iter().find(|(o, _)| *o == object).expect("oracle").1;
                assert_eq!(&one.text, want, "page {}: text_run_at({object})", page + 1);
            }
        }
        println!(
            "page {}: {} text objects ({blank} blank), {} kept by text_runs, {} spot-checked through text_run_at: all words identical to the old call",
            page + 1,
            oracle.len(),
            filtered.len(),
            sample.len()
        );
    }
}

/// **`text_run_at` reads the same run the page-wide list does**, not a run with
/// another size.
///
/// It used the bare `Tf` size where `text_runs_all` and `text_runs_some` use the
/// effective one — the text matrix's scale folded in. On this datasheet, whose
/// producer writes `1 Tf` and puts the real size in the matrix, that is a run of
/// size 1 where the paragraph is set at 8 pt; and the editing code divides a
/// requested size by `run.size / found.size`, which it can only do right with the
/// effective size in `run.size` — see `set_run_in_stream`'s `vertical_scale`.
#[test]
fn text_run_at_reads_the_same_run_as_the_page_wide_list() {
    let Some(doc) = datasheet() else { return };
    let _lock = serial();

    let mut checked = 0;
    for page in 0..3usize {
        let all = doc.text_runs_unfiltered(page).expect("text_runs_unfiltered");
        let oracle: Vec<(usize, String)> = all.iter().map(|r| (r.object, r.text.clone())).collect();
        for object in sampled(page, &oracle) {
            let Some(one) = doc.text_run_at(page, object).expect("text_run_at") else { continue };
            let listed = all.iter().find(|r| r.object == object).expect("in the list");
            assert!(
                one == *listed,
                "page {} object {object}: text_run_at says size {} (origin {:?}, rect {:?}) where the page-wide list says size {} (origin {:?}, rect {:?})",
                page + 1, one.size, one.origin, one.rect, listed.size, listed.origin, listed.rect
            );
            checked += 1;
        }
    }
    println!("{checked} runs read through text_run_at are identical, size included, to the page-wide list's");
    assert!(checked > 300, "only {checked} runs were compared");
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

/// **A size asked for lands on that size when the producer put the scale in the
/// text matrix** — `1 Tf` under a 9x matrix, which is how Illustrator writes the
/// whole datasheet.
///
/// The edit divides the size it was asked for by `run.size / found.size`, the
/// stretch the matrix carries, to turn "draw this at 9 pt" back into the number
/// `Tf` needs. `run.size` came from `text_run_at` and was the bare `Tf` size, 1,
/// so the stretch read as 1 and 9 pt was written as `9 Tf` under the 9x matrix:
/// 81 pt on the page.
#[test]
fn a_requested_size_lands_on_that_size_when_the_matrix_carries_the_scale() {
    use pdf_core::document::{DocumentMut, TextStyle};
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 1 Tf 9 0 0 9 72 700 Tm (Hello world) Tj ET";
    let mut doc = PdfiumDocument::open_bytes(page_drawing(content), None).expect("open");
    let size_now = |doc: &PdfiumDocument| {
        let runs = doc.text_runs_unfiltered(0).expect("runs");
        assert_eq!(runs.len(), 1, "one text object");
        runs[0].size
    };
    assert!((size_now(&doc) - 9.0).abs() < 0.01, "the page draws it at 9 pt to begin with");

    for wanted in [9.0f32, 14.0, 6.0] {
        let style = TextStyle { size: Some(wanted), ..Default::default() };
        doc.set_text_run_styled(0, 0, "Hello world", &style).expect("retype at a size");
        let got = size_now(&doc);
        println!("asked for {wanted} pt, the page draws it at {got} pt");
        assert!((got - wanted).abs() < 0.01, "asked for {wanted} pt, the page draws it at {got} pt");
    }
}

/// What reading a page's words costs, each way, on the same machine: the old
/// call per object, the page's one text layer per object, the engine's page-wide
/// list and one object through `text_run_at`. Not asserted. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test text_words -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_the_words_cost() {
    let Some(doc) = datasheet() else { return };
    let _lock = serial();

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }
    fn time_ms(runs: usize, mut f: impl FnMut()) -> f64 {
        median((0..runs).map(|_| { let t = Instant::now(); f(); t.elapsed().as_secs_f64() * 1000.0 }).collect())
    }

    let raw = pdfium().expect("pdfium").load_pdf_from_file(DATASHEET, None).expect("a second copy");
    for page_index in 0..3usize {
        let page = raw.pages().get(i32::try_from(page_index).unwrap()).expect("page");
        let objects = page.objects().iter().filter(|o| o.as_text_object().is_some()).count();
        let old_call = time_ms(3, || {
            for object in page.objects().iter() {
                if let Some(text) = object.as_text_object() {
                    std::hint::black_box(text.text());
                }
            }
        });
        let one_layer = time_ms(5, || {
            let layer = page.text().expect("text layer");
            for object in page.objects().iter() {
                if let Some(text) = object.as_text_object() {
                    std::hint::black_box(layer.for_object(text));
                }
            }
        });
        let _ = doc.text_runs_unfiltered(page_index).expect("warm");
        let runs_all = time_ms(5, || {
            std::hint::black_box(doc.text_runs_unfiltered(page_index).expect("runs"));
        });
        let in_list: Vec<usize> = doc.text_runs(page_index).expect("runs").iter().map(|r| r.object).collect();
        let calls = in_list.iter().step_by((in_list.len() / 30).max(1)).take(30).copied().collect::<Vec<_>>();
        let at = time_ms(1, || {
            for &object in &calls {
                std::hint::black_box(doc.text_run_at(page_index, object).expect("text_run_at"));
            }
        }) / calls.len() as f64;
        println!(
            "page {}: {objects:>4} text objects   words, old call per object {old_call:8.1} ms   one text layer {one_layer:6.1} ms   | text_runs_unfiltered {runs_all:6.1} ms   text_run_at {at:5.2} ms per run (mean of {})",
            page_index + 1,
            calls.len()
        );
    }
}

/// **The words of a whole page take milliseconds, not seconds.**
///
/// Measured on the datasheet before the change: 1.1, 2.6 and 2.6 s for pages 1,
/// 2 and 3 through `text()` (a fresh text layer per object), against 20, 34 and
/// 38 ms through the page's one text layer. The limit is generous on purpose —
/// 25 times what it should take, a quarter of what it did — because a timing
/// assertion on a shared machine fails for reasons that are not the code, and
/// what this must catch is a return to the old call, not a slow afternoon.
#[test]
fn the_words_of_a_whole_page_take_milliseconds_not_seconds() {
    let Some(doc) = datasheet() else { return };
    let _lock = serial();

    let page = 2; // the busiest: 1394 text objects
    let _ = doc.text_runs_unfiltered(page).expect("warm"); // PDFium's own first-call costs
    let best = (0..3)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(doc.text_runs_unfiltered(page).expect("text_runs_unfiltered"));
            start.elapsed()
        })
        .min()
        .expect("three runs");
    println!("page 3: text_runs_unfiltered, best of 3: {:.1} ms", best.as_secs_f64() * 1000.0);
    assert!(
        best < Duration::from_millis(1000),
        "the words of page 3 took {best:?}: the text layer is being reloaded per object again"
    );
}
