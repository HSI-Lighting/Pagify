//! `Session::page_text_snapshot`: one lock, one page, everything the paragraph
//! detector reads about its text.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --release --test page_text_snapshot -- --nocapture
//! ```
//!
//! `--nocapture` is the point of the flag, not a convenience: a test here that
//! cannot find PDFium or the real datasheet prints why and returns, and still
//! counts as passed. Look for the numbers each test prints.

use std::collections::HashSet;
use std::path::PathBuf;

use pagify_shell::session::PageTextSnapshot;
use pagify_shell::Session;
use pdf_core::document::{DrawnKind, DrawnObject, Rect, TextRun};

/// The datasheet this was built against: three A4 pages from Illustrator. Not in
/// the repository (42 MB), so a machine without it skips.
const DATASHEET: &str = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";

fn have_pdfium() -> bool {
    if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
        eprintln!("skipped: set PAGIFY_PDFIUM_LIB to a desktop PDFium to run this");
        return false;
    }
    true
}

/// A PDF made of these numbered objects (the first is object 1, and must be the
/// catalogue), with a classic cross-reference table.
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

/// A one-page US Letter PDF drawing `content`, with Helvetica-Bold as `/F1`,
/// Helvetica as `/F2` and a graphics state `/GS0` that draws at alpha 0.
fn page_drawing(content: &str) -> Vec<u8> {
    page_drawing_of_height(content, 792)
}

/// [`page_drawing`] on a page `height` points tall (a page of tens of thousands
/// of words is a very tall page).
fn page_drawing_of_height(content: &str, height: usize) -> Vec<u8> {
    pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 {height}] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R /F2 6 0 R >> /ExtGState << /GS0 7 0 R >> >> >>"
        ),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /ExtGState /ca 0 >>".to_string(),
    ])
}

/// A file that removes itself, named per test so two running at once cannot
/// collide.
struct TempPdf(PathBuf);

