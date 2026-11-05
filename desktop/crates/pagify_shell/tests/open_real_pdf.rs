//! Opening a CAD set: `DWTC C3, Unit 110 - Arch.Pack.pdf`, six pages, two of
//! them ~720,000 lines. Asking every page for markup to restore loaded each
//! page whole, on the UI thread, before the first frame; a file that cannot
//! hold any is now told apart without opening a page.
//!
//! Skips, with a line saying so, when the file is not on this machine.
//!
//! `cargo test -p pagify_shell --release --test open_real_pdf -- --nocapture`

use pagify_shell::Session;

const SET: &str = r"C:\Users\hsili\Desktop\DWTC C3, Unit 110 - Arch.Pack.pdf";

#[test]
fn a_cad_set_nobody_marked_is_not_asked_page_by_page() {
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let started = std::time::Instant::now();
    let maybe = session.may_hold_markup();
    let told = started.elapsed();

    let started = std::time::Instant::now();
    let mut found = 0;
    for page in 0..session.page_count().expect("pages") {
        found += usize::from(session.restore_markup(page).expect("restore").is_some());
    }
    let asked = started.elapsed();
    eprintln!("told apart in {told:?}; asking every page took {asked:?} and found {found}");

    assert_eq!(found, 0, "the set holds markup after all");
    assert!(!maybe, "a set with no markup was a maybe, so every page would be loaded to find out");
}

/// A render of a page of 720,000 lines that nobody wants any more gives the
/// engine back soon after it is told to — but only once the page has loaded,
/// which no render can be stopped inside. Measured 2026-11-05: a whole render
/// 2.9 s, the first pause 1.2 s in (the load), let go 0.16 s after the stop
/// (the page freed and the lock given back included).
#[test]
fn a_heavy_page_render_told_to_stop_lets_go_at_once() {
    use std::time::{Duration, Instant};
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let started = Instant::now();
    session.render_page_rotated(1, 1.0, pdf_core::Rotation::None).expect("render");
    let full = started.elapsed();

    // Drawn for 100 ms after the first pause, then told to stop.
    let started = Instant::now();
    let (mut first, mut told) = (None::<Instant>, None::<Instant>);
    let stopped = session
        .render_page_rotated_until(1, 1.0, pdf_core::Rotation::None, Some(&mut || {
            let now = Instant::now();
            let since = now - *first.get_or_insert(now);
            if since > Duration::from_millis(100) && told.is_none() {
                told = Some(now);
            }
            told.is_some()
        }))
        .expect("render");
    let (first, told) = (first.expect("never asked"), told.expect("never told to stop"));
    let let_go = told.elapsed();
    eprintln!(
        "whole render {full:?}; first pause after {:?}; let go {let_go:?} after it was told to stop",
        first - started
    );
    assert!(stopped.is_none(), "the render was drawn to the end");
    assert!(let_go < Duration::from_millis(500), "it went on for {let_go:?} after it was told to stop");
}

/// A thumbnail of a page of 720,000 lines drawn by hayro (`pagify_shell::preview`),
/// against PDFium's: about as much ink, in a fraction of the time, and without
/// PDFium's lock.
#[test]
fn a_heavy_page_thumbnail_drawn_by_hayro() {
    use std::time::Instant;
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let started = Instant::now();
    let preview = pagify_shell::preview::Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
    let read = started.elapsed();
    let started = Instant::now();
    let ours = preview.render(1, 160).expect("hayro draws it");
    let hayro = started.elapsed();
    let started = Instant::now();
    let width_pt = session.page_size(1).expect("size").width_pt;
    let theirs = session.render_page(1, 160.0 / width_pt).expect("PDFium draws it");
    let pdfium = started.elapsed();
    let ink = |r: &pagify_shell::PageRaster| r.pixels.chunks_exact(4).filter(|p| p[..3].iter().any(|&c| c < 160)).count();
    eprintln!(
        "read by hayro {read:?}; its thumbnail {hayro:?} ({}x{}, {} inked); PDFium's {pdfium:?} ({}x{}, {} inked)",
        ours.width, ours.height, ink(&ours), theirs.width, theirs.height, ink(&theirs)
    );
    let ratio = ink(&ours) as f64 / ink(&theirs).max(1) as f64;
    assert!((0.5..=2.0).contains(&ratio), "hayro inked {} pixels where PDFium inked {}", ink(&ours), ink(&theirs));
    assert!(hayro * 2 < pdfium, "hayro took {hayro:?} against PDFium's {pdfium:?}");
}

