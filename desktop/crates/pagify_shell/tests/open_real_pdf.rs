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