impl TempPdf {
    fn new(name: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("pagify-snapshot-{}-{name}.pdf", std::process::id()));
        std::fs::write(&path, bytes).expect("write the synthetic pdf");
        TempPdf(path)
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn near(rect: &Rect, left: f32, top: f32, right: f32, bottom: f32) -> bool {
    [(rect.left, left), (rect.top, top), (rect.right, right), (rect.bottom, bottom)]
        .iter()
        .all(|(got, want)| (got - want).abs() < 0.05)
}

/// What the snapshot must agree with, got by other routes: the page's own
/// drawing list tells which objects are text and which are shapes, and
/// `text_runs_some` reads their words with no filter at all.
struct Oracle {
    word_objects: Vec<usize>,
    runs: Vec<TextRun>,
    shapes: Vec<DrawnObject>,
}

fn oracle(session: &Session, page: usize) -> Oracle {
    let drawn = session.drawn_objects(page).expect("drawn objects");
    // A text object inside a form is numbered inside the form and is not a page
    // object, which is all a snapshot (or `text_runs`) walks.
    let word_objects: Vec<usize> =
        drawn.iter().filter(|d| d.kind == DrawnKind::Words && d.depth == 0).map(|d| d.object).collect();
    let wanted: HashSet<usize> = word_objects.iter().copied().collect();
    let runs = session.text_runs_some(page, &wanted).expect("text_runs_some");
    let shapes = drawn.into_iter().filter(|d| d.kind == DrawnKind::Shape).collect();
    Oracle { word_objects, runs, shapes }
}

/// Where two lists first differ, said briefly: a failing page has thousands of
/// entries and a message of all of them says nothing.
fn first_difference<T: PartialEq + std::fmt::Debug>(got: &[T], want: &[T]) -> Option<String> {
    let at = got.iter().zip(want).position(|(g, w)| g != w);
    match at {
        Some(i) => Some(format!("entry {i}: got {:?}, want {:?}", got[i], want[i])),
        None if got.len() != want.len() => {
            Some(format!("got {} entries, want {} (the first {} agree)", got.len(), want.len(), got.len().min(want.len())))
        }
        None => None,
    }
}

/// The claims that hold on any page: what the snapshot holds is exactly what the
/// four calls it is made of say, with the same filtering, in the same order.
fn assert_agrees_with_the_parts(session: &Session, page: usize, snap: &PageTextSnapshot) -> Oracle {
    let oracle = oracle(session, page);

    // Every text object, none dropped and none added, in page order — whose
    // words, boxes, origins, sizes and colours are what `text_runs_some` reads.
    let objects: Vec<usize> = snap.runs.iter().map(|r| r.object).collect();
    if let Some(why) = first_difference(&objects, &oracle.word_objects) {
        panic!("page {}: the snapshot's runs are not the page's text objects: {why}", page + 1);
    }
    if let Some(why) = first_difference(&snap.runs, &oracle.runs) {
        panic!("page {}: the snapshot's runs differ from text_runs_some's: {why}", page + 1);
    }

    // Every run has a style, and no style is for anything else.
    assert!(
        snap.runs.iter().all(|r| snap.styles.contains_key(&r.object)),
        "page {}: a run has no style",
        page + 1
    );
    assert_eq!(snap.styles.len(), snap.runs.len(), "page {}: styles for objects that are not runs", page + 1);
    let styles = session.run_styles(page).expect("run_styles");
    assert!(snap.styles == styles, "page {}: styles differ from run_styles' ({} vs {})", page + 1, snap.styles.len(), styles.len());
    let faces = session.run_font_names(page).expect("run_font_names");
    assert!(snap.faces == faces, "page {}: faces differ from run_font_names' ({} vs {})", page + 1, snap.faces.len(), faces.len());

    // The shapes, and only the shapes, with the drawing list's own entries.
    assert!(snap.shapes.iter().all(|s| s.kind == DrawnKind::Shape), "page {}: a non-shape among the shapes", page + 1);
    if let Some(why) = first_difference(&snap.shapes, &oracle.shapes) {
        panic!("page {}: shapes differ from drawn_objects' shapes: {why}", page + 1);
    }
    oracle
}

/// **The snapshot holds every text object — the blank one, the one with no ink
/// and the one drawn at alpha 0 included — with the effective size, plus the
/// page's rules and its word-sized paths.**
#[test]
fn a_synthetic_page_is_snapshotted_whole() {
    if !have_pdfium() {
        return;
    }
    let content = "\
        BT /F1 14 Tf 72 720 Td (Heading words) Tj ET \n\
        BT /F2 9 Tf 72 700 Td (Body line one) Tj ET \n\
        BT /F2 9 Tf 72 690 Td ( ) Tj ET \n\
        q /GS0 gs BT /F2 9 Tf 72 680 Td (ghost) Tj ET Q \n\
        BT /F2 1 Tf 9 0 0 9 72 670 Tm (scaled) Tj ET \n\
        72 600 468 1 re f \n\
        300 500 m 340 500 l 340 508 l 300 508 l h f";
    let file = TempPdf::new("whole", &page_drawing(content));
    let session = Session::open(&file.0).expect("open");

    // What the page weighs, counted before anything is read: five text objects
    // (the blank, the invisible and the scaled one included) and seven objects in
    // all, the rule and the word-sized path among them.
    let scale = session.page_scale(0).expect("page_scale");
    println!("page_scale {scale:?}");
    assert_eq!((scale.text_objects, scale.page_objects), (5, 7));

    let snap = session.page_text_snapshot(0).expect("snapshot");
    let by_text = |needle: &str| {
        snap.runs
            .iter()
            .find(|r| r.text.contains(needle))
            .unwrap_or_else(|| panic!("no run contains {needle:?}: {:?}", snap.runs.iter().map(|r| &r.text).collect::<Vec<_>>()))
    };
    println!(
        "runs {:?}  styles {}  faces {}  shapes {:?}",
        snap.runs.iter().map(|r| (r.object, r.text.as_str(), r.color.a)).collect::<Vec<_>>(),
        snap.styles.len(),
        snap.faces.len(),
        snap.shapes.iter().map(|s| (s.object, s.rect)).collect::<Vec<_>>()
    );

    assert_agrees_with_the_parts(&session, 0, &snap);

    // Five text objects, none filtered. `text_runs` drops the blank one, which
    // has no ink area — so the snapshot is longer than it.
    assert_eq!(snap.runs.len(), 5, "five text objects on the page");
    let filtered = session.text_runs(0).expect("text_runs");
    println!("text_runs keeps {} of the {}", filtered.len(), snap.runs.len());
    assert!(filtered.len() < snap.runs.len(), "text_runs should have dropped the run with no ink area");
    let blank = snap.runs.iter().find(|r| r.text.trim().is_empty()).expect("the blank run is kept");
    assert!(!filtered.iter().any(|r| r.object == blank.object), "the blank run is what text_runs drops");

    // Alpha 0 is kept, and says so.
    assert_eq!(by_text("ghost").color.a, 0, "the invisible run keeps its alpha of 0");
    assert_eq!(by_text("Body").color.a, 255);

    // The effective size, with the size carried by the text matrix folded in:
    // `1 Tf` under a 9x matrix is 9 pt, not 1.
    assert!((by_text("scaled").size - 9.0).abs() < 0.01, "scaled run's size is {}", by_text("scaled").size);
    assert!((by_text("Heading").size - 14.0).abs() < 0.01);

    // Styles: two fonts on the page, numbered in order of first use. Neither is
    // embedded, so neither has a stem (PDFium would measure a stand-in from this
    // computer); the faces below are what tell the bold from the regular.
    let (heading, body) = (snap.styles[&by_text("Heading").object], snap.styles[&by_text("Body").object]);
    assert_eq!((heading.font, body.font), (0, 1), "two fonts, in order of first use");
    assert_eq!((heading.stem_milli_em, body.stem_milli_em), (None, None), "{heading:?} vs {body:?}");
    assert_eq!(snap.faces[&by_text("Heading").object], "Helvetica-Bold");
    assert_eq!(snap.faces[&by_text("Body").object], "Helvetica");

    // Shapes: the rule and the word-sized path, and nothing else.
    assert_eq!(snap.shapes.len(), 2, "one rule and one word-sized path");
    assert!(
        snap.shapes.iter().any(|s| near(&s.rect, 72.0, 191.0, 540.0, 192.0)),
        "the 468 x 1 pt rule is a shape"
    );
    assert!(
        snap.shapes.iter().any(|s| near(&s.rect, 300.0, 284.0, 340.0, 292.0)),
        "the 40 x 8 pt path is a shape"
    );
}

/// **A page with no text objects is not asked for its shapes.**
///
/// An outlined-only page has tens of thousands of them and the snapshot has no
/// use for any: the page here is all paths, `drawn_objects` knows about both,
/// and the snapshot says none.
#[test]
fn a_page_with_no_text_skips_the_shapes() {
    if !have_pdfium() {
        return;
    }
    let content = "72 600 468 1 re f \n300 500 m 340 500 l 340 508 l 300 508 l h f";
    let file = TempPdf::new("outlines", &page_drawing(content));
    let session = Session::open(&file.0).expect("open");

    let drawn_shapes = session
        .drawn_objects(0)
        .expect("drawn")
        .into_iter()
        .filter(|d| d.kind == DrawnKind::Shape)
        .count();
    assert_eq!(drawn_shapes, 2, "the page does draw two paths");

    let snap = session.page_text_snapshot(0).expect("snapshot");
    println!("no text: {} runs, {} shapes (the page draws {drawn_shapes})", snap.runs.len(), snap.shapes.len());
    assert!(snap.runs.is_empty() && snap.styles.is_empty() && snap.faces.is_empty());
    assert!(snap.shapes.is_empty(), "shapes were collected for a page with no text");
}

/// **On the real datasheet the snapshot agrees with its four parts on all three
/// pages, keeps the runs `text_runs` drops, and holds the page's rules and
/// word-sized paths.** Prints what each page's snapshot took, which is the
/// number a caller pays.
#[test]
fn on_the_datasheet_a_snapshot_is_the_whole_page() {
    if !have_pdfium() {
        return;
    }
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let session = Session::open(DATASHEET).expect("open the datasheet");

    // The text objects on each page, as counted when the page was first probed
    // through PDFium's own object list.
    for (page, text_objects) in [(0usize, 883usize), (1, 1488), (2, 1394)] {
        // Counted first, as a caller deciding whether to read the page at all would.
        let start = std::time::Instant::now();
        let scale = session.page_scale(page).expect("page_scale");
        let counted = start.elapsed();
        println!("page {}: page_scale {scale:?} in {:.1} ms", page + 1, counted.as_secs_f64() * 1000.0);
        assert_eq!(scale.text_objects, text_objects, "page {}: text objects counted", page + 1);

        let start = std::time::Instant::now();
        let snap = session.page_text_snapshot(page).expect("snapshot");
        let took = start.elapsed();

        let oracle = assert_agrees_with_the_parts(&session, page, &snap);
        assert_eq!(snap.runs.len(), text_objects, "page {}: text objects", page + 1);
        assert_eq!(scale.text_objects, snap.runs.len(), "page {}: the count is what the snapshot holds", page + 1);
        assert!(scale.page_objects >= snap.runs.len() + snap.shapes.len(), "page {}: fewer objects than the snapshot lists", page + 1);
        assert_eq!(oracle.word_objects.len(), text_objects);

        // What `text_runs` drops for having no ink area is here.
        let filtered = session.text_runs(page).expect("text_runs").len();
        assert!(filtered < snap.runs.len(), "page {}: text_runs kept all {filtered}", page + 1);
        let blank = snap.runs.iter().filter(|r| r.text.trim().is_empty()).count();

        // Rules (a line a page wide is the thinnest shape there is) and the
        // paths a heading or a ligature became.
        let (w, h) = (|s: &DrawnObject| s.rect.right - s.rect.left, |s: &DrawnObject| (s.rect.bottom - s.rect.top).abs());
        let rules = snap.shapes.iter().filter(|s| h(s) <= 2.0 && w(s) >= 100.0).count();
        let words = snap.shapes.iter().filter(|s| (3.0..=80.0).contains(&w(s)) && (4.0..=14.0).contains(&h(s))).count();
        println!(
            "page {}: {} runs ({} that text_runs keeps, {blank} blank), {} styles, {} faces, {} shapes ({rules} rules, {words} word-sized) in {:.1} ms",
            page + 1,
            snap.runs.len(),
            filtered,
            snap.styles.len(),
            snap.faces.len(),
            snap.shapes.len(),
            took.as_secs_f64() * 1000.0
        );
        assert!(rules >= 20, "page {}: only {rules} rules among {} shapes", page + 1, snap.shapes.len());
        assert!(words >= 16, "page {}: only {words} word-sized paths among {} shapes", page + 1, snap.shapes.len());
    }
}

/// What a snapshot costs a page. Not asserted: a timing assertion on a shared
/// machine fails for reasons that are not the code. Run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --release --test page_text_snapshot -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_a_snapshot_costs() {
    if !have_pdfium() {
        return;
    }
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the real datasheet is not at {DATASHEET}");
        return;
    }
    let session = Session::open(DATASHEET).expect("open the datasheet");
    fn median_ms(mut f: impl FnMut()) -> f64 {
        f(); // warm: the first call of a process pays for PDFium's own caches
        let mut samples: Vec<f64> = (0..7)
            .map(|_| {
                let start = std::time::Instant::now();
                f();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        samples[3]
    }
    for page in 0..3usize {
        let snapshot = median_ms(|| {
            std::hint::black_box(session.page_text_snapshot(page).expect("snapshot"));
        });
        // The four reads a snapshot is made of, each on its own. `text_runs`
        // stands in for the unfiltered run list, which costs the same.
        let runs = median_ms(|| {
            std::hint::black_box(session.text_runs(page).expect("text_runs"));
        });
        let styles = median_ms(|| {
            std::hint::black_box(session.run_styles(page).expect("run_styles"));
        });
        let faces = median_ms(|| {
            std::hint::black_box(session.run_font_names(page).expect("run_font_names"));
        });
        let drawn = median_ms(|| {
            std::hint::black_box(session.drawn_objects(page).expect("drawn_objects"));
        });
        println!(
            "page {}: snapshot {snapshot:6.1} ms = text runs {runs:5.1} + styles {styles:4.1} + faces {faces:4.1} + drawn objects {drawn:6.1} (sum {:.1})   (medians of 7)",
            page + 1,
            runs + styles + faces + drawn
        );
    }
}

/// What the page's count and its snapshot cost a page of 1,000, 5,000, 10,000 and
/// 40,000 words (24 to a line, a rule under every line), through the session a
/// caller holds. Not asserted; run on demand:
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --release --test page_text_snapshot -- --ignored --nocapture what_a_dense_page
/// ```
///
/// The words are read in one pass from 1,500 text objects up (see
/// `Document::text_runs_unfiltered`); until then, as before.
#[test]
#[ignore = "a measurement, not a check; run with --ignored --nocapture"]
fn what_a_dense_page_snapshot_costs() {
    if !have_pdfium() {
        return;
    }
    for n in [1_000usize, 5_000, 10_000, 40_000] {
        let height = n.div_ceil(24) * 12 + 80;
        let mut content = String::with_capacity(n * 60);
        for i in 0..n {
            let (line, column) = (i / 24, i % 24);
            content.push_str(&format!("BT /F2 9 Tf {} {} Td (w{i:05}) Tj ET\n", 20 + column * 24, height - 40 - line * 12));
            if column == 23 {
                content.push_str(&format!("20 {} 570 0.5 re f\n", height - 42 - line * 12));
            }
        }
        let file = TempPdf::new(&format!("dense-{n}"), &page_drawing_of_height(&content, height));
        let session = Session::open(&file.0).expect("open the dense page");

        let start = std::time::Instant::now();
        let scale = session.page_scale(0).expect("page_scale");
        let counted = start.elapsed().as_secs_f64() * 1000.0;
        let start = std::time::Instant::now();
        let snap = session.page_text_snapshot(0).expect("snapshot");
        let took = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(scale.text_objects, n);
        assert_eq!(snap.runs.len(), n);
        println!(
            "{n:>6} words ({} objects): page_scale {counted:6.1} ms | snapshot {took:8.1} ms ({} runs, {} styles, {} faces, {} shapes)",
            scale.page_objects,
            snap.runs.len(),
            snap.styles.len(),
            snap.faces.len(),
            snap.shapes.len()
        );
    }
}