/// The page itself, not its thumbnail: page 2 of the set at 1 px a point drawn
/// by hayro looks like PDFium's picture, in a fraction of the time.
#[test]
fn a_heavy_page_drawn_by_hayro() {
    use pagify_shell::preview::{compare, Preview};
    use std::time::Instant;
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let preview = Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
    let started = Instant::now();
    let ours = preview.render_scaled(1, 1.0).expect("hayro draws it");
    let hayro = started.elapsed();
    let started = Instant::now();
    let theirs = session.render_page(1, 1.0).expect("PDFium draws it");
    let pdfium = started.elapsed();
    let c = compare(&ours, &theirs).expect("the same size");
    eprintln!("page 2 at 1.0 ({}x{}): hayro {hayro:?}, PDFium {pdfium:?}; {c:?}", ours.width, ours.height);
    assert!(!c.differs(), "{c:?}");
    assert!(hayro * 2 < pdfium, "hayro took {hayro:?} against PDFium's {pdfium:?}");
}

/// Pagify writes a highlight without an appearance stream (pdf_core never calls
/// `FPDFAnnot_SetAP`): PDFium draws it from its colour and quads, hayro does
/// not draw it at all, so the page it is on is one for PDFium.
#[test]
fn pagifys_own_highlight_is_one_hayro_may_draw_differently() {
    use pagify_shell::preview::{Difference, Preview};
    use pdf_core::document::{Color, Rect};
    let fixture = format!("{}/../../../rust/pdf_core/fixtures/two-column.pdf", env!("CARGO_MANIFEST_DIR"));
    let session = Session::open(fixture).expect("open");
    let before = Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
    assert_eq!(before.differs_because(0), None);
    session
        .highlight(0, vec![Rect { left: 72.0, top: 72.0, right: 200.0, bottom: 90.0 }], Color { r: 255, g: 230, b: 0, a: 255 })
        .expect("highlight");
    let after = Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
    assert_eq!(after.differs_because(0), Some(Difference::AnnotationWithoutAppearance));
}

/// What the pointer needs to know about a page of 720,000 lines, read in one
/// load of it, says what the four reads said, in a fraction of their time.
#[test]
fn a_heavy_page_is_read_for_the_pointer_in_one_load() {
    if !std::path::Path::new(SET).is_file() {
        eprintln!("skipped: the set is not at {SET}");
        return;
    }
    let session = Session::open(SET).expect("open");
    let page = 1;
    let started = std::time::Instant::now();
    let scale = session.page_scale(page).expect("scale");
    let marks = session.annotations(page).expect("marks");
    let links = session.internal_links(page).expect("links");
    let characters = session.characters(page).expect("characters");
    let apart = started.elapsed();

    let started = std::time::Instant::now();
    let reads = session.page_reads(page).expect("reads");
    let once = started.elapsed();
    eprintln!("four reads {apart:?}, one load {once:?}");

    assert_eq!(reads.scale.expect("scale"), scale);
    assert_eq!(reads.annotations.expect("marks"), marks);
    assert_eq!(reads.links.expect("links"), links);
    let read_once = pagify_shell::reader::Characters::new(reads.characters.expect("characters"));
    assert_eq!(read_once.text(), characters.text());
    assert!(once * 2 < apart, "one load took {once:?} against {apart:?} for four");
}
